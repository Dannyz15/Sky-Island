//! File icons for shelf tiles, from Explorer on Windows or XDG icon themes on Linux.

#[cfg(windows)]
use std::ffi::c_void;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::thread;

#[cfg(not(any(windows, target_os = "linux")))]
use tiny_skia::Pixmap;
#[cfg(any(windows, target_os = "linux"))]
use tiny_skia::{Pixmap, PremultipliedColorU8};
#[cfg(windows)]
use windows::Win32::Foundation::SIZE;
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits, GetObjectW, HGDIOBJ, ReleaseDC};
#[cfg(windows)]
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
#[cfg(windows)]
use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY};
#[cfg(windows)]
use windows::core::PCWSTR;
use winit::event_loop::EventLoopProxy;

use crate::UserEvent;

/// Requested icon size in pixels; drawn scaled down on screen so it stays sharp on high-DPI displays.
#[cfg(any(windows, target_os = "linux"))]
const SIZE_PX: i32 = 64;
/// The largest icon, across or down, that is read. The shell may return a bigger one than asked for
/// (`SIIGBF_BIGGERSIZEOK`), and the picture size comes from it, so it is checked before any memory is taken for it.
#[cfg(any(windows, target_os = "linux", test))]
const MAX_SIDE: i32 = 1024;

/// The bytes of a 32-bit picture of `w` x `h` pixels: `None` for an empty one, one bigger than any icon, or a size
/// that does not fit (the sum used to be `w * h * 4` in `i32`, which wraps for a huge size reported by the shell).
#[cfg(any(windows, target_os = "linux", test))]
fn buffer_len(w: i32, h: i32) -> Option<usize> {
    if w <= 0 || h <= 0 || w > MAX_SIDE || h > MAX_SIDE {
        return None;
    }
    (w as usize).checked_mul(h as usize)?.checked_mul(4)
}

/// Start the thread that loads icons one at a time, so a file on a slow or unreachable network drive cannot
/// freeze the notch. Each path sent to it comes back as `UserEvent::Icon`.
pub fn spawn_loader(proxy: EventLoopProxy<UserEvent>) -> Sender<PathBuf> {
    let (tx, rx) = mpsc::channel::<PathBuf>();
    thread::spawn(move || {
        // The shell wants COM on the thread that asks it for an icon.
        #[cfg(windows)]
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        while let Ok(path) = rx.recv() {
            let icon = load(&path);
            let _ = proxy.send_event(UserEvent::Icon { path, icon });
        }
    });
    tx
}

/// The shell icon for `path` as a premultiplied RGBA pixmap, or `None` if the shell has none.
#[cfg(windows)]
pub fn load(path: &Path) -> Option<Pixmap> {
    // A file on another computer is never asked about on its own (see `shelf::is_remote`): asking makes Windows sign
    // in to that server, and waits for it if it is gone. Its tile shows the plain look with the file's extension.
    if crate::shelf::is_remote(path) {
        return None;
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        let factory: IShellItemImageFactory = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None).ok()?;
        let bitmap = factory.GetImage(SIZE { cx: SIZE_PX, cy: SIZE_PX }, SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK).ok()?;

        let mut info = BITMAP::default();
        let got = GetObjectW(HGDIOBJ(bitmap.0), std::mem::size_of::<BITMAP>() as i32, Some(&mut info as *mut BITMAP as *mut c_void));
        let (w, h) = (info.bmWidth, info.bmHeight);
        let Some(len) = buffer_len(w, h).filter(|_| got != 0) else {
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            return None;
        };

        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // negative height: rows run top to bottom
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bgra = vec![0u8; len];
        let hdc = GetDC(None);
        let rows = GetDIBits(hdc, bitmap, 0, h as u32, Some(bgra.as_mut_ptr() as *mut c_void), &mut bmi, DIB_RGB_COLORS);
        ReleaseDC(None, hdc);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        if rows == 0 {
            return None;
        }

        // Some icons come back with no alpha channel at all; treat those as opaque.
        let opaque = bgra.as_chunks::<4>().0.iter().all(|p| p[3] == 0);
        let mut pixmap = Pixmap::new(w as u32, h as u32)?;
        for (dst, p) in pixmap.pixels_mut().iter_mut().zip(bgra.as_chunks::<4>().0) {
            let a = if opaque { 255 } else { p[3] };
            // The shell returns premultiplied colour; clamp so the value is always valid.
            *dst = PremultipliedColorU8::from_rgba(p[2].min(a), p[1].min(a), p[0].min(a), a)?;
        }
        Some(pixmap)
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{Pixmap, PremultipliedColorU8, SIZE_PX, buffer_len};
    use std::collections::{HashMap, HashSet};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::{Mutex, OnceLock};

    const MAX_ICON_FILE_BYTES: usize = 4 * 1024 * 1024;
    const MAX_CACHED_ICONS: usize = 256;
    const MAX_THEMES: usize = 16;

    static CACHE: OnceLock<Mutex<HashMap<Vec<String>, Option<Pixmap>>>> = OnceLock::new();

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(super) enum IconFile {
        Png(PathBuf),
        Svg(PathBuf),
    }

    pub(super) fn parse_globs2(content: &str) -> HashMap<String, String> {
        let mut weighted: HashMap<String, (u32, String)> = HashMap::new();
        for raw in content.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.split(':');
            let Some(weight_str) = parts.next() else { continue };
            let Some(mime) = parts.next() else { continue };
            let Some(glob) = parts.next() else { continue };
            if glob == "__NOGLOBS__" || mime.is_empty() {
                continue;
            }
            let Ok(weight) = weight_str.parse::<u32>() else { continue };
            let Some(ext) = glob.strip_prefix("*.") else { continue };
            if ext.is_empty() || ext.contains(['*', '?', '[', '.', '/']) {
                continue;
            }
            let ext = ext.to_ascii_lowercase();
            match weighted.get(&ext) {
                Some((prev_w, _)) if *prev_w >= weight => {}
                _ => {
                    weighted.insert(ext, (weight, mime.to_string()));
                }
            }
        }
        weighted.into_iter().map(|(ext, (_, mime))| (ext, mime)).collect()
    }

    fn mime_globs() -> &'static HashMap<String, String> {
        static GLOBS: OnceLock<HashMap<String, String>> = OnceLock::new();
        GLOBS.get_or_init(|| {
            let mut merged = HashMap::new();
            for dir in xdg_data_dirs() {
                if let Ok(content) = fs::read_to_string(dir.join("mime/globs2")) {
                    for (ext, mime) in parse_globs2(&content) {
                        merged.entry(ext).or_insert(mime);
                    }
                }
            }
            merged
        })
    }

    fn builtin_mime(ext: &str) -> Option<&'static str> {
        match ext {
            "txt" | "log" | "conf" | "ini" => Some("text/plain"),
            "md" | "markdown" => Some("text/markdown"),
            "pdf" => Some("application/pdf"),
            "png" => Some("image/png"),
            "jpg" | "jpeg" => Some("image/jpeg"),
            "gif" => Some("image/gif"),
            "webp" => Some("image/webp"),
            "svg" => Some("image/svg+xml"),
            "mp3" => Some("audio/mpeg"),
            "flac" => Some("audio/flac"),
            "wav" => Some("audio/wav"),
            "ogg" | "opus" => Some("audio/ogg"),
            "mp4" | "m4v" => Some("video/mp4"),
            "mkv" => Some("video/x-matroska"),
            "webm" => Some("video/webm"),
            "zip" => Some("application/zip"),
            "tar" => Some("application/x-tar"),
            "gz" => Some("application/gzip"),
            "xz" => Some("application/x-xz"),
            "7z" => Some("application/x-7z-compressed"),
            "rs" => Some("text/rust"),
            "py" => Some("text/x-python"),
            "js" | "mjs" => Some("application/javascript"),
            "ts" | "tsx" => Some("application/typescript"),
            "json" => Some("application/json"),
            "toml" => Some("application/toml"),
            "yaml" | "yml" => Some("application/yaml"),
            "html" | "htm" => Some("text/html"),
            "css" => Some("text/css"),
            "sh" => Some("application/x-shellscript"),
            "csv" => Some("text/csv"),
            "doc" | "docx" => Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
            "xls" | "xlsx" => Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
            "ppt" | "pptx" => Some("application/vnd.openxmlformats-officedocument.presentationml.presentation"),
            "deb" => Some("application/vnd.debian.binary-package"),
            "appimage" => Some("application/vnd.appimage"),
            _ => None,
        }
    }

    fn mime_for_extension(ext: &str) -> Option<String> {
        let lower = ext.to_ascii_lowercase();
        mime_globs().get(&lower).cloned().or_else(|| builtin_mime(&lower).map(str::to_string))
    }

    fn push_unique(list: &mut Vec<String>, item: impl Into<String>) {
        let item = item.into();
        if !item.is_empty() && !list.contains(&item) {
            list.push(item);
        }
    }

    pub(super) fn icon_candidates(path: &Path, is_dir: bool, is_exec: bool) -> Vec<String> {
        let mut names = Vec::new();
        if is_dir {
            push_unique(&mut names, "folder");
            push_unique(&mut names, "inode-directory");
            return names;
        }

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if let Some(mime) = (!ext.is_empty()).then(|| mime_for_extension(ext)).flatten() {
            if let Some((major, minor)) = mime.split_once('/') {
                push_unique(&mut names, format!("{major}-{minor}"));
                match major {
                    "image" => push_unique(&mut names, "image-x-generic"),
                    "audio" => push_unique(&mut names, "audio-x-generic"),
                    "video" => push_unique(&mut names, "video-x-generic"),
                    "text" => push_unique(&mut names, "text-x-generic"),
                    "font" => push_unique(&mut names, "font-x-generic"),
                    _ => {}
                }
                if matches!(minor, "zip" | "gzip" | "x-tar" | "x-xz" | "x-7z-compressed" | "x-bzip2" | "zstd" | "x-compressed-tar") {
                    push_unique(&mut names, "package-x-generic");
                }
                if minor.contains("wordprocessingml") || minor == "msword" || minor.contains("opendocument.text") {
                    push_unique(&mut names, "x-office-document");
                } else if minor.contains("spreadsheetml") || minor == "vnd.ms-excel" || minor.contains("opendocument.spreadsheet") {
                    push_unique(&mut names, "x-office-spreadsheet");
                } else if minor.contains("presentationml") || minor == "vnd.ms-powerpoint" || minor.contains("opendocument.presentation") {
                    push_unique(&mut names, "x-office-presentation");
                }
            }
        } else if is_exec {
            push_unique(&mut names, "application-x-executable");
            push_unique(&mut names, "text-x-script");
        }

        push_unique(&mut names, "text-x-generic");
        push_unique(&mut names, "application-octet-stream");
        push_unique(&mut names, "unknown");
        names
    }

    pub(super) fn parse_gtk_icon_theme(content: &str) -> Option<String> {
        for raw in content.lines() {
            let line = raw.trim();
            if line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=')
                && k.trim() == "gtk-icon-theme-name"
            {
                let theme = v.trim().trim_matches('"').trim_matches('\'').trim();
                if !theme.is_empty() {
                    return Some(theme.to_string());
                }
            }
        }
        None
    }

    pub(super) fn parse_kde_icon_theme(content: &str) -> Option<String> {
        let mut in_icons = false;
        for raw in content.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                in_icons = &line[1..line.len() - 1] == "Icons";
                continue;
            }
            if in_icons
                && let Some((k, v)) = line.split_once('=')
                && k.trim() == "Theme"
            {
                let theme = v.trim();
                if !theme.is_empty() {
                    return Some(theme.to_string());
                }
            }
        }
        None
    }

    fn parse_inherits(content: &str) -> Vec<String> {
        let mut in_header = false;
        for raw in content.lines() {
            let line = raw.trim();
            if line.starts_with('[') && line.ends_with(']') {
                in_header = &line[1..line.len() - 1] == "Icon Theme";
                continue;
            }
            if in_header
                && let Some((k, v)) = line.split_once('=')
                && k.trim() == "Inherits"
            {
                return v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect();
            }
        }
        Vec::new()
    }

    fn xdg_config_dir() -> Option<PathBuf> {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").filter(|v| !v.is_empty()).map(|h| PathBuf::from(h).join(".config")))
    }

    fn xdg_data_dirs() -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        if let Some(data_home) = std::env::var_os("XDG_DATA_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").filter(|v| !v.is_empty()).map(|h| PathBuf::from(h).join(".local/share")))
        {
            dirs.push(data_home);
        }
        let system = std::env::var("XDG_DATA_DIRS").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".to_string());
        for entry in system.split(':').filter(|s| !s.is_empty()) {
            let p = PathBuf::from(entry);
            if !dirs.contains(&p) {
                dirs.push(p);
            }
        }
        dirs
    }

    fn icon_roots() -> &'static Vec<PathBuf> {
        static ROOTS: OnceLock<Vec<PathBuf>> = OnceLock::new();
        ROOTS.get_or_init(|| {
            let mut roots = Vec::new();
            for data in xdg_data_dirs() {
                let icons = data.join("icons");
                if !roots.contains(&icons) {
                    roots.push(icons);
                }
            }
            if let Some(home) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
                let dot_icons = PathBuf::from(home).join(".icons");
                if !roots.contains(&dot_icons) {
                    roots.push(dot_icons);
                }
            }
            roots
        })
    }

    fn active_themes() -> &'static Vec<String> {
        static THEMES: OnceLock<Vec<String>> = OnceLock::new();
        THEMES.get_or_init(|| {
            let mut seeds = Vec::new();
            if let Some(cfg) = xdg_config_dir() {
                for rel in ["gtk-4.0/settings.ini", "gtk-3.0/settings.ini"] {
                    if let Ok(content) = fs::read_to_string(cfg.join(rel))
                        && let Some(theme) = parse_gtk_icon_theme(&content)
                    {
                        push_unique(&mut seeds, theme);
                    }
                }
                if let Ok(content) = fs::read_to_string(cfg.join("kdeglobals"))
                    && let Some(theme) = parse_kde_icon_theme(&content)
                {
                    push_unique(&mut seeds, theme);
                }
            }
            if let Ok(out) = Command::new("gsettings").args(["get", "org.gnome.desktop.interface", "icon-theme"]).output()
                && out.status.success()
            {
                let theme = String::from_utf8_lossy(&out.stdout).trim().trim_matches('\'').trim_matches('"').trim().to_string();
                push_unique(&mut seeds, theme);
            }
            for fallback in ["Adwaita", "breeze", "AdwaitaLegacy", "hicolor"] {
                push_unique(&mut seeds, fallback);
            }

            let roots = icon_roots();
            let mut ordered = Vec::new();
            let mut seen = HashSet::new();
            let mut queue: Vec<(String, usize)> = seeds.into_iter().map(|t| (t, 0)).collect();
            let mut idx = 0;
            while idx < queue.len() && ordered.len() < MAX_THEMES {
                let (theme, depth) = queue[idx].clone();
                idx += 1;
                if !seen.insert(theme.clone()) {
                    continue;
                }
                ordered.push(theme.clone());
                if depth < 3 {
                    for root in roots {
                        if let Ok(index) = fs::read_to_string(root.join(&theme).join("index.theme")) {
                            for parent in parse_inherits(&index) {
                                if !seen.contains(&parent) {
                                    queue.push((parent, depth + 1));
                                }
                            }
                            break;
                        }
                    }
                }
            }
            ordered
        })
    }

    pub(super) fn find_icon_in_dirs(roots: &[PathBuf], themes: &[String], names: &[String], svg_supported: bool) -> Option<IconFile> {
        const CATEGORIES: [&str; 4] = ["places", "mimetypes", "apps", "actions"];
        const BOX_SIZES: [&str; 8] = ["64x64", "48x48", "128x128", "32x32", "96x96", "256x256", "24x24", "16x16"];
        const NUM_SIZES: [&str; 7] = ["64", "48", "32", "96", "24", "22", "16"];

        let mut svg_fallback: Option<PathBuf> = None;
        for name in names {
            for theme in themes {
                for root in roots {
                    let base = root.join(theme);
                    if !base.is_dir() {
                        continue;
                    }
                    for cat in CATEGORIES {
                        for size in BOX_SIZES {
                            let p = base.join(size).join(cat).join(format!("{name}.png"));
                            if p.is_file() {
                                return Some(IconFile::Png(p));
                            }
                        }
                        for size in NUM_SIZES {
                            let p = base.join(cat).join(size).join(format!("{name}.png"));
                            if p.is_file() {
                                return Some(IconFile::Png(p));
                            }
                        }
                        if svg_supported && svg_fallback.is_none() {
                            let scalable = base.join("scalable").join(cat).join(format!("{name}.svg"));
                            if scalable.is_file() {
                                svg_fallback = Some(scalable);
                            } else {
                                for size in NUM_SIZES {
                                    let p = base.join(cat).join(size).join(format!("{name}.svg"));
                                    if p.is_file() {
                                        svg_fallback = Some(p);
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        svg_fallback.map(IconFile::Svg)
    }

    fn has_rsvg_convert() -> bool {
        static HAS_RSVG: OnceLock<bool> = OnceLock::new();
        *HAS_RSVG.get_or_init(|| Command::new("rsvg-convert").arg("--version").output().is_ok_and(|o| o.status.success()))
    }

    pub(super) fn decode_png_to_pixmap(bytes: &[u8]) -> Option<Pixmap> {
        let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png).ok()?;
        let (w, h) = (img.width() as i32, img.height() as i32);
        let _ = buffer_len(w, h)?;
        let resized = if w > SIZE_PX || h > SIZE_PX { img.resize(SIZE_PX as u32, SIZE_PX as u32, image::imageops::FilterType::Triangle) } else { img };
        let rgba = resized.to_rgba8();
        let (rw, rh) = rgba.dimensions();
        let mut pixmap = Pixmap::new(rw, rh)?;
        for (dst, p) in pixmap.pixels_mut().iter_mut().zip(rgba.as_raw().as_chunks::<4>().0) {
            let a = p[3] as u16;
            let premul = |c: u8| ((c as u16 * a + 127) / 255) as u8;
            *dst = PremultipliedColorU8::from_rgba(premul(p[0]), premul(p[1]), premul(p[2]), p[3])?;
        }
        Some(pixmap)
    }

    fn render_icon(file: &IconFile) -> Option<Pixmap> {
        match file {
            IconFile::Png(path) => {
                let len = fs::metadata(path).ok()?.len();
                if len == 0 || len > MAX_ICON_FILE_BYTES as u64 {
                    return None;
                }
                let bytes = fs::read(path).ok()?;
                decode_png_to_pixmap(&bytes)
            }
            IconFile::Svg(path) => {
                let size = SIZE_PX.to_string();
                let out = Command::new("rsvg-convert").args(["-w", &size, "-h", &size, "-f", "png"]).arg(path).output().ok()?;
                if !out.status.success() || out.stdout.is_empty() || out.stdout.len() > MAX_ICON_FILE_BYTES {
                    return None;
                }
                decode_png_to_pixmap(&out.stdout)
            }
        }
    }

    pub(super) fn load(path: &Path) -> Option<Pixmap> {
        if crate::shelf::is_remote(path) {
            return None;
        }
        let meta = fs::metadata(path).ok()?;
        let is_dir = meta.is_dir();
        let is_exec = !is_dir && (meta.permissions().mode() & 0o111 != 0);
        let names = icon_candidates(path, is_dir, is_exec);

        let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        if let Ok(guard) = cache.lock()
            && let Some(cached) = guard.get(&names)
        {
            return cached.clone();
        }

        let found = find_icon_in_dirs(icon_roots(), active_themes(), &names, has_rsvg_convert());
        let rendered = found.as_ref().and_then(render_icon);

        if let Ok(mut guard) = cache.lock() {
            if guard.len() >= MAX_CACHED_ICONS {
                guard.clear();
            }
            guard.insert(names, rendered.clone());
        }
        rendered
    }
}

#[cfg(target_os = "linux")]
pub fn load(path: &Path) -> Option<Pixmap> {
    linux::load(path)
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn load(_path: &Path) -> Option<Pixmap> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_buffer_for_a_normal_icon_is_four_bytes_a_pixel() {
        assert_eq!(buffer_len(64, 64), Some(64 * 64 * 4));
        assert_eq!(buffer_len(256, 256), Some(256 * 256 * 4));
        assert_eq!(buffer_len(MAX_SIDE, MAX_SIDE), Some(MAX_SIDE as usize * MAX_SIDE as usize * 4));
    }

    #[test]
    fn an_empty_or_enormous_size_from_the_shell_gets_no_buffer() {
        for (w, h) in [(0, 5), (5, 0), (-1, 5), (5, -1), (MAX_SIDE + 1, 1), (1, MAX_SIDE + 1), (40_000, 40_000), (i32::MAX, i32::MAX), (i32::MAX, 1)] {
            assert_eq!(buffer_len(w, h), None, "{w} x {h}");
        }
        // The old sum in i32: 32768 x 32768 x 4 wraps to 0 and (46341 x 46341 x 4) to a negative number.
        assert_eq!((32768i32).wrapping_mul(32768).wrapping_mul(4), 0);
        assert_eq!(buffer_len(32768, 32768), None);
    }

    /// A file on another computer gets no icon and is never asked about (so Windows does not sign in to the server).
    #[test]
    fn a_file_on_another_computer_is_not_asked_about() {
        let started = std::time::Instant::now();
        for path in [r"\\sky-island-no-such-host.invalid\share\a.txt", "//sky-island-no-such-host.invalid/share/a.txt", r"\/sky-island-no-such-host.invalid\share\a.txt"] {
            assert!(load(Path::new(path)).is_none(), "{path}");
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(2), "it did not wait for a server: {:?}", started.elapsed());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn globs2_and_desktop_theme_parsers_handle_weights_and_sections() {
        let globs = "\
# comment line
0:model/materialx:__NOGLOBS__
20:text/plain:*.ts
80:application/typescript:*.ts
50:application/pdf:*.PDF
50:text/x-csrc:*.[ch]
";
        let map = linux::parse_globs2(globs);
        assert_eq!(map.get("ts").map(String::as_str), Some("application/typescript"));
        assert_eq!(map.get("pdf").map(String::as_str), Some("application/pdf"));
        assert!(!map.contains_key("[ch]"));

        let gtk = "[Settings]\n# comment\ngtk-icon-theme-name = \"Omarchy-Dark\"\n";
        assert_eq!(linux::parse_gtk_icon_theme(gtk).as_deref(), Some("Omarchy-Dark"));

        let kde = "[General]\nTheme=Wrong\n\n[Icons]\nTheme=breeze-dark\n";
        assert_eq!(linux::parse_kde_icon_theme(kde).as_deref(), Some("breeze-dark"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn theme_lookup_supports_adwaita_png_and_breeze_svg_layouts() {
        let root = std::env::temp_dir().join(format!("sky-island-icons-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);

        let png = root.join("Custom/48x48/mimetypes/application-pdf.png");
        let svg = root.join("breeze/places/64/folder.svg");
        std::fs::create_dir_all(png.parent().unwrap()).unwrap();
        std::fs::create_dir_all(svg.parent().unwrap()).unwrap();
        std::fs::write(&png, b"png").unwrap();
        std::fs::write(&svg, b"<svg/>").unwrap();

        let themes = vec!["Custom".to_string(), "breeze".to_string()];
        let roots = vec![root.clone()];

        let pdf_names = linux::icon_candidates(Path::new("/tmp/manual.pdf"), false, false);
        assert!(pdf_names.first().map(String::as_str) == Some("application-pdf"));
        assert!(linux::find_icon_in_dirs(&roots, &themes, &pdf_names, true).is_some());

        let dir_names = linux::icon_candidates(Path::new("/tmp/folder"), true, false);
        assert_eq!(dir_names, vec!["folder".to_string(), "inode-directory".to_string()]);
        assert!(linux::find_icon_in_dirs(&roots, &themes, &dir_names, true).is_some());
        assert!(linux::find_icon_in_dirs(&roots, &themes, &dir_names, false).is_none());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn png_decoder_premultiplies_alpha_and_rejects_invalid_bytes() {
        use image::{ImageFormat, Rgba, RgbaImage};
        use std::io::Cursor;

        assert!(linux::decode_png_to_pixmap(b"not a png").is_none());

        let img = RgbaImage::from_pixel(16, 16, Rgba([200, 100, 50, 128]));
        let mut bytes = Vec::new();
        img.write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png).unwrap();
        let pm = linux::decode_png_to_pixmap(&bytes).unwrap();
        assert_eq!((pm.width(), pm.height()), (16, 16));
        let p = pm.pixels()[0];
        assert_eq!((p.red(), p.green(), p.blue(), p.alpha()), (100, 50, 25, 128));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn load_resolves_real_system_directory_icon_without_panicking() {
        let has_theme = Path::new("/usr/share/icons/Adwaita").is_dir() || Path::new("/usr/share/icons/breeze").is_dir();
        let loaded = load(Path::new("/tmp"));
        if has_theme {
            let pm = loaded.expect("directory icon on desktop with Adwaita/Breeze installed");
            assert!(pm.width() > 0 && pm.height() > 0);
        }
    }
}
