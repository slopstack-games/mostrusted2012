//! What the ground under a wheel does beyond its grip numbers (docs/decomp.md "Tyres", "Surface factors"):
//!
//! - [`SurfaceGrip`]: the per-wheel grip state (`vehicle +0x17e0`, four 0x7c-byte records). When a wheel
//!   moves onto another surface the grip of the one it left lingers and fades towards the new surface's link
//!   grip, and the tyre gets the lower of the two (`FUN_009d4460`, `FUN_009ceb00`, `FUN_009cec40`,
//!   `FUN_009cece0`). The effective lateral grip of the four wheels, averaged, is also the drift's surface
//!   factor (`FUN_009cede0`).
//! - [`roughness`]: the ground's bumps (`FUN_009d77b0` → `FUN_009d4650`): the contact point is pushed along
//!   the contact normal by Perlin noise of the world position, and the normal is rebuilt from two nearby
//!   noise samples. The surface's `Surface_roughness` gives the noise's spatial frequency and amplitude.

/// `FUN_009ceb00`: the fade's length is (old link grip − the lower old grip) × this × the time spent on the
/// old surface × [`FADE_TIME_SCALE`] (`.rdata` 0x01025dcc).
const FADE_GRIP_SCALE: f32 = 3.0;
/// `.rdata` 0x00f8a810.
const FADE_TIME_SCALE: f32 = 0.5;
/// The time on a surface stops counting here (s; `FUN_009d4460`, `.rdata` 0x01026028).
const MAX_SURFACE_TIME: f32 = 2.0;
/// The three fading slots of a wheel's record.
const FADE_SLOTS: usize = 3;

/// One surface just left, fading out (7 words of the record each).
#[derive(Debug, Clone, Copy, Default)]
struct Fade {
    /// `SurfacePhysics` unique id of the surface left; 0 = slot free.
    surface: u32,
    time: f32,
    duration: f32,
    /// (lateral, longitudinal) grip when it was left, and now.
    start: [f32; 2],
    now: [f32; 2],
}

/// The surface grip state of one wheel (`FUN_009d4460` runs it once per tick while the wheel is on the
/// ground; airborne wheels keep their values).
#[derive(Debug, Clone)]
pub struct SurfaceGrip {
    fades: [Fade; FADE_SLOTS],
    /// The surface under the wheel (`SurfacePhysics` unique id, record +0x54; 0 before the first contact).
    surface: u32,
    /// Seconds on this surface (+0x64), capped at [`MAX_SURFACE_TIME`].
    time: f32,
    /// (lateral, longitudinal) grip of this surface for this wheel: `Surface_Grip` blended towards the car's
    /// link grip (+0x68 / +0x6c, `FUN_009cec40`).
    grip: [f32; 2],
    /// The car's link grip for this surface (`f43804ae`, 1 without a link; +0x78): what the fades head for.
    target: f32,
    /// What the tyre gets: the lower of `grip` and every fading slot (+0x70 / +0x74, `FUN_009cece0`).
    pub effective: [f32; 2],
}

impl Default for SurfaceGrip {
    fn default() -> Self {
        Self {
            fades: [Fade::default(); FADE_SLOTS],
            surface: 0,
            time: 0.0,
            grip: [1.0; 2],
            target: 1.0,
            effective: [1.0; 2],
        }
    }
}

impl SurfaceGrip {
    /// One tick on the ground (`FUN_009d4460`): `surface` is the `SurfacePhysics` unique id under the wheel,
    /// `grip` its (lateral, longitudinal) grip blended for this wheel, `target` the car's link grip for it
    /// (1 without a link). Returns the effective (lateral, longitudinal) grip.
    pub fn step(&mut self, surface: u32, grip: [f32; 2], target: f32, dt: f32) -> [f32; 2] {
        // The fades run towards the current surface's link grip.
        for f in self.fades.iter_mut().filter(|f| f.surface != 0) {
            f.time += dt;
            if f.time < f.duration {
                let t = f.time / f.duration;
                f.now = [0, 1].map(|k| f.start[k] + (self.target - f.start[k]) * t);
            } else {
                *f = Fade { now: [1.0; 2], ..Fade::default() };
            }
        }
        if surface != self.surface {
            self.leave(surface);
            // `FUN_009cec40`.
            self.surface = surface;
            self.time = 0.0;
            self.grip = grip;
            self.target = target;
        }
        self.time = (self.time + dt).min(MAX_SURFACE_TIME);
        // `FUN_009cece0`.
        self.effective = self.grip;
        for f in self.fades.iter().filter(|f| f.surface != 0) {
            for k in 0..2 {
                self.effective[k] = self.effective[k].min(f.now[k]);
            }
        }
        self.effective
    }

    /// `FUN_009ceb00`: the surface being left starts fading; a fade of the surface being entered ends.
    fn leave(&mut self, entering: u32) {
        if let Some(f) = self.fades.iter_mut().find(|f| f.surface != 0 && f.surface == entering) {
            *f = Fade { now: [1.0; 2], ..Fade::default() };
        }
        // The first free slot; with none free, the one whose grip is highest (the least limiting).
        let slot = self.fades.iter().position(|f| f.surface == 0).unwrap_or_else(|| {
            let mut best = (0, 0.0);
            for (i, f) in self.fades.iter().enumerate() {
                let g = f.now[0].min(f.now[1]);
                if best.1 < g {
                    best = (i, g);
                }
            }
            best.0
        });
        let lower = self.grip[0].min(self.grip[1]);
        self.fades[slot] = Fade {
            surface: self.surface,
            time: 0.0,
            duration: (self.target - lower) * FADE_GRIP_SCALE * self.time * FADE_TIME_SCALE,
            start: self.grip,
            now: self.grip,
        };
    }
}

/// The drift's surface factor (`FUN_009cede0`): the mean effective lateral grip of the four wheels.
pub fn drift_factor<'a>(wheels: impl IntoIterator<Item = &'a SurfaceGrip>) -> f32 {
    let (sum, n) = wheels.into_iter().fold((0.0, 0), |(s, n), w| (s + w.effective[0], n + 1));
    if n == 0 { 1.0 } else { sum / n as f32 }
}

/// `Surface_roughness` of a `SurfacePhysics` scaled as the game reads it (`FUN_0054d750`): the noise's
/// spatial frequency (field `8193fd9b`, per metre) and its amplitude (`7cb58933` × 0.001, `.rdata`
/// 0x00f8bb34: millimetres to metres).
pub fn roughness_scale(raw: [f32; 2]) -> (f32, f32) {
    (raw[0], raw[1] * 0.001)
}

/// Distance of the two extra samples that tilt the normal (m; `.bss` 0x01186ea0, read live: 0.03).
const NORMAL_SAMPLE: f32 = 0.03;

/// The bumped contact (`FUN_009d4650`): `point` and `normal` of the ground contact, the noise's `frequency`
/// (per metre) and `amplitude` (m). Returns the contact moved along the normal by amplitude × noise, the
/// normal of the bumped ground there, and the displacement (m, along the normal).
pub fn roughness(point: [f32; 3], normal: [f32; 3], frequency: f32, amplitude: f32) -> ([f32; 3], [f32; 3], f32) {
    let bump = |p: [f32; 3]| {
        let n = perlin(p[0] * frequency, p[1] * frequency, p[2] * frequency);
        add(p, scale(normal, amplitude * n))
    };
    let moved = bump(point);
    // Two tangents: normal × X, then normal × that.
    let t1 = normalize(cross(normal, [1.0, 0.0, 0.0]));
    let t2 = normalize(cross(normal, t1));
    let e1 = sub(bump(add(point, scale(t1, NORMAL_SAMPLE))), moved);
    let e2 = sub(bump(add(point, scale(t2, NORMAL_SAMPLE))), moved);
    let tilted = normalize(cross(e1, e2));
    let d = amplitude * perlin(point[0] * frequency, point[1] * frequency, point[2] * frequency);
    (moved, tilted, d)
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn normalize(a: [f32; 3]) -> [f32; 3] {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    if l > 0.0 { scale(a, 1.0 / l) } else { a }
}

/// Ken Perlin's reference permutation ("Improved Noise", 2002). The game's noise (`FUN_009e31e0`, table at
/// `.data` 0x010cfcb0, 512 entries) is this table twice over.
const PERM: [u8; 256] = [
    151, 160, 137, 91, 90, 15, 131, 13, 201, 95, 96, 53, 194, 233, 7, 225, 140, 36, 103, 30, 69, 142, 8, 99, 37, 240,
    21, 10, 23, 190, 6, 148, 247, 120, 234, 75, 0, 26, 197, 62, 94, 252, 219, 203, 117, 35, 11, 32, 57, 177, 33, 88,
    237, 149, 56, 87, 174, 20, 125, 136, 171, 168, 68, 175, 74, 165, 71, 134, 139, 48, 27, 166, 77, 146, 158, 231, 83,
    111, 229, 122, 60, 211, 133, 230, 220, 105, 92, 41, 55, 46, 245, 40, 244, 102, 143, 54, 65, 25, 63, 161, 1, 216,
    80, 73, 209, 76, 132, 187, 208, 89, 18, 169, 200, 196, 135, 130, 116, 188, 159, 86, 164, 100, 109, 198, 173, 186,
    3, 64, 52, 217, 226, 250, 124, 123, 5, 202, 38, 147, 118, 126, 255, 82, 85, 212, 207, 206, 59, 227, 47, 16, 58, 17,
    182, 189, 28, 42, 223, 183, 170, 213, 119, 248, 152, 2, 44, 154, 163, 70, 221, 153, 101, 155, 167, 43, 172, 9, 129,
    22, 39, 253, 19, 98, 108, 110, 79, 113, 224, 232, 178, 185, 112, 104, 218, 246, 97, 228, 251, 34, 242, 193, 238,
    210, 144, 12, 191, 179, 162, 241, 81, 51, 145, 235, 249, 14, 239, 107, 49, 192, 214, 31, 181, 199, 106, 157, 184,
    84, 204, 176, 115, 121, 50, 45, 127, 4, 150, 254, 138, 236, 205, 93, 222, 114, 67, 29, 24, 72, 243, 141, 128, 195,
    78, 66, 215, 61, 156, 180,
];

fn p(i: usize) -> usize {
    PERM[i & 255] as usize
}

fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn lerp(t: f32, a: f32, b: f32) -> f32 {
    a + t * (b - a)
}

fn grad(hash: usize, x: f32, y: f32, z: f32) -> f32 {
    let h = hash & 15;
    let u = if h < 8 { x } else { y };
    let v = if h < 4 {
        y
    } else if h == 12 || h == 14 {
        x
    } else {
        z
    };
    (if h & 1 == 0 { u } else { -u }) + (if h & 2 == 0 { v } else { -v })
}

/// Perlin's improved noise, about −1..1 (`FUN_009e31e0`).
pub fn perlin(x: f32, y: f32, z: f32) -> f32 {
    let (fx, fy, fz) = (x.floor(), y.floor(), z.floor());
    let (xi, yi, zi) = ((fx as i32 & 255) as usize, (fy as i32 & 255) as usize, (fz as i32 & 255) as usize);
    let (x, y, z) = (x - fx, y - fy, z - fz);
    let (u, v, w) = (fade(x), fade(y), fade(z));
    let a = p(xi) + yi;
    let aa = p(a) + zi;
    let ab = p(a + 1) + zi;
    let b = p(xi + 1) + yi;
    let ba = p(b) + zi;
    let bb = p(b + 1) + zi;
    lerp(
        w,
        lerp(
            v,
            lerp(u, grad(p(aa), x, y, z), grad(p(ba), x - 1.0, y, z)),
            lerp(u, grad(p(ab), x, y - 1.0, z), grad(p(bb), x - 1.0, y - 1.0, z)),
        ),
        lerp(
            v,
            lerp(u, grad(p(aa + 1), x, y, z - 1.0), grad(p(ba + 1), x - 1.0, y, z - 1.0)),
            lerp(u, grad(p(ab + 1), x, y - 1.0, z - 1.0), grad(p(bb + 1), x - 1.0, y - 1.0, z - 1.0)),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASPHALT: u32 = 283001;
    const GRASS: u32 = 283005;

    #[test]
    fn grip_fades_back_after_grass() {
        let mut g = SurfaceGrip::default();
        let dt = 1.0 / 60.0;
        for _ in 0..180 {
            g.step(ASPHALT, [1.0, 1.0], 1.0, dt);
        }
        assert_eq!(g.effective, [1.0, 1.0]);
        // Onto grass: no fade from asphalt (its link grip is no higher than its grip).
        let e = g.step(GRASS, [0.835, 0.88], 1.3, dt);
        assert_eq!(e, [0.835, 0.88]);
        for _ in 0..180 {
            g.step(GRASS, [0.835, 0.88], 1.3, dt);
        }
        // Back on asphalt after 2 s or more of grass: (1.3 − 0.835) × 3 × 2 × 0.5 = 1.395 s back to 1.
        let e = g.step(ASPHALT, [1.0, 1.0], 1.0, dt);
        assert!((e[0] - 0.835).abs() < 0.01, "{e:?}");
        let mut t = dt;
        while g.effective[0] < 0.999 {
            g.step(ASPHALT, [1.0, 1.0], 1.0, dt);
            t += dt;
            assert!(t < 3.0);
        }
        assert!((t - 1.395).abs() < 0.05, "{t}");
    }

    #[test]
    fn perlin_matches_reference_points() {
        // Zero on the lattice, bounded, smooth.
        assert_eq!(perlin(1.0, 2.0, 3.0), 0.0);
        let mut max: f32 = 0.0;
        for i in 0..1000 {
            let x = i as f32 * 0.137;
            max = max.max(perlin(x, x * 0.7 + 0.3, x * 0.31 + 0.5).abs());
        }
        assert!(max > 0.3 && max <= 1.1, "{max}");
        // Ken Perlin's reference implementation gives 0.13691995878400012 at (3.14, 42, 7).
        assert!((perlin(3.14, 42.0, 7.0) - 0.136_92).abs() < 1e-4, "{}", perlin(3.14, 42.0, 7.0));
    }

    #[test]
    fn roughness_keeps_flat_ground_up() {
        let (p, n, d) = roughness([10.3, 0.0, 20.7], [0.0, 1.0, 0.0], 0.1, 0.08);
        assert!((p[1] - d).abs() < 1e-6);
        assert!(n[1] > 0.99, "{n:?}");
        assert!(d.abs() <= 0.08 * 1.1);
    }
}
