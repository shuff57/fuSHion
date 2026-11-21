//! A minimal PDF 1.4 writer: pages of text (base-14 Helvetica and Helvetica-Bold,
//! WinAnsi encoding), filled and stroked rectangles, lines and JPEG images.
//!
//! Layout code works in points with the origin at the **top-left** of a US Letter page;
//! [`Page`] flips to PDF's bottom-left origin when it writes operators.

use flate2::Compression;
use flate2::write::ZlibEncoder;
use std::fmt::Write as _;
use std::io::Write as _;

/// US Letter, portrait, in points.
pub const PAGE_W: f32 = 612.0;
pub const PAGE_H: f32 = 792.0;

/// A font: regular or bold Helvetica.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Font {
    Regular,
    Bold,
}

impl Font {
    fn resource(self) -> &'static str {
        match self {
            Font::Regular => "F1",
            Font::Bold => "F2",
        }
    }
}

/// An image added to the document, referenced by pages.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ImageId {
    idx: usize,
    w: u32,
    h: u32,
}

struct Image {
    w: u32,
    h: u32,
    jpeg: Vec<u8>,
}

/// One page's drawing operators (top-left coordinates in, PDF coordinates out).
#[derive(Default)]
pub struct Page {
    ops: String,
    images: Vec<ImageId>,
}

fn f(v: f32) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r == r.trunc() { format!("{}", r as i64) } else { format!("{r}") }
}

fn rgb(c: [u8; 3]) -> String {
    format!("{} {} {}", f(c[0] as f32 / 255.0), f(c[1] as f32 / 255.0), f(c[2] as f32 / 255.0))
}

impl Page {
    /// Text with its baseline at `y` (from the top).
    pub fn text(&mut self, x: f32, y: f32, size: f32, font: Font, color: [u8; 3], s: &str) {
        let _ = writeln!(
            self.ops,
            "BT /{} {} Tf {} rg {} {} Td {} Tj ET",
            font.resource(),
            f(size),
            rgb(color),
            f(x),
            f(PAGE_H - y),
            pdf_string(s)
        );
    }

    /// Text centred on `cx`.
    pub fn text_center(&mut self, cx: f32, y: f32, size: f32, font: Font, color: [u8; 3], s: &str) {
        self.text(cx - text_width(s, font, size) / 2.0, y, size, font, color, s);
    }

    /// Text ending at `right`.
    pub fn text_right(&mut self, right: f32, y: f32, size: f32, font: Font, color: [u8; 3], s: &str) {
        self.text(right - text_width(s, font, size), y, size, font, color, s);
    }

    /// A filled rectangle; `(x, y)` is its top-left corner.
    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: [u8; 3]) {
        let _ = writeln!(self.ops, "{} rg {} {} {} {} re f", rgb(color), f(x), f(PAGE_H - y - h), f(w), f(h));
    }

    /// A rectangle outline.
    pub fn stroke_rect(&mut self, x: f32, y: f32, w: f32, h: f32, width: f32, color: [u8; 3]) {
        let _ = writeln!(self.ops, "{} RG {} w {} {} {} {} re S", rgb(color), f(width), f(x), f(PAGE_H - y - h), f(w), f(h));
    }

    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: [u8; 3]) {
        let _ = writeln!(
            self.ops,
            "{} RG {} w {} {} m {} {} l S",
            rgb(color),
            f(width),
            f(x0),
            f(PAGE_H - y0),
            f(x1),
            f(PAGE_H - y1)
        );
    }

    /// An image scaled into the box `(x, y, w, h)` (top-left corner), keeping its aspect.
    pub fn image(&mut self, id: ImageId, x: f32, y: f32, w: f32, h: f32) {
        let (ix, iy, s) = id.fit(x, y, w, h);
        let (iw, ih) = (id.w as f32 * s, id.h as f32 * s);
        let _ = writeln!(self.ops, "q {} 0 0 {} {} {} cm /Im{} Do Q", f(iw), f(ih), f(ix), f(PAGE_H - iy - ih), id.idx);
        if !self.images.contains(&id) {
            self.images.push(id);
        }
    }

    /// A circle centred on `(cx, cy)`, filled and/or outlined.
    pub fn circle(&mut self, cx: f32, cy: f32, r: f32, fill: Option<[u8; 3]>, stroke: Option<([u8; 3], f32)>) {
        // Four cubic Béziers.
        let k = 0.552_284_8 * r;
        let y = PAGE_H - cy;
        let mut ops = String::new();
        if let Some(c) = fill {
            let _ = write!(ops, "{} rg ", rgb(c));
        }
        if let Some((c, w)) = stroke {
            let _ = write!(ops, "{} RG {} w ", rgb(c), f(w));
        }
        let _ = write!(
            ops,
            "{} {} m {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c h ",
            f(cx + r), f(y),
            f(cx + r), f(y + k), f(cx + k), f(y + r), f(cx), f(y + r),
            f(cx - k), f(y + r), f(cx - r), f(y + k), f(cx - r), f(y),
            f(cx - r), f(y - k), f(cx - k), f(y - r), f(cx), f(y - r),
            f(cx + k), f(y - r), f(cx + r), f(y - k), f(cx + r), f(y),
        );
        ops.push_str(match (fill.is_some(), stroke.is_some()) {
            (true, true) => "B",
            (true, false) => "f",
            (false, true) => "S",
            (false, false) => "n",
        });
        let _ = writeln!(self.ops, "{ops}");
    }

    /// Wrapped text starting with its first baseline at `y`; returns the y below the last line.
    #[allow(clippy::too_many_arguments)]
    pub fn paragraph(&mut self, x: f32, y: f32, width: f32, size: f32, font: Font, color: [u8; 3], s: &str) -> f32 {
        let lead = size * 1.3;
        let mut yy = y;
        for line in wrap(s, font, size, width) {
            self.text(x, yy, size, font, color, &line);
            yy += lead;
        }
        yy
    }
}

impl ImageId {
    /// Pixel size.
    pub fn size(self) -> (u32, u32) {
        (self.w, self.h)
    }

    /// Where the image lands when fitted into the box `(x, y, w, h)`: its top-left corner and
    /// points per pixel.
    pub fn fit(self, x: f32, y: f32, w: f32, h: f32) -> (f32, f32, f32) {
        let s = (w / self.w as f32).min(h / self.h as f32);
        let (iw, ih) = (self.w as f32 * s, self.h as f32 * s);
        (x + (w - iw) / 2.0, y + (h - ih) / 2.0, s)
    }
}

/// A document under construction.
#[derive(Default)]
pub struct Pdf {
    images: Vec<Image>,
    pub pages: Vec<Page>,
}

impl Pdf {
    /// Add an RGB image, stored as JPEG at `quality` (1–100).
    pub fn add_image(&mut self, img: &image::RgbImage, quality: u8) -> anyhow::Result<ImageId> {
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, quality.clamp(1, 100)).encode_image(img)?;
        self.images.push(Image { w: img.width(), h: img.height(), jpeg });
        Ok(ImageId { idx: self.images.len() - 1, w: img.width(), h: img.height() })
    }

    /// Serialise the document.
    pub fn to_bytes(&self, title: &str) -> anyhow::Result<Vec<u8>> {
        // Object numbers: 1 catalog, 2 pages, 3 Helvetica, 4 Helvetica-Bold, 5 info,
        // then images, then (page, content) pairs.
        let img0 = 6;
        let page0 = img0 + self.images.len();
        let n_objects = page0 + 2 * self.pages.len() - 1;
        let mut out: Vec<u8> = Vec::new();
        let mut offsets = vec![0usize; n_objects + 1];
        out.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
        let mut obj = |out: &mut Vec<u8>, n: usize, body: &[u8]| {
            offsets[n] = out.len();
            out.extend_from_slice(format!("{n} 0 obj\n").as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        };
        obj(&mut out, 1, b"<< /Type /Catalog /Pages 2 0 R >>");
        let kids: Vec<String> = (0..self.pages.len()).map(|i| format!("{} 0 R", page0 + 2 * i)).collect();
        obj(
            &mut out,
            2,
            format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), self.pages.len()).as_bytes(),
        );
        obj(&mut out, 3, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>");
        obj(&mut out, 4, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>");
        obj(
            &mut out,
            5,
            format!("<< /Title {} /Producer (fuSHion sf-pamphlet) >>", pdf_string(title)).as_bytes(),
        );
        for (i, img) in self.images.iter().enumerate() {
            let mut body = format!(
                "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
                img.w,
                img.h,
                img.jpeg.len()
            )
            .into_bytes();
            body.extend_from_slice(&img.jpeg);
            body.extend_from_slice(b"\nendstream");
            obj(&mut out, img0 + i, &body);
        }
        for (i, page) in self.pages.iter().enumerate() {
            let (pn, cn) = (page0 + 2 * i, page0 + 2 * i + 1);
            let xobjects: String = page.images.iter().map(|id| format!("/Im{} {} 0 R ", id.idx, img0 + id.idx)).collect();
            obj(
                &mut out,
                pn,
                format!(
                    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << /Font << /F1 3 0 R /F2 4 0 R >> /XObject << {}>> >> /Contents {} 0 R >>",
                    f(PAGE_W),
                    f(PAGE_H),
                    xobjects,
                    cn
                )
                .as_bytes(),
            );
            let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
            z.write_all(page.ops.as_bytes())?;
            let data = z.finish()?;
            let mut body = format!("<< /Length {} /Filter /FlateDecode >>\nstream\n", data.len()).into_bytes();
            body.extend_from_slice(&data);
            body.extend_from_slice(b"\nendstream");
            obj(&mut out, cn, &body);
        }
        let xref = out.len();
        let mut x = format!("xref\n0 {}\n0000000000 65535 f \n", n_objects + 1);
        for off in &offsets[1..] {
            let _ = writeln!(x, "{off:010} 00000 n ");
        }
        let _ = write!(x, "trailer\n<< /Size {} /Root 1 0 R /Info 5 0 R >>\nstartxref\n{xref}\n%%EOF\n", n_objects + 1);
        out.extend_from_slice(x.as_bytes());
        Ok(out)
    }
}

/// A character in WinAnsi, or `?` when it has none.
fn win_ansi(c: char) -> u8 {
    match c {
        ' '..='~' => c as u8,
        '×' => 0xD7,
        '•' => 0x95,
        '–' => 0x96,
        '—' => 0x97,
        '°' => 0xB0,
        '·' => 0xB7,
        '‘' => 0x91,
        '’' => 0x92,
        '“' => 0x93,
        '”' => 0x94,
        '…' => 0x85,
        '\u{a0}'..='\u{ff}' => c as u8,
        _ => b'?',
    }
}

/// A PDF literal string in WinAnsi.
pub fn pdf_string(s: &str) -> String {
    let mut out = String::from("(");
    for c in s.chars() {
        match win_ansi(c) {
            b'(' => out.push_str("\\("),
            b')' => out.push_str("\\)"),
            b'\\' => out.push_str("\\\\"),
            b @ 32..=126 => out.push(b as char),
            b => {
                let _ = write!(out, "\\{b:03o}");
            }
        }
    }
    out.push(')');
    out
}

// Advance widths (1/1000 em) of Helvetica and Helvetica-Bold for ASCII 32..=126 (Adobe AFM).
const HELV: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556,
    556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833,
    722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556,
    556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334,
    260, 334, 584,
];
const HELV_BOLD: [u16; 95] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556,
    556, 556, 556, 333, 333, 584, 584, 584, 611, 975, 722, 722, 722, 722, 667, 611, 778, 722, 278, 556, 722, 611, 833,
    722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 333, 278, 333, 584, 556, 333, 556, 611, 556, 611,
    556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611, 611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, 389,
    280, 389, 584,
];

fn char_width(b: u8, font: Font) -> u16 {
    let table = match font {
        Font::Regular => &HELV,
        Font::Bold => &HELV_BOLD,
    };
    match b {
        32..=126 => table[(b - 32) as usize],
        0xD7 => 584,
        0x95 => 350,
        0x96 => 556,
        0x97 | 0x85 => 1000,
        0xB0 => 400,
        0xB7 => 278,
        0x91 | 0x92 => if font == Font::Bold { 278 } else { 222 },
        0x93 | 0x94 => if font == Font::Bold { 500 } else { 333 },
        _ => 556,
    }
}

/// Width of `s` in points.
pub fn text_width(s: &str, font: Font, size: f32) -> f32 {
    s.chars().map(|c| char_width(win_ansi(c), font) as f32).sum::<f32>() * size / 1000.0
}

/// Greedy word wrap to `width` points (a word longer than a line is cut).
pub fn wrap(s: &str, font: Font, size: f32, width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for para in s.split('\n') {
        let mut line = String::new();
        for word in para.split_whitespace() {
            let cand = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if text_width(&cand, font, size) <= width || line.is_empty() && text_width(word, font, size) <= width {
                line = cand;
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            // Cut words wider than the line.
            let mut w = String::new();
            for c in word.chars() {
                w.push(c);
                if text_width(&w, font, size) > width && w.chars().count() > 1 {
                    w.pop();
                    lines.push(std::mem::take(&mut w));
                    w.push(c);
                }
            }
            line = w;
        }
        lines.push(line);
    }
    lines
}

/// Truncate `s` with an ellipsis to fit `width`.
pub fn fit(s: &str, font: Font, size: f32, width: f32) -> String {
    if text_width(s, font, size) <= width {
        return s.to_string();
    }
    let mut out = String::new();
    for c in s.chars() {
        out.push(c);
        if text_width(&format!("{out}…"), font, size) > width {
            out.pop();
            break;
        }
    }
    format!("{}…", out.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_escape_and_encode() {
        assert_eq!(pdf_string("a(b)c\\"), "(a\\(b\\)c\\\\)");
        assert_eq!(pdf_string("2×3 – ok"), "(2\\3273 \\226 ok)");
        assert_eq!(pdf_string("☐"), "(?)");
    }

    #[test]
    fn widths_and_wrap() {
        assert!((text_width("Hello", Font::Regular, 10.0) - 22.78).abs() < 0.01);
        assert!(text_width("Hello", Font::Bold, 10.0) > text_width("Hello", Font::Regular, 10.0));
        let lines = wrap("the quick brown fox jumps over the lazy dog", Font::Regular, 10.0, 60.0);
        assert!(lines.len() > 2);
        assert!(lines.iter().all(|l| text_width(l, Font::Regular, 10.0) <= 60.0));
        assert_eq!(fit("short", Font::Regular, 10.0, 100.0), "short");
        assert!(fit("a much longer piece of text", Font::Regular, 10.0, 50.0).ends_with('…'));
    }
}
