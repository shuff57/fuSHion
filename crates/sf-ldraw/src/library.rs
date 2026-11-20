//! [`MeshLibrary`]: cached part meshes from an LDraw directory or generated
//! procedurally.

use crate::install::SEARCH_DIRS;
use crate::parse::{Flattener, PartMesh, normalize_name};
use crate::procedural::procedural_mesh;
use anyhow::{Context, Result, bail};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

struct DirSource {
    root: PathBuf,
    /// Lowercase relative path (`parts/s/3001s01.dat`) → real path.
    index: HashMap<String, PathBuf>,
}

impl DirSource {
    fn find(&self, name: &str) -> Option<&PathBuf> {
        SEARCH_DIRS.iter().find_map(|d| self.index.get(&format!("{d}{name}")))
    }
}

#[derive(Default)]
struct Cache {
    flattener: Flattener,
    meshes: HashMap<String, Option<Arc<PartMesh>>>,
    warned: HashSet<String>,
}

/// Part meshes in LDraw coordinates, loaded lazily and cached.
/// `Send + Sync`; lookups lock an internal mutex.
pub struct MeshLibrary {
    dir: Option<DirSource>,
    cache: Mutex<Cache>,
}

impl MeshLibrary {
    /// Open an LDraw library directory (a full library or a studforge cache).
    /// File names are matched case-insensitively.
    pub fn open(dir: &Path) -> Result<Self> {
        let mut index = HashMap::new();
        let entries = fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))?;
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            if (name == "parts" || name == "p") && e.path().is_dir() {
                index_dir(&e.path(), &name, &mut index);
            }
        }
        if !index.keys().any(|k| k.starts_with("parts/")) {
            bail!("{} does not look like an LDraw library (no parts/ directory)", dir.display());
        }
        Ok(Self { dir: Some(DirSource { root: dir.to_path_buf(), index }), cache: Mutex::new(Cache::default()) })
    }

    /// Open the default cache, installing it from the downloaded zip if
    /// needed.
    pub fn open_default() -> Result<Self> {
        Self::open(&crate::install::ensure_installed()?)
    }

    /// No LDraw library: meshes are generated from the catalog definitions.
    pub fn procedural() -> Self {
        Self { dir: None, cache: Mutex::new(Cache::default()) }
    }

    pub fn is_procedural(&self) -> bool {
        self.dir.is_none()
    }

    pub fn root(&self) -> Option<&Path> {
        self.dir.as_ref().map(|d| d.root.as_path())
    }

    /// The mesh for an LDraw part id (e.g. `"3001"`). Falls back to the
    /// procedural mesh for catalog parts whose files are missing; `None` for
    /// unknown parts.
    pub fn mesh(&self, part_id: &str) -> Option<Arc<PartMesh>> {
        let key = normalize_name(part_id);
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(m) = cache.meshes.get(&key) {
            return m.clone();
        }
        let file = if key.ends_with(".dat") { key.clone() } else { format!("{key}.dat") };
        let loaded = self.dir.as_ref().and_then(|dir| {
            let src = |n: &str| dir.find(n).and_then(|p| fs::read(p).ok()).map(|b| String::from_utf8_lossy(&b).into_owned());
            cache.flattener.flatten(&src, &file)
        });
        let mesh = match loaded {
            Some(m) => Some(m),
            None => {
                let def = sf_core::parts::part(part_id);
                if self.dir.is_some() && cache.warned.insert(key.clone()) {
                    eprintln!(
                        "sf-ldraw: part {part_id} not found in LDraw library{}",
                        if def.is_some() { "; using procedural mesh" } else { "" }
                    );
                }
                def.map(procedural_mesh)
            }
        }
        .map(Arc::new);
        cache.meshes.insert(key, mesh.clone());
        mesh
    }

    /// Subfile references that could not be found so far.
    pub fn missing_files(&self) -> Vec<String> {
        let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let mut v: Vec<String> = cache.flattener.missing.iter().cloned().collect();
        v.sort();
        v
    }

    /// `(code, name, rgb)` for every `!COLOUR` in `LDConfig.ldr`; empty when
    /// there is no library.
    pub fn ldconfig_colors(&self) -> Vec<(u16, String, [u8; 3])> {
        let Some(dir) = &self.dir else { return Vec::new() };
        let Ok(entries) = fs::read_dir(&dir.root) else { return Vec::new() };
        let path = entries
            .flatten()
            .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case("ldconfig.ldr"))
            .map(|e| e.path());
        path.and_then(|p| fs::read_to_string(p).ok()).map(|t| parse_ldconfig(&t)).unwrap_or_default()
    }
}

fn index_dir(dir: &Path, rel: &str, index: &mut HashMap<String, PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_ascii_lowercase();
        let path = e.path();
        let child = format!("{rel}/{name}");
        if path.is_dir() {
            index_dir(&path, &child, index);
        } else {
            index.insert(child, path);
        }
    }
}

/// Parse `0 !COLOUR <name> CODE <n> VALUE #RRGGBB ...` lines.
pub fn parse_ldconfig(text: &str) -> Vec<(u16, String, [u8; 3])> {
    let mut out = Vec::new();
    for line in text.lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        if t.len() < 7 || t[0] != "0" || t[1] != "!COLOUR" {
            continue;
        }
        let find = |key: &str| t.iter().position(|s| *s == key).and_then(|i| t.get(i + 1));
        let (Some(code), Some(value)) = (find("CODE"), find("VALUE")) else { continue };
        let Ok(code) = code.parse::<u16>() else { continue };
        let Some(rgb) = sf_core::color::parse_hex(value) else { continue };
        out.push((code, t[2].to_string(), rgb));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ldconfig_lines() {
        let c = parse_ldconfig(
            "0 !COLOUR Black                                                 CODE   0   VALUE #1B2A34   EDGE #808080\n\
             0 // comment\n0 !COLOUR Trans_Clear CODE 47 VALUE #FCFCFC EDGE #C3C3C3 ALPHA 128\n",
        );
        assert_eq!(c, vec![(0, "Black".into(), [0x1b, 0x2a, 0x34]), (47, "Trans_Clear".into(), [0xfc, 0xfc, 0xfc])]);
    }

    #[test]
    fn procedural_library_covers_catalog() {
        let lib = MeshLibrary::procedural();
        for d in sf_core::CATALOG {
            let m = lib.mesh(d.id).expect(d.id);
            assert!(m.triangle_count() > 0);
            assert!(Arc::ptr_eq(&m, &lib.mesh(d.id).unwrap()), "cached");
        }
        assert!(lib.mesh("nonexistent").is_none());
        assert!(lib.ldconfig_colors().is_empty());
    }
}
