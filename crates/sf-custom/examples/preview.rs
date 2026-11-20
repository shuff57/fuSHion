//! Builds sample custom parts, writes their STLs and renders each to a PNG.
//!
//! `cargo run -p sf-custom --example preview [out_dir]` (default `/tmp/fushion-custom`).
//! Each PNG has three orthographic panels: from front-left above (studs, slope direction),
//! from below (hollow, tubes) and from the right side (profile). The Slant v2 bricks and the
//! standard 2×4 also get `<name>_views.png`: straight down, from above, from below, and a
//! close-up of one peanut (or, for the standard brick, of the centre rib from below).
//! Faces nearer the viewer are drawn lighter, so slots and pockets show in the top view.
//! It also draws `v2_fit_overlay.svg`: one v2 peanut from above with the tubes and walls of a
//! brick pushed on, aligned and one row to the fin side, labelled with the clearances and the
//! fin's preload (convert it with any SVG renderer, e.g. `magick v2_fit_overlay.svg
//! v2_fit_overlay.png`).

use sf_custom::{
    CustomShape, StudProfile, TopBrick, analyze_stl, build, fit, slant_dims, slant_outline,
};
use std::fmt::Write as _;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "/tmp/fushion-custom".into()),
    );
    std::fs::create_dir_all(&out)?;
    let std = StudProfile::standard_fdm();
    let v2 = StudProfile::slant_v2_experimental();
    let svg = out.join("v2_fit_overlay.svg");
    std::fs::write(&svg, fit_overlay(&v2)?)?;
    println!("fit overlay -> {}", svg.display());
    let parts: Vec<(&str, CustomShape, StudProfile)> = vec![
        (
            "box_2x4x3",
            CustomShape::Box {
                w: 2,
                d: 4,
                plates: 3,
            },
            std,
        ),
        (
            "plate_2x2x1",
            CustomShape::Box {
                w: 2,
                d: 2,
                plates: 1,
            },
            std,
        ),
        (
            "slope45_2x2x3",
            CustomShape::Slope45 {
                w: 2,
                d: 2,
                plates: 3,
            },
            std,
        ),
        (
            "slope45_2x3x3",
            CustomShape::Slope45 {
                w: 2,
                d: 3,
                plates: 3,
            },
            std,
        ),
        (
            "curve_2x3x3",
            CustomShape::Curve {
                w: 2,
                d: 3,
                plates: 3,
            },
            std,
        ),
        (
            "footprint_l_x3",
            CustomShape::Footprint {
                cells: vec![(0, 0), (1, 0), (2, 0), (0, 1), (1, 1), (0, 2), (1, 2)],
                plates: 3,
            },
            std,
        ),
        (
            "v2_box_2x2x3",
            CustomShape::Box {
                w: 2,
                d: 2,
                plates: 3,
            },
            v2,
        ),
        (
            "v2_box_2x4x3",
            CustomShape::Box {
                w: 2,
                d: 4,
                plates: 3,
            },
            v2,
        ),
    ];
    for (name, shape, profile) in parts {
        let t = std::time::Instant::now();
        let part = build(&shape, &profile)?;
        let report = analyze_stl(&part.stl)?;
        std::fs::write(out.join(format!("{name}.stl")), &part.stl)?;
        std::fs::write(out.join(format!("{name}.3mf")), &part.three_mf)?;
        let png = out.join(format!("{name}.png"));
        render(&part.stl, &png)?;
        if name.starts_with("v2_") || name == "box_2x4x3" {
            let tris = triangles(&part.stl);
            let v = |eye: V3, focus: Option<(V3, V3)>| View { eye, focus };
            let views: Vec<View> = if name.starts_with("v2_") {
                // Row 0's lower peanut: its fins face −X.
                let peanut = ([0.2, -0.2, 9.0], [7.8, 16.2, 11.4]);
                vec![
                    v([0.0, 0.0, 1.0], None),
                    v([-1.0, -1.6, 1.2], None),
                    v([-1.0, -1.6, -1.2], None),
                    v([0.0, 0.0, 1.0], Some(peanut)),
                    v([-1.0, -0.8, 1.6], Some(peanut)),
                    v(
                        [-1.0, -0.25, 0.3],
                        Some(([0.0, 0.0, 8.6], [8.0, 16.0, 11.4])),
                    ),
                ]
            } else {
                vec![
                    v([0.0, 0.0, 1.0], None),
                    v([-1.0, -1.6, 1.2], None),
                    v([-1.0, -1.6, -1.2], None),
                    v(
                        [-0.6, -0.5, -1.0],
                        Some(([0.0, 10.0, 0.0], [16.0, 22.0, 9.6])),
                    ),
                ]
            };
            let png = out.join(format!("{name}_views.png"));
            render_views(&tris, &views, 520, &png)?;
            println!("  -> {}", png.display());
        }
        println!(
            "{name}: {:.2} mm³, bbox {:?}, {} triangles, watertight {}, {:.1} s -> {}",
            part.volume_mm3,
            part.bbox_mm,
            report.triangles,
            report.is_watertight(),
            t.elapsed().as_secs_f64(),
            png.display()
        );
    }
    Ok(())
}

type V3 = [f64; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn norm(a: V3) -> V3 {
    let l = dot(a, a).sqrt().max(1e-12);
    [a[0] / l, a[1] / l, a[2] / l]
}

fn triangles(stl: &[u8]) -> Vec<[V3; 3]> {
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
fn panel(
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

fn render(stl: &[u8], path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
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
struct View {
    eye: V3,
    focus: Option<(V3, V3)>,
}

/// Panels in a 2 × 2 grid.
fn render_views(
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

/// The fit overlay: one peanut in its own frame (fin side to the right, lobe B up), in two
/// panels: the brick on top aligned, and one row to the fin side.
fn fit_overlay(p: &StudProfile) -> Result<String, Box<dyn std::error::Error>> {
    let o = slant_outline(p)?;
    let dims = slant_dims(p)?;
    let a = p.pitch / 2.0;
    let (ro, ri) = (p.tube_outer_diameter / 2.0, p.tube_inner_diameter / 2.0);
    let wall_in = a - p.clearance - p.wall;
    let wall_out = a - p.clearance;
    // The fit reports these panels stand for (a 2 × 2 part, row 0, whose fins face −X).
    let part = CustomShape::Box {
        w: 2,
        d: 4,
        plates: 3,
    };
    let aligned = fit(
        &part,
        p,
        &TopBrick {
            w: 2,
            d: 4,
            at: (0, 0),
        },
    )?;
    let sideways = fit(
        &part,
        p,
        &TopBrick {
            w: 2,
            d: 4,
            at: (-1, 0),
        },
    )?;
    println!("aligned: {aligned:?}\nsideways: {sideways:?}");

    // Least clearance from the outline (body or fin) to a tube at `c`, and how far past the
    // line u = `x` (toward +u when `right`) it reaches.
    let tube = |c: [f64; 2], fin: bool| {
        o.edge
            .iter()
            .filter(|e| e.1 == fin)
            .map(|e| (e.0[0] - c[0]).hypot(e.0[1] - c[1]) - ro)
            .fold(f64::INFINITY, f64::min)
    };
    let reach = |fin: bool, right: bool| {
        o.edge
            .iter()
            .filter(|e| e.1 == fin)
            .map(|e| if right { e.0[0] } else { -e.0[0] })
            .fold(f64::NEG_INFINITY, f64::max)
    };
    let top_end = o
        .edge
        .iter()
        .map(|e| e.0[1])
        .fold(f64::NEG_INFINITY, f64::max);

    let (sc, pw, ph) = (44.0, 640.0, 860.0);
    let mut svg = String::new();
    let width = 2.0 * pw + 40.0;
    let height = ph + 230.0;
    writeln!(
        svg,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" font-family="DejaVu Sans, sans-serif">
<rect width="100%" height="100%" fill="#fbfaf7"/>"##
    )?;
    let panels: [(&str, f64, &sf_custom::FitReport); 2] = [
        ("Brick on top aligned", -1.0, &aligned),
        ("Brick on top one row over the fin side", 1.0, &sideways),
    ];
    for (k, (title, side, rep)) in panels.iter().enumerate() {
        let ox = 20.0 + k as f64 * (pw + 20.0);
        let cx = ox + pw / 2.0;
        let cy = 430.0;
        let x = |u: f64| cx + u * sc;
        let y = |v: f64| cy - v * sc;
        writeln!(
            svg,
            r##"<g><rect x="{ox}" y="20" width="{pw}" height="{ph}" fill="#ffffff" stroke="#d8d4cc"/>
<text x="{}" y="48" font-size="22" font-weight="bold" fill="#222">{title}</text>"##,
            ox + 16.0
        )?;
        // Grid lines through the stud centres and the cell edges.
        for v in [-8.0, -4.0, 0.0, 4.0, 8.0] {
            writeln!(
                svg,
                r##"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="#eee" stroke-dasharray="4 4"/>"##,
                ox,
                y(v),
                ox + pw,
                y(v)
            )?;
        }
        // The brick on top, in this peanut's frame: the part's peanut over cells (0, 0) and
        // (0, 1) has its waist at (4, 8) and its fins toward −X, so u = 4 − X, v = Y − 8.
        let top = if *side < 0.0 { (0, 0) } else { (-1, 0) };
        let to = |px: f64, py: f64| [4.0 - px, py - 8.0];
        let (bx0, by0) = (f64::from(top.0) * p.pitch, f64::from(top.1) * p.pitch);
        let (bx1, by1) = (bx0 + 2.0 * p.pitch, by0 + 4.0 * p.pitch);
        let ring = |inset: f64| {
            let lo = to(bx1 - inset, by0 + inset);
            let hi = to(bx0 + inset, by1 - inset);
            format!(
                "M{:.2},{:.2} L{:.2},{:.2} L{:.2},{:.2} L{:.2},{:.2} Z ",
                x(lo[0]),
                y(lo[1]),
                x(hi[0]),
                y(lo[1]),
                x(hi[0]),
                y(hi[1]),
                x(lo[0]),
                y(hi[1])
            )
        };
        writeln!(
            svg,
            r##"<clipPath id="c{k}"><rect x="{ox}" y="60" width="{pw}" height="{}"/></clipPath>
<g clip-path="url(#c{k})"><path d="{}{}" fill="#9db4d6" fill-opacity="0.55" stroke="#4a6fa5" fill-rule="evenodd"/>"##,
            ph - 120.0,
            ring(p.clearance),
            ring(p.clearance + p.wall)
        )?;
        let mut tubes = Vec::new();
        for i in 1..2 {
            for j in 1..4 {
                tubes.push(to(
                    bx0 + f64::from(i) * p.pitch,
                    by0 + f64::from(j) * p.pitch,
                ));
            }
        }
        for t in &tubes {
            writeln!(
                svg,
                r##"<circle cx="{}" cy="{}" r="{}" fill="#9db4d6" fill-opacity="0.55" stroke="#4a6fa5"/>
<circle cx="{}" cy="{}" r="{}" fill="#ffffff" stroke="#4a6fa5"/>"##,
                x(t[0]),
                y(t[1]),
                ro * sc,
                x(t[0]),
                y(t[1]),
                ri * sc
            )?;
        }
        writeln!(svg, "</g>")?;
        let tu = if *side < 0.0 { -a } else { a };
        writeln!(
            svg,
            r##"<circle cx="{}" cy="{}" r="{}" fill="none" stroke="#4a6fa5" stroke-dasharray="3 3"/>"##,
            x(tu),
            y(0.0),
            dims.waist_radius * sc
        )?;
        // Pockets (dashed), the peanut, its slots.
        for pk in &o.pockets {
            writeln!(
                svg,
                r##"<path d="{}" fill="none" stroke="#8a6d3b" stroke-dasharray="5 3"/>"##,
                path(pk, &x, &y)
            )?;
        }
        writeln!(
            svg,
            r##"<path d="{}" fill="#e2683c" fill-opacity="0.85" stroke="#7a2e12" stroke-width="1.2"/>"##,
            path(&o.peanut, &x, &y)
        )?;
        for sl in &o.slots {
            writeln!(
                svg,
                r##"<path d="{}" fill="#ffffff" stroke="#7a2e12" stroke-width="1"/>"##,
                path(sl, &x, &y)
            )?;
        }
        // Fin edges.
        for e in o.edge.iter().filter(|e| e.1).step_by(3) {
            writeln!(
                svg,
                r##"<circle cx="{:.2}" cy="{:.2}" r="1.6" fill="#1f7a3a"/>"##,
                x(e.0[0]),
                y(e.0[1])
            )?;
        }
        // Labels.
        let label = |svg: &mut String,
                     at: (f64, f64),
                     anchor: &str,
                     lines: &[String]|
         -> std::fmt::Result {
            for (i, l) in lines.iter().enumerate() {
                writeln!(
                    svg,
                    r##"<text x="{:.1}" y="{:.1}" font-size="15" text-anchor="{anchor}" fill="#111">{l}</text>"##,
                    at.0,
                    at.1 + 18.0 * i as f64
                )?;
            }
            Ok(())
        };
        let waist = tube([tu, 0.0], false).min(tube([tu, 0.0], true));
        if *side < 0.0 {
            let pre = reach(true, true) - wall_in;
            label(
                &mut svg,
                (x(wall_out) + 6.0, y(1.2)),
                "start",
                &[
                    "fin presses wall".into(),
                    format!("<tspan font-weight=\"bold\">{pre:.3} mm</tspan>"),
                    format!("(reach {:.2}, face {wall_in:.2})", dims.fin_reach),
                ],
            )?;
            label(
                &mut svg,
                (x(-a) + 4.0, y(0.0) - 6.0),
                "middle",
                &[format!("waist ↔ tube {waist:.3}")],
            )?;
            let diag = tube([-a, 2.0 * a], false);
            label(
                &mut svg,
                (x(-a) + 4.0, y(2.0 * a) + 4.0),
                "middle",
                &[
                    format!("lobe ↔ tube {diag:.4}"),
                    "(tangent, as Ø4.8)".into(),
                ],
            )?;
        } else {
            let inner = wall_in - reach(false, false);
            let fin_t = tube([a, 2.0 * a], true);
            let cap_t = tube([a, 2.0 * a], false);
            label(
                &mut svg,
                (x(-wall_out) - 6.0, y(1.2)),
                "end",
                &[
                    "inner side ↔ wall".into(),
                    format!("<tspan font-weight=\"bold\">{inner:.3} mm</tspan>"),
                ],
            )?;
            label(
                &mut svg,
                (x(a) - 4.0, y(0.0) - 6.0),
                "middle",
                &[format!("waist ↔ tube {waist:.3}")],
            )?;
            label(
                &mut svg,
                (x(a) - 4.0, y(2.0 * a) + 4.0),
                "middle",
                &[
                    format!("cap ↔ tube {cap_t:.4}"),
                    format!("fin ↔ tube {fin_t:.3}"),
                    "fin free (no preload)".into(),
                ],
            )?;
        }
        let end = 2.0 * a - p.clearance - p.wall - top_end;
        label(
            &mut svg,
            (ox + 16.0, 40.0 + ph - 54.0),
            "start",
            &[
                format!(
                    "whole part: body ↔ tubes {:+.4}, body ↔ walls {:+.3}, end walls {end:.3}",
                    rep.body_tube_clearance, rep.body_wall_clearance
                ),
                format!(
                    "fins → walls {:.3}, fins → tubes {:.3} (budget 0.45)",
                    rep.fin_wall_preload, rep.fin_tube_preload
                ),
            ],
        )?;
        writeln!(svg, "</g>")?;
    }
    // Construction notes.
    let d = dims;
    let notes = [
        format!(
            "Lobe: stud circle r {:.2} on the inner side and far end; fin bump r {:.3} about ({:.3}, {:.3}) peaking at u {:.2}, v {:.2};",
            d.stud_radius,
            d.bump_radius,
            d.bump_center[0],
            d.bump_center[1],
            d.fin_reach,
            d.fin_peak[1]
        ),
        format!(
            "blend r {:.3} about ({:.3}, {:.3}) from ({:.3}, {:.3}) to the stud circle at 40° ({:.3}, {:.3}); inner round r {:.2}. All junctions tangent.",
            d.blend_radius,
            d.blend_center[0],
            d.blend_center[1],
            d.blend_start[0],
            d.blend_start[1],
            d.blend_end[0],
            d.blend_end[1],
            d.round_radius
        ),
        format!(
            "Waist arcs r {:.3} about the tube points: waist {:.3}. Slot {:.2} wide at u {:.2}–{:.2}, from {:.3} to {:.3} past the waist line, hook r {:.2}; cap from {:.1}°.",
            d.waist_radius,
            d.waist_width,
            d.slot_width,
            d.slot_center - d.slot_width / 2.0,
            d.slot_center + d.slot_width / 2.0,
            d.root_length,
            d.slot_v.1 + a,
            d.hook_radius,
            d.cap_angle
        ),
        format!(
            "Fin {:.2} wide at most, root {:.2} across at the slot's end, least section to the waist {:.3}. Pocket out to u {:.3}. Lobe frame: u to the fin side, v away from the waist (mm).",
            d.fin_width, d.root_width, d.min_section, d.pocket_out
        ),
    ];
    for (i, n) in notes.iter().enumerate() {
        writeln!(
            svg,
            r##"<text x="24" y="{}" font-size="15" fill="#333">{n}</text>"##,
            ph + 70.0 + 26.0 * i as f64
        )?;
    }
    writeln!(
        svg,
        r##"<text x="24" y="{}" font-size="15" fill="#333">Green: fin edges (may press up to 0.45 mm). Blue: walls and tubes of the brick on top; dashed circle: waist clearance (r {:.3}). Brown dashed: pockets.</text>"##,
        ph + 70.0 + 26.0 * notes.len() as f64,
        dims.waist_radius
    )?;
    writeln!(svg, "</svg>")?;
    Ok(svg)
}

/// An SVG path through a closed polyline.
fn path(pts: &[[f64; 2]], x: &dyn Fn(f64) -> f64, y: &dyn Fn(f64) -> f64) -> String {
    let mut d = String::new();
    for (i, q) in pts.iter().enumerate() {
        let _ = write!(
            d,
            "{}{:.2},{:.2} ",
            if i == 0 { "M" } else { "L" },
            x(q[0]),
            y(q[1])
        );
    }
    d.push('Z');
    d
}
