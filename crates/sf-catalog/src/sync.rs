//! Building a [`CatalogIndex`] from Rebrickable dumps and the API.

use crate::api::{Api, ext_ids};
use crate::{CatalogIndex, ColorEntry, ColorStat, LDRAW_ALIASES, PartEntry, builtin_bricklink_part, canonical_alias};
use anyhow::{Context, Result, bail};
use serde_json::Value;
use sf_core::{CATALOG, ColorId};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

/// Dump files read by [`CatalogIndex::sync`].
pub const DUMP_FILES: &[&str] = &["sets.csv.gz", "inventories.csv.gz", "inventory_parts.csv.gz", "parts.csv.gz", "colors.csv.gz"];
const DUMP_BASE: &str = "https://cdn.rebrickable.com/media/downloads/";

/// Options for [`CatalogIndex::sync`].
#[derive(Clone, Debug, Default)]
pub struct SyncOptions {
    /// Folder with the Rebrickable `*.csv.gz` dumps.
    pub downloads_dir: PathBuf,
    /// Query the Rebrickable API (needs a key) for the part mapping,
    /// BrickLink ids and colours. Two to a few calls, ≥ 1.1 s apart.
    pub use_api: bool,
    /// Download missing dump files from Rebrickable's public CDN first.
    pub download_missing: bool,
    /// Rebrickable API colours response to use when `use_api` is false.
    /// Defaults to `rb_colors.json` in `downloads_dir` or its parent; falls
    /// back to `colors.csv.gz` (no BrickLink colour ids) if absent.
    pub colors_json: Option<PathBuf>,
    /// A previous index whose part mappings are reused when `use_api` is
    /// false (otherwise exact part numbers and the builtin BrickLink table).
    pub previous: Option<CatalogIndex>,
}

impl SyncOptions {
    pub fn new(downloads_dir: impl Into<PathBuf>, use_api: bool) -> Self {
        Self { downloads_dir: downloads_dir.into(), use_api, ..Default::default() }
    }
}

/// `dirs::cache_dir()/studforge/downloads`.
pub fn default_downloads_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("studforge").join("downloads"))
}

/// Download any missing [`DUMP_FILES`] into `dir` (public CDN, no key).
pub fn ensure_downloads(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("studforge/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(300))
        .build()?;
    for f in DUMP_FILES {
        let path = dir.join(f);
        if path.exists() {
            continue;
        }
        let url = format!("{DUMP_BASE}{f}");
        let mut resp = client.get(&url).send().with_context(|| format!("downloading {url}"))?;
        if !resp.status().is_success() {
            bail!("{url}: HTTP {}", resp.status());
        }
        let tmp = path.with_extension("part");
        let mut out = std::fs::File::create(&tmp)?;
        resp.copy_to(&mut out).with_context(|| format!("downloading {url}"))?;
        drop(out);
        std::fs::rename(&tmp, &path)?;
    }
    Ok(())
}

fn open_gz(path: &Path) -> Result<impl Read> {
    let f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    Ok(flate2::read::MultiGzDecoder::new(std::io::BufReader::with_capacity(1 << 16, f)))
}

fn csv_reader<R: Read>(r: R) -> csv::Reader<R> {
    csv::ReaderBuilder::new().has_headers(true).flexible(true).from_reader(r)
}

/// Column indices by header name.
fn columns<R: Read>(rdr: &mut csv::Reader<R>, what: &str, names: &[&str]) -> Result<Vec<usize>> {
    let h = rdr.byte_headers().with_context(|| format!("{what}: reading header"))?.clone();
    names
        .iter()
        .map(|n| h.iter().position(|c| c == n.as_bytes()).with_context(|| format!("{what}: missing column {n}")))
        .collect()
}

fn field(rec: &csv::ByteRecord, i: usize) -> &str {
    rec.get(i).and_then(|b| std::str::from_utf8(b).ok()).unwrap_or("")
}

/// All part numbers in `parts.csv`.
pub(crate) fn read_part_nums<R: Read>(r: R) -> Result<HashSet<String>> {
    let mut rdr = csv_reader(r);
    let c = columns(&mut rdr, "parts.csv", &["part_num"])?;
    let mut out = HashSet::new();
    let mut rec = csv::ByteRecord::new();
    while rdr.read_byte_record(&mut rec)? {
        out.insert(field(&rec, c[0]).to_string());
    }
    Ok(out)
}

/// Colours from a Rebrickable API colours response (`results[]` or a bare array).
pub(crate) fn colors_from_api(items: &[Value]) -> Vec<ColorEntry> {
    let mut out: Vec<ColorEntry> = Vec::new();
    for it in items {
        let Some(rb) = it.get("id").and_then(Value::as_i64) else { continue };
        let Some(ldraw) = ext_ids(it, "LDraw").iter().find_map(|s| s.parse::<ColorId>().ok()) else { continue };
        if out.iter().any(|c| c.ldraw == ldraw) {
            continue;
        }
        out.push(ColorEntry {
            ldraw,
            rebrickable: rb as i32,
            name: it.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
            rgb: it.get("rgb").and_then(Value::as_str).unwrap_or("").to_string(),
            trans: it.get("is_trans").and_then(Value::as_bool).unwrap_or(false),
            bricklink: ext_ids(it, "BrickLink").iter().find_map(|s| s.parse().ok()),
        });
    }
    out
}

/// Colours from `colors.csv` (assumes Rebrickable id == LDraw code; no
/// BrickLink ids).
pub(crate) fn colors_from_csv<R: Read>(r: R) -> Result<Vec<ColorEntry>> {
    let mut rdr = csv_reader(r);
    let c = columns(&mut rdr, "colors.csv", &["id", "name", "rgb", "is_trans"])?;
    let mut out = Vec::new();
    let mut rec = csv::ByteRecord::new();
    while rdr.read_byte_record(&mut rec)? {
        let Ok(id) = field(&rec, c[0]).parse::<i32>() else { continue };
        let Ok(ldraw) = ColorId::try_from(id) else { continue };
        out.push(ColorEntry {
            ldraw,
            rebrickable: id,
            name: field(&rec, c[1]).to_string(),
            rgb: field(&rec, c[2]).to_string(),
            trans: field(&rec, c[3]).eq_ignore_ascii_case("true"),
            bricklink: None,
        });
    }
    Ok(out)
}

/// Count distinct sets and the latest year per (part index, LDraw colour).
///
/// `rb_to_part` maps Rebrickable part numbers to indices into the output;
/// `rb_color` maps Rebrickable colour ids to LDraw codes.
pub(crate) fn count_sets<R1: Read, R2: Read, R3: Read>(
    sets: R1,
    inventories: R2,
    inventory_parts: R3,
    rb_to_part: &HashMap<String, usize>,
    rb_color: &HashMap<i32, ColorId>,
    n_parts: usize,
) -> Result<(Vec<BTreeMap<ColorId, ColorStat>>, u32)> {
    // set_num → (index, year)
    let mut set_ix: HashMap<String, (u32, u16)> = HashMap::new();
    {
        let mut rdr = csv_reader(sets);
        let c = columns(&mut rdr, "sets.csv", &["set_num", "year"])?;
        let mut rec = csv::ByteRecord::new();
        while rdr.read_byte_record(&mut rec)? {
            let year = field(&rec, c[1]).parse::<u16>().unwrap_or(0);
            let n = set_ix.len() as u32;
            set_ix.entry(field(&rec, c[0]).to_string()).or_insert((n, year));
        }
    }
    // inventory id → (set index, year)
    let mut inv: HashMap<u32, (u32, u16)> = HashMap::new();
    {
        let mut rdr = csv_reader(inventories);
        let c = columns(&mut rdr, "inventories.csv", &["id", "set_num"])?;
        let mut rec = csv::ByteRecord::new();
        while rdr.read_byte_record(&mut rec)? {
            let Ok(id) = field(&rec, c[0]).parse::<u32>() else { continue };
            if let Some(&s) = set_ix.get(field(&rec, c[1])) {
                inv.insert(id, s);
            }
        }
    }
    let mut acc: HashMap<(usize, ColorId), (HashSet<u32>, u16)> = HashMap::new();
    {
        let mut rdr = csv_reader(inventory_parts);
        let c = columns(&mut rdr, "inventory_parts.csv", &["inventory_id", "part_num", "color_id", "is_spare"])?;
        let mut rec = csv::ByteRecord::new();
        while rdr.read_byte_record(&mut rec)? {
            let Some(&p) = rb_to_part.get(field(&rec, c[1])) else { continue };
            if field(&rec, c[3]).eq_ignore_ascii_case("true") {
                continue;
            }
            let Ok(inv_id) = field(&rec, c[0]).parse::<u32>() else { continue };
            let Some(&(set, year)) = inv.get(&inv_id) else { continue };
            let Some(&code) = field(&rec, c[2]).parse::<i32>().ok().and_then(|id| rb_color.get(&id)) else { continue };
            let e = acc.entry((p, code)).or_default();
            e.0.insert(set);
            e.1 = e.1.max(year);
        }
    }
    let mut out = vec![BTreeMap::new(); n_parts];
    for ((p, code), (sets, year)) in acc {
        if let Some(m) = out.get_mut(p) {
            m.insert(code, ColorStat(sets.len() as u32, year));
        }
    }
    Ok((out, set_ix.len() as u32))
}

/// Mapping for one catalog part.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Mapping {
    pub rebrickable: Option<String>,
    pub bricklink: Option<String>,
    pub aliases: Vec<String>,
}

/// Offline mapping: previous index, else exact number / known alias, with
/// the builtin BrickLink table.
fn map_offline(ldraw: &str, rb_parts: &HashSet<String>, previous: Option<&CatalogIndex>) -> Mapping {
    if let Some(p) = previous.and_then(|p| p.part(ldraw)).filter(|p| p.rebrickable.is_some()) {
        return Mapping { rebrickable: p.rebrickable.clone(), bricklink: p.bricklink.clone(), aliases: p.aliases.clone() };
    }
    let canon = canonical_alias(ldraw);
    let rb = [ldraw, canon].into_iter().find(|c| rb_parts.contains(*c)).map(str::to_string);
    let aliases = LDRAW_ALIASES.iter().filter(|(_, c)| *c == canon && *c != ldraw).map(|(a, _)| a.to_string()).collect();
    Mapping { bricklink: rb.as_ref().and(builtin_bricklink_part(ldraw).map(str::to_string)), rebrickable: rb, aliases }
}

/// Info for one Rebrickable part from the API.
#[derive(Clone, Debug, Default)]
pub(crate) struct RbPart {
    pub ldraw: Vec<String>,
    pub bricklink: Vec<String>,
}

fn rb_parts_from(items: &[Value]) -> HashMap<String, RbPart> {
    items
        .iter()
        .filter_map(|it| {
            let pn = it.get("part_num")?.as_str()?.to_string();
            Some((pn, RbPart { ldraw: ext_ids(it, "LDraw"), bricklink: ext_ids(it, "BrickLink") }))
        })
        .collect()
}

/// Pick the Rebrickable part for an LDraw id from API data. Order:
/// 1. the same number, if Rebrickable lists this LDraw id for it;
/// 2. any queried part listing this LDraw id;
/// 3. the same number, if it exists (e.g. `3660`, whose LDraw file is a
///    redirect Rebrickable does not list);
/// 4. `None` (the caller then asks the API by `ldraw_id`).
pub(crate) fn pick(ldraw: &str, candidates: &[String], info: &HashMap<String, RbPart>) -> Option<String> {
    let lists = |pn: &str| info.get(pn).is_some_and(|p| p.ldraw.iter().any(|l| l == ldraw));
    if lists(ldraw) {
        return Some(ldraw.to_string());
    }
    if let Some(c) = candidates.iter().find(|c| lists(c)) {
        return Some(c.clone());
    }
    let mut others: Vec<&String> = info.keys().filter(|k| lists(k)).collect();
    others.sort();
    if let Some(c) = others.first() {
        return Some((*c).clone());
    }
    info.contains_key(ldraw).then(|| ldraw.to_string())
}

fn candidates(ldraw: &str, rb_parts: &HashSet<String>) -> Vec<String> {
    let mut v = vec![ldraw.to_string(), canonical_alias(ldraw).to_string()];
    if ldraw.len() > 1 && ldraw.ends_with(|c: char| c.is_ascii_lowercase()) {
        v.push(ldraw[..ldraw.len() - 1].to_string());
    }
    v.dedup();
    v.retain(|c| rb_parts.is_empty() || rb_parts.contains(c));
    v
}

fn map_online(api: &mut Api, ids: &[&str], rb_parts: &HashSet<String>) -> Result<Vec<Mapping>> {
    let mut all: Vec<String> = ids.iter().flat_map(|id| candidates(id, rb_parts)).collect();
    all.sort();
    all.dedup();
    let mut info = HashMap::new();
    for chunk in all.chunks(80) {
        let q = format!("/parts/?part_nums={}&inc_part_details=1&page_size=1000", chunk.join(","));
        info.extend(rb_parts_from(&api.get_all(&q)?));
    }
    let mut out = Vec::with_capacity(ids.len());
    for &id in ids {
        let cands = candidates(id, rb_parts);
        let mut rb = pick(id, &cands, &info);
        if rb.is_none() {
            let found = rb_parts_from(&api.get_all(&format!("/parts/?ldraw_id={id}&inc_part_details=1&page_size=100"))?);
            info.extend(found);
            rb = pick(id, &[], &info);
        }
        let m = match rb.as_ref().and_then(|r| info.get(r)) {
            Some(p) => Mapping {
                rebrickable: rb.clone(),
                bricklink: p.bricklink.first().cloned(),
                aliases: p.ldraw.iter().filter(|l| l.as_str() != id).cloned().collect(),
            },
            None => Mapping::default(),
        };
        out.push(m);
    }
    Ok(out)
}

/// UTC date of a file's modification time as `YYYY-MM-DD`.
fn file_date(path: &Path) -> Option<String> {
    let t = std::fs::metadata(path).ok()?.modified().ok()?;
    let secs = t.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
    Some(civil_date(secs.div_euclid(86_400)))
}

/// Days since 1970-01-01 → `YYYY-MM-DD` (Howard Hinnant's algorithm).
pub(crate) fn civil_date(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

impl CatalogIndex {
    /// Build the index from the dumps in `opts.downloads_dir` (and the API if
    /// `opts.use_api`). Does not save; call [`CatalogIndex::save_default`].
    pub fn sync(opts: SyncOptions) -> Result<CatalogIndex> {
        let dir = &opts.downloads_dir;
        if opts.download_missing {
            ensure_downloads(dir)?;
        }
        let mut api = if opts.use_api { Some(Api::new()?) } else { None };

        // Colours.
        let colors = if let Some(api) = api.as_mut() {
            colors_from_api(&api.get_all("/colors/?page_size=1000")?)
        } else {
            let json = opts.colors_json.clone().or_else(|| {
                [Some(dir.join("rb_colors.json")), dir.parent().map(|p| p.join("rb_colors.json"))]
                    .into_iter()
                    .flatten()
                    .find(|p| p.exists())
            });
            match json {
                Some(p) => {
                    let v: Value = serde_json::from_str(
                        &std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?,
                    )
                    .with_context(|| format!("parsing {}", p.display()))?;
                    let items = v.get("results").and_then(Value::as_array).or_else(|| v.as_array()).cloned().unwrap_or_default();
                    colors_from_api(&items)
                }
                None => colors_from_csv(open_gz(&dir.join("colors.csv.gz"))?)?,
            }
        };
        if colors.is_empty() {
            bail!("no colours with LDraw codes found");
        }

        // Parts.
        let rb_parts = read_part_nums(open_gz(&dir.join("parts.csv.gz"))?)?;
        let ids: Vec<&str> = CATALOG.iter().map(|p| p.id).collect();
        let mappings = match api.as_mut() {
            Some(api) => map_online(api, &ids, &rb_parts)?,
            None => ids.iter().map(|id| map_offline(id, &rb_parts, opts.previous.as_ref())).collect(),
        };
        let rb_to_part: HashMap<String, usize> = mappings
            .iter()
            .enumerate()
            .filter_map(|(i, m)| m.rebrickable.clone().map(|r| (r, i)))
            .collect();
        let rb_color: HashMap<i32, ColorId> = colors.iter().map(|c| (c.rebrickable, c.ldraw)).collect();

        let inv_parts_path = dir.join("inventory_parts.csv.gz");
        let (stats, set_total) = count_sets(
            open_gz(&dir.join("sets.csv.gz"))?,
            open_gz(&dir.join("inventories.csv.gz"))?,
            open_gz(&inv_parts_path)?,
            &rb_to_part,
            &rb_color,
            ids.len(),
        )?;

        let parts = ids
            .iter()
            .zip(mappings)
            .zip(stats)
            .map(|((id, m), colors)| PartEntry {
                ldraw: id.to_string(),
                rebrickable: m.rebrickable,
                bricklink: m.bricklink,
                aliases: m.aliases,
                colors,
            })
            .collect();
        let dump_date = file_date(&inv_parts_path).unwrap_or_default();
        Ok(CatalogIndex::new(dump_date, set_total, parts, colors))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_core::Availability;

    const SETS: &str = "set_num,name,year,theme_id,num_parts,img_url\n\
        100-1,Old,1990,1,1,x\n200-1,Mid,2010,1,1,x\n300-1,New,2024,1,1,x\n400-1,Newer,2025,1,1,x\n";
    // Inventory 5 is a second version of 300-1; 9 is a minifig.
    const INVS: &str = "id,version,set_num\n1,1,100-1\n2,1,200-1\n3,1,300-1\n5,2,300-1\n4,1,400-1\n9,1,fig-000001\n";
    const INV_PARTS: &str = "inventory_id,part_num,color_id,quantity,is_spare,img_url\n\
        1,3001,4,2,False,x\n\
        2,3001,4,1,False,x\n\
        3,3001,4,6,False,x\n\
        5,3001,4,6,False,x\n\
        4,3001,25,1,True,x\n\
        9,3001,15,1,False,x\n\
        4,3001a,4,1,False,x\n\
        4,3001pr0001,4,1,False,x\n\
        4,3005,14,1,False,x\n\
        4,3005,9999,1,False,x\n\
        bad,3005,14,1,False,x\n";

    #[test]
    fn counts_distinct_sets_without_spares() {
        let rb_to_part: HashMap<String, usize> = [("3001".to_string(), 0), ("3005".to_string(), 1)].into();
        let rb_color: HashMap<i32, ColorId> = [(4, 4), (25, 25), (15, 15), (14, 14)].into();
        let (stats, total) =
            count_sets(SETS.as_bytes(), INVS.as_bytes(), INV_PARTS.as_bytes(), &rb_to_part, &rb_color, 2).unwrap();
        assert_eq!(total, 4);
        assert_eq!(stats[0].get(&4), Some(&ColorStat(3, 2024)), "two versions of 300-1 count once; 3001a/pr ignored");
        assert_eq!(stats[0].get(&25), None, "spare only");
        assert_eq!(stats[0].get(&15), None, "minifig inventory");
        assert_eq!(stats[1].get(&14), Some(&ColorStat(1, 2025)));
    }

    #[test]
    fn bad_csv_header_is_an_error() {
        let err = count_sets("nope\n1\n".as_bytes(), INVS.as_bytes(), INV_PARTS.as_bytes(), &HashMap::new(), &HashMap::new(), 0);
        assert!(err.is_err());
    }

    #[test]
    fn colours_from_api_shape() {
        let v: Value = serde_json::from_str(
            r#"{"results":[
              {"id":1,"name":"Blue","rgb":"0055BF","is_trans":false,"external_ids":{"BrickLink":{"ext_ids":[7]},"LDraw":{"ext_ids":[1]}}},
              {"id":-1,"name":"[Unknown]","rgb":"0033B2","is_trans":false,"external_ids":{}},
              {"id":1050,"name":"Weird","rgb":"000000","is_trans":true,"external_ids":{"LDraw":{"ext_ids":[10050]}}}
            ]}"#,
        )
        .unwrap();
        let c = colors_from_api(v["results"].as_array().unwrap());
        assert_eq!(c.len(), 2);
        assert_eq!((c[0].ldraw, c[0].bricklink), (1, Some(7)));
        assert_eq!((c[1].ldraw, c[1].rebrickable, c[1].bricklink, c[1].trans), (10050, 1050, None, true));
    }

    #[test]
    fn colours_from_csv_shape() {
        let csv = "id,name,rgb,is_trans,num_parts,num_sets,y1,y2\n-1,[Unknown],0033B2,False,1,1,2000,2000\n36,Trans-Red,C91A09,True,1,1,1,1\n";
        let c = colors_from_csv(csv.as_bytes()).unwrap();
        assert_eq!(c.len(), 1);
        assert!(c[0].trans);
    }

    #[test]
    fn pick_prefers_rebrickables_ldraw_ids() {
        let info: HashMap<String, RbPart> = [
            ("3005".to_string(), RbPart { ldraw: vec!["3005".into(), "30071".into()], bricklink: vec!["3005".into()] }),
            ("3660".to_string(), RbPart { ldraw: vec!["3660b".into(), "3660c".into()], bricklink: vec!["3660".into()] }),
            ("3660b".to_string(), RbPart { ldraw: vec!["3660a".into()], bricklink: vec!["3660b".into()] }),
        ]
        .into();
        assert_eq!(pick("3005", &[], &info).as_deref(), Some("3005"));
        assert_eq!(pick("30071", &[], &info).as_deref(), Some("3005"));
        // LDraw 3660b is Rebrickable's current 3660, not Rebrickable 3660b.
        assert_eq!(pick("3660b", &["3660b".into(), "3660".into()], &info).as_deref(), Some("3660"));
        // LDraw 3660 (a redirect) falls back to the identical number.
        assert_eq!(pick("3660", &["3660".into()], &info).as_deref(), Some("3660"));
        assert_eq!(pick("9999", &[], &info), None);
    }

    #[test]
    fn offline_mapping() {
        let rb: HashSet<String> = ["3001", "3665", "3070b"].iter().map(|s| s.to_string()).collect();
        let m = map_offline("3665b", &rb, None);
        assert_eq!(m.rebrickable.as_deref(), Some("3665"));
        assert_eq!(m.bricklink.as_deref(), Some("3665"));
        let m = map_offline("3070b", &rb, None);
        assert_eq!(m.bricklink.as_deref(), Some("3070"));
        assert_eq!(map_offline("3039", &rb, None), Mapping::default());
    }

    #[test]
    fn civil_dates() {
        assert_eq!(civil_date(0), "1970-01-01");
        assert_eq!(civil_date(20_412), "2025-11-20");
        assert_eq!(civil_date(11_016), "2000-02-29");
    }

    /// Real sync against the local dumps and the live API. Run with
    /// `cargo test -p sf-catalog -- --ignored --nocapture real_sync`.
    #[test]
    #[ignore]
    fn real_sync() {
        let dir = default_downloads_dir().unwrap();
        let use_api = std::env::var("SF_SYNC_NO_API").is_err();
        let t = std::time::Instant::now();
        let idx = CatalogIndex::sync(SyncOptions { download_missing: true, ..SyncOptions::new(&dir, use_api) }).unwrap();
        let took = t.elapsed();
        let path = CatalogIndex::default_path().unwrap();
        idx.save(&path).unwrap();
        let size = std::fs::metadata(&path).unwrap().len();
        println!("sync took {took:.2?} (api: {use_api}); {} sets; dump {}; {} bytes at {}", idx.set_total, idx.dump_date, size, path.display());
        println!("{:<7} {:<8} {:<8} {:>7} aliases", "ldraw", "rebrick", "bricklk", "colours");
        for p in &idx.parts {
            println!(
                "{:<7} {:<8} {:<8} {:>7} {:?}",
                p.ldraw,
                p.rebrickable.as_deref().unwrap_or("-"),
                p.bricklink.as_deref().unwrap_or("-"),
                p.colors.len(),
                p.aliases
            );
            if use_api {
                assert_eq!(p.bricklink.as_deref(), builtin_bricklink_part(&p.ldraw), "builtin table out of date for {}", p.ldraw);
            }
        }
        let orange: Vec<_> = sf_core::CATALOG.iter().filter(|p| idx.is_made(p.id, 25) == Some(true)).map(|p| p.id).collect();
        println!("made in Orange: {}/{} ({:?})", orange.len(), sf_core::CATALOG.len(), orange);
        for (part, col) in [("3039", 27), ("3005", 4), ("3005", 25), ("54200", 27), ("3665", 26), ("3660", 31), ("3958", 25)] {
            println!(
                "{part} in {}: made={:?} sets={:?} last={:?} rare={}",
                idx.color(col).map(|c| c.name.as_str()).unwrap_or("?"),
                idx.is_made(part, col),
                idx.set_count(part, col),
                idx.last_year(part, col),
                idx.is_rare(part, col)
            );
        }
        let pal = idx.palette();
        let dropped: Vec<_> = sf_core::Palette::builtin().colors.into_iter().filter(|c| !pal.colors.iter().any(|p| p.name == c.name)).map(|c| c.name).collect();
        println!("palette: {} colours; dropped {:?}", pal.colors.len(), dropped);
        for c in &pal.colors {
            let builtin = sf_core::Palette::builtin().colors.into_iter().find(|b| b.name == c.name);
            if builtin.as_ref().map(|b| (b.code, b.bricklink)) != Some((c.code, c.bricklink)) {
                println!("  {} differs: builtin {:?}, synced ({}, {:?})", c.name, builtin.map(|b| (b.code, b.bricklink)), c.code, c.bricklink);
            }
        }
        assert!(idx.is_made("3005", 4) == Some(true));
    }
}
