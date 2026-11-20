//! A small headless rasteriser for the examples: STL triangles drawn as orthographic, flat
//! shaded panels into PNGs (nearer faces lighter, so slots and pockets show from above).

#![allow(dead_code)]

pub type V3 = [f64; 3];

pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn norm(a: V3) -> V3 {
    let l = dot(a, a).sqrt().max(1e-12);
    [a[0] / l, a[1] / l, a[2] / l]
}

pub fn triangles(stl: &[u8]) -> Vec<[V3; 3]> {
    let n = u32::from_le_bytes([stl[80], stl[81], stl[82], stl[83]]) as usize;
    let f = |o: usize| {
        f64::from(f32::from_le_bytes([
            stl[o],
            stl[o + 1],
            stl[o + 2],
            stl[o + 3],
        ]))
    };
    (0..n)
        .map(|t| {
            let b = 84 + t * 50 + 12;
            [0, 1, 2].map(|k| [f(b + k * 12), f(b + k * 12 + 4), f(b + k * 12 + 8)])
        })
        .collect()
}

/// One orthographic panel: the camera looks along `-eye` (eye is the direction toward the
/// viewer), with world +Z (or +Y when looking straight down) as screen up.
pub fn panel(
    tris: &[[V3; 3]],
    eye: V3,
    focus: Option<(V3, V3)>,
    size: usize,
    pixels: &mut [u8],
    stride: usize,
    x0: usize,
) {
    let fwd = norm(eye);
    let up0 = if fwd[2].abs() > 0.99 {
        [0.0, 1.0, 0.0]
    } else {
        [0.0, 0.0, 1.0]
    };
    let right = norm(cross(up0, fwd));
    let up = cross(fwd, right);
    let pts: Vec<[V3; 3]> = tris
        .iter()
        .map(|t| t.map(|p| [dot(p, right), dot(p, up), dot(p, fwd)]))
        .collect();
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    let (mut zlo, mut zhi) = (f64::MAX, f64::MIN);
    let fit: Vec<V3> = match focus {
        // The eight corners of the box to frame.
        Some((a, b)) => (0..8)
            .map(|k| {
                let c = [
                    if k & 1 == 0 { a[0] } else { b[0] },
                    if k & 2 == 0 { a[1] } else { b[1] },
                    if k & 4 == 0 { a[2] } else { b[2] },
                ];
                [dot(c, right), dot(c, up), dot(c, fwd)]
            })
            .collect(),
        None => pts.iter().flatten().copied().collect(),
    };
    for p in &fit {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
        zlo = zlo.min(p[2]);
        zhi = zhi.max(p[2]);
    }
    let scale = (size as f64 * 0.9) / (hi[0] - lo[0]).max(hi[1] - lo[1]);
    let cx = (lo[0] + hi[0]) / 2.0;
    let cy = (lo[1] + hi[1]) / 2.0;
    let light = norm([0.4, 0.7, 1.0]);
    let mut zbuf = vec![f64::MIN; size * size];
    for (t, world) in pts.iter().zip(tris) {
        let n = norm(cross(sub(world[1], world[0]), sub(world[2], world[0])));
        let facing = dot(n, fwd);
        if facing <= 0.0 {
            continue; // back face
        }
        // Light fixed to the camera, so every panel is lit the same way.
        let nv = [dot(n, right), dot(n, up), facing];
        // Nearer faces lighter, so steps show even where faces are parallel.
        let mid = (t[0][2] + t[1][2] + t[2][2]) / 3.0;
        let depth = ((mid - zlo) / (zhi - zlo).max(1e-9)).clamp(0.0, 1.0);
        let shade = (0.25 + 0.75 * dot(nv, light).max(0.0)) * (0.45 + 0.55 * depth);
        let s = t.map(|p| {
            [
                (p[0] - cx) * scale + size as f64 / 2.0,
                size as f64 / 2.0 - (p[1] - cy) * scale,
                p[2],
            ]
        });
        let minx = s
            .iter()
            .map(|p| p[0])
            .fold(f64::MAX, f64::min)
            .floor()
            .max(0.0) as usize;
        let maxx = (s.iter().map(|p| p[0]).fold(f64::MIN, f64::max).ceil() as usize).min(size - 1);
        let miny = s
            .iter()
            .map(|p| p[1])
            .fold(f64::MAX, f64::min)
            .floor()
            .max(0.0) as usize;
        let maxy = (s.iter().map(|p| p[1]).fold(f64::MIN, f64::max).ceil() as usize).min(size - 1);
        let area =
            (s[1][0] - s[0][0]) * (s[2][1] - s[0][1]) - (s[2][0] - s[0][0]) * (s[1][1] - s[0][1]);
        if area.abs() < 1e-12 {
            continue;
        }
        for y in miny..=maxy {
            for x in minx..=maxx {
                let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                let w0 = ((s[1][0] - px) * (s[2][1] - py) - (s[2][0] - px) * (s[1][1] - py)) / area;
                let w1 = ((s[2][0] - px) * (s[0][1] - py) - (s[0][0] - px) * (s[2][1] - py)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let z = w0 * s[0][2] + w1 * s[1][2] + w2 * s[2][2];
                let k = y * size + x;
                if z > zbuf[k] {
                    zbuf[k] = z;
                    let o = y * stride + (x0 + x) * 3;
                    // A warm brick colour, darker on faces turned away from the light.
                    pixels[o] = (230.0 * shade) as u8;
                    pixels[o + 1] = (90.0 * shade) as u8;
                    pixels[o + 2] = (50.0 * shade) as u8;
                }
            }
        }
    }
}

pub fn render(stl: &[u8], path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let tris = triangles(stl);
    let size = 360;
    let views = [[-1.0, -1.6, 1.2], [-1.0, -1.6, -1.2], [1.0, 0.0, 0.0]];
    let width = size * views.len();
    let mut pixels = vec![245u8; width * size * 3];
    for (k, eye) in views.iter().enumerate() {
        panel(&tris, *eye, None, size, &mut pixels, width * 3, k * size);
    }
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width as u32, size as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&pixels)?;
    Ok(())
}

/// A panel: the direction toward the viewer, and optionally the box (mm) to frame.
pub struct View {
    pub eye: V3,
    pub focus: Option<(V3, V3)>,
}

/// Panels in a 2 × 2 grid.
pub fn render_views(
    tris: &[[V3; 3]],
    views: &[View],
    size: usize,
    path: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let (cols, rows) = (2, views.len().div_ceil(2));
    let (width, height) = (cols * size, rows * size);
    let mut pixels = vec![245u8; width * height * 3];
    for (k, v) in views.iter().enumerate() {
        let (col, row) = (k % cols, k / cols);
        let mut tile = vec![245u8; size * size * 3];
        panel(tris, v.eye, v.focus, size, &mut tile, size * 3, 0);
        for y in 0..size {
            let src = &tile[y * size * 3..(y + 1) * size * 3];
            let o = ((row * size + y) * width + col * size) * 3;
            pixels[o..o + size * 3].copy_from_slice(src);
        }
    }
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width as u32, height as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&pixels)?;
    Ok(())
}

/// One panel, `size` pixels square.
pub fn render_one(
    tris: &[[V3; 3]],
    view: &View,
    size: usize,
    path: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut pixels = vec![245u8; size * size * 3];
    panel(tris, view.eye, view.focus, size, &mut pixels, size * 3, 0);
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), size as u32, size as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&pixels)?;
    Ok(())
}
