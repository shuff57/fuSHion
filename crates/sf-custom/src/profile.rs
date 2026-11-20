//! Connector dimensions: how a printed brick meets real bricks.

use crate::{CustomError, Result};
use serde::{Deserialize, Serialize};

/// The shape of the studs on top of a brick.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StudStyle {
    /// A plain solid cylinder, [`StudProfile::stud_diameter`] wide.
    Solid,
    /// **Experimental.** Slant 3D's "LEGO v2" stud, as shown in *We Made a Better LEGO (For 3D
    /// Printing)*, reshaped so it fits standard bricks: two neighbouring studs of a row along
    /// the long outside edge are fused into one peanut (two lobes joined by a narrow waist),
    /// and a leaf-spring fin is cut out of each lobe on the outer side. The fin is split from
    /// the stud by a narrow slot that hooks out through the lobe's outer edge, and a pocket in
    /// the brick's top frees its underside, so it hangs from its root near the waist and
    /// presses on the receiving brick's wall with a steady force. A stud left over at the end
    /// of a row (or on an inner row) stays a plain round [`StudProfile::stud_diameter`] stud.
    ///
    /// The outline is made of tangent arcs only. In each lobe's own frame (origin at the lobe
    /// centre, `u` toward the fin side, `v` away from the waist):
    /// * the **inner side** and the far end are the plain stud circle (`stud_diameter / 2`),
    ///   so the tubes of a brick pushed on sit tangent to it as on a round stud;
    /// * the **waist** is cut on both sides by arcs about the two tube positions beside it,
    ///   `tube_outer_diameter / 2 + waist_clearance` in radius, so a tube clears the waist
    ///   whether the brick on top is aligned or one row to the side;
    /// * on the **fin side** a convex arc tangent to the waist arc carries the fin out to
    ///   `u = fin_reach` at `v = -fin_peak`, and a wide arc tangent to it rejoins the stud
    ///   circle tangentially at `blend_angle` degrees, inside the slot's mouth, so the cap
    ///   beyond the hook stays on the stud circle;
    /// * on the inner side a round of radius `waist_round` joins the stud circle to the waist.
    ///
    /// The slot's centre line runs straight along `v` at `u = slot_offset + slot_width / 2`
    /// from where the fin's root is `root_width` wide to a hook: a 135° turn of radius
    /// `hook_radius` toward the fin side, then a short straight tail that leaves the lobe
    /// heading 45° back toward the waist. The hook's outer edge peaks [`SLANT_CAP`] mm (along
    /// `v`) short of the stud circle's far edge, so the stud keeps a cap there.
    SlantV2 {
        /// How far the fin reaches from the lobe centre across the row, in mm. A brick pushed
        /// on has its wall's inside face `pitch / 2 - clearance - wall` out (2.7 mm), so 2.85
        /// presses the fin 0.15 mm into its slot.
        fin_reach: f64,
        /// How far toward the waist from the lobe centre the fin reaches furthest, in mm.
        fin_peak: f64,
        /// Where the fin's outline rejoins the stud circle, in degrees from the fin direction
        /// toward the lobe's far end. It must lie inside the slot's mouth.
        blend_angle: f64,
        /// Gap left between the waist and a tube of a brick pushed on, in mm.
        waist_clearance: f64,
        /// Radius of the round joining a lobe's inner side to the waist, in mm.
        waist_round: f64,
        /// Width of the slot that frees each fin, in mm (also the gap the fin flexes into).
        slot_width: f64,
        /// How far the slot's inner edge lies outboard of the lobe's centre line, in mm.
        slot_offset: f64,
        /// Width of the fin's solid root, across the row from the slot's end to the waist
        /// arc, in mm. It sets where the slot starts.
        root_width: f64,
        /// Radius of the slot's hook, on its centre line, in mm.
        hook_radius: f64,
        /// Depth of the pocket cut into the brick's top under each fin, in mm.
        pocket_depth: f64,
    },
}

/// Solid cap a Slant v2 lobe keeps beyond its slot's hook, in mm.
pub const SLANT_CAP: f64 = 0.75;
/// Thinnest floor a Slant v2 pocket may leave above the hollow, in mm.
pub const MIN_POCKET_FLOOR: f64 = 0.8;
/// Thinnest solid a Slant v2 slot may leave to the waist, in mm (thinner does not print
/// reliably).
pub const MIN_FIN_SECTION: f64 = 0.5;

/// The connector dimensions of a printed brick, all in millimetres.
///
/// A brick `w × d` studs is `pitch·n − 2·clearance` wide on each axis and
/// `plates · plate_height` tall (without its studs).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StudProfile {
    /// Distance between neighbouring stud centres.
    pub pitch: f64,
    /// How far each outside wall is pulled in from the stud grid, per side.
    pub clearance: f64,
    /// Outside diameter of a stud.
    pub stud_diameter: f64,
    /// Height of a stud above the brick's top face.
    pub stud_height: f64,
    /// Height of one plate (a brick is three plates).
    pub plate_height: f64,
    /// Thickness of the solid top above the hollow underside.
    pub top_thickness: f64,
    /// Thickness of the side walls (and, on slopes, of the sloped roof).
    pub wall: f64,
    /// Outside diameter of the bottom tubes between each 2×2 group of studs.
    pub tube_outer_diameter: f64,
    /// Inside diameter of the bottom tubes.
    pub tube_inner_diameter: f64,
    /// Solid round studs, or Slant 3D's v2 peanut studs.
    pub stud_style: StudStyle,
    /// Centre ribs under 2-wide bricks: a cross-wall between the two long walls through every
    /// second tube, which keeps the long walls from bowing (see [`RIB_THICKNESS`]).
    #[serde(default = "yes")]
    pub center_ribs: bool,
    /// Chamfer on the bottom outer edges and the tube bottoms, against elephant's foot, in mm
    /// (0 for none).
    #[serde(default = "default_chamfer")]
    pub bottom_chamfer: f64,
}

/// Thickness of a centre rib, in mm.
pub const RIB_THICKNESS: f64 = 1.0;
/// Height of a centre rib's lower edge above the bottom of the brick, in mm: above a stud
/// (1.8 mm) pushed in from below, with room for elephant's foot.
pub const RIB_GAP: f64 = 2.2;

fn yes() -> bool {
    true
}

fn default_chamfer() -> f64 {
    0.2
}

impl Default for StudProfile {
    fn default() -> Self {
        Self::standard_fdm()
    }
}

impl StudProfile {
    /// The dimensions Slant 3D's online brick generator uses for FDM prints (read from its
    /// client-side code): pitch 8.0 mm, 0.1 mm inset per side, studs Ø4.8 × 1.8 mm, plate
    /// 3.2 mm (brick 9.6 mm), top 1.0 mm, walls 1.2 mm, tubes Ø6.51 / Ø4.8 mm. Like Slant's
    /// v2 brick, it also has centre ribs and a 0.2 mm bottom chamfer.
    pub fn standard_fdm() -> Self {
        Self {
            pitch: 8.0,
            clearance: 0.1,
            stud_diameter: 4.8,
            stud_height: 1.8,
            plate_height: 3.2,
            top_thickness: 1.0,
            wall: 1.2,
            tube_outer_diameter: 6.51,
            tube_inner_diameter: 4.8,
            stud_style: StudStyle::Solid,
            center_ribs: true,
            bottom_chamfer: default_chamfer(),
        }
    }

    /// **Experimental, not yet test-printed.** Slant 3D's "LEGO v2" peanut stud with
    /// leaf-spring fins (see [`StudStyle::SlantV2`]), in a geometry that fits standard
    /// bricks: lobes are the Ø4.8 mm stud circle on the inner side and reach 2.85 mm on the
    /// fin side (0.15 mm into the wall of a brick pushed on), the waist is cut 0.05 mm clear
    /// of the tubes beside it (1.39 mm wide), and 0.5 mm slots 0.25–0.75 mm outboard of each
    /// lobe's centre line leave a 0.8 mm fin root and hook out with a 1.2 mm hook, over
    /// 0.6 mm pockets. The top is 1.4 mm thick (instead of 1.0) so 0.8 mm of floor stays under
    /// each pocket. Everything else is [`StudProfile::standard_fdm`]. Print a test pair before
    /// trusting the clutch.
    pub fn slant_v2_experimental() -> Self {
        Self {
            top_thickness: 1.4,
            stud_style: StudStyle::SlantV2 {
                fin_reach: 2.85,
                fin_peak: 0.3,
                blend_angle: 40.0,
                waist_clearance: 0.05,
                waist_round: 0.5,
                slot_width: 0.5,
                slot_offset: 0.25,
                root_width: 0.8,
                hook_radius: 1.2,
                pocket_depth: 0.6,
            },
            ..Self::standard_fdm()
        }
    }

    /// The old name of [`StudProfile::slant_v2_experimental`].
    #[deprecated(note = "renamed to `slant_v2_experimental`")]
    pub fn compliant_v2_experimental() -> Self {
        Self::slant_v2_experimental()
    }

    /// Height of a body `plates` plates tall (studs not included).
    pub fn body_height(&self, plates: u32) -> f64 {
        f64::from(plates) * self.plate_height
    }

    /// Checks that the numbers make a printable brick: every size finite and positive, the
    /// walls thinner than half a stud cell, the tubes and studs fitting inside a cell, and a
    /// v2 stud's slot, hook and pocket fitting inside its lobe and the brick's top.
    pub fn validate(&self) -> Result<()> {
        let named = [
            ("pitch", self.pitch),
            ("clearance", self.clearance),
            ("stud_diameter", self.stud_diameter),
            ("stud_height", self.stud_height),
            ("plate_height", self.plate_height),
            ("top_thickness", self.top_thickness),
            ("wall", self.wall),
            ("tube_outer_diameter", self.tube_outer_diameter),
            ("tube_inner_diameter", self.tube_inner_diameter),
        ];
        for (name, v) in named {
            if !(v.is_finite() && v > 0.0) {
                return Err(CustomError::InvalidProfile(format!(
                    "{name} must be a positive number of mm, got {v}"
                )));
            }
        }
        let bad = |why: String| Err(CustomError::InvalidProfile(why));
        if !(self.bottom_chamfer.is_finite() && self.bottom_chamfer >= 0.0) {
            return bad(format!(
                "bottom_chamfer must be 0 or a positive number of mm, got {}",
                self.bottom_chamfer
            ));
        }
        if self.bottom_chamfer > self.wall / 2.0 {
            return bad("the bottom chamfer must be at most half a wall".into());
        }
        if self.pitch > 32.0 {
            return bad(format!("pitch {} mm is not a brick pitch", self.pitch));
        }
        if self.clearance * 2.0 + self.wall * 2.0 >= self.pitch * 0.9 {
            return bad("walls and clearance leave no hollow inside a 1-stud cell".into());
        }
        if self.stud_diameter >= self.pitch - 2.0 * self.clearance {
            return bad("a stud is wider than its cell".into());
        }
        if self.tube_inner_diameter >= self.tube_outer_diameter {
            return bad("tube inner diameter must be smaller than its outer diameter".into());
        }
        if self.bottom_chamfer > (self.tube_outer_diameter - self.tube_inner_diameter) / 4.0 {
            return bad("the bottom chamfer must be at most half a tube wall".into());
        }
        // A tube sits on a cell corner; it must clear the inner walls of a 2-wide brick.
        if self.tube_outer_diameter / 2.0 >= self.pitch - self.clearance - self.wall {
            return bad("bottom tubes would cut into the side walls".into());
        }
        if self.top_thickness >= self.plate_height {
            return bad("top thickness must be less than one plate".into());
        }
        if let StudStyle::SlantV2 { .. } = self.stud_style {
            crate::slant::Lobe::new(self)?;
        }
        Ok(())
    }
}
