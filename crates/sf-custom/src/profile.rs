//! Connector dimensions: how a printed brick meets real bricks.

use crate::{CustomError, Result};
use serde::{Deserialize, Serialize};

/// The shape of the studs on top of a brick.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StudStyle {
    /// A plain solid cylinder, [`StudProfile::stud_diameter`] wide.
    Solid,
    /// **Experimental.** A hollow stud split by two vertical relief slots into two flexing
    /// fingers. The stud is printed oversize and the fingers spring in when a brick is pressed
    /// on, so grip depends less on the printer's exact diameter. Needs test prints before use.
    Slotted {
        /// Diameter of the hole down the middle of the stud, in mm.
        inner_diameter: f64,
        /// Width of each relief slot, in mm.
        slot_width: f64,
        /// How far the slots reach down from the stud top, in mm (less than the stud height,
        /// so the fingers stay joined to the brick by a full ring at their root).
        slot_depth: f64,
    },
}

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
    /// Solid or slotted studs.
    pub stud_style: StudStyle,
}

impl Default for StudProfile {
    fn default() -> Self {
        Self::standard_fdm()
    }
}

impl StudProfile {
    /// The dimensions Slant 3D's online brick generator uses for FDM prints (read from its
    /// client-side code): pitch 8.0 mm, 0.1 mm inset per side, studs Ø4.8 × 1.8 mm, plate
    /// 3.2 mm (brick 9.6 mm), top 1.0 mm, walls 1.2 mm, tubes Ø6.51 / Ø4.8 mm.
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
        }
    }

    /// **Experimental, not yet test-printed.** Our own compliant stud, inspired by the
    /// "v2" stud Slant 3D has described: studs 10 % wider (Ø5.28 mm) made hollow (Ø3.28 mm
    /// bore, 1.0 mm wall) and split by two 0.5 mm relief slots, 1.5 mm deep, into two
    /// leaf-spring fingers that flex inward to grip. Everything else is
    /// [`StudProfile::standard_fdm`]. Print a test pair before trusting the clutch.
    pub fn compliant_v2_experimental() -> Self {
        Self {
            stud_diameter: 5.28,
            stud_style: StudStyle::Slotted {
                inner_diameter: 3.28,
                slot_width: 0.5,
                slot_depth: 1.5,
            },
            ..Self::standard_fdm()
        }
    }

    /// Height of a body `plates` plates tall (studs not included).
    pub fn body_height(&self, plates: u32) -> f64 {
        f64::from(plates) * self.plate_height
    }

    /// Checks that the numbers make a printable brick: every size finite and positive, the
    /// walls thinner than half a stud cell, the tubes and studs fitting inside a cell.
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
        // A tube sits on a cell corner; it must clear the inner walls of a 2-wide brick.
        if self.tube_outer_diameter / 2.0 >= self.pitch - self.clearance - self.wall {
            return bad("bottom tubes would cut into the side walls".into());
        }
        if self.top_thickness >= self.plate_height {
            return bad("top thickness must be less than one plate".into());
        }
        if let StudStyle::Slotted {
            inner_diameter,
            slot_width,
            slot_depth,
        } = self.stud_style
        {
            for (name, v) in [
                ("inner_diameter", inner_diameter),
                ("slot_width", slot_width),
                ("slot_depth", slot_depth),
            ] {
                if !(v.is_finite() && v > 0.0) {
                    return bad(format!(
                        "slotted stud {name} must be a positive number of mm, got {v}"
                    ));
                }
            }
            if inner_diameter >= self.stud_diameter - 0.4 {
                return bad("slotted stud bore leaves a finger wall under 0.2 mm".into());
            }
            if slot_width >= inner_diameter {
                return bad("relief slots must be narrower than the stud bore".into());
            }
            if slot_depth >= self.stud_height {
                return bad("relief slots must stop above the stud root".into());
            }
        }
        Ok(())
    }
}
