//! Print files for a fused brick: STEP (for CAD), STL and 3MF (for slicing).
//!
//! `cargo run --release -p sf-custom --example export -- <out_dir> [WxDxPLATES] [variant] [nozzle04]`
//!
//! `variant` is `centre_rib` (default), `wall_ribs` (end-wall ribs, no centre rib) or
//! `centre_and_wall_ribs`. `nozzle04` uses [`StudProfile::fused_nozzle04`] (0.6 mm slot,
//! 0.45 mm pocket band, same fin) for a 0.4 mm nozzle. The part is built and checked exactly as `sf_custom::build`
//! does (per-stage volumes, watertight mesh), then the same commands are replayed in a
//! fresh session to write the STEP.

use serde_json::json;
use sf_custom::{CustomShape, StudProfile, build, plan};
use solidcraft_engine::Session;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let out = PathBuf::from(args.next().unwrap_or_else(|| "/tmp/fushion-print".into()));
    let size = args.next().unwrap_or_else(|| "2x4x3".into());
    let variant = args.next().unwrap_or_else(|| "centre_rib".into());
    let dims: Vec<u32> = size.split('x').map(str::parse).collect::<Result<_, _>>()?;
    let [w, d, plates] = dims[..] else {
        return Err("size is WxDxPLATES, e.g. 2x4x3".into());
    };
    let base = StudProfile::fused();
    let profile = match variant.as_str() {
        "centre_rib" => base,
        "wall_ribs" => StudProfile {
            center_ribs: false,
            wall_ribs: true,
            ..base
        },
        "centre_and_wall_ribs" => StudProfile {
            wall_ribs: true,
            ..base
        },
        v => return Err(format!("unknown variant `{v}`").into()),
    };
    let nozzle04 = args.next().as_deref() == Some("nozzle04");
    let profile = if nozzle04 {
        let n = StudProfile::fused_nozzle04();
        StudProfile {
            center_ribs: profile.center_ribs,
            wall_ribs: profile.wall_ribs,
            ..n
        }
    } else {
        profile
    };
    let shape = CustomShape::Box { w, d, plates };
    std::fs::create_dir_all(&out)?;
    let name = format!(
        "fushion_{w}x{d}x{plates}_r{}_{variant}{}",
        profile.edge_radius,
        if nozzle04 { "_nozzle04" } else { "" }
    );

    // Checked build: STL + 3MF.
    let part = build(&shape, &profile)?;
    std::fs::write(out.join(format!("{name}.stl")), &part.stl)?;
    std::fs::write(out.join(format!("{name}.3mf")), &part.three_mf)?;

    // Same commands in a fresh session, exported as STEP.
    let mut s = Session::default();
    for stage in &plan(&shape, &profile)?.stages {
        for c in &stage.commands {
            let id = c["command"].as_str().ok_or("command without id")?;
            s.execute(id, &c["params"])
                .map_err(|e| format!("{id}: {e}"))?;
        }
    }
    let step = out.join(format!("{name}.step"));
    s.execute("file.export", &json!({"path": step.to_string_lossy()}))
        .map_err(|e| format!("STEP export: {e}"))?;
    println!(
        "{name}: {:.2} mm³, bbox {:?}..{:?} mm\n  {}\n  {}\n  {}",
        part.volume_mm3,
        part.bbox_mm.0,
        part.bbox_mm.1,
        step.display(),
        out.join(format!("{name}.stl")).display(),
        out.join(format!("{name}.3mf")).display()
    );
    Ok(())
}
