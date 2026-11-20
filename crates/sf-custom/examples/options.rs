//! Renders the two optional features (thin wall ribs, rounded vertical edges) for comparison.
//!
//! `cargo run -p sf-custom --example options [out_dir]` (default `/tmp/fushion-custom/options`)
//! builds a standard 2×4 brick four ways ((a) as is, (b) with wall ribs, (c) with edges
//! rounded to r 0.5, (d) both) and the Slant v2 2×4 with both, writes their STL and 3MF, and
//! renders one PNG per view into `out_dir/panels/`: `under_<x>.png` and `top_<x>.png` (every
//! brick at the same angle and zoom), `close_under_<x>.png` (one corner of the underside),
//! `close_top_<x>.png` (one corner from straight above), `rib_detail_<x>.png` (the foot of
//! one rib) and `v2_top.png`, `v2_under.png`,
//! `v2_close.png`. Lay them out and label them with any image tool.

use sf_custom::{CustomShape, StudProfile, build};
use std::path::{Path, PathBuf};

type V3 = [f64; 3];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "/tmp/fushion-custom/options".into()),
    );
    let panels = out.join("panels");
    std::fs::create_dir_all(&panels)?;
    let std = StudProfile::standard_fdm();
    let ribs = StudProfile {
        wall_ribs: true,
        ..std
    };
    let round = StudProfile {
        edge_radius: 0.5,
        ..std
    };
    let both = StudProfile {
        wall_ribs: true,
        edge_radius: 0.5,
        ..std
    };
    let v2 = StudProfile {
        wall_ribs: true,
        edge_radius: 0.5,
        ..StudProfile::slant_v2_experimental()
    };
    let brick = CustomShape::Box {
        w: 2,
        d: 4,
        plates: 3,
    };
    // The whole brick, framed the same in every panel.
    let whole = ([0.0, 0.0, 0.0], [16.0, 32.0, 11.4]);
    // One corner of the underside: the end wall and two studs' worth of the −X long wall.
    let corner_under = ([0.0, 0.0, 0.0], [5.0, 14.0, 9.6]);
    let corner_top = ([0.0, 0.0, 9.0], [5.0, 5.0, 11.4]);
    // From below, tipped so the −X long wall's inner face and the −Y end wall show (as in
    // LEGO's and Slant's own underside shots).
    let from_below: V3 = [0.5, -0.45, -1.0];
    let from_above: V3 = [-1.0, -1.6, 1.2];
    let close_below: V3 = [1.0, -0.3, -0.55];
    for (key, name, profile) in [
        ("a", "box_2x4x3_a_plain", std),
        ("b", "box_2x4x3_b_wall_ribs", ribs),
        ("c", "box_2x4x3_c_r0.5", round),
        ("d", "box_2x4x3_d_ribs_r0.5", both),
    ] {
        let part = build(&brick, &profile)?;
        println!("({key}) {name}: {:.3} mm³", part.volume_mm3);
        std::fs::write(out.join(format!("{name}.stl")), &part.stl)?;
        std::fs::write(out.join(format!("{name}.3mf")), &part.three_mf)?;
        let tris = triangles(&part.stl);
        let shot = |file: String, eye: V3, focus: (V3, V3), size: usize| {
            render(&tris, eye, focus, size, &panels.join(file))
        };
        shot(format!("under_{key}.png"), from_below, whole, 560)?;
        shot(format!("top_{key}.png"), from_above, whole, 560)?;
        shot(
            format!("close_under_{key}.png"),
            close_below,
            corner_under,
            560,
        )?;
        shot(
            format!("close_top_{key}.png"),
            [0.0, 0.0, 1.0],
            corner_top,
            360,
        )?;
        // The foot of the first rib on the −X wall (stud row 0), lead-in and all.
        shot(
            format!("rib_detail_{key}.png"),
            [1.0, -0.55, -0.45],
            ([0.0, 1.0, 0.0], [3.5, 7.0, 4.5]),
            560,
        )?;
    }
    let part = build(&brick, &v2)?;
    println!("v2 2x4, ribs + r 0.5: {:.3} mm³", part.volume_mm3);
    std::fs::write(out.join("v2_box_2x4x3_ribs_r0.5.stl"), &part.stl)?;
    std::fs::write(out.join("v2_box_2x4x3_ribs_r0.5.3mf"), &part.three_mf)?;
    let tris = triangles(&part.stl);
    render(&tris, from_above, whole, 620, &panels.join("v2_top.png"))?;
    render(&tris, from_below, whole, 620, &panels.join("v2_under.png"))?;
    render(
        &tris,
        close_below,
        corner_under,
        620,
        &panels.join("v2_close.png"),
    )?;
    Ok(())
}

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

/// An orthographic view looking along `-eye` (world +Z up on screen, or +Y when looking
/// straight down), framing the box `focus` (mm). Faces are lit from the camera's upper left
/// and drawn lighter when nearer, and every pixel where the depth or the face direction jumps
/// is outlined, so thin ribs and small rounds read clearly.
fn render(
    tris: &[[V3; 3]],
    eye: V3,
    focus: (V3, V3),
    size: usize,
    path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let fwd = norm(eye);
    let up0 = if fwd[2].abs() > 0.99 {
        [0.0, 1.0, 0.0]
    } else {
        [0.0, 0.0, 1.0]
    };
    let right = norm(cross(up0, fwd));
    let up = cross(fwd, right);
    let to_view = |p: V3| [dot(p, right), dot(p, up), dot(p, fwd)];
    let (a, b) = focus;
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for k in 0..8 {
        let c = to_view([
            if k & 1 == 0 { a[0] } else { b[0] },
            if k & 2 == 0 { a[1] } else { b[1] },
            if k & 4 == 0 { a[2] } else { b[2] },
        ]);
        for i in 0..2 {
            lo[i] = lo[i].min(c[i]);
            hi[i] = hi[i].max(c[i]);
        }
    }
    let (zlo, zhi) = tris
        .iter()
        .flatten()
        .map(|p| dot(*p, fwd))
        .fold((f64::MAX, f64::MIN), |(l, h), z| (l.min(z), h.max(z)));
    let scale = (size as f64 * 0.92) / (hi[0] - lo[0]).max(hi[1] - lo[1]);
    let (cx, cy) = ((lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0);
    let light = norm([-0.35, 0.55, 1.0]);
    let mut zbuf = vec![f64::MIN; size * size];
    let mut normal = vec![[0.0f64; 3]; size * size];
    let mut rgb = vec![[247u8, 246, 243]; size * size];
    for world in tris {
        let n = norm(cross(sub(world[1], world[0]), sub(world[2], world[0])));
        let facing = dot(n, fwd);
        if facing <= 0.0 {
            continue;
        }
        let nv = [dot(n, right), dot(n, up), facing];
        let t = world.map(to_view);
        let mid = (t[0][2] + t[1][2] + t[2][2]) / 3.0;
        let depth = ((mid - zlo) / (zhi - zlo).max(1e-9)).clamp(0.0, 1.0);
        let diffuse = dot(nv, light).max(0.0);
        let shade = (0.28 + 0.72 * diffuse) * (0.6 + 0.4 * depth);
        let col = [
            (226.0 * shade + 18.0) as u8,
            (96.0 * shade + 10.0) as u8,
            (58.0 * shade + 8.0) as u8,
        ];
        let s = t.map(|p| {
            [
                (p[0] - cx) * scale + size as f64 / 2.0,
                size as f64 / 2.0 - (p[1] - cy) * scale,
                p[2],
            ]
        });
        let bx = |k: usize, f: fn(f64, f64) -> f64, init: f64| s.iter().map(|p| p[k]).fold(init, f);
        let minx = bx(0, f64::min, f64::MAX).floor().max(0.0) as usize;
        let maxx = (bx(0, f64::max, f64::MIN).ceil().max(0.0) as usize).min(size - 1);
        let miny = bx(1, f64::min, f64::MAX).floor().max(0.0) as usize;
        let maxy = (bx(1, f64::max, f64::MIN).ceil().max(0.0) as usize).min(size - 1);
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
                    normal[k] = n;
                    rgb[k] = col;
                }
            }
        }
    }
    // Outlines: a depth step of more than 0.15 mm, or a crease of more than 30°.
    let mut out = rgb.clone();
    for y in 1..size - 1 {
        for x in 1..size - 1 {
            let k = y * size + x;
            let edge = [k - 1, k + 1, k - size, k + size].iter().any(|&q| {
                let (za, zb) = (zbuf[k], zbuf[q]);
                if za == f64::MIN || zb == f64::MIN {
                    return za != zb;
                }
                (za - zb).abs() > 0.15 || dot(normal[k], normal[q]) < 0.866
            });
            if edge {
                out[k] = [40, 22, 16];
            }
        }
    }
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), size as u32, size as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let bytes: Vec<u8> = out.into_iter().flatten().collect();
    enc.write_header()?.write_image_data(&bytes)?;
    Ok(())
}
