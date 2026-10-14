"""Compare our chase camera with the original's on the same car motion.

    python3 tools/camcompare.py inputs ref/live1 <inputs.csv>
    ./target/release/mostrusted --camera-replay <inputs.csv>,<ours.csv>
    python3 tools/camcompare.py compare ref/live1 <ours.csv>

`inputs` turns a recording of the original (ref/<name>/cam.bin, .claude/tools/camrec.py: the camera director
and the player car at ~120 Hz) into one line per camera update: the followed car's model origin (cam + 0x500)
and orientation, its velocity (body + 0xb0), signed speed, yaw rate and air time, and the car's id (by its mass:
ref/live1 holds three cars). `mostrusted --camera-replay` runs our chase camera over it (camera::replay_recording).
`compare` puts both cameras in the car's frame and prints, per component, the original's mean / std, ours, the
RMS difference and the delay (s) that best lines ours up with the original (cross-correlation; positive = ours
late), overall and per speed band, plus the response to acceleration, braking, turns and landings.
Needs camload.py (MW_TOOLS=<main checkout>/.claude/tools). Reads only the recording; prints numbers.
"""
import sys, os, numpy as np
sys.path.insert(0, os.environ.get('MW_TOOLS', ''))
from camload import load

# Cars in ref/live1, by the body's mass (body + 0x7c): Jaguar XKR, McLaren MP4-12C, Mercedes SL 65 AMG.
CARS = {1762.0: '1399602', 1350.0: '122765', 1807.0: '122769'}


def frames(rec):
    D = load(rec); d = D['dir']; C = 0x8d0 // 4
    cm = d[:, C + 4:C + 20].reshape(-1, 4, 4)
    first = np.r_[True, np.any(np.diff(cm.reshape(len(cm), -1), axis=0) != 0, axis=1)]
    idx = np.where(first)[0]
    body = D['body']; bm = body[:, :16].reshape(-1, 4, 4)
    cam500 = d[:, C + 0x500 // 4:C + 0x50c // 4]
    # The body block is read after the director: pick, per camera update, the record whose body matches the
    # camera's car position best (its offset from the model origin is the car's constant centre of mass).
    off = np.einsum('nij,nj->ni', bm[:, :3, :3], bm[:, 3, :3] - cam500)
    ends = np.r_[idx[1:], len(d)]
    mass = body[:, 0x7c // 4]
    pick = np.empty(len(idx), int)
    for k, (a, b) in enumerate(zip(idx, ends)):
        car = mass[a]; ref = np.median(off[idx[max(0, k - 50):k + 50]][mass[idx[max(0, k - 50):k + 50]] == car], 0)
        pick[k] = a + int(np.argmin(np.linalg.norm(off[a:b] - ref, axis=1)))
    return D, idx, pick


def inputs(rec, out):
    D, idx, pick = frames(rec)
    d = D['dir']; C = 0x8d0 // 4; body = D['body']; veh = D['veh']
    t = D['t'][idx]
    dt = np.clip(np.round(np.diff(t, prepend=t[0] - 1 / 60) * 60), 1, 2) / 60
    bm = body[pick, :16].reshape(-1, 4, 4)
    pos = d[idx, C + 0x500 // 4:C + 0x50c // 4]
    car = d[idx, C + 0x4d0 // 4:C + 0x510 // 4].reshape(-1, 4, 4)
    vel = body[pick, 0xb0 // 4:0xb0 // 4 + 3]; om = body[pick, 0xc0 // 4:0xc0 // 4 + 3]
    air = veh[idx, (0x1630 - 0x1600) // 4]
    mass = body[pick, 0x7c // 4]
    with open(out, 'w') as f:
        f.write('t,dt,car,px,py,pz,qx,qy,qz,qw,gqx,gqy,gqz,gqw,vx,vy,vz,speed,yaw_rate,wheels_down,air_time\n')
        for k in range(len(idx)):
            r = bm[k, :3, :3].astype(float)
            q = quat(r[0], r[1], r[2])
            # The camera's own car frame (cam + 0x4d0, vehicle + 0x2db0: the wheels' plane); its stored forward
            # has a low-passed height, so the forward is rebuilt from rows 0 and 1.
            g0, g1 = car[k, 0, :3].astype(float), car[k, 1, :3].astype(float)
            gf = np.cross(g1, g0); gf /= np.linalg.norm(gf); gx = np.cross(g1, gf); gx /= np.linalg.norm(gx)
            gq = quat(gx, np.cross(gf, gx), gf)
            speed = vel[k] @ r[2]; yaw_rate = om[k] @ r[1]
            f.write(f"{t[k]:.5f},{dt[k]:.6f},{CARS.get(float(mass[k]), '?')},{pos[k][0]:.5f},{pos[k][1]:.5f},{pos[k][2]:.5f},"
                    f"{q[0]:.7f},{q[1]:.7f},{q[2]:.7f},{q[3]:.7f},{gq[0]:.7f},{gq[1]:.7f},{gq[2]:.7f},{gq[3]:.7f},{vel[k][0]:.5f},{vel[k][1]:.5f},{vel[k][2]:.5f},"
                    f"{speed:.5f},{yaw_rate:.5f},{0 if air[k] > 0 else 4},{air[k]:.4f}\n")
    print(f'{out}: {len(idx)} updates, cars {sorted(set(CARS.get(float(m), "?") for m in mass))}')


def quat(x, y, z):
    """Quaternion (x, y, z, w) of the rotation whose columns are the car's right, up and forward rows."""
    m = np.array([x, y, z]).T
    tr = m[0, 0] + m[1, 1] + m[2, 2]
    if tr > 0:
        s = np.sqrt(tr + 1) * 2
        return np.array([(m[2, 1] - m[1, 2]) / s, (m[0, 2] - m[2, 0]) / s, (m[1, 0] - m[0, 1]) / s, s / 4])
    i = int(np.argmax([m[0, 0], m[1, 1], m[2, 2]])); j, k = (i + 1) % 3, (i + 2) % 3
    s = np.sqrt(1 + m[i, i] - m[j, j] - m[k, k]) * 2
    q = np.zeros(4); q[i] = s / 4; q[j] = (m[j, i] + m[i, j]) / s; q[k] = (m[k, i] + m[i, k]) / s
    q[3] = (m[k, j] - m[j, k]) / s
    return q


def describe(eye, fwd, up, rows, pos):
    """Car-frame components: eye x / y / z (m), view yaw and pitch relative to the car, world roll (deg)."""
    e = np.einsum('nij,nj->ni', rows, eye - pos)
    fl = np.einsum('nij,nj->ni', rows, fwd)
    right = np.cross(up, fwd); right /= np.linalg.norm(right, axis=1)[:, None]
    yaw = np.degrees(np.arctan2(fl[:, 0], fl[:, 2]))
    pitch = np.degrees(np.arcsin(np.clip(fl[:, 1], -1, 1)))
    wpitch = np.degrees(np.arcsin(np.clip(fwd[:, 1], -1, 1)))
    roll = np.degrees(np.arcsin(np.clip(right[:, 1], -1, 1)))
    return dict(eye_x=e[:, 0], eye_y=e[:, 1], eye_z=e[:, 2], dist=np.hypot(e[:, 0], e[:, 2]), yaw=yaw, pitch=pitch,
                world_pitch=wpitch, roll=roll)


def delay(a, b, hz=60, max_lag=60):
    """Lag (s) of b behind a by cross-correlation of their high-passed parts (positive: b late)."""
    a = a - np.convolve(a, np.ones(61) / 61, 'same'); b = b - np.convolve(b, np.ones(61) / 61, 'same')
    best = max(range(-max_lag, max_lag + 1), key=lambda L: np.dot(a[max(0, -L):len(a) - max(0, L)], b[max(0, L):len(b) - max(0, -L)]))
    return best / hz


def compare(rec, ours_csv):
    D, idx, pick = frames(rec)
    d = D['dir']; C = 0x8d0 // 4; body = D['body']; veh = D['veh']
    O = np.genfromtxt(ours_csv, delimiter=',', names=True)
    n = min(len(idx), len(O)); idx, pick, O = idx[:n], pick[:n], O[:n]
    cm = d[idx, C + 4:C + 20].reshape(-1, 4, 4)
    rows = body[pick, :12].reshape(-1, 3, 4)[:, :, :3].astype(float)
    pos = d[idx, C + 0x500 // 4:C + 0x50c // 4].astype(float)
    orig = describe(cm[:, 3, :3].astype(float), cm[:, 2, :3].astype(float), cm[:, 1, :3].astype(float), rows, pos)
    orig['fov'] = d[idx, C + 0x90 // 4]
    # Records torn by the sampler (a camera matrix read in the middle of an update: the eye a kilometre off, or
    # rows that are not orthonormal) take the previous update's values.
    r3 = cm[:, :3, :3].astype(float)
    torn = (np.abs(np.einsum('nij,nkj->nik', r3, r3) - np.eye(3)).max((1, 2)) > 1e-3) | (orig['dist'] > 15)
    print(f'{torn.sum()} torn records replaced by their predecessors')
    for k in orig:
        v = orig[k]
        for i in np.flatnonzero(torn):
            v[i] = v[i - 1] if i else v[i]
    ours = describe(np.stack([O['ex'], O['ey'], O['ez']], 1), np.stack([O['fx'], O['fy'], O['fz']], 1),
                    np.stack([O['ux'], O['uy'], O['uz']], 1), rows, pos)
    ours['fov'] = O['fov']
    vel = body[pick, 0xb0 // 4:0xb0 // 4 + 3].astype(float)
    mph = np.linalg.norm(vel[:, [0, 2]], axis=1) * 2.23694
    t = D['t'][idx]
    dts = np.clip(np.round(np.diff(t, prepend=t[0] - 1 / 60) * 60), 1, 2) / 60
    acc = np.gradient(vel, axis=0) / dts[:, None]
    a_long = np.einsum('ni,ni->n', acc, rows[:, 2]); yaw_rate = np.einsum('ni,ni->n', body[pick, 0xc0 // 4:0xc0 // 4 + 3], rows[:, 1])
    air = veh[idx, (0x1630 - 0x1600) // 4]
    keys = ['eye_x', 'eye_y', 'eye_z', 'dist', 'yaw', 'pitch', 'world_pitch', 'roll', 'fov']
    # The player's look-around / look-back (the yaw offset cam + 0x6f0, `FUN_005c75b0`) and 1 s after it:
    # left out (the replay has no look input).
    look = np.abs(d[idx, C + 0x6f0 // 4]) > 1e-3
    look = np.convolve(look.astype(float), np.r_[np.zeros(59), np.ones(60)], 'same') > 0
    print(f'{look.sum()} updates with the look-around (or within 1 s of it) left out')
    # Impact shakes are random noise (the original's generator state is not replayed): updates with a shake
    # level above 0.05 in either camera (cam + 0x280; ours `impact_level`) are left out of the table.
    shaking = (d[idx, C + 0x280 // 4] > 0.05) | (O['impact_level'] > 0.05)
    print(f'{(shaking & (mph > 1) & ~look).sum()} moving updates with an impact shake left out')
    moving = (mph > 1) & ~look & ~shaking
    print(f'{n} updates, {moving.sum()} moving (> 1 mph)')
    print(f"{'component':12} {'orig mean/std':>16} {'ours mean/std':>16} {'RMS diff':>9} {'corr':>6} {'delay s':>8}")
    for k in keys:
        a, b = orig[k][moving], ours[k][moving]
        dly = delay(a, b) if k in ('eye_x', 'yaw', 'roll', 'pitch', 'eye_y', 'eye_z') else float('nan')
        print(f"{k:12} {a.mean():8.3f}/{a.std():6.3f} {b.mean():8.3f}/{b.std():6.3f} {np.sqrt(np.mean((a - b) ** 2)):9.3f} "
              f"{np.corrcoef(a, b)[0, 1]:6.3f} {dly:8.3f}")
    print('per speed band (mean orig / ours):')
    for lo, hi in [(0, 1), (1, 30), (30, 60), (60, 90), (90, 140)]:
        m = (mph >= lo) & (mph < hi) & ~look & ~shaking
        if m.sum() < 10: continue
        print(f"  {lo:3}-{hi:3} mph n={m.sum():5}: " + ', '.join(
            f"{k} {orig[k][m].mean():.2f}/{ours[k][m].mean():.2f}" for k in ['eye_y', 'eye_z', 'dist', 'pitch', 'fov']))
    print('responses (orig / ours):')
    def window(name, m):
        if m.sum() < 5: return
        print(f"  {name:28} n={m.sum():5}: " + ', '.join(
            f"{k} {orig[k][m].mean():.2f}/{ours[k][m].mean():.2f}" for k in ['eye_y', 'eye_z', 'pitch', 'world_pitch', 'fov']))
    base = moving & (np.abs(a_long) < 1) & (np.abs(yaw_rate) < 0.1) & (air == 0)
    window('steady (|a|<1, |yaw rate|<0.1)', base & (mph > 30))
    window('accelerating a > 4 m/s2', moving & (a_long > 4) & (air == 0))
    window('braking a < -6 m/s2', moving & (a_long < -6) & (air == 0))
    # Turns: the yaw lag (view yaw relative to the car) against the yaw rate.
    m = moving & (np.abs(yaw_rate) > 0.3)
    if m.sum() > 5:
        sa = np.polyfit(yaw_rate[m], orig['yaw'][m], 1)[0]; sb = np.polyfit(yaw_rate[m], ours['yaw'][m], 1)[0]
        sx = np.polyfit(yaw_rate[m], orig['eye_x'][m], 1)[0]; sy = np.polyfit(yaw_rate[m], ours['eye_x'][m], 1)[0]
        sr = np.polyfit(yaw_rate[m], orig['roll'][m], 1)[0]; so = np.polyfit(yaw_rate[m], ours['roll'][m], 1)[0]
        print(f"  turns (|yaw rate| > 0.3) n={m.sum()}: view yaw per rad/s {sa:.2f}/{sb:.2f} deg, eye x per rad/s "
              f"{sx:.3f}/{sy:.3f} m, roll per rad/s {sr:.2f}/{so:.2f} deg")
    # Landings: 0.5 s after each touch-down that followed >= 0.2 s in the air.
    land = np.where((air[:-1] > 0.2) & (air[1:] == 0))[0] + 1
    if len(land):
        ex = [(orig['eye_y'][l:l + 30] - orig['eye_y'][l - 1], ours['eye_y'][l:l + 30] - ours['eye_y'][l - 1]) for l in land if l + 30 < n]
        print(f"  landings ({len(ex)}): eye height dip over 0.5 s, orig {np.mean([e[0].min() for e in ex]):.3f} m, "
              f"ours {np.mean([e[1].min() for e in ex]):.3f} m; world pitch std "
              f"{np.mean([orig['world_pitch'][l:l + 30].std() for l in land if l + 30 < n]):.2f}/"
              f"{np.mean([ours['world_pitch'][l:l + 30].std() for l in land if l + 30 < n]):.2f} deg")
    per_car = veh[idx, 0]  # placeholder to keep the arrays aligned
    mass = body[pick, 0x7c // 4]
    print('at rest (< 0.5 mph), per car (orig / ours):')
    for m_, car in CARS.items():
        m = (mass == m_) & (mph < 0.5)
        if m.sum() < 5: continue
        print(f"  {car}: eye y {orig['eye_y'][m].mean():.3f}/{ours['eye_y'][m].mean():.3f}, eye z {orig['eye_z'][m].mean():.3f}/"
              f"{ours['eye_z'][m].mean():.3f}, pitch {orig['pitch'][m].mean():.2f}/{ours['pitch'][m].mean():.2f}, fov "
              f"{orig['fov'][m].mean():.2f}/{ours['fov'][m].mean():.2f}")
    if len(sys.argv) > 4:
        np.savez(sys.argv[4], t=t, mph=mph, a_long=a_long, yaw_rate=yaw_rate, air=air,
                 **{f'o_{k}': v for k, v in orig.items()}, **{f'u_{k}': v for k, v in ours.items()})


if __name__ == '__main__':
    {'inputs': inputs, 'compare': compare}[sys.argv[1]](sys.argv[2], sys.argv[3])
