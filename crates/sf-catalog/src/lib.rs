//! Part and colour availability from Rebrickable data.
//!
//! [`CatalogIndex`] maps every [`sf_core::CATALOG`] part (by LDraw id) to its
//! Rebrickable and BrickLink part numbers, and records, for each LDraw colour,
//! how many distinct official sets contain that part in that colour and the
//! latest year it appeared. It is built by [`CatalogIndex::sync`] from the
//! Rebrickable CSV dumps (plus an optional handful of API calls for the
//! BrickLink mapping) and persisted as one compact JSON file.
//!
//! # Counting policy
//! * A set counts once however many inventory versions or copies of the part
//!   it has. Only inventories whose `set_num` is in `sets.csv` count, so
//!   minifig inventories (`fig-…`) are ignored. Spare parts are excluded.
//! * **Mould variants are not merged.** Each catalog part counts only the
//!   exact Rebrickable part number it maps to, which is the current mould
//!   (e.g. `3001`, not `3001a`–`3001e`; `3070b`, not the grooveless `3070a`).
//!   The old moulds are separate BrickLink items, mostly pre-1980, and cannot
//!   be bought new, so folding them in would make old colours look available
//!   when they are not. Printed (`pr…`) and patterned variants are excluded
//!   for the same reason: a builder needs the plain part.
//! * Colours are keyed by LDraw code, taken from Rebrickable's
//!   `external_ids.LDraw`. Rebrickable colours without an LDraw id are dropped.
//!
//! # Rarity
//! [`CatalogIndex::is_rare`] is true when a part *was* made in a colour but in
//! at most [`RARE_MAX_SETS`] sets, or was last seen before
//! [`RARE_BEFORE_YEAR`]. Such parts exist but are expensive or hard to find.

mod api;
mod sync;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sf_core::{Availability, ColorId, Palette};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

pub use sync::{DUMP_FILES, SyncOptions, default_downloads_dir, ensure_downloads};

/// Bumped whenever the JSON layout changes; older files are ignored.
pub const SCHEMA_VERSION: u32 = 1;
/// A part in at most this many sets counts as rare.
pub const RARE_MAX_SETS: u32 = 3;
/// A part last seen before this year counts as rare.
pub const RARE_BEFORE_YEAR: u16 = 2005;
/// Minimum sets for a colour to enter [`CatalogIndex::palette`].
pub const PALETTE_MIN_SETS: u32 = 5;

/// BrickLink item ids for the catalog, verified against the Rebrickable API
/// (`external_ids.BrickLink`) on 2026-11-20. Used when no synced index is
/// available; a synced [`CatalogIndex`] takes precedence.
pub static BUILTIN_BRICKLINK_PARTS: &[(&str, &str)] = &[
    ("3005", "3005"),
    ("3004", "3004"),
    ("3622", "3622"),
    ("3010", "3010"),
    ("3009", "3009"),
    ("3008", "3008"),
    ("3003", "3003"),
    ("3002", "3002"),
    ("3001", "3001"),
    ("2456", "2456"),
    ("3007", "3007"),
    ("3024", "3024"),
    ("3023", "3023"),
    ("3623", "3623"),
    ("3710", "3710"),
    ("3666", "3666"),
    ("3460", "3460"),
    ("3022", "3022"),
    ("3021", "3021"),
    ("3020", "3020"),
    ("3795", "3795"),
    ("3034", "3034"),
    ("3031", "3031"),
    ("3032", "3032"),
    ("3035", "3035"),
    ("3958", "3958"),
    ("3070b", "3070"),
    ("3069b", "3069"),
    ("63864", "63864"),
    ("2431", "2431"),
    ("3068b", "3068"),
    ("3040b", "3040"),
    ("3039", "3039"),
    ("3665", "3665"),
    ("3660", "3660"),
    ("54200", "54200"),
    ("85984", "85984"),
];

/// LDraw ids that name the same physical part as a catalog id. LDraw keeps
/// `3660.dat`/`3665.dat` as "~Moved to" redirects to the `a` variants, and
/// Rebrickable lists `3660b`/`3665a`/`3665b` as the LDraw files of its
/// current `3660`/`3665`. Used only when the API is not consulted.
pub(crate) static LDRAW_ALIASES: &[(&str, &str)] =
    &[("3660b", "3660"), ("3660c", "3660"), ("3665a", "3665"), ("3665b", "3665"), ("3040", "3040b")];

/// The builtin BrickLink item id for an LDraw catalog id (or alias).
pub fn builtin_bricklink_part(ldraw_id: &str) -> Option<&'static str> {
    let id = canonical_alias(ldraw_id);
    BUILTIN_BRICKLINK_PARTS.iter().find(|(l, _)| *l == id).map(|(_, b)| *b)
}

fn same_name(a: &str, b: &str) -> bool {
    let n = |s: &str| {
        s.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect::<String>().replace("grey", "gray")
    };
    n(a) == n(b)
}

fn canonical_alias(id: &str) -> &str {
    LDRAW_ALIASES.iter().find(|(a, _)| *a == id).map(|(_, c)| *c).unwrap_or(id)
}

/// Normalise an LDraw part reference: lower case, no `.dat`, no `parts\`.
pub fn normalize_ldraw_id(id: &str) -> String {
    let id = id.trim().to_ascii_lowercase().replace('\\', "/");
    let id = id.rsplit('/').next().unwrap_or(&id).to_string();
    id.strip_suffix(".dat").map(str::to_string).unwrap_or(id)
}

/// Sets and latest year for one (part, colour).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColorStat(
    /// Distinct sets.
    pub u32,
    /// Latest set year.
    pub u16,
);

impl ColorStat {
    pub fn sets(self) -> u32 {
        self.0
    }
    pub fn last_year(self) -> u16 {
        self.1
    }
}

/// One catalog part.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PartEntry {
    /// LDraw id as used in [`sf_core::CATALOG`].
    pub ldraw: String,
    /// Rebrickable part number; `None` if it could not be mapped.
    pub rebrickable: Option<String>,
    /// BrickLink item id; `None` if unknown.
    pub bricklink: Option<String>,
    /// Other LDraw ids for the same part (from Rebrickable's LDraw ids).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    /// LDraw colour code → (sets, last year). Colours never produced are absent.
    #[serde(default)]
    pub colors: BTreeMap<ColorId, ColorStat>,
}

/// One colour known to Rebrickable that has an LDraw code.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColorEntry {
    pub ldraw: ColorId,
    pub rebrickable: i32,
    pub name: String,
    /// `RRGGBB` hex.
    pub rgb: String,
    #[serde(default)]
    pub trans: bool,
    pub bricklink: Option<u32>,
}

/// The persisted availability index. See the crate docs for the policy.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CatalogIndex {
    pub schema: u32,
    /// Date (UTC, `YYYY-MM-DD`) of the Rebrickable dump used.
    pub dump_date: String,
    /// Number of sets considered.
    pub set_total: u32,
    pub parts: Vec<PartEntry>,
    pub colors: Vec<ColorEntry>,
    #[serde(skip)]
    part_lookup: HashMap<String, usize>,
    #[serde(skip)]
    color_lookup: HashMap<ColorId, usize>,
}

impl CatalogIndex {
    /// Build from parts and colours (used by `sync` and tests).
    pub fn new(dump_date: String, set_total: u32, parts: Vec<PartEntry>, colors: Vec<ColorEntry>) -> Self {
        let mut idx = Self {
            schema: SCHEMA_VERSION,
            dump_date,
            set_total,
            parts,
            colors,
            part_lookup: HashMap::new(),
            color_lookup: HashMap::new(),
        };
        idx.rebuild_lookup();
        idx
    }

    fn rebuild_lookup(&mut self) {
        self.part_lookup.clear();
        // Primary ids first so an alias never shadows a real catalog id.
        for (i, p) in self.parts.iter().enumerate() {
            self.part_lookup.insert(normalize_ldraw_id(&p.ldraw), i);
        }
        for (i, p) in self.parts.iter().enumerate() {
            for a in &p.aliases {
                self.part_lookup.entry(normalize_ldraw_id(a)).or_insert(i);
            }
        }
        for (a, c) in LDRAW_ALIASES {
            if let Some(&i) = self.part_lookup.get(*c) {
                self.part_lookup.entry((*a).to_string()).or_insert(i);
            }
        }
        self.color_lookup = self.colors.iter().enumerate().map(|(i, c)| (c.ldraw, i)).collect();
    }

    /// `dirs::cache_dir()/studforge/catalog.json`.
    pub fn default_path() -> Option<PathBuf> {
        dirs::cache_dir().map(|d| d.join("studforge").join("catalog.json"))
    }

    /// Load from the default path. `Ok(None)` if the file is missing or was
    /// written with another schema version.
    pub fn load_default() -> Result<Option<Self>> {
        match Self::default_path() {
            Some(p) => Self::load(&p),
            None => Ok(None),
        }
    }

    /// Load from `path`. `Ok(None)` if missing or of another schema version.
    pub fn load(path: &Path) -> Result<Option<Self>> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        Self::from_json(&text).with_context(|| format!("parsing {}", path.display()))
    }

    /// Parse JSON. `Ok(None)` for another schema version.
    pub fn from_json(text: &str) -> Result<Option<Self>> {
        #[derive(Deserialize)]
        struct Probe {
            schema: u32,
        }
        let probe: Probe = serde_json::from_str(text)?;
        if probe.schema != SCHEMA_VERSION {
            return Ok(None);
        }
        let mut idx: Self = serde_json::from_str(text)?;
        idx.rebuild_lookup();
        Ok(Some(idx))
    }

    /// Compact JSON.
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }

    /// Write to `path` atomically (temp file + rename), creating parent dirs.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, self.to_json()?).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, path).with_context(|| format!("renaming to {}", path.display()))?;
        Ok(())
    }

    /// Save to [`Self::default_path`] and return it.
    pub fn save_default(&self) -> Result<PathBuf> {
        let p = Self::default_path().context("no cache directory on this system")?;
        self.save(&p)?;
        Ok(p)
    }

    /// The entry for an LDraw id (or a known alias of one).
    pub fn part(&self, ldraw_id: &str) -> Option<&PartEntry> {
        self.part_lookup.get(&normalize_ldraw_id(ldraw_id)).map(|&i| &self.parts[i])
    }

    /// The colour entry for an LDraw code.
    pub fn color(&self, code: ColorId) -> Option<&ColorEntry> {
        self.color_lookup.get(&code).map(|&i| &self.colors[i])
    }

    /// Stats for a (part, colour) the index can answer about: `None` if the
    /// part or colour is unknown, `Some(None)` if known but never produced.
    fn stat(&self, ldraw_id: &str, code: ColorId) -> Option<Option<ColorStat>> {
        let p = self.part(ldraw_id)?;
        p.rebrickable.as_ref()?;
        self.color(code)?;
        Some(p.colors.get(&code).copied())
    }

    pub fn rebrickable_part(&self, ldraw_id: &str) -> Option<&str> {
        self.part(ldraw_id)?.rebrickable.as_deref()
    }

    pub fn bricklink_part(&self, ldraw_id: &str) -> Option<&str> {
        self.part(ldraw_id)?.bricklink.as_deref()
    }

    pub fn bricklink_color(&self, code: ColorId) -> Option<u32> {
        self.color(code)?.bricklink
    }

    /// Latest year a set contained this part in this colour.
    pub fn last_year(&self, ldraw_id: &str, code: ColorId) -> Option<u16> {
        self.stat(ldraw_id, code).flatten().map(ColorStat::last_year)
    }

    /// Produced, but in ≤ [`RARE_MAX_SETS`] sets or last seen before
    /// [`RARE_BEFORE_YEAR`]. False for never-produced or unknown parts.
    pub fn is_rare(&self, ldraw_id: &str, code: ColorId) -> bool {
        match self.stat(ldraw_id, code).flatten() {
            Some(s) => s.sets() <= RARE_MAX_SETS || s.last_year() < RARE_BEFORE_YEAR,
            None => false,
        }
    }

    /// The builtin palette restricted to colours produced as both the 1×1
    /// brick (3005) and the 1×1 plate (3024) in at least
    /// [`PALETTE_MIN_SETS`] sets each, with BrickLink ids from Rebrickable.
    ///
    /// If a builtin colour's LDraw code names a different colour in the
    /// Rebrickable data but its *name* matches another LDraw code, the entry
    /// is re-keyed to that code (sf-core lists Olive Green as 326, which is
    /// LDraw Yellowish Green; LDraw Olive Green is 330).
    pub fn palette(&self) -> Palette {
        let mut pal = Palette::builtin();
        for c in &mut pal.colors {
            let name_ok = self.color(c.code).is_none_or(|e| same_name(&e.name, &c.name));
            if !name_ok && let Some(e) = self.colors.iter().find(|e| same_name(&e.name, &c.name)) {
                c.code = e.ldraw;
            }
        }
        pal.colors.retain(|c| {
            ["3005", "3024"].iter().all(|p| self.set_count(p, c.code).is_some_and(|n| n >= PALETTE_MIN_SETS))
        });
        for c in &mut pal.colors {
            if let Some(bl) = self.bricklink_color(c.code) {
                c.bricklink = Some(bl);
            }
        }
        pal
    }
}

impl Availability for CatalogIndex {
    fn is_made(&self, part: &str, color: ColorId) -> Option<bool> {
        self.stat(part, color).map(|s| s.is_some_and(|s| s.sets() > 0))
    }

    fn set_count(&self, part: &str, color: ColorId) -> Option<u32> {
        self.stat(part, color).map(|s| s.map_or(0, ColorStat::sets))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn sample() -> CatalogIndex {
        let mut c3005 = BTreeMap::new();
        c3005.insert(4, ColorStat(500, 2026));
        c3005.insert(25, ColorStat(40, 2025));
        c3005.insert(5, ColorStat(2, 2024));
        c3005.insert(26, ColorStat(30, 1999));
        c3005.insert(330, ColorStat(10, 2020));
        let mut c3024 = BTreeMap::new();
        c3024.insert(4, ColorStat(900, 2026));
        c3024.insert(25, ColorStat(4, 2020));
        c3024.insert(26, ColorStat(30, 2010));
        c3024.insert(330, ColorStat(12, 2021));
        let parts = vec![
            PartEntry {
                ldraw: "3005".into(),
                rebrickable: Some("3005".into()),
                bricklink: Some("3005".into()),
                aliases: vec!["30071".into()],
                colors: c3005,
            },
            PartEntry {
                ldraw: "3024".into(),
                rebrickable: Some("3024".into()),
                bricklink: Some("3024".into()),
                aliases: vec![],
                colors: c3024,
            },
            PartEntry {
                ldraw: "3660".into(),
                rebrickable: Some("3660".into()),
                bricklink: Some("3660".into()),
                aliases: vec!["3660b".into()],
                colors: BTreeMap::new(),
            },
            PartEntry { ldraw: "3039".into(), rebrickable: None, bricklink: None, aliases: vec![], colors: BTreeMap::new() },
        ];
        let col = |ldraw, name: &str, bl| ColorEntry {
            ldraw,
            rebrickable: ldraw as i32,
            name: name.into(),
            rgb: "000000".into(),
            trans: false,
            bricklink: Some(bl),
        };
        let colors = vec![col(4, "Red", 5), col(25, "Orange", 4), col(5, "Dark Pink", 47), col(26, "Magenta", 71), col(15, "White", 1), col(326, "Yellowish Green", 158), col(330, "Olive Green", 155)];
        CatalogIndex::new("2026-11-20".into(), 1000, parts, colors)
    }

    #[test]
    fn availability_answers() {
        let idx = sample();
        assert_eq!(idx.is_made("3005", 4), Some(true));
        assert_eq!(idx.is_made("3005", 15), Some(false));
        assert_eq!(idx.set_count("3005", 15), Some(0));
        assert_eq!(idx.is_made("3005", 999), None, "unknown colour");
        assert_eq!(idx.is_made("9999", 4), None, "unknown part");
        assert_eq!(idx.is_made("3039", 4), None, "unmapped part");
        assert_eq!(idx.is_made("3005.DAT", 4), Some(true));
        assert_eq!(idx.set_count("30071", 4), Some(500), "alias");
        assert_eq!(idx.rebrickable_part("3660b"), Some("3660"));
        assert_eq!(idx.last_year("3005", 25), Some(2025));
        assert_eq!(idx.bricklink_color(25), Some(4));
    }

    #[test]
    fn rarity() {
        let idx = sample();
        assert!(!idx.is_rare("3005", 4));
        assert!(idx.is_rare("3005", 5), "two sets");
        assert!(idx.is_rare("3005", 26), "last seen 1999");
        assert!(!idx.is_rare("3005", 15), "never made is not rare");
    }

    #[test]
    fn palette_needs_both_parts() {
        let pal = sample().palette();
        let codes: Vec<_> = pal.colors.iter().map(|c| c.code).collect();
        // Red: both. Orange: plate only in 4 sets. Magenta: both ≥ 5.
        // Olive Green is re-keyed from sf-core's 326 to LDraw 330.
        assert_eq!(codes, vec![4, 26, 330]);
        assert_eq!(pal.get(330).map(|c| (c.name.as_str(), c.bricklink)), Some(("Olive Green", Some(155))));
    }

    #[test]
    fn json_round_trip_and_schema() {
        let idx = sample();
        let json = idx.to_json().unwrap();
        let back = CatalogIndex::from_json(&json).unwrap().unwrap();
        assert_eq!(back.parts, idx.parts);
        assert_eq!(back.set_count("30071", 4), Some(500));
        let old = json.replacen("\"schema\":1", "\"schema\":0", 1);
        assert!(CatalogIndex::from_json(&old).unwrap().is_none());
        assert!(CatalogIndex::from_json("not json").is_err());
    }

    #[test]
    fn save_and_load() {
        let dir = std::env::temp_dir().join(format!("sf-catalog-test-{}", std::process::id()));
        let path = dir.join("catalog.json");
        sample().save(&path).unwrap();
        let back = CatalogIndex::load(&path).unwrap().unwrap();
        assert_eq!(back.dump_date, "2026-11-20");
        assert!(CatalogIndex::load(&dir.join("missing.json")).unwrap().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn builtin_bricklink_covers_catalog() {
        for p in sf_core::CATALOG {
            assert!(builtin_bricklink_part(p.id).is_some(), "{}", p.id);
        }
        assert_eq!(builtin_bricklink_part("3665b"), Some("3665"));
        assert_eq!(normalize_ldraw_id("parts\\3001.DAT"), "3001");
    }
}
