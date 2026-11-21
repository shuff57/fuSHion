//! Relief (2.5D) specs: a picture built lying flat, studs up, with optional
//! raised areas. The picture's top row is the far edge (`+z`), so the top
//! view reads like the image.

#![allow(clippy::needless_range_loop)] // grid code reads clearer with indices

use crate::{CompileOptions, Compiled, MAX_RELIEF_PLATES, MAX_RELIEF_SIDE, SAMPLES, check_dims, resolve_color};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use sf_core::{Cell, ColorId, Fill, Palette, VoxelGrid};
use std::collections::BTreeMap;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReliefSource {
    /// Colours come from `rows` + `legend`.
    #[default]
    Rows,
    /// Colours are sampled from the source image.
    Image,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReliefHeights {
    /// Every cell is `base_plates` tall.
    #[default]
    Flat,
    /// Brighter cells rise up to `max_plates` above the base.
    Luminance { max_plates: u32 },
    /// Per-cell extra plates, one character per cell (`0`–`9`, then `a`–`z`
    /// for 10–35), same layout as the colours.
    Rows { rows: Vec<String> },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReliefSpec {
    #[serde(default)]
    pub source: ReliefSource,
    /// Colour rows, top row first. Each character is a key in `legend`;
    /// `.` or space means empty.
    #[serde(default)]
    pub rows: Vec<String>,
    #[serde(default)]
    pub legend: BTreeMap<String, String>,
    /// Width in studs for `source: image`.
    #[serde(default)]
    pub width: Option<u32>,
    /// Limit the number of colours for `source: image`.
    #[serde(default)]
    pub max_colors: Option<u32>,
    /// For `source: image`: remove the background, i.e. the region joined
    /// to the picture's border whose colour matches the border colour.
    #[serde(default)]
    pub drop_background: bool,
    #[serde(default)]
    pub heights: ReliefHeights,
    /// Plates under every cell (minimum 1).
    #[serde(default = "one")]
    pub base_plates: u32,
}

fn one() -> u32 {
    1
}

fn height_char(c: char) -> Option<u32> {
    c.to_digit(36)
}

pub(crate) fn compile(
    r: &ReliefSpec,
    palette: &Palette,
    opts: &CompileOptions,
    _default: ColorId,
    warnings: &mut Vec<String>,
) -> Result<Compiled> {
    // colours[z_row][x] in image order (row 0 = top)
    let colours: Vec<Vec<Option<ColorId>>> = match r.source {
        ReliefSource::Rows => {
            if r.rows.is_empty() {
                bail!("relief with `source: rows` needs `rows`");
            }
            let mut legend: BTreeMap<char, ColorId> = BTreeMap::new();
            for (k, v) in &r.legend {
                let mut chars = k.chars();
                let (Some(ch), None) = (chars.next(), chars.next()) else {
                    bail!("relief legend keys must be single characters, got `{k}`");
                };
                legend.insert(ch, resolve_color(palette, v, warnings));
            }
            let mut unknown = std::collections::BTreeSet::new();
            let rows = r
                .rows
                .iter()
                .map(|row| {
                    row.chars()
                        .map(|ch| match ch {
                            '.' | ' ' => None,
                            _ => match legend.get(&ch) {
                                Some(c) => Some(*c),
                                None => {
                                    unknown.insert(ch);
                                    None
                                }
                            },
                        })
                        .collect()
                })
                .collect();
            if !unknown.is_empty() {
                warnings.push(format!("relief rows use characters missing from `legend`: {unknown:?}; left empty"));
            }
            rows
        }
        ReliefSource::Image => {
            let Some(img) = opts.image else { bail!("relief with `source: image` needs the source image") };
            let w = opts.resolution.or(r.width).unwrap_or(32).clamp(4, 256);
            let mut cells = sample_image(img, w, palette, r.max_colors);
            if r.drop_background {
                drop_background(&mut cells, palette);
                // Crop the picture to the subject, then sample again so the
                // subject spans the full width.
                if let Some(cropped) = crop_to_cells(img, &cells) {
                    cells = sample_image(&cropped, w, palette, r.max_colors);
                    drop_background(&mut cells, palette);
                }
            }
            cells
        }
    };
    if colours.len() > MAX_RELIEF_SIDE || colours.iter().any(|r| r.len() > MAX_RELIEF_SIDE) {
        bail!("relief is larger than {MAX_RELIEF_SIDE} × {MAX_RELIEF_SIDE} cells; use fewer or shorter rows");
    }
    let depth = colours.len() as i32;
    let width = colours.iter().map(|r| r.len()).max().unwrap_or(0) as i32;
    if width == 0 {
        bail!("relief is empty");
    }
    let extra = |x: usize, row: usize| -> u32 {
        match &r.heights {
            ReliefHeights::Flat => 0,
            ReliefHeights::Rows { rows } => rows.get(row).and_then(|s| s.chars().nth(x)).and_then(height_char).unwrap_or(0),
            ReliefHeights::Luminance { max_plates } => {
                let c = colours[row][x].map(|c| palette.rgb(c)).unwrap_or([0, 0, 0]);
                let l = sf_core::color::srgb_to_lab(c)[0] / 100.0;
                (l * (*max_plates).min(MAX_RELIEF_PLATES) as f64).round() as u32
            }
        }
    };
    let base = r.base_plates.clamp(1, MAX_RELIEF_PLATES);
    let mut max_h = base;
    for (row, cols) in colours.iter().enumerate() {
        for x in 0..cols.len() {
            if cols[x].is_some() {
                max_h = max_h.max(base + extra(x, row));
            }
        }
    }
    let dims = [width, max_h.min(2 * MAX_RELIEF_PLATES) as i32, depth];
    check_dims(dims)?;
    let mut grid = VoxelGrid::new(dims);
    for (row, cols) in colours.iter().enumerate() {
        let z = depth - 1 - row as i32;
        for (x, c) in cols.iter().enumerate() {
            if let Some(c) = c {
                let h = base + extra(x, row);
                for y in 0..h as i32 {
                    grid.set(Cell::new(x as i32, y, z), Fill::Solid(*c));
                }
            }
        }
    }
    let full = u32::MAX >> (32 - SAMPLES);
    let samples = (0..dims[1])
        .flat_map(|y| (0..dims[2]).flat_map(move |z| (0..dims[0]).map(move |x| Cell::new(x, y, z))))
        .map(|c| if grid.get(c).is_filled() { full } else { 0 })
        .collect();
    Ok(Compiled {
        grid,
        samples,
        warnings: Vec::new(),
        units_per_stud: 0.0,
        map: crate::GridMap::Relief { depth, shift: [0; 3] },
        details: Vec::new(),
        surface: Vec::new(),
        bouquet: None,
    })
}

/// Flood-fill from the border through cells close in colour (ΔE00 < 12) to
/// the most common border colour, and clear them.
fn drop_background(cells: &mut [Vec<Option<ColorId>>], palette: &Palette) {
    let h = cells.len();
    let Some(w) = cells.first().map(|r| r.len()) else { return };
    let mut counts: BTreeMap<ColorId, usize> = BTreeMap::new();
    for (y, row) in cells.iter().enumerate() {
        for (x, c) in row.iter().enumerate() {
            if (y == 0 || y + 1 == h || x == 0 || x + 1 == w)
                && let Some(c) = c
            {
                *counts.entry(*c).or_default() += 1;
            }
        }
    }
    let Some((&bg, _)) = counts.iter().max_by_key(|(_, n)| **n) else { return };
    let bg_lab = sf_core::color::srgb_to_lab(palette.rgb(bg));
    let close = |c: Option<ColorId>| {
        c.is_some_and(|c| sf_core::color::ciede2000(bg_lab, sf_core::color::srgb_to_lab(palette.rgb(c))) < 12.0)
    };
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for y in 0..h {
        for x in 0..w {
            if (y == 0 || y + 1 == h || x == 0 || x + 1 == w) && close(cells[y][x]) {
                stack.push((x, y));
            }
        }
    }
    while let Some((x, y)) = stack.pop() {
        if !close(cells[y][x]) {
            continue;
        }
        cells[y][x] = None;
        if x > 0 { stack.push((x - 1, y)); }
        if x + 1 < w { stack.push((x + 1, y)); }
        if y > 0 { stack.push((x, y - 1)); }
        if y + 1 < h { stack.push((x, y + 1)); }
    }
}

/// Crop `img` to the bounding box of non-empty cells (plus one cell).
fn crop_to_cells(img: &image::RgbaImage, cells: &[Vec<Option<ColorId>>]) -> Option<image::RgbaImage> {
    let h = cells.len();
    let w = cells.first()?.len();
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for (y, row) in cells.iter().enumerate() {
        for (x, c) in row.iter().enumerate() {
            if c.is_some() {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if x0 == usize::MAX {
        return None;
    }
    let (iw, ih) = img.dimensions();
    let px = |c: usize, n: usize, size: u32| (c as u64 * size as u64 / n as u64) as u32;
    let (ax, ay) = (px(x0.saturating_sub(1), w, iw), px(y0.saturating_sub(1), h, ih));
    let (bx, by) = (px((x1 + 2).min(w), w, iw), px((y1 + 2).min(h), h, ih));
    if bx <= ax || by <= ay {
        return None;
    }
    Some(image::imageops::crop_imm(img, ax, ay, bx - ax, by - ay).to_image())
}

/// Average the image over a `w`-wide grid of cells and snap to the palette.
fn sample_image(img: &image::RgbaImage, w: u32, palette: &Palette, max_colors: Option<u32>) -> Vec<Vec<Option<ColorId>>> {
    let (iw, ih) = img.dimensions();
    let h = ((w as f64 * ih as f64 / iw.max(1) as f64).round() as u32).clamp(1, MAX_RELIEF_SIDE as u32);
    let mut avg = vec![vec![None; w as usize]; h as usize];
    for (row, line) in avg.iter_mut().enumerate() {
        for (x, slot) in line.iter_mut().enumerate() {
            let (x0, x1) = (x as u32 * iw / w, ((x as u32 + 1) * iw / w).max(x as u32 * iw / w + 1));
            let (y0, y1) = (row as u32 * ih / h, ((row as u32 + 1) * ih / h).max(row as u32 * ih / h + 1));
            let (mut r, mut g, mut b, mut a, mut n) = (0f64, 0f64, 0f64, 0f64, 0f64);
            for py in y0..y1.min(ih) {
                for px in x0..x1.min(iw) {
                    let p = img.get_pixel(px, py).0;
                    let al = p[3] as f64 / 255.0;
                    r += p[0] as f64 * al;
                    g += p[1] as f64 * al;
                    b += p[2] as f64 * al;
                    a += al;
                    n += 1.0;
                }
            }
            if n > 0.0 && a / n >= 0.5 {
                *slot = Some([(r / a) as u8, (g / a) as u8, (b / a) as u8]);
            }
        }
    }
    let snap = |pal: &Palette| -> Vec<Vec<Option<ColorId>>> {
        avg.iter().map(|l| l.iter().map(|c| c.and_then(|c| pal.nearest(c))).collect()).collect()
    };
    let first = snap(palette);
    let Some(k) = max_colors else { return first };
    let mut counts: BTreeMap<ColorId, usize> = BTreeMap::new();
    for c in first.iter().flatten().flatten() {
        *counts.entry(*c).or_default() += 1;
    }
    if counts.len() <= k as usize {
        return first;
    }
    let mut by: Vec<_> = counts.into_iter().collect();
    by.sort_by_key(|e| std::cmp::Reverse(e.1));
    let keep: Vec<ColorId> = by.iter().take(k.max(1) as usize).map(|(c, _)| *c).collect();
    let sub = Palette { colors: palette.colors.iter().filter(|c| keep.contains(&c.code)).cloned().collect() };
    snap(&sub)
}

#[cfg(test)]
mod tests {
    use crate::{CompileOptions, Spec, compile};
    use sf_core::{Cell, Fill, Palette};

    #[test]
    fn rows_relief_with_heights() {
        let s = Spec::from_json(
            r#"{"mode":"relief","relief":{"rows":["rr.","bbb"],"legend":{"r":"Red","b":"Blue"},
            "heights":{"rows":{"rows":["20.","000"]}}}}"#,
        )
        .unwrap();
        let c = compile(&s, &Palette::builtin(), &CompileOptions::default()).unwrap();
        assert_eq!(c.grid.dims, [3, 3, 2]);
        // top row is the far edge (z = 1)
        assert_eq!(c.grid.get(Cell::new(0, 2, 1)), Fill::Solid(4));
        assert_eq!(c.grid.get(Cell::new(2, 0, 1)), Fill::Empty);
        assert_eq!(c.grid.get(Cell::new(2, 0, 0)), Fill::Solid(1));
    }

    #[test]
    fn image_relief_snaps_colours() {
        let mut img = image::RgbaImage::new(4, 2);
        for (x, _, p) in img.enumerate_pixels_mut() {
            *p = if x < 2 { image::Rgba([250, 0, 0, 255]) } else { image::Rgba([0, 0, 0, 0]) };
        }
        let s = Spec::from_json(r#"{"mode":"relief","relief":{"source":"image","width":4}}"#).unwrap();
        let c = compile(&s, &Palette::builtin(), &CompileOptions { resolution: None, image: Some(&img) }).unwrap();
        assert_eq!(c.grid.dims, [4, 1, 2]);
        assert_eq!(c.grid.get(Cell::new(0, 0, 0)), Fill::Solid(4));
        assert_eq!(c.grid.get(Cell::new(3, 0, 0)), Fill::Empty);
    }
}
