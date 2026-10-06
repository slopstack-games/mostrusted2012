"""Replay a recorded car trajectory (ref/<name>/cam.bin, mwtool live recording) through the chase camera's
followed-frame and FOV rules, and report the error against the original's recorded camera.

Usage: python3 tools/camreplay.py ref/live1 [before|after]

The rules mirror crates/mostrusted/src/camera.rs (chase_view): `before` is the model up to 2026-10-06, `after`
the one ported from FUN_005c89c0 / FUN_005c90a0 / FUN_005c76a0 (docs/decomp.md "Cameras"), with the FOV's
acceleration term (cam + 0x6ec), the side-on pull-in (FUN_005c8ad0) and the FOV dolly (FUN_005c1f90). The eye is
compared as cam + 0x5a0..0x5a8 (the followed-frame offset before the z offset cam + 0x588), with the recorded
look-at point cam + 0x5c0.. and this run's data eye (0, 1.575, -4.6); the width / length ratio is a fit
(WL_RATIO: this run's car is not identified). Needs
camload.py (MW_TOOLS=<main checkout>/.claude/tools). Reads only the recording; prints numbers.
"""
import sys, os, numpy as np
sys.path.insert(0, os.environ.get('MW_TOOLS', ''))
from camload import load

D = load(sys.argv[1]); which = sys.argv[2] if len(sys.argv) > 2 else 'after'
d = D['dir']; C = 0x8d0 // 4
cm = d[:, C + 4:C + 20].reshape(-1, 4, 4)
idx = np.where(np.r_[True, np.any(np.diff(cm.reshape(len(cm), -1), axis=0) != 0, axis=1)])[0]
V = lambda o: d[idx, C + o // 4]
t = D['t'][idx]; car = d[idx, C + 0x4d0 // 4:C + 0x510 // 4].reshape(-1, 4, 4)
mph = np.abs(D['veh'][idx, (0x1620 - 0x1600) // 4]); fov_ref = V(0x90)
f, r, u = car[:, 2, :3], car[:, 0, :3], car[:, 1, :3]
tgt = np.stack([-np.arcsin(np.clip(f[:, 1], -1, 1)), np.arctan2(f[:, 0], f[:, 2]), np.arctan2(r[:, 1], u[:, 1])], 1)
ref = np.stack([V(0x590), V(0x594), V(0x598)], 1)
if which == 'after':
    tgt[:, 2] = 0.0  # the followed frame carries no roll (recorded cam + 0x598 stays within 0.25 deg)
wrap = lambda a: (a + np.pi) % (2 * np.pi) - np.pi
ang = ref[0].copy(); out = np.zeros_like(ref); fov = np.zeros(len(t)); fs = 70.0
body = D['body'][idx]; vel = body[:, 0xb0 // 4:0xb0 // 4 + 3]; fwd = body[:, 0x20 // 4:0x20 // 4 + 3]
L = np.stack([V(0x5c0), V(0x5c4), V(0x5c8)], 1); E_ref = np.stack([V(0x5a0), V(0x5a4), V(0x5a8)], 1)
E = np.zeros_like(E_ref); fa = 0.0; WL_RATIO = 0.425
for i in range(len(t)):
    dt = 0.0 if i == 0 else min(t[i] - t[i - 1], 0.1); k60 = dt * 60
    if which == 'before':
        ky = 0.075 + (0.05 - 0.075) * min(mph[i] / 100, 1); kp = kr = 2 / 60
        pitch_mul = min(mph[i] * 0.44704 * 0.02, 1)
        fs += (70 + 25 * min(mph[i] / 190, 1) ** 2 - fs) * (1 - np.exp(-dt * 3)); fov[i] = fs
    else:
        ky = 0.09 if mph[i] < 0.5 else 0.05 + (0.075 - 0.05) * min(mph[i] / 100, 1); kp, kr = 0.55, 1.0
        pitch_mul = min(mph[i] * 0.02, 1)
        fov[i] = 70 + 25 * min(mph[i] / 125, 1)
        a = (vel[i] - vel[i - 1]) / dt if dt > 0 else np.zeros(3)
        at = min(np.linalg.norm(a), 20) if fwd[i] @ a > 0 and fwd[i] @ vel[i] > 0 else 0.0
        fa += (at - fa) * (1 - (1 - (0.01 if at > fa else 0.05)) ** k60); fov[i] += fa
    retain = lambda k: 1 - min(max(k * k60, 0), 1)
    gains = np.array([1 - retain(kp) * pitch_mul, 1 - retain(ky), 1 - retain(kr)])
    ang = wrap(ang + gains * wrap(tgt[i] - ang)); out[i] = ang
    e0 = np.array([0.0, 1.575, -4.6])
    if which == 'after':
        e0[[0, 2]] *= 1 + abs(np.sin(wrap(ang[1] - tgt[i, 1]))) * (WL_RATIO - 1) * 0.5
        u = L[i] - e0; e0 = e0 + u / np.linalg.norm(u) * (3.15 + 0.15 - 3.15 / np.tan(np.radians(fov[i] / 2)))
    E[i] = e0
e = np.degrees(wrap(out - ref)); moving = mph > 1
print(f'{which}: n={len(t)} yaw err RMS {np.sqrt(np.mean(e[:, 1] ** 2)):.3f} deg (p99 {np.percentile(np.abs(e[:, 1]), 99):.2f}), '
      f'pitch {np.sqrt(np.mean(e[:, 0] ** 2)):.3f}, roll {np.sqrt(np.mean(e[:, 2] ** 2)):.3f}, '
      f'FOV RMS {np.sqrt(np.mean((fov - fov_ref) ** 2)):.2f} deg (moving {np.sqrt(np.mean((fov - fov_ref)[moving] ** 2)):.2f})')
ee = E - E_ref
print(f'  eye (cam + 0x5a0..0x5a8) RMS: z {np.sqrt(np.mean(ee[:, 2] ** 2)):.3f} m, y {np.sqrt(np.mean(ee[:, 1] ** 2)):.3f}, '
      f'x {np.sqrt(np.mean(ee[:, 0] ** 2)):.3f}; distance {np.sqrt(np.mean((np.linalg.norm(E, axis=1) - np.linalg.norm(E_ref, axis=1)) ** 2)):.3f}')
lag_ref = np.degrees(wrap(ref[:, 1] - tgt[:, 1])); lag = np.degrees(wrap(out[:, 1] - tgt[:, 1]))
print(f'  yaw lag p1/p99: original {np.percentile(lag_ref, [1, 99]).round(2)}, model {np.percentile(lag, [1, 99]).round(2)}')
