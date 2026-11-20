//! Extract the parts studforge needs from the LDraw `complete.zip`.

use crate::parse::{normalize_name, parse_file, references};
use anyhow::{Context, Result, bail};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Directories searched for a type-1 reference, in order (relative to the
/// library root, lowercase).
pub const SEARCH_DIRS: &[&str] = &["parts/", "p/"];

/// Marker written last by [`install_from_zip`]; its presence means the cache
/// is complete.
pub const MARKER: &str = ".studforge-installed";

/// Extra top-level files copied verbatim (licence and colours).
const EXTRA_FILES: &[&str] = &["LDConfig.ldr", "CAreadme.txt", "CAlicense.txt", "CAlicense4.txt"];

#[derive(Clone, Debug, Default)]
pub struct InstallReport {
    /// Number of files written.
    pub files: usize,
    /// Total bytes written.
    pub bytes: u64,
    /// References that could not be found in the zip.
    pub missing: Vec<String>,
}

/// `<cache>/studforge/ldraw`, falling back to `./studforge-cache/ldraw`.
pub fn default_cache_dir() -> PathBuf {
    dirs::cache_dir().unwrap_or_else(|| PathBuf::from("studforge-cache")).join("studforge").join("ldraw")
}

/// Where the downloader stores the LDraw `complete.zip`.
pub fn default_zip_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("studforge-cache"))
        .join("studforge")
        .join("downloads")
        .join("complete.zip")
}

/// Is `dir` a completed install containing every catalog part? (A catalog
/// that has grown since the last install triggers a re-install.)
pub fn is_installed(dir: &Path) -> bool {
    dir.join(MARKER).is_file()
        && sf_core::CATALOG
            .iter()
            .all(|p| dir.join("parts").join(format!("{}.dat", p.id.to_ascii_lowercase())).is_file())
}

/// Return the default cache dir, installing from [`default_zip_path`] first
/// if needed.
pub fn ensure_installed() -> Result<PathBuf> {
    let dest = default_cache_dir();
    if is_installed(&dest) {
        return Ok(dest);
    }
    let zip = default_zip_path();
    if !zip.is_file() {
        bail!("LDraw library not installed and {} is missing", zip.display());
    }
    let report = install_from_zip(&zip, &dest)?;
    if !report.missing.is_empty() {
        eprintln!("sf-ldraw: {} referenced files missing from {}: {:?}", report.missing.len(), zip.display(), report.missing);
    }
    Ok(dest)
}

/// Extract the dependency closure of every catalog part, plus `LDConfig.ldr`
/// and the licence files, into `dest` (files are written with lowercase
/// relative paths, e.g. `parts/s/3001s01.dat`).
pub fn install_from_zip(zip: &Path, dest: &Path) -> Result<InstallReport> {
    let ids: Vec<&str> = sf_core::CATALOG.iter().map(|p| p.id).collect();
    install_parts_from_zip(zip, dest, &ids)
}

/// Like [`install_from_zip`] for an explicit list of part ids.
pub fn install_parts_from_zip(zip: &Path, dest: &Path, part_ids: &[&str]) -> Result<InstallReport> {
    let file = fs::File::open(zip).with_context(|| format!("opening {}", zip.display()))?;
    let mut archive = zip::ZipArchive::new(file).with_context(|| format!("reading {}", zip.display()))?;

    // Index entries by lowercase path relative to the `ldraw/` root.
    let mut index: HashMap<String, usize> = HashMap::new();
    for i in 0..archive.len() {
        let Some(Ok(name)) = archive.name_for_index(i) else { continue };
        let lower = name.replace('\\', "/").to_ascii_lowercase();
        let rel = lower.strip_prefix("ldraw/").unwrap_or(&lower).to_string();
        index.entry(rel).or_insert(i);
    }

    let mut read = |idx: usize| -> Result<Vec<u8>> {
        let mut f = archive.by_index(idx)?;
        let mut buf = Vec::with_capacity(f.size() as usize);
        f.read_to_end(&mut buf)?;
        Ok(buf)
    };

    let mut report = InstallReport::default();
    let mut to_write: Vec<(String, Vec<u8>)> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut queue: VecDeque<String> = part_ids.iter().map(|id| format!("{}.dat", normalize_name(id))).collect();
    while let Some(name) = queue.pop_front() {
        if !seen.insert(name.clone()) {
            continue;
        }
        let Some((rel, idx)) = SEARCH_DIRS.iter().find_map(|d| {
            let rel = format!("{d}{name}");
            index.get(&rel).map(|&i| (rel, i))
        }) else {
            report.missing.push(name);
            continue;
        };
        let bytes = read(idx)?;
        let text = String::from_utf8_lossy(&bytes);
        for r in references(&parse_file(&text)) {
            if !seen.contains(r) {
                queue.push_back(r.to_string());
            }
        }
        to_write.push((rel, bytes));
    }
    for extra in EXTRA_FILES {
        if let Some(&idx) = index.get(&extra.to_ascii_lowercase()) {
            to_write.push((extra.to_string(), read(idx)?));
        }
    }

    fs::create_dir_all(dest).with_context(|| format!("creating {}", dest.display()))?;
    let _ = fs::remove_file(dest.join(MARKER));
    for (rel, bytes) in &to_write {
        let path = dest.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))?;
        report.files += 1;
        report.bytes += bytes.len() as u64;
    }
    fs::write(
        dest.join(MARKER),
        "Extracted from the LDraw parts library (CC BY 4.0, see CAreadme.txt) by studforge.\n",
    )?;
    Ok(report)
}
