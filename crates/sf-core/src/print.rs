//! Printed bricks: which catalog parts can be printed with fused studs, how their studs
//! group into **units**, and the one placement rule that fused studs add.
//!
//! A printed brick's studs are fused along each row into pairs and triples (see `sf-custom`).
//! Between two fused studs the stud narrows to a waist, so a brick pushed on top may not
//! have an end wall between them: a unit's cells must all lie under **one** part (or under
//! none). [`fused_splits`] finds the places where a model breaks this.
//!
//! The unit layout here mirrors `sf_custom`'s (`layout` and `split_run`); a test in
//! `sf-agent` checks the two agree for every printable catalog part.

use crate::{Cell, PartDef, Placement, Shape, parts};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// A part designed for one model and 3D printed, not taken from LEGO's catalog. Stored
/// inline in its [`Placement`] (`Placement::custom`); the placement's `part` is its `id`.
///
/// Local frame: columns `(lx, lz)` of the unrotated part, all `>= 0`, in studs; the part's
/// size is the bounding box of `cells`. `Placement::rot` turns it like a catalog part.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CustomDef {
    /// Stable id, unique in the model: also the print file's stem and the number stamped on
    /// the part (e.g. `"B017"` for a brick, `"D003"` for a detail).
    pub id: String,
    pub kind: CustomKind,
    /// The columns the part fills, from `y = origin.y` up `plates` plates.
    pub cells: Vec<(u8, u8)>,
    /// A detail kit's colours by role, besides "main" (the placement's colour).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub colors: BTreeMap<String, crate::ColorId>,
    /// Height in plates of the filled volume.
    pub plates: u8,
    /// Columns with a stud on top. For a brick these are chosen for its spot (see
    /// [`stud_layout`]); a detail usually has none.
    pub studs: Vec<(u8, u8)>,
    /// How `studs` are fused, each unit a row of 2 or 3 neighbouring columns in order. The
    /// engine chooses them for the part's spot (no unit under an edge of what sits on it),
    /// and the print must use exactly these. Empty: every unit by the standard rule.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub units: Vec<Vec<(u8, u8)>>,
    /// Columns that take studs from below (a brick: all its cells; a detail: its base).
    pub sockets: Vec<(u8, u8)>,
    /// A brick that follows the model's true curved surface where it is exposed (print
    /// mode with curves). `None`: the plain extruded cells.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trim: Option<Box<Trim>>,
    /// Key into `Model::meshes` of an approximate display mesh (trimmed bricks), in the same
    /// frame as catalog parts' meshes. `None`: draw from `cells`, `studs` and `units`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh: Option<String>,
}

/// The true surface a printed brick follows. Frame: millimetres in the brick's own print
/// frame, as `sf-custom` builds it: x along local `lx`, y along local `lz` (column
/// `(lx, lz)` spans `x ∈ [8·lx, 8·lx + 8]`, `y ∈ [8·lz, 8·lz + 8]`), z up from the bottom
/// of the body (`z ∈ [0, 3.2·plates]`; studs stand above).
///
/// The finished brick keeps exactly `block ∩ (S ∪ P)`, where `S` is the model's solid
/// (the `shapes`, applied in order: `Add` unions, `Subtract` removes, from empty) and `P`
/// the union of the `protect` boxes: everything that touches another part (a flat top
/// with studs under a part above, faces against neighbours, the bottom and its sockets)
/// stays as the plain brick. Boxes in `protect` reach past the brick's faces by at least
/// a wall's thickness, so an inward offset of `S ∪ P` still covers those faces.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Trim {
    pub shapes: Vec<TrimShape>,
    pub protect: Vec<TrimBox>,
    /// The kept solid has a downward-facing surface steeper than 45° (as measured on the
    /// display mesh): printing it studs up needs supports.
    #[serde(default)]
    pub overhang: bool,
}

/// One primitive of a [`Trim`]: `kind` in its own frame, placed by `rotation` (row-major,
/// local → brick) and `translation` (mm, brick frame).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrimShape {
    pub op: TrimOp,
    pub kind: TrimKind,
    pub rotation: [[f64; 3]; 3],
    pub translation: [f64; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrimOp {
    Add,
    Subtract,
}

/// Primitive shapes, centred on their own origin, axis along local z where they have one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TrimKind {
    /// Semi-axes `radii` along local x, y, z.
    Ellipsoid { radii: [f64; 3] },
    /// Full `size` along local x, y, z.
    Box { size: [f64; 3] },
    /// Elliptic cylinder: semi-axes `radii` in local x, y; `height` along z, centred.
    Cylinder { radii: [f64; 2], height: f64 },
    /// Elliptic cone or frustum from `radii` at z = −height/2 to `radii · top_scale` at
    /// z = +height/2.
    Cone { radii: [f64; 2], top_scale: f64, height: f64 },
    /// All points within `radius` of the segment from `a` to `b` (local frame).
    Capsule { a: [f64; 3], b: [f64; 3], radius: f64 },
    /// A simple polygon in local x, y extruded `height` along z, centred.
    Prism { points: Vec<[f64; 2]>, height: f64 },
}

/// An axis-aligned box, mm, brick frame.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrimBox {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

/// What a printed part is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CustomKind {
    /// A plain body part: `cells` extruded `plates` high, hollow underneath, fused studs on
    /// `studs`.
    Brick,
    /// A shaped part Claude designed in SolidCraft (a flower head, a petal, an eye, a beak):
    /// its display mesh is `Model::meshes[mesh]`, its print source `Model::details[mesh]`.
    Detail {
        /// Key into `Model::meshes` and `Model::details`; parts sharing a design share it.
        mesh: String,
        /// Short human name, e.g. "flower head".
        name: String,
    },
}

impl CustomDef {
    /// Size `[x, z]` of the local bounding box, in studs.
    pub fn size(&self) -> [u8; 2] {
        let mx = self.cells.iter().map(|c| c.0 + 1).max().unwrap_or(1);
        let mz = self.cells.iter().map(|c| c.1 + 1).max().unwrap_or(1);
        [mx, mz]
    }

    /// A short description, e.g. "printed brick B017, 6 studs, 3 plates".
    pub fn describe(&self) -> String {
        match &self.kind {
            CustomKind::Brick => format!(
                "printed {} {}, {} stud{} area, {} plate{}",
                if self.plates.is_multiple_of(3) {
                    "brick"
                } else {
                    "plate"
                },
                self.id,
                self.cells.len(),
                if self.cells.len() == 1 { "" } else { "s" },
                self.plates,
                if self.plates == 1 { "" } else { "s" }
            ),
            CustomKind::Detail { name, .. } => format!("printed {name} {}", self.id),
        }
    }
}

/// A triangle mesh for a detail part, in the same frame as catalog parts' LDraw meshes: LDU,
/// LDraw axes (`-Y` up), origin at the top centre of the part's local bounding box (so
/// `Placement::ldraw_transform` places it like any part). Counter-clockwise outside faces.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DetailMesh {
    pub positions: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
}

/// A detail part's design: its geometry, how it mounts on the grid, and how to print it.
/// Made by `sf-custom` (a template from its detail library, or a SolidCraft batch Claude
/// wrote), stored in the session, and placed by the engine wherever the spec's `details`
/// use it. Local frame as for [`CustomDef`]: `cells` are the columns the part occupies
/// (its bounding volume on the grid, used for collisions), `sockets` the columns of its
/// base that take studs from below. A socket base must cover at least two neighbouring
/// columns in a row, since a fused stud can't stand alone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DetailDesign {
    /// Key, unique in the session, e.g. `"flower_head_red"`.
    pub name: String,
    /// Short human name, e.g. "flower head".
    pub label: String,
    pub cells: Vec<(u8, u8)>,
    /// Height of the occupied volume, in plates (the part may poke a little past it).
    pub plates: u8,
    pub sockets: Vec<(u8, u8)>,
    /// Studs on top, if anything can be built on it (usually none).
    #[serde(default)]
    pub studs: Vec<(u8, u8)>,
    pub mesh: DetailMesh,
    pub source: DetailSource,
    /// The parts the detail is built from, when it is a kit (a flower from a hub, petals and
    /// a centre; an eye from a dome and a pupil). `None`: one printed piece made from
    /// `source`. `mesh` is always the assembled detail in its main colour, for quick views.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kit: Option<Kit>,
}

/// A detail built from separate printed parts, like the flowers of LEGO's Botanical
/// bouquets: petals clipped onto a hub with LEGO's 3.18 mm bar and clip, a centre pressed
/// in, a stem, leaves.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Kit {
    pub parts: Vec<KitPart>,
    /// The sub-build, in order: what to attach in each step.
    pub steps: Vec<KitStep>,
    /// Colour roles and their default colour names, e.g. `("main", "Red")`,
    /// `("centre", "Yellow")`, `("leaf", "Green")`. A use of the design sets "main"
    /// (the placement's colour) and may override any other role.
    pub roles: Vec<(String, String)>,
}

/// One kind of part in a kit, with every copy's place in the assembled detail.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KitPart {
    /// Key, unique in the kit, e.g. `"outer_petal"`.
    pub key: String,
    /// Short human name, e.g. "outer petal".
    pub label: String,
    /// Colour role (one of `Kit::roles`).
    pub role: String,
    /// One copy, in its own frame (LDU, LDraw axes, `-Y` up; origin where it connects).
    pub mesh: DetailMesh,
    /// How to print one copy, already laid out on the print bed in its best direction.
    pub source: DetailSource,
    /// Needs support material when printed as laid out.
    #[serde(default)]
    pub supports: bool,
    /// Where each copy goes in the assembled detail (the frame of `DetailDesign::mesh`).
    pub copies: Vec<KitPose>,
}

/// A rigid placement: `world = rotation · local + position` (LDU, rotation row-major).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct KitPose {
    pub rotation: [[f32; 3]; 3],
    pub position: [f32; 3],
}

/// One step of a kit's sub-build: the copies attached in it, as `(part index, copy index)`,
/// and an optional short note ("clip the outer petals onto the ring").
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct KitStep {
    pub parts: Vec<(usize, usize)>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

impl Kit {
    /// The colour of a role, given the main colour and any overrides; falls back to the
    /// role's default colour name resolved by `resolve`, then to `main`.
    pub fn role_color(
        &self,
        role: &str,
        main: crate::ColorId,
        overrides: &BTreeMap<String, crate::ColorId>,
        resolve: impl Fn(&str) -> Option<crate::ColorId>,
    ) -> crate::ColorId {
        if role == "main" {
            return main;
        }
        if let Some(&c) = overrides.get(role) {
            return c;
        }
        self.roles.iter().find(|(r, _)| r == role).and_then(|(_, name)| resolve(name)).unwrap_or(main)
    }
}

impl DetailDesign {
    /// The placement definition for one use of this design, numbered `id`.
    pub fn instance(&self, id: &str) -> CustomDef {
        CustomDef {
            id: id.to_string(),
            kind: CustomKind::Detail {
                mesh: self.name.clone(),
                name: self.label.clone(),
            },
            cells: self.cells.clone(),
            colors: BTreeMap::new(),
            trim: None,
            mesh: None,
            plates: self.plates,
            studs: self.studs.clone(),
            units: Vec::new(),
            sockets: self.sockets.clone(),
        }
    }
}

/// How to make a detail's print file: a SolidCraft command batch (`{"commands": [...],
/// "expect": {...}}`, millimetres, z up, the part standing on `z = 0` with its socket base
/// at the origin corner) or a template from `sf-custom`'s detail library.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DetailSource {
    pub source: serde_json::Value,
}

/// How a run of `len` studs (at least 2) splits into units: pairs, and for an odd run one
/// triple as near the middle as parity allows. 3 → T, 5 → P T, 7 → P T P, 9 → P P T P.
pub fn split_run(len: usize) -> Vec<usize> {
    if len.is_multiple_of(2) {
        return vec![2; len / 2];
    }
    let k = (len - 3) / 2;
    let before = k.div_ceil(2);
    let mut out = vec![2; before];
    out.push(3);
    out.extend(std::iter::repeat_n(2, k - before));
    out
}

/// The fused units on top of a part in its unrotated local frame: each unit is its columns
/// `(lx, lz)` in row order. `None` when a stud would stand alone in its row (a 1×1, or a
/// slope with a single stud): such a part can't be printed.
///
/// Rows run along the longer side of the stud area (ties: along `z`); if that leaves a lone
/// stud the other axis is tried, as `sf_custom` does.
pub fn units_local(def: &PartDef) -> Option<Vec<Vec<(u8, u8)>>> {
    let mut studs: Vec<(u8, u8)> = Vec::new();
    for lz in 0..def.size[1] {
        for lx in 0..def.size[0] {
            if def.stud_on_top(lx, lz) {
                studs.push((lx, lz));
            }
        }
    }
    if studs.is_empty() {
        return None;
    }
    let span = |k: fn(&(u8, u8)) -> u8| {
        let lo = studs.iter().map(k).min().unwrap_or(0);
        let hi = studs.iter().map(k).max().unwrap_or(0);
        hi - lo
    };
    let first = span(|c| c.1) >= span(|c| c.0);
    'axis: for along_z in [first, !first] {
        // Rows, then runs of neighbours along them.
        let key = |c: &(u8, u8)| if along_z { (c.0, c.1) } else { (c.1, c.0) };
        let mut order = studs.clone();
        order.sort_by_key(key);
        let mut runs: Vec<Vec<(u8, u8)>> = Vec::new();
        for c in order {
            let prev = |last: &(u8, u8)| {
                if along_z {
                    (last.0, last.1 + 1) == c
                } else {
                    (last.0 + 1, last.1) == c
                }
            };
            match runs.last_mut() {
                Some(run) if run.last().is_some_and(prev) => run.push(c),
                _ => runs.push(vec![c]),
            }
        }
        if runs.iter().any(|r| r.len() == 1) {
            continue 'axis;
        }
        let mut units = Vec::new();
        for run in runs {
            let mut i = 0;
            for n in split_run(run.len()) {
                units.push(run[i..i + n].to_vec());
                i += n;
            }
        }
        return Some(units);
    }
    None
}

/// Fused units (each a row of grid columns) and the columns left flat.
pub type StudLayout = (Vec<Vec<(i32, i32)>>, Vec<(i32, i32)>);

/// Groups a part's stud columns into fused units (rows of pairs, one triple per odd run)
/// so that **no unit straddles an edge of what sits on it**: `above(x, z)` names the part
/// over each column (`None` when nothing is there). Runs are split wherever the part above
/// changes; a piece of run one stud long gets no stud (its column is left flat), since a
/// stud can't stand alone. Returns the units and the columns left without a stud.
///
/// `studs` are grid columns; rows run along `x` when `along_x`, else along `z`.
pub fn stud_layout(
    studs: &[(i32, i32)],
    along_x: bool,
    above: impl Fn(i32, i32) -> Option<usize>,
) -> StudLayout {
    let key = |c: &(i32, i32)| if along_x { (c.1, c.0) } else { (c.0, c.1) };
    let mut order = studs.to_vec();
    order.sort_by_key(key);
    order.dedup();
    let next = |c: (i32, i32)| {
        if along_x {
            (c.0 + 1, c.1)
        } else {
            (c.0, c.1 + 1)
        }
    };
    let mut runs: Vec<Vec<(i32, i32)>> = Vec::new();
    for c in order {
        match runs.last_mut() {
            Some(run)
                if run.last().is_some_and(|&l| next(l) == c)
                    && above(l_of(run).0, l_of(run).1) == above(c.0, c.1) =>
            {
                run.push(c)
            }
            _ => runs.push(vec![c]),
        }
    }
    fn l_of(run: &[(i32, i32)]) -> (i32, i32) {
        *run.last().expect("runs are never empty")
    }
    let mut units = Vec::new();
    let mut flat = Vec::new();
    for run in runs {
        if run.len() == 1 {
            flat.push(run[0]);
            continue;
        }
        let mut i = 0;
        for n in split_run(run.len()) {
            units.push(run[i..i + n].to_vec());
            i += n;
        }
    }
    (units, flat)
}

/// Can this catalog part be printed with fused studs? Plain bricks and plates (not a 1×1)
/// and slopes with at least two studs (the 2×2 slope). Tiles, inverted slopes, cheese
/// slopes and the 2×1 slope have no printable form yet.
pub fn printable(def: &PartDef) -> bool {
    match def.shape {
        Shape::Box => units_local(def).is_some(),
        Shape::Slope { high } => high >= 1 && def.size[0] >= 2 && units_local(def).is_some(),
        Shape::Tile | Shape::InvertedSlope { .. } => false,
    }
}

/// [`printable`] for a part id (`false` for unknown ids).
pub fn part_printable(id: &str) -> bool {
    parts::part(id).is_some_and(printable)
}

impl Placement {
    /// The fused units on top of this part, as grid `(x, z)` columns (studs sit at
    /// `y = top()`). Empty for parts without printable studs.
    pub fn stud_units(&self) -> Vec<Vec<(i32, i32)>> {
        if let Some(c) = &self.custom {
            if !c.units.is_empty() {
                return c
                    .units
                    .iter()
                    .map(|u| u.iter().map(|&(lx, lz)| self.local_to_grid(lx, lz)).collect())
                    .collect();
            }
            let studs = self.top_studs();
            let [fx, fz] = self.footprint();
            return stud_layout(&studs, fx > fz, |_, _| None).0;
        }
        let Some(def) = self.def() else {
            return Vec::new();
        };
        let Some(units) = units_local(def) else {
            return Vec::new();
        };
        units
            .into_iter()
            .map(|u| {
                u.into_iter()
                    .map(|(lx, lz)| self.local_to_grid(lx, lz))
                    .collect()
            })
            .collect()
    }
}

/// A fused unit with a brick end between its studs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FusedSplit {
    /// Index of the part carrying the unit.
    pub lower: usize,
    /// The unit's columns.
    pub unit: Vec<(i32, i32)>,
}

/// Every fused unit whose cells (the level just above its part) are not all held by the
/// same part, or all empty. Each is a brick end resting on a waist, which will not seat.
pub fn fused_splits(ps: &[Placement]) -> Vec<FusedSplit> {
    let mut at: HashMap<Cell, usize> = HashMap::new();
    for (i, p) in ps.iter().enumerate() {
        for c in p.cells() {
            at.insert(c, i);
        }
    }
    let mut out = Vec::new();
    for (i, p) in ps.iter().enumerate() {
        let y = p.top();
        for unit in p.stud_units() {
            let mut it = unit
                .iter()
                .map(|&(x, z)| at.get(&Cell::new(x, y, z)).copied());
            let first = it.next().flatten();
            if it.any(|o| o != first) {
                out.push(FusedSplit { lower: i, unit });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Rot;

    fn def(id: &str) -> &'static PartDef {
        parts::part(id).unwrap()
    }

    #[test]
    fn units_follow_the_long_row() {
        // 2 × 4 brick (size [4, 2]): rows along x, two rows of two pairs.
        let u = units_local(def("3001")).unwrap();
        assert_eq!(u.len(), 4);
        assert!(
            u.iter()
                .all(|g| g.len() == 2 && g[0].1 == g[1].1 && g[1].0 == g[0].0 + 1)
        );
        // 1 × 5-ish: 1 × 6 is three pairs; 1 × 3 is one triple.
        assert_eq!(units_local(def("3009")).unwrap().len(), 3);
        let t = units_local(def("3622")).unwrap();
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].len(), 3);
        // 2 × 2: rows along z (a tie), two pairs.
        let sq = units_local(def("3003")).unwrap();
        assert_eq!(sq.len(), 2);
        assert!(sq.iter().all(|g| g[0].0 == g[1].0));
    }

    #[test]
    fn printable_parts() {
        assert!(!part_printable("3005"), "1 × 1 brick has a lone stud");
        assert!(!part_printable("3024"), "1 × 1 plate has a lone stud");
        assert!(!part_printable("3040b"), "2 × 1 slope has one stud");
        assert!(!part_printable("3665b"), "inverted slope");
        assert!(!part_printable("54200"), "cheese slope");
        assert!(!part_printable("3070b"), "tile");
        for id in [
            "3001", "3004", "3622", "3003", "3023", "3020", "3958", "3039",
        ] {
            assert!(part_printable(id), "{id}");
        }
    }

    #[test]
    fn units_follow_rotation() {
        let p = Placement::new("3001", 4, Cell::new(2, 0, 3), Rot(1));
        let units = p.stud_units();
        assert_eq!(units.len(), 4);
        for g in &units {
            // Rotated a quarter turn the rows run along z.
            assert_eq!(g[0].0, g[1].0);
            assert_eq!((g[1].1 - g[0].1).abs(), 1);
        }
    }

    #[test]
    fn a_brick_ending_between_two_fused_studs_is_a_split() {
        // A 1 × 2 brick over cells x 0..2; a 1 × 1 plate-ish part ending between them.
        let low = Placement::new("3004", 4, Cell::new(0, 0, 0), Rot(0));
        let a = Placement::new("3024", 4, Cell::new(0, 3, 0), Rot(0));
        let ps = [low.clone(), a];
        assert_eq!(fused_splits(&ps).len(), 1);
        // One 1 × 2 over both is fine, and so is nothing at all.
        let ok = Placement::new("3004", 4, Cell::new(0, 3, 0), Rot(0));
        assert!(fused_splits(&[low.clone(), ok]).is_empty());
        assert!(fused_splits(&[low]).is_empty());
    }

    fn l_brick() -> CustomDef {
        // An L: three along x, one more at (0, 1).
        CustomDef {
            id: "B001".into(),
            kind: CustomKind::Brick,
            cells: vec![(0, 0), (1, 0), (2, 0), (0, 1)],
            colors: BTreeMap::new(),
            trim: None,
            mesh: None,
            plates: 3,
            studs: vec![(1, 0), (2, 0)],
            units: vec![vec![(1, 0), (2, 0)]],
            sockets: vec![(0, 0), (1, 0), (2, 0), (0, 1)],
        }
    }

    #[test]
    fn printed_parts_fill_only_their_cells_and_turn() {
        let p = Placement::printed(l_brick(), 4, Cell::new(10, 0, 20), Rot(0));
        assert_eq!(p.footprint(), [3, 2]);
        assert_eq!(p.cells().count(), 12);
        assert!(
            !p.cells().any(|c| c == Cell::new(11, 0, 21)),
            "the L's notch is empty"
        );
        assert_eq!(p.top_studs(), vec![(11, 20), (12, 20)]);
        assert!(p.def().is_none());
        let r = Placement::printed(l_brick(), 4, Cell::new(0, 0, 0), Rot(1));
        assert_eq!(r.footprint(), [2, 3]);
        assert_eq!(r.cells().count(), 12);
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<Placement>(&json).unwrap(), r);
    }

    #[test]
    fn stud_layout_splits_runs_under_edges_above() {
        // A run of 6 along x under two parts above (x 0..3 → part 1, x 3..6 → part 2):
        // 3 + 3 → two triples; a run of 5 under 1 + 4 → one flat column and two pairs.
        let studs: Vec<(i32, i32)> = (0..6).map(|x| (x, 0)).collect();
        let (u, flat) = stud_layout(&studs, true, |x, _| Some(if x < 3 { 1 } else { 2 }));
        assert_eq!(u.iter().map(Vec::len).collect::<Vec<_>>(), vec![3, 3]);
        assert!(flat.is_empty());
        let studs: Vec<(i32, i32)> = (0..5).map(|x| (x, 0)).collect();
        let (u, flat) = stud_layout(&studs, true, |x, _| Some(if x < 1 { 1 } else { 2 }));
        assert_eq!(flat, vec![(0, 0)]);
        assert_eq!(u.iter().map(Vec::len).collect::<Vec<_>>(), vec![2, 2]);
        // Nothing above: one run.
        let (u, _) = stud_layout(&studs, true, |_, _| None);
        assert_eq!(u.iter().map(Vec::len).sum::<usize>(), 5);
        // No unit ever straddles a change above.
        for cut in 1..5 {
            let (u, _) = stud_layout(&studs, true, |x, _| Some((x >= cut) as usize));
            for g in u {
                assert!(g.iter().all(|c| (c.0 >= cut) == (g[0].0 >= cut)));
            }
        }
    }
}
