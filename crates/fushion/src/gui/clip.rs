//! Read a picture off the system clipboard: an image (a screenshot, "Copy image" in a
//! browser) or a copied image file (the path or `file://` link a file manager puts there).

use image::RgbaImage;

/// The picture on the clipboard, or why there isn't one.
pub fn read_image() -> Result<RgbaImage, String> {
    #[cfg(target_os = "linux")]
    {
        // arboard speaks X11 only here, which on a Wayland session sees only XWayland
        // programs' clipboard, so ask the compositor's own tools first.
        let tried = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            from_tool("wl-paste", &["--list-types"], |t| {
                vec!["--type".into(), t.into()]
            })
        } else {
            None
        };
        if let Some(r) = tried {
            return r;
        }
        if let Some(r) = from_tool(
            "xclip",
            &["-selection", "clipboard", "-t", "TARGETS", "-o"],
            |t| {
                ["-selection", "clipboard", "-t", t, "-o"]
                    .map(String::from)
                    .to_vec()
            },
        ) {
            return r;
        }
    }
    from_arboard()
}

fn from_arboard() -> Result<RgbaImage, String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| format!("no clipboard: {e}"))?;
    match cb.get_image() {
        Ok(img) => RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned())
            .ok_or_else(|| "the clipboard image is malformed".to_string()),
        Err(_) => {
            // A copied file path as text.
            let text = cb
                .get_text()
                .map_err(|_| "the clipboard has no picture".to_string())?;
            from_path_text(&text)
        }
    }
}

/// Runs `tool list_args` to see what the clipboard holds, then reads the best match:
/// an image type, else a `text/uri-list` or plain-text path. `None` when the tool isn't
/// installed or fails, so the caller can try another route.
#[cfg(target_os = "linux")]
fn from_tool(
    tool: &str,
    list_args: &[&str],
    read_args: impl Fn(&str) -> Vec<String>,
) -> Option<Result<RgbaImage, String>> {
    use std::process::Command;
    let out = Command::new(tool).args(list_args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let types = String::from_utf8_lossy(&out.stdout).into_owned();
    let has = |t: &str| types.lines().any(|l| l.trim() == t);
    let read = |t: &str| -> Option<Vec<u8>> {
        let o = Command::new(tool).args(read_args(t)).output().ok()?;
        (o.status.success() && !o.stdout.is_empty()).then_some(o.stdout)
    };
    for t in ["image/png", "image/jpeg", "image/webp"] {
        if has(t) {
            let bytes = read(t)?;
            return Some(
                image::load_from_memory(&bytes)
                    .map(|i| i.to_rgba8())
                    .map_err(|e| format!("could not read the clipboard image: {e}")),
            );
        }
    }
    for t in [
        "text/uri-list",
        "text/plain;charset=utf-8",
        "text/plain",
        "UTF8_STRING",
    ] {
        if has(t) {
            let bytes = read(t)?;
            return Some(from_path_text(&String::from_utf8_lossy(&bytes)));
        }
    }
    Some(Err("the clipboard has no picture".into()))
}

/// The first line of `text` as an image file: a path or a `file://` link.
fn from_path_text(text: &str) -> Result<RgbaImage, String> {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'));
    let Some(line) = line else {
        return Err("the clipboard has no picture".into());
    };
    let path = match line.strip_prefix("file://") {
        Some(p) => percent_decode(p),
        None => line.to_string(),
    };
    image::open(&path)
        .map(|i| i.to_rgba8())
        .map_err(|_| "the clipboard has no picture".to_string())
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Some(v) = std::str::from_utf8(&b[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
