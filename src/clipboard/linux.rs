//! The Linux side: watching the clipboard for changes, reading text or images, and putting an entry back.
//! Uses native Wayland `ext_data_control_v1` first (Hyprland, KDE 6.3+, Sway 1.11+, etc.) and falls back cleanly to
//! `wl-clipboard`, `xclip`, or `xsel` when `ext_data_control_manager_v1` is absent or running under X11.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsFd;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use calloop::channel::{self, Sender};
use calloop::{EventLoop, LoopSignal};
use sha2::{Digest, Sha256};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use wayland_client::backend::ObjectId;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, event_created_child};
use wayland_protocols::ext::data_control::v1::client::{ext_data_control_device_v1, ext_data_control_manager_v1, ext_data_control_offer_v1, ext_data_control_source_v1};
use winit::event_loop::EventLoopProxy;

use super::pictures::{self, MAX_PICTURE_BYTES};
use super::{Content, MAX_CHARS};
use crate::UserEvent;

const MAX_TEXT_BYTES: usize = MAX_CHARS * 4 + 1024;
const X11_POLL: Duration = Duration::from_millis(600);

static ENABLED: AtomicBool = AtomicBool::new(true);
static WATCH_PROXY: OnceLock<EventLoopProxy<UserEvent>> = OnceLock::new();
static BACKEND: OnceLock<Backend> = OnceLock::new();
static LAST_CLI_HASH: Mutex<Option<[u8; 32]>> = Mutex::new(None);

#[derive(Clone)]
enum Backend {
    Wayland(Sender<Cmd>),
    Cli(CliBackend),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CliBackend {
    WlClipboard,
    Xclip,
    Xsel,
}

enum Cmd {
    Set(Content),
    Clear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MimeKind {
    Text,
    Png,
    Image,
}

fn is_sensitive_mime(mime: &str) -> bool {
    let lower = mime.trim().to_ascii_lowercase();
    matches!(lower.as_str(), "x-kde-passwordmanagerhint" | "application/x-password" | "excludeclipboardcontentfrommonitorprocessing" | "clipboard viewer ignore")
}

fn pick_mime(mimes: &[String]) -> Option<(String, MimeKind)> {
    if mimes.iter().any(|m| is_sensitive_mime(m)) {
        return None;
    }
    let find = |pred: fn(&str) -> bool| -> Option<String> { mimes.iter().find(|m| pred(m.trim())).cloned() };

    if let Some(m) = find(|m| m.eq_ignore_ascii_case("text/plain;charset=utf-8") || m.eq_ignore_ascii_case("text/plain; charset=utf-8")) {
        return Some((m, MimeKind::Text));
    }
    if let Some(m) = find(|m| m == "UTF8_STRING") {
        return Some((m, MimeKind::Text));
    }
    if let Some(m) = find(|m| m.eq_ignore_ascii_case("text/plain")) {
        return Some((m, MimeKind::Text));
    }
    if let Some(m) = find(|m| m.eq_ignore_ascii_case("image/png")) {
        return Some((m, MimeKind::Png));
    }
    if let Some(m) = find(|m| m.eq_ignore_ascii_case("image/jpeg") || m.eq_ignore_ascii_case("image/bmp")) {
        return Some((m, MimeKind::Image));
    }
    None
}

fn read_bounded_text(reader: impl Read) -> Option<Content> {
    let mut buf = Vec::new();
    reader.take(MAX_TEXT_BYTES as u64).read_to_end(&mut buf).ok()?;
    while buf.last() == Some(&0) {
        buf.pop();
    }
    let text = String::from_utf8_lossy(&buf);
    if text.trim().is_empty() {
        return None;
    }
    Some(Content::Text(text.chars().take(MAX_CHARS).collect()))
}

fn read_bounded_png(mut reader: impl Read) -> Option<Content> {
    let mut head = [0u8; 64];
    let mut filled = 0;
    while filled < head.len() {
        match reader.read(&mut head[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(_) => return None,
        }
    }
    if filled < 64 || !pictures::png_fits(&head) {
        return None;
    }
    let mut bytes = Vec::with_capacity(filled);
    bytes.extend_from_slice(&head);
    let extra_limit = (MAX_PICTURE_BYTES - filled) as u64 + 1;
    reader.take(extra_limit).read_to_end(&mut bytes).ok()?;
    if bytes.len() > MAX_PICTURE_BYTES {
        return None;
    }
    pictures::image_from_png(bytes).map(Content::Image)
}

fn read_bounded_image(reader: impl Read) -> Option<Content> {
    let mut bytes = Vec::new();
    reader.take(MAX_PICTURE_BYTES as u64 + 1).read_to_end(&mut bytes).ok()?;
    if bytes.is_empty() || bytes.len() > MAX_PICTURE_BYTES {
        return None;
    }
    let decoded = pictures::decode_picture(&bytes)?.to_rgba8();
    let (w, h) = decoded.dimensions();
    pictures::image_from_rgba(w, h, decoded.into_raw()).map(Content::Image)
}

fn read_content_from_reader(reader: impl Read, kind: MimeKind) -> Option<Content> {
    match kind {
        MimeKind::Text => read_bounded_text(reader),
        MimeKind::Png => read_bounded_png(reader),
        MimeKind::Image => read_bounded_image(reader),
    }
}

fn content_digest(content: &Content) -> [u8; 32] {
    let mut hasher = Sha256::new();
    match content {
        Content::Text(t) => {
            hasher.update([0u8]);
            hasher.update(t.as_bytes());
        }
        Content::Image(img) => {
            hasher.update([1u8]);
            hasher.update(&img.png);
        }
    }
    hasher.finalize().into()
}

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

pub fn spawn_watcher(proxy: EventLoopProxy<UserEvent>) {
    if WATCH_PROXY.set(proxy.clone()).is_err() {
        return;
    }
    if let Some(tx) = try_start_wayland(proxy.clone()) {
        let _ = BACKEND.set(Backend::Wayland(tx));
        return;
    }
    if let Some(cli) = detect_cli_backend() {
        let _ = BACKEND.set(Backend::Cli(cli));
        let _ = thread::Builder::new().name("clipboard-cli".into()).spawn(move || run_cli_watcher(cli, proxy));
    }
}

pub fn set_content(content: &Content) -> bool {
    match BACKEND.get() {
        Some(Backend::Wayland(tx)) => tx.send(Cmd::Set(content.clone())).is_ok(),
        Some(Backend::Cli(cli)) => {
            let ok = write_cli_content(*cli, content);
            if ok {
                if let Ok(mut guard) = LAST_CLI_HASH.lock() {
                    *guard = Some(content_digest(content));
                }
                if ENABLED.load(Ordering::Relaxed)
                    && let Some(proxy) = WATCH_PROXY.get()
                {
                    let _ = proxy.send_event(UserEvent::Clip(content.clone()));
                }
            }
            ok
        }
        None => false,
    }
}

pub fn empty() {
    match BACKEND.get() {
        Some(Backend::Wayland(tx)) => {
            let _ = tx.send(Cmd::Clear);
        }
        Some(Backend::Cli(cli)) => {
            clear_cli_content(*cli);
            if let Ok(mut guard) = LAST_CLI_HASH.lock() {
                *guard = None;
            }
        }
        None => {}
    }
}

// ---- Native Wayland (ext_data_control_v1) ----

struct WlClipState {
    conn: Connection,
    qh: QueueHandle<Self>,
    proxy: EventLoopProxy<UserEvent>,
    manager: ext_data_control_manager_v1::ExtDataControlManagerV1,
    device: ext_data_control_device_v1::ExtDataControlDeviceV1,
    pending_offers: HashMap<ObjectId, Vec<String>>,
    last_offer: Option<ext_data_control_offer_v1::ExtDataControlOfferV1>,
    sources: HashMap<ObjectId, (ext_data_control_source_v1::ExtDataControlSourceV1, Arc<Vec<u8>>)>,
    initialized: bool,
    self_selection_pending: usize,
    loop_signal: Option<LoopSignal>,
}

impl WlClipState {
    fn handle_cmd(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Set(content) => {
                let bytes = match &content {
                    Content::Text(t) => Arc::new(t.as_bytes().to_vec()),
                    Content::Image(img) => Arc::new(img.png.clone()),
                };
                let source = self.manager.create_data_source(&self.qh, ());
                match &content {
                    Content::Text(_) => {
                        source.offer("text/plain;charset=utf-8".into());
                        source.offer("UTF8_STRING".into());
                        source.offer("text/plain".into());
                    }
                    Content::Image(_) => {
                        source.offer("image/png".into());
                    }
                }
                self.sources.insert(source.id(), (source.clone(), bytes));
                self.self_selection_pending = self.self_selection_pending.saturating_add(1);
                self.device.set_selection(Some(&source));
                let _ = self.conn.flush();
                if ENABLED.load(Ordering::Relaxed) {
                    let _ = self.proxy.send_event(UserEvent::Clip(content));
                }
            }
            Cmd::Clear => {
                self.device.set_selection(None);
                for (source, _) in self.sources.drain().map(|(_, v)| v) {
                    source.destroy();
                }
                let _ = self.conn.flush();
            }
        }
    }
}

impl Drop for WlClipState {
    fn drop(&mut self) {
        if let Some(offer) = self.last_offer.take() {
            offer.destroy();
        }
        for (_, (source, _)) in self.sources.drain() {
            source.destroy();
        }
        self.device.destroy();
        self.manager.destroy();
    }
}

fn try_start_wayland(proxy: EventLoopProxy<UserEvent>) -> Option<Sender<Cmd>> {
    let conn = Connection::connect_to_env().ok()?;
    let (globals, mut event_queue) = registry_queue_init::<WlClipState>(&conn).ok()?;
    let qh = event_queue.handle();
    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=1, ()).ok()?;
    let manager: ext_data_control_manager_v1::ExtDataControlManagerV1 = globals.bind(&qh, 1..=1, ()).ok()?;
    let device = manager.get_data_device(&seat, &qh, ());

    let mut state = WlClipState {
        conn: conn.clone(),
        qh,
        proxy,
        manager,
        device,
        pending_offers: HashMap::new(),
        last_offer: None,
        sources: HashMap::new(),
        initialized: false,
        self_selection_pending: 0,
        loop_signal: None,
    };

    // Drain initial selection event so whatever was already copied before startup is skipped cleanly.
    if event_queue.roundtrip(&mut state).is_err() {
        return None;
    }
    state.initialized = true;

    let (cmd_tx, cmd_rx) = channel::channel::<Cmd>();
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<bool>(1);
    if thread::Builder::new()
        .name("clipboard-wl".into())
        .spawn(move || {
            let Ok(mut event_loop) = EventLoop::<WlClipState>::try_new() else {
                let _ = ready_tx.send(false);
                return;
            };
            let handle = event_loop.handle();
            state.loop_signal = Some(event_loop.get_signal());
            if WaylandSource::new(conn, event_queue).insert(handle.clone()).is_err() {
                let _ = ready_tx.send(false);
                return;
            }
            if handle
                .insert_source(cmd_rx, |ev, _, state| {
                    if let channel::Event::Msg(cmd) = ev {
                        state.handle_cmd(cmd);
                    } else if let Some(sig) = &state.loop_signal {
                        sig.stop();
                    }
                })
                .is_err()
            {
                let _ = ready_tx.send(false);
                return;
            }
            let _ = ready_tx.send(true);
            let _ = event_loop.run(None, &mut state, |_| {});
        })
        .is_err()
    {
        return None;
    }

    matches!(ready_rx.recv(), Ok(true)).then_some(cmd_tx)
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for WlClipState {
    fn event(_state: &mut Self, _proxy: &wl_registry::WlRegistry, _event: wl_registry::Event, _data: &GlobalListContents, _conn: &Connection, _qh: &QueueHandle<Self>) {}
}

impl Dispatch<wl_seat::WlSeat, ()> for WlClipState {
    fn event(_state: &mut Self, _proxy: &wl_seat::WlSeat, _event: wl_seat::Event, _data: &(), _conn: &Connection, _qh: &QueueHandle<Self>) {}
}

impl Dispatch<ext_data_control_manager_v1::ExtDataControlManagerV1, ()> for WlClipState {
    fn event(
        _state: &mut Self,
        _proxy: &ext_data_control_manager_v1::ExtDataControlManagerV1,
        _event: ext_data_control_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ext_data_control_device_v1::ExtDataControlDeviceV1, ()> for WlClipState {
    fn event(
        state: &mut Self,
        _proxy: &ext_data_control_device_v1::ExtDataControlDeviceV1,
        event: ext_data_control_device_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            ext_data_control_device_v1::Event::DataOffer { id } => {
                state.pending_offers.insert(id.id(), Vec::new());
            }
            ext_data_control_device_v1::Event::PrimarySelection { id: Some(offer) } => {
                state.pending_offers.remove(&offer.id());
                offer.destroy();
            }
            ext_data_control_device_v1::Event::Selection { id } => {
                if let Some(old) = state.last_offer.take() {
                    old.destroy();
                }
                let Some(offer) = id else {
                    state.self_selection_pending = 0;
                    return;
                };
                let mimes = state.pending_offers.remove(&offer.id()).unwrap_or_default();
                if state.self_selection_pending > 0 {
                    state.self_selection_pending -= 1;
                    offer.destroy();
                    return;
                }
                if !state.initialized || !ENABLED.load(Ordering::Relaxed) {
                    offer.destroy();
                    return;
                }
                let Some((mime, kind)) = pick_mime(&mimes) else {
                    offer.destroy();
                    return;
                };
                let Ok((reader, writer)) = std::io::pipe() else {
                    offer.destroy();
                    return;
                };
                offer.receive(mime, writer.as_fd());
                drop(writer);
                let _ = state.conn.flush();
                state.last_offer = Some(offer);

                let proxy = state.proxy.clone();
                let _ = thread::Builder::new().name("clip-read".into()).spawn(move || {
                    if let Some(content) = read_content_from_reader(reader, kind)
                        && ENABLED.load(Ordering::Relaxed)
                    {
                        let _ = proxy.send_event(UserEvent::Clip(content));
                    }
                });
            }
            ext_data_control_device_v1::Event::Finished => {
                if let Some(sig) = &state.loop_signal {
                    sig.stop();
                }
            }
            _ => {}
        }
    }

    event_created_child!(WlClipState, ext_data_control_device_v1::ExtDataControlDeviceV1, [
        ext_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ext_data_control_offer_v1::ExtDataControlOfferV1, ()),
    ]);
}

impl Dispatch<ext_data_control_offer_v1::ExtDataControlOfferV1, ()> for WlClipState {
    fn event(
        state: &mut Self,
        proxy: &ext_data_control_offer_v1::ExtDataControlOfferV1,
        event: ext_data_control_offer_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let ext_data_control_offer_v1::Event::Offer { mime_type } = event
            && let Some(mimes) = state.pending_offers.get_mut(&proxy.id())
        {
            mimes.push(mime_type);
        }
    }
}

impl Dispatch<ext_data_control_source_v1::ExtDataControlSourceV1, ()> for WlClipState {
    fn event(
        state: &mut Self,
        proxy: &ext_data_control_source_v1::ExtDataControlSourceV1,
        event: ext_data_control_source_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            ext_data_control_source_v1::Event::Send { mime_type: _, fd } => {
                if let Some((_, bytes)) = state.sources.get(&proxy.id()) {
                    let bytes = Arc::clone(bytes);
                    let _ = thread::Builder::new().name("clip-write".into()).spawn(move || {
                        let mut file = std::fs::File::from(fd);
                        let _ = file.write_all(&bytes);
                    });
                }
            }
            ext_data_control_source_v1::Event::Cancelled => {
                if let Some((source, _)) = state.sources.remove(&proxy.id()) {
                    source.destroy();
                }
            }
            _ => {}
        }
    }
}

// ---- CLI Fallbacks (wl-clipboard, xclip, xsel) ----

fn tool_exists(program: &str, args: &[&str]) -> bool {
    Command::new(program).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

fn detect_cli_backend() -> Option<CliBackend> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty()) && tool_exists("wl-paste", &["--version"]) && tool_exists("wl-copy", &["--version"]) {
        return Some(CliBackend::WlClipboard);
    }
    if std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty()) {
        if tool_exists("xclip", &["-version"]) {
            return Some(CliBackend::Xclip);
        }
        if tool_exists("xsel", &["--version"]) {
            return Some(CliBackend::Xsel);
        }
    }
    None
}

fn read_cli_output(program: &str, args: &[&str], kind: MimeKind) -> Option<Content> {
    let mut child = Command::new(program).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let stdout = child.stdout.take()?;
    let content = read_content_from_reader(stdout, kind);
    if content.is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
    content
}

fn read_cli_content(cli: CliBackend) -> Option<Content> {
    match cli {
        CliBackend::WlClipboard => {
            let out = Command::new("wl-paste").arg("--list-types").stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
            if !out.status.success() {
                return None;
            }
            let mimes: Vec<String> = String::from_utf8_lossy(&out.stdout).lines().map(|l| l.trim().to_string()).collect();
            let (mime, kind) = pick_mime(&mimes)?;
            read_cli_output("wl-paste", &["--no-newline", "--type", &mime], kind)
        }
        CliBackend::Xclip => {
            let out = Command::new("xclip").args(["-selection", "clipboard", "-o", "-t", "TARGETS"]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
            if !out.status.success() {
                return None;
            }
            let mimes: Vec<String> = String::from_utf8_lossy(&out.stdout).lines().map(|l| l.trim().to_string()).collect();
            let (mime, kind) = pick_mime(&mimes)?;
            read_cli_output("xclip", &["-selection", "clipboard", "-o", "-t", &mime], kind)
        }
        CliBackend::Xsel => read_cli_output("xsel", &["--clipboard", "--output"], MimeKind::Text),
    }
}

fn write_cli_content(cli: CliBackend, content: &Content) -> bool {
    let (mime, bytes): (&str, &[u8]) = match content {
        Content::Text(t) => ("text/plain;charset=utf-8", t.as_bytes()),
        Content::Image(img) => ("image/png", &img.png),
    };
    let mut cmd = match cli {
        CliBackend::WlClipboard => {
            let mut c = Command::new("wl-copy");
            c.args(["--type", mime]);
            c
        }
        CliBackend::Xclip => {
            let xmime = if matches!(content, Content::Text(_)) { "UTF8_STRING" } else { mime };
            let mut c = Command::new("xclip");
            c.args(["-selection", "clipboard", "-i", "-t", xmime]);
            c
        }
        CliBackend::Xsel => {
            if !matches!(content, Content::Text(_)) {
                return false;
            }
            let mut c = Command::new("xsel");
            c.args(["--clipboard", "--input"]);
            c
        }
    };
    let Ok(mut child) = cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take()
        && stdin.write_all(bytes).is_err()
    {
        let _ = child.kill();
        let _ = child.wait();
        return false;
    }
    child.wait().is_ok_and(|s| s.success())
}

fn clear_cli_content(cli: CliBackend) {
    match cli {
        CliBackend::WlClipboard => {
            let _ = Command::new("wl-copy").arg("--clear").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
        }
        CliBackend::Xclip => {
            if let Ok(mut child) = Command::new("xclip").args(["-selection", "clipboard", "-i"]).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
                drop(child.stdin.take());
                let _ = child.wait();
            }
        }
        CliBackend::Xsel => {
            let _ = Command::new("xsel").args(["--clipboard", "--clear"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
        }
    }
}

fn poll_cli_once(cli: CliBackend, proxy: &EventLoopProxy<UserEvent>, emit: bool) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let Some(content) = read_cli_content(cli) else { return };
    let digest = content_digest(&content);
    let changed = if let Ok(mut guard) = LAST_CLI_HASH.lock() {
        if *guard == Some(digest) {
            false
        } else {
            *guard = Some(digest);
            true
        }
    } else {
        false
    };
    if emit && changed {
        let _ = proxy.send_event(UserEvent::Clip(content));
    }
}

fn run_cli_watcher(cli: CliBackend, proxy: EventLoopProxy<UserEvent>) {
    poll_cli_once(cli, &proxy, false);

    if cli == CliBackend::WlClipboard {
        loop {
            let mut cmd = Command::new("wl-paste");
            cmd.args(["--watch", "echo", "changed"]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
            crate::sys::kill_on_parent_exit(&mut cmd);
            let Ok(mut child) = cmd.spawn() else { break };
            let mut first = true;
            if let Some(stdout) = child.stdout.take() {
                for line in BufReader::new(stdout).lines() {
                    if line.is_err() {
                        break;
                    }
                    if first {
                        first = false;
                        continue;
                    }
                    poll_cli_once(cli, &proxy, true);
                }
            }
            let _ = child.wait();
            thread::sleep(Duration::from_secs(1));
        }
        return;
    }

    loop {
        thread::sleep(X11_POLL);
        poll_cli_once(cli, &proxy, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageFormat, Rgba, RgbaImage};
    use std::io::Cursor;

    #[test]
    fn mime_selection_prefers_text_and_respects_password_managers() {
        let secret = vec!["text/plain;charset=utf-8".into(), "x-kde-passwordManagerHint".into()];
        assert_eq!(pick_mime(&secret), None);

        let mixed = vec!["image/png".into(), "TEXT/PLAIN".into(), "text/plain;charset=utf-8".into()];
        assert_eq!(pick_mime(&mixed), Some(("text/plain;charset=utf-8".into(), MimeKind::Text)));

        let imgs = vec!["image/jpeg".into(), "image/png".into()];
        assert_eq!(pick_mime(&imgs), Some(("image/png".into(), MimeKind::Png)));

        let jpg_only = vec!["image/jpeg".into()];
        assert_eq!(pick_mime(&jpg_only), Some(("image/jpeg".into(), MimeKind::Image)));

        assert_eq!(pick_mime(&["text/html".into()]), None);
    }

    #[test]
    fn bounded_text_reader_handles_blank_nul_and_truncation() {
        assert!(read_bounded_text(Cursor::new(b"   \n\t\0")).is_none());
        let Some(Content::Text(s)) = read_bounded_text(Cursor::new(b"hello linux\0\0")) else {
            panic!("expected text");
        };
        assert_eq!(s, "hello linux");

        let huge = "a".repeat(MAX_CHARS + 500);
        let Some(Content::Text(trimmed)) = read_bounded_text(Cursor::new(huge.as_bytes())) else {
            panic!("expected capped text");
        };
        assert_eq!(trimmed.chars().count(), MAX_CHARS);
    }

    #[test]
    fn bounded_png_reader_accepts_valid_and_rejects_huge_header() {
        let mut png = Vec::new();
        RgbaImage::from_pixel(32, 24, Rgba([10, 20, 30, 255])).write_to(&mut Cursor::new(&mut png), ImageFormat::Png).unwrap();
        let Some(Content::Image(img)) = read_bounded_png(Cursor::new(&png)) else {
            panic!("expected png image");
        };
        assert_eq!((img.width, img.height), (32, 24));

        // 70,000 x 70,000 PNG header followed by padding should fail at the 64-byte header check.
        let mut bomb = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        bomb.extend_from_slice(&70_000u32.to_be_bytes());
        bomb.extend_from_slice(&70_000u32.to_be_bytes());
        bomb.extend_from_slice(&[8, 6, 0, 0, 0]);
        bomb.resize(256, 0);
        assert!(read_bounded_png(Cursor::new(&bomb)).is_none());
    }

    #[test]
    fn bounded_image_reader_decodes_jpeg() {
        let mut jpg = Vec::new();
        image::DynamicImage::ImageRgba8(RgbaImage::from_pixel(16, 12, Rgba([200, 100, 50, 255]))).to_rgb8().write_to(&mut Cursor::new(&mut jpg), ImageFormat::Jpeg).unwrap();
        let Some(Content::Image(img)) = read_bounded_image(Cursor::new(&jpg)) else {
            panic!("expected jpeg converted to Content::Image");
        };
        assert_eq!((img.width, img.height), (16, 12));
    }
}
