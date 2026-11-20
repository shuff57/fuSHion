//! Colours: LDraw colour codes, a curated building palette, and CIEDE2000 matching.

use serde::{Deserialize, Serialize};

/// An LDraw colour code. For solid colours Rebrickable uses the same ids.
pub type ColorId = u16;

/// The colour used for hidden interior parts when the spec doesn't care.
pub const FILLER: ColorId = 71; // Light Bluish Gray

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Color {
    /// LDraw colour code.
    pub code: ColorId,
    pub name: String,
    pub rgb: [u8; 3],
    /// BrickLink colour id, if known.
    pub bricklink: Option<u32>,
}

/// (LDraw code, name, hex RGB, BrickLink id). RGB and ids come from the
/// Rebrickable colour table (fetched 2026-11-20).
const BUILTIN: &[(ColorId, &str, u32, u32)] = &[
    (0, "Black", 0x05131D, 11),
    (1, "Blue", 0x0055BF, 7),
    (2, "Green", 0x237841, 6),
    (3, "Dark Turquoise", 0x008F9B, 39),
    (4, "Red", 0xC91A09, 5),
    (5, "Dark Pink", 0xC870A0, 47),
    (6, "Brown", 0x583927, 8),
    (10, "Bright Green", 0x4B9F4A, 36),
    (14, "Yellow", 0xF2CD37, 3),
    (15, "White", 0xFFFFFF, 1),
    (19, "Tan", 0xE4CD9E, 2),
    (25, "Orange", 0xFE8A18, 4),
    (26, "Magenta", 0x923978, 71),
    (27, "Lime", 0xBBE90B, 34),
    (28, "Dark Tan", 0x958A73, 69),
    (29, "Bright Pink", 0xE4ADC8, 104),
    (30, "Medium Lavender", 0xAC78BA, 157),
    (31, "Lavender", 0xE1D5ED, 154),
    (70, "Reddish Brown", 0x582A12, 88),
    (71, "Light Bluish Gray", 0xA0A5A9, 86),
    (72, "Dark Bluish Gray", 0x6C6E68, 85),
    (73, "Medium Blue", 0x5A93DB, 42),
    (78, "Light Nougat", 0xF6D7B3, 90),
    (84, "Medium Nougat", 0xAA7D55, 150),
    (85, "Dark Purple", 0x3F3691, 89),
    (92, "Nougat", 0xD09168, 28),
    (191, "Bright Light Orange", 0xF8BB3D, 110),
    (212, "Bright Light Blue", 0x9FC3E9, 105),
    (226, "Bright Light Yellow", 0xFFF03A, 103),
    (272, "Dark Blue", 0x0A3463, 63),
    (288, "Dark Green", 0x184632, 80),
    (308, "Dark Brown", 0x352100, 120),
    (320, "Dark Red", 0x720E0F, 59),
    (321, "Dark Azure", 0x078BC9, 153),
    (322, "Medium Azure", 0x36AEBF, 156),
    (330, "Olive Green", 0x9B9A5A, 155),
    (378, "Sand Green", 0xA0BCAC, 48),
    (379, "Sand Blue", 0x6074A1, 55),
    (335, "Sand Red", 0xD67572, 58),
    (115, "Medium Lime", 0xC7D23C, 76),
    (151, "Very Light Bluish Gray", 0xE6E3E0, 99),
];

/// A set of colours the engine may use.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Palette {
    pub colors: Vec<Color>,
}

impl Default for Palette {
    fn default() -> Self {
        Self::builtin()
    }
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect::<String>().replace("grey", "gray")
}

impl Palette {
    /// The curated solid building palette.
    pub fn builtin() -> Self {
        Self {
            colors: BUILTIN
                .iter()
                .map(|&(code, name, hex, bl)| Color {
                    code,
                    name: name.to_string(),
                    rgb: [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8],
                    bricklink: Some(bl),
                })
                .collect(),
        }
    }

    pub fn get(&self, code: ColorId) -> Option<&Color> {
        self.colors.iter().find(|c| c.code == code)
    }

    /// Find by name (case, spacing and grey/gray insensitive) or by `#RRGGBB`,
    /// which snaps to the nearest palette colour.
    pub fn resolve(&self, s: &str) -> Option<ColorId> {
        let t = s.trim();
        if let Some(rgb) = parse_hex(t) {
            return self.nearest(rgb);
        }
        if let Ok(code) = t.parse::<ColorId>() {
            return self.get(code).map(|c| c.code);
        }
        let n = norm(t);
        self.colors.iter().find(|c| norm(&c.name) == n).map(|c| c.code)
    }

    /// Nearest colour by CIEDE2000.
    pub fn nearest(&self, rgb: [u8; 3]) -> Option<ColorId> {
        let lab = srgb_to_lab(rgb);
        self.colors
            .iter()
            .map(|c| (c.code, ciede2000(lab, srgb_to_lab(c.rgb))))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(c, _)| c)
    }

    /// RGB for a code, falling back to magenta for unknown codes.
    pub fn rgb(&self, code: ColorId) -> [u8; 3] {
        self.get(code).map(|c| c.rgb).unwrap_or([255, 0, 255])
    }

    pub fn name(&self, code: ColorId) -> String {
        self.get(code).map(|c| c.name.clone()).unwrap_or_else(|| format!("LDraw colour {code}"))
    }
}

/// Parse `#RRGGBB` or `RRGGBB`.
pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let h = s.strip_prefix('#').unwrap_or(s);
    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// sRGB (D65) to CIELAB.
pub fn srgb_to_lab(rgb: [u8; 3]) -> [f64; 3] {
    fn lin(c: u8) -> f64 {
        let c = c as f64 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    }
    let (r, g, b) = (lin(rgb[0]), lin(rgb[1]), lin(rgb[2]));
    let x = (0.4124564 * r + 0.3575761 * g + 0.1804375 * b) / 0.95047;
    let y = 0.2126729 * r + 0.7151522 * g + 0.0721750 * b;
    let z = (0.0193339 * r + 0.1191920 * g + 0.9503041 * b) / 1.08883;
    fn f(t: f64) -> f64 {
        if t > 216.0 / 24389.0 { t.cbrt() } else { (24389.0 / 27.0 * t + 16.0) / 116.0 }
    }
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// CIEDE2000 colour difference.
pub fn ciede2000(a: [f64; 3], b: [f64; 3]) -> f64 {
    use std::f64::consts::PI;
    let (l1, a1, b1) = (a[0], a[1], a[2]);
    let (l2, a2, b2) = (b[0], b[1], b[2]);
    let c1 = (a1 * a1 + b1 * b1).sqrt();
    let c2 = (a2 * a2 + b2 * b2).sqrt();
    let cb = (c1 + c2) / 2.0;
    let g = 0.5 * (1.0 - (cb.powi(7) / (cb.powi(7) + 25f64.powi(7))).sqrt());
    let a1p = (1.0 + g) * a1;
    let a2p = (1.0 + g) * a2;
    let c1p = (a1p * a1p + b1 * b1).sqrt();
    let c2p = (a2p * a2p + b2 * b2).sqrt();
    let h = |bb: f64, ap: f64| {
        if bb == 0.0 && ap == 0.0 {
            0.0
        } else {
            let v = bb.atan2(ap);
            if v < 0.0 { v + 2.0 * PI } else { v }
        }
    };
    let h1p = h(b1, a1p);
    let h2p = h(b2, a2p);
    let dlp = l2 - l1;
    let dcp = c2p - c1p;
    let mut dhp = h2p - h1p;
    if c1p * c2p == 0.0 {
        dhp = 0.0;
    } else if dhp > PI {
        dhp -= 2.0 * PI;
    } else if dhp < -PI {
        dhp += 2.0 * PI;
    }
    let dhp_big = 2.0 * (c1p * c2p).sqrt() * (dhp / 2.0).sin();
    let lbp = (l1 + l2) / 2.0;
    let cbp = (c1p + c2p) / 2.0;
    let hbp = if c1p * c2p == 0.0 {
        h1p + h2p
    } else if (h1p - h2p).abs() <= PI {
        (h1p + h2p) / 2.0
    } else if h1p + h2p < 2.0 * PI {
        (h1p + h2p + 2.0 * PI) / 2.0
    } else {
        (h1p + h2p - 2.0 * PI) / 2.0
    };
    let t = 1.0 - 0.17 * (hbp - PI / 6.0).cos() + 0.24 * (2.0 * hbp).cos() + 0.32 * (3.0 * hbp + PI / 30.0).cos()
        - 0.20 * (4.0 * hbp - 63.0 * PI / 180.0).cos();
    let dtheta = (30.0 * PI / 180.0) * (-((hbp * 180.0 / PI - 275.0) / 25.0).powi(2)).exp();
    let rc = 2.0 * (cbp.powi(7) / (cbp.powi(7) + 25f64.powi(7))).sqrt();
    let sl = 1.0 + 0.015 * (lbp - 50.0).powi(2) / (20.0 + (lbp - 50.0).powi(2)).sqrt();
    let sc = 1.0 + 0.045 * cbp;
    let sh = 1.0 + 0.015 * cbp * t;
    let rt = -(2.0 * dtheta).sin() * rc;
    ((dlp / sl).powi(2) + (dcp / sc).powi(2) + (dhp_big / sh).powi(2) + rt * (dcp / sc) * (dhp_big / sh)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_names_and_hex() {
        let p = Palette::builtin();
        assert_eq!(p.resolve("light bluish grey"), Some(71));
        assert_eq!(p.resolve("Reddish Brown"), Some(70));
        assert_eq!(p.resolve("#FF0000"), Some(4));
        assert_eq!(p.resolve("#fefefe"), Some(15));
        assert_eq!(p.resolve("not a colour"), None);
    }

    #[test]
    fn ciede2000_reference_pair() {
        // Sharma et al. test data, pair 1: ΔE00 = 2.0425
        let d = ciede2000([50.0, 2.6772, -79.7751], [50.0, 0.0, -82.7485]);
        assert!((d - 2.0425).abs() < 1e-3, "{d}");
    }
}
