//! Tiny software rasterizer for previewing geometry without a GPU: renders
//! flat-shaded triangles from three orthographic views into one image.

pub struct Tri {
    pub p: [[f32; 3]; 3],
    pub color: [u8; 3],
}

/// Render side / front / top views side by side.
pub fn render_views(tris: &[Tri], size: u32) -> image::RgbImage {
    let mut img = image::RgbImage::from_pixel(size * 3, size, image::Rgb([24, 24, 28]));
    if tris.is_empty() {
        return img;
    }
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for t in tris {
        for p in &t.p {
            for a in 0..3 {
                lo[a] = lo[a].min(p[a]);
                hi[a] = hi[a].max(p[a]);
            }
        }
    }
    let extent = (0..3).map(|a| hi[a] - lo[a]).fold(0.0f32, f32::max).max(1e-6);
    let center: [f32; 3] = std::array::from_fn(|a| (lo[a] + hi[a]) * 0.5);
    // (screen-x axis, screen-y axis, depth axis, depth sign)
    let views = [(0usize, 1usize, 2usize, 1.0f32), (2, 1, 0, -1.0), (0, 2, 1, -1.0)];
    for (vi, &(ax, ay, az, dsign)) in views.iter().enumerate() {
        let mut depth = vec![f32::MIN; (size * size) as usize];
        let scale = size as f32 * 0.9 / extent;
        let project = |p: &[f32; 3]| {
            [
                (p[ax] - center[ax]) * scale + size as f32 * 0.5,
                size as f32 * 0.5 - (p[ay] - center[ay]) * scale,
                (p[az] - center[az]) * dsign,
            ]
        };
        for t in tris {
            let s = t.p.map(|p| project(&p));
            let n = normal(&t.p);
            let light = 0.35 + 0.65 * (n[0] * 0.4 + n[1] * 0.8 + n[2] * 0.45).abs().min(1.0);
            let col = t.color.map(|c| (c as f32 * light) as u8);
            raster(&s, size, &mut depth, |x, y| {
                img.put_pixel(vi as u32 * size + x, y, image::Rgb(col));
            });
        }
    }
    img
}

fn normal(p: &[[f32; 3]; 3]) -> [f32; 3] {
    let u = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
    let v = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
    let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
    let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-12);
    n.map(|c| c / l)
}

fn raster(s: &[[f32; 3]; 3], size: u32, depth: &mut [f32], mut plot: impl FnMut(u32, u32)) {
    let min_x = s.iter().map(|p| p[0]).fold(f32::MAX, f32::min).floor().max(0.0) as i32;
    let max_x = s.iter().map(|p| p[0]).fold(f32::MIN, f32::max).ceil().min(size as f32 - 1.0) as i32;
    let min_y = s.iter().map(|p| p[1]).fold(f32::MAX, f32::min).floor().max(0.0) as i32;
    let max_y = s.iter().map(|p| p[1]).fold(f32::MIN, f32::max).ceil().min(size as f32 - 1.0) as i32;
    let area = (s[1][0] - s[0][0]) * (s[2][1] - s[0][1]) - (s[2][0] - s[0][0]) * (s[1][1] - s[0][1]);
    if area.abs() < 1e-9 {
        return;
    }
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let w0 = ((s[1][0] - px) * (s[2][1] - py) - (s[2][0] - px) * (s[1][1] - py)) / area;
            let w1 = ((s[2][0] - px) * (s[0][1] - py) - (s[0][0] - px) * (s[2][1] - py)) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            let z = w0 * s[0][2] + w1 * s[1][2] + w2 * s[2][2];
            let d = &mut depth[(y as u32 * size + x as u32) as usize];
            if z > *d {
                *d = z;
                plot(x as u32, y as u32);
            }
        }
    }
}
