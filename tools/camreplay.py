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

# High-speed shake (camera.rs SpeedShakeState; FUN_005e2a70's matrix at cam + 0x360). The recorded rotation's
# pitch (sin, from the matrix's skew part; positive = looking down) against the model's, per speed band. The
# run's car is not identified: SHAKE = the install's (Amplitude, Frequency, Maximum, Minimum, Power) entry
# that fits best. The model's curve is a stand-in (same statistics, not the same samples), so compare the
# mean and standard deviation, not frame by frame.
SHAKE = (1.2, 1.0, 200.0, 0.0, 1.5)
MEAN, STD, WIDTH = [-0.00486, -0.00633, -0.00587], [0.0123, 0.0106, 0.00285], [3.8, 7.6, 4.0]
rng = 0x2545f4914f6cdd1d; curve = np.zeros((199, 3))
for lane in range(3):
    noise = []
    for _ in range(199 + 80):
        rng = (rng * 0x5851f42d4c957f2d + 1) & (2 ** 64 - 1)
        noise.append(np.frombuffer(np.uint32(((rng >> 41) & 0xffffffff) | 0x3f800000).tobytes(), '<f4')[0] * 2 - 3)
    k = np.exp(-np.arange(-40, 41) ** 2 / (2 * WIDTH[lane] ** 2)); sm = np.convolve(noise, k[::-1], 'valid')
    curve[:, lane] = MEAN[lane] + (sm - sm.mean()) / sm.std() * STD[lane]
mx = d[idx, C + 0x360 // 4:C + 0x390 // 4].reshape(-1, 3, 4)
rec = (mx[:, 1, 2] - mx[:, 2, 1]) / 2
amp, freq, vmax, vmin, power = SHAKE; ph = 0.0; mod = np.zeros(len(t))
for i in range(len(t)):
    dt = 0.0 if i == 0 else min(t[i] - t[i - 1], 0.1)
    kk = amp * np.clip((mph[i] - vmin) / (vmax - vmin), 0, 1) ** power / 10
    ph = (ph + freq * dt) % (398 / 30); x = ph * 30; j = int(x); fr = x - j
    a, b = (min(j, 198), min(j + 1, 198)) if j < 199 else (min(398 - j, 198), min(397 - j, 198))
    q = curve[a, 0] + (curve[b, 0] - curve[a, 0]) * fr
    mod[i] = np.sin(2 * q * kk + kk * 1.0693 * 0.04)
print('  high-speed shake pitch (deg), recorded vs model: mean / std per band')
for lo, hi in [(1, 40), (40, 80), (80, 140)]:
    m = (mph >= lo) & (mph < hi)
    r, o = np.degrees(rec[m]), np.degrees(mod[m])
    print(f'    {lo}-{hi} mph: recorded {r.mean():+.4f} / {r.std():.4f}, model {o.mean():+.4f} / {o.std():.4f}')

# The eye's vertical spring (camera.rs HeightSpring; FUN_005d12b0's vertical stage, the step FUN_005bd470,
# the landing window from FUN_005d4970's tail). Input: the recorded body velocity, its per-frame difference
# as the acceleration, airborne = vehicle + 0x1630 (air time) > 0. Compared with cam + 0x550. The data
# values are this run's (gains 1.5 / 1.75, limits -1.2 / 2.5; cam + 0x558 / 0x55c confirm the limits).
GAIN, LIMITS = (1.5, 1.75), (-1.2, 2.5)
air_t = D['veh'][idx, (0x1630 - 0x1600) // 4]; x_ref = V(0x550)
x, vs = x_ref[0], V(0x554)[0]; damp = 2 * 0.8 * np.sqrt(20); was_air = land = False; land_t = peak = 0.0
X = np.zeros(len(t))
for i in range(len(t)):
    dt = 1 / 60 if i == 0 else min(t[i] - t[i - 1], 0.1); k60 = dt * 60
    v = vel[i].astype(float); a = (v - vel[i - 1]) / dt if i > 0 else np.zeros(3)
    air = air_t[i] > 0; vy, ay = v[1], a[1]
    if abs(v[0]) <= 0.1 and abs(v[2]) <= 0.1:
        vy = ay = 0.0
    if abs(v[0]) > 0.1 or abs(vy) > 0.1 or abs(v[2]) > 0.1:
        up = ay > 0
        g = 3.0 if land else GAIN[0 if up else 1]
        z = 0.5 + np.clip((peak - 10) / 490, 0, 1) * k60 * 0.25 if land and vs < 0 else (0.75 if vs < 0 else 0.8)
        damp = 2 * z * np.sqrt(20)
        if air:
            f = 4 - 3 * k60 * np.clip(vy / 7, 0, 1) if v[1] > 0 else \
                8 - 5 * k60 * np.clip(np.hypot(v[0], v[2]) * 2.23694 / 60, 0, 1)
            u = -vy * f
        else:
            n, e = (185.0, 0.8) if up else (15.0, 0.5)
            u = -ay * g * min(np.clip(abs(ay * g) / n, 0, 1) ** e, 1)
        vs += u * dt
    vs -= damp * vs * dt; vs -= 20 * x * dt; x += vs * dt
    if x <= LIMITS[0] or x >= LIMITS[1]:
        x = np.clip(x, *LIMITS); vs = 0.0
    X[i] = x
    if not air and was_air:
        land, was_air = True, False
        continue
    if land:
        land_t += dt; peak = max(peak, a[1])
        if land_t > 0.3:
            land, land_t, peak = False, 0.0, 0.0
    was_air = air
print(f'  eye height spring (cam + 0x550): RMS error {np.sqrt(np.mean((X - x_ref) ** 2)):.4f} m, '
      f'correlation {np.corrcoef(X, x_ref)[0, 1]:.4f}; std recorded / model per band:')
for lo, hi in [(1, 40), (40, 80), (80, 140)]:
    m = (mph >= lo) & (mph < hi)
    print(f'    {lo}-{hi} mph: {x_ref[m].std():.3f} / {X[m].std():.3f} (error RMS {np.sqrt(np.mean((X - x_ref)[m] ** 2)):.3f})')
