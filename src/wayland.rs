//! Wayland layer-shell surface backend for the notch on Linux (Hyprland, Sway, KDE, etc.).
//! Uses `zwlr_layer_shell_v1` directly on its own calloop thread so the notch sits at the top edge
//! without appearing in taskbars or stealing keyboard focus unless explicitly editing text.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;

use calloop::channel::{self, Sender};
use calloop::{EventLoop, LoopHandle, LoopSignal};
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::pointer::{BTN_LEFT, PointerEvent, PointerEventKind, PointerHandler};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure};
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{delegate_compositor, delegate_layer, delegate_output, delegate_pointer, delegate_registry, delegate_seat, delegate_shm, registry_handlers};
use tiny_skia::Pixmap;
use wayland_client::backend::ObjectId;
use wayland_client::globals::{GlobalList, registry_queue_init};
use wayland_client::protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1;
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1::{self, WpFractionalScaleV1};
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;
use wayland_protocols::wp::viewporter::client::wp_viewporter::WpViewporter;
use winit::event::MouseScrollDelta;
use winit::event_loop::EventLoopProxy;
use winit::keyboard::{Key, NamedKey};

use crate::{UserEvent, settings};

/// Connected Wayland output metadata shared with the UI thread.
#[derive(Clone, Debug, PartialEq)]
pub struct OutputInfo {
    pub id: u32,
    pub name: String,
    pub logical_pos: (i32, i32),
    pub logical_size: (i32, i32),
    pub scale: f64,
    pub refresh_millihertz: u32,
}

/// Premultiplied ARGB8888 pixel frame ready to upload into a `wl_shm` buffer.
#[derive(Clone)]
pub struct Frame {
    pub logical_size: (u32, u32),
    pub buffer_size: (u32, u32),
    pub pixels: Vec<u8>,
}

/// Role of a secondary layer surface on another monitor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MirrorRole {
    /// Visible closed notch replica (`Screen::Every`).
    ClosedNotch,
    /// Invisible 2px top-edge trigger strip (`Screen::All`).
    EdgeTrigger,
}

/// Events forwarded from the Wayland thread to the winit event loop.
#[derive(Debug)]
pub enum Event {
    CursorEntered { output_id: Option<u32>, mirror: bool, x: f64, y: f64 },
    CursorLeft { output_id: Option<u32>, mirror: bool },
    CursorMoved { output_id: Option<u32>, mirror: bool, x: f64, y: f64 },
    MouseInput { pressed: bool },
    MouseWheel { delta: MouseScrollDelta },
    ScaleChanged(f64),
    KeyPressed { key: Key, text: Option<String> },
    Focused(bool),
    OutputsChanged,
    Closed,
}

enum Cmd {
    Present(Frame),
    SetVisible(bool),
    SetOutput(Option<u32>),
    SetKeyboardFocus(bool),
    SyncMirrors(Vec<(u32, MirrorRole)>),
    PresentMirror(u32, Frame),
    HideMirror(u32),
    Shutdown,
}

/// Handle held by `App` to drive the Wayland notch surface and mirror surfaces.
pub struct Handle {
    tx: Sender<Cmd>,
    outputs: Arc<Mutex<Vec<OutputInfo>>>,
}

impl Handle {
    /// Connect to the Wayland compositor, bind `zwlr_layer_shell_v1`, and spawn the event thread.
    /// Returns `Err` when Wayland or `zwlr_layer_shell_v1` is unavailable (e.g. X11 or GNOME).
    pub fn spawn(proxy: EventLoopProxy<UserEvent>) -> Result<Self, String> {
        let conn = Connection::connect_to_env().map_err(|e| format!("wayland connect: {e}"))?;
        let (globals, mut event_queue) = registry_queue_init(&conn).map_err(|e| format!("wayland registry: {e}"))?;
        let qh = event_queue.handle();

        let compositor = CompositorState::bind(&globals, &qh).map_err(|e| format!("wl_compositor: {e}"))?;
        let layer_shell = LayerShell::bind(&globals, &qh).map_err(|e| format!("zwlr_layer_shell_v1: {e}"))?;
        let shm = Shm::bind(&globals, &qh).map_err(|e| format!("wl_shm: {e}"))?;
        let output_state = OutputState::new(&globals, &qh);
        let seat_state = SeatState::new(&globals, &qh);
        let fractional_mgr = globals.bind::<WpFractionalScaleManagerV1, _, _>(&qh, 1..=1, ()).ok();
        let viewporter = globals.bind::<WpViewporter, _, _>(&qh, 1..=1, ()).ok();

        let (tx, rx) = channel::channel::<Cmd>();
        let shared_outputs = Arc::new(Mutex::new(Vec::new()));

        let mut state = WaylandState {
            conn: conn.clone(),
            qh: qh.clone(),
            proxy,
            registry_state: RegistryState::new(&globals),
            compositor,
            layer_shell,
            shm,
            output_state,
            seat_state,
            fractional_mgr,
            viewporter,
            pool: SlotPool::new(512 * 300 * 4, &shm_bind_ref(&globals, &qh)?).map_err(|e| format!("wl_shm pool: {e}"))?,
            shared_outputs: shared_outputs.clone(),
            primary: None,
            primary_visible: true,
            primary_output_id: None,
            primary_keyboard_focus: false,
            primary_scale: 1.0,
            mirrors: HashMap::new(),
            pointer: None,
            keyboard: None,
            shift_down: false,
            loop_signal: None,
            closed: false,
        };

        // Populate outputs and seat capabilities synchronously before handing control back.
        event_queue.roundtrip(&mut state).map_err(|e| format!("wayland roundtrip: {e}"))?;
        event_queue.roundtrip(&mut state).map_err(|e| format!("wayland output roundtrip: {e}"))?;
        state.refresh_shared_outputs();

        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<Result<(), String>>(1);
        thread::Builder::new().name("wayland-notch".into()).spawn(move || run_loop(conn, event_queue, state, rx, ready_tx)).map_err(|e| format!("spawn wayland thread: {e}"))?;

        ready_rx.recv().map_err(|e| format!("wayland thread startup: {e}"))??;
        Ok(Self { tx, outputs: shared_outputs })
    }

    pub fn outputs(&self) -> Vec<OutputInfo> {
        self.outputs.lock().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn present(&self, frame: Frame) {
        let _ = self.tx.send(Cmd::Present(frame));
    }

    pub fn set_visible(&self, visible: bool) {
        let _ = self.tx.send(Cmd::SetVisible(visible));
    }

    pub fn set_output(&self, output_id: Option<u32>) {
        let _ = self.tx.send(Cmd::SetOutput(output_id));
    }

    pub fn set_keyboard_focus(&self, focus: bool) {
        let _ = self.tx.send(Cmd::SetKeyboardFocus(focus));
    }

    pub fn sync_mirrors(&self, wanted: Vec<(u32, MirrorRole)>) {
        let _ = self.tx.send(Cmd::SyncMirrors(wanted));
    }

    pub fn present_mirror(&self, output_id: u32, frame: Frame) {
        let _ = self.tx.send(Cmd::PresentMirror(output_id, frame));
    }

    pub fn hide_mirror(&self, output_id: u32) {
        let _ = self.tx.send(Cmd::HideMirror(output_id));
    }

    pub fn shutdown(&self) {
        let _ = self.tx.send(Cmd::Shutdown);
    }
}

fn shm_bind_ref(globals: &GlobalList, qh: &QueueHandle<WaylandState>) -> Result<Shm, String> {
    Shm::bind(globals, qh).map_err(|e| format!("wl_shm: {e}"))
}

fn run_loop(
    conn: Connection,
    event_queue: wayland_client::EventQueue<WaylandState>,
    mut state: WaylandState,
    rx: channel::Channel<Cmd>,
    ready_tx: std::sync::mpsc::SyncSender<Result<(), String>>,
) {
    let mut event_loop: EventLoop<WaylandState> = match EventLoop::try_new() {
        Ok(el) => el,
        Err(e) => {
            let _ = ready_tx.send(Err(format!("calloop init: {e}")));
            return;
        }
    };
    let handle: LoopHandle<WaylandState> = event_loop.handle();
    state.loop_signal = Some(event_loop.get_signal());

    if let Err(e) = WaylandSource::new(conn, event_queue).insert(handle.clone()) {
        let _ = ready_tx.send(Err(format!("wayland source: {e}")));
        return;
    }

    if let Err(e) = handle.insert_source(rx, |ev, _, state| {
        if let channel::Event::Msg(cmd) = ev {
            state.handle_cmd(cmd);
        } else if let Some(sig) = &state.loop_signal {
            sig.stop();
        }
    }) {
        let _ = ready_tx.send(Err(format!("channel source: {e}")));
        return;
    }

    let _ = ready_tx.send(Ok(()));
    let _ = event_loop.run(None, &mut state, |_| {});
}

struct ManagedSurface {
    layer: LayerSurface,
    viewport: Option<WpViewport>,
    fractional: Option<WpFractionalScaleV1>,
    output_id: Option<u32>,
    configured: Option<(u32, u32)>,
    requested: Option<(u32, u32)>,
    pending: Option<Frame>,
    scale: f64,
    buffer_scale: i32,
}

impl Drop for ManagedSurface {
    fn drop(&mut self) {
        if let Some(fs) = self.fractional.take() {
            fs.destroy();
        }
        if let Some(vp) = self.viewport.take() {
            vp.destroy();
        }
    }
}

struct MirrorSurface {
    role: MirrorRole,
    surface: Option<ManagedSurface>,
    hidden: bool,
}

struct WaylandState {
    conn: Connection,
    qh: QueueHandle<Self>,
    proxy: EventLoopProxy<UserEvent>,
    registry_state: RegistryState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    output_state: OutputState,
    seat_state: SeatState,
    fractional_mgr: Option<WpFractionalScaleManagerV1>,
    viewporter: Option<WpViewporter>,
    pool: SlotPool,
    shared_outputs: Arc<Mutex<Vec<OutputInfo>>>,
    primary: Option<ManagedSurface>,
    primary_visible: bool,
    primary_output_id: Option<u32>,
    primary_keyboard_focus: bool,
    primary_scale: f64,
    mirrors: HashMap<u32, MirrorSurface>,
    pointer: Option<wl_pointer::WlPointer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    shift_down: bool,
    loop_signal: Option<LoopSignal>,
    closed: bool,
}

impl WaylandState {
    fn emit(&self, ev: Event) {
        let _ = self.proxy.send_event(UserEvent::Wayland(ev));
    }

    fn wl_output_by_id(&self, id: Option<u32>) -> Option<wl_output::WlOutput> {
        let want = id?;
        self.output_state.outputs().find(|o| self.output_state.info(o).is_some_and(|info| info.id == want))
    }

    fn output_integer_scale(&self, output_id: Option<u32>) -> i32 {
        let outputs = self.shared_outputs.lock().ok();
        let scale = outputs.as_ref().and_then(|list| output_id.and_then(|id| list.iter().find(|o| o.id == id)).or_else(|| list.first())).map_or(1.0, |o| o.scale);
        scale.round().max(1.0) as i32
    }

    fn create_managed(&self, output_id: Option<u32>, namespace: &str, interactivity: KeyboardInteractivity, initial_size: (u32, u32)) -> ManagedSurface {
        let surface = self.compositor.create_surface(&self.qh);
        let wl_out = self.wl_output_by_id(output_id);
        let layer = self.layer_shell.create_layer_surface(&self.qh, surface, Layer::Top, Some(namespace), wl_out.as_ref());
        layer.set_anchor(Anchor::TOP);
        layer.set_exclusive_zone(0);
        layer.set_keyboard_interactivity(interactivity);
        let (lw, lh) = (initial_size.0.max(1), initial_size.1.max(1));
        layer.set_size(lw, lh);

        let (viewport, fractional) = match (&self.viewporter, &self.fractional_mgr) {
            (Some(vp_mgr), Some(fs_mgr)) => {
                let wl_surf = layer.wl_surface();
                let vp = vp_mgr.get_viewport(wl_surf, &self.qh, ());
                let fs = fs_mgr.get_fractional_scale(wl_surf, &self.qh, wl_surf.id());
                (Some(vp), Some(fs))
            }
            _ => (None, None),
        };

        let int_scale = self.output_integer_scale(output_id);
        layer.commit();
        ManagedSurface { layer, viewport, fractional, output_id, configured: None, requested: Some((lw, lh)), pending: None, scale: int_scale as f64, buffer_scale: int_scale }
    }

    fn handle_cmd(&mut self, cmd: Cmd) {
        if self.closed {
            return;
        }
        match cmd {
            Cmd::Present(frame) => {
                if !self.primary_visible {
                    return;
                }
                if self.primary.is_none() {
                    let inter = if self.primary_keyboard_focus { KeyboardInteractivity::Exclusive } else { KeyboardInteractivity::None };
                    let mut m = self.create_managed(self.primary_output_id, "sky-island", inter, frame.logical_size);
                    m.pending = Some(frame);
                    self.primary = Some(m);
                    let _ = self.conn.flush();
                    return;
                }
                let mut surface = self.primary.take().unwrap();
                self.present_managed(&mut surface, frame);
                self.primary = Some(surface);
                let _ = self.conn.flush();
            }
            Cmd::SetVisible(visible) => {
                self.primary_visible = visible;
                if !visible {
                    self.primary = None;
                    let _ = self.conn.flush();
                } else if self.primary.is_none() {
                    let inter = if self.primary_keyboard_focus { KeyboardInteractivity::Exclusive } else { KeyboardInteractivity::None };
                    self.primary = Some(self.create_managed(self.primary_output_id, "sky-island", inter, (172, 28)));
                    let _ = self.conn.flush();
                }
            }
            Cmd::SetOutput(output_id) => {
                if self.primary_output_id != output_id {
                    self.primary_output_id = output_id;
                    if let Some(old) = self.primary.take() {
                        let inter = if self.primary_keyboard_focus { KeyboardInteractivity::Exclusive } else { KeyboardInteractivity::None };
                        let size = old.pending.as_ref().map(|f| f.logical_size).or(old.requested).unwrap_or((172, 28));
                        let mut fresh = self.create_managed(output_id, "sky-island", inter, size);
                        fresh.pending = old.pending.clone();
                        drop(old);
                        self.primary = Some(fresh);
                        let _ = self.conn.flush();
                    }
                }
            }
            Cmd::SetKeyboardFocus(focus) => {
                self.primary_keyboard_focus = focus;
                if let Some(p) = &self.primary {
                    p.layer.set_keyboard_interactivity(if focus { KeyboardInteractivity::Exclusive } else { KeyboardInteractivity::None });
                    p.layer.commit();
                    let _ = self.conn.flush();
                }
            }
            Cmd::SyncMirrors(wanted) => {
                self.mirrors.retain(|id, _| wanted.iter().any(|(w, _)| w == id));
                for (id, role) in wanted {
                    if let Some(existing) = self.mirrors.get_mut(&id) {
                        if existing.role != role {
                            existing.role = role;
                            existing.surface = None;
                            existing.hidden = false;
                        }
                        if role == MirrorRole::EdgeTrigger && existing.surface.is_none() && !existing.hidden {
                            self.spawn_edge_trigger(id);
                        }
                    } else {
                        self.mirrors.insert(id, MirrorSurface { role, surface: None, hidden: false });
                        if role == MirrorRole::EdgeTrigger {
                            self.spawn_edge_trigger(id);
                        }
                    }
                }
                let _ = self.conn.flush();
            }
            Cmd::PresentMirror(output_id, frame) => {
                let Some(entry) = self.mirrors.get(&output_id) else { return };
                if entry.role != MirrorRole::ClosedNotch {
                    return;
                }
                let mut managed = self.mirrors.get_mut(&output_id).and_then(|m| m.surface.take());
                if let Some(entry) = self.mirrors.get_mut(&output_id) {
                    entry.hidden = false;
                }
                match managed.take() {
                    None => {
                        let mut m = self.create_managed(Some(output_id), "sky-island-mirror", KeyboardInteractivity::None, frame.logical_size);
                        m.pending = Some(frame);
                        if let Some(entry) = self.mirrors.get_mut(&output_id) {
                            entry.surface = Some(m);
                        }
                    }
                    Some(mut m) => {
                        self.present_managed(&mut m, frame);
                        if let Some(entry) = self.mirrors.get_mut(&output_id) {
                            entry.surface = Some(m);
                        }
                    }
                }
                let _ = self.conn.flush();
            }
            Cmd::HideMirror(output_id) => {
                if let Some(entry) = self.mirrors.get_mut(&output_id) {
                    entry.hidden = true;
                    entry.surface = None;
                    let _ = self.conn.flush();
                }
            }
            Cmd::Shutdown => {
                self.primary = None;
                self.mirrors.clear();
                let _ = self.conn.flush();
                if let Some(sig) = &self.loop_signal {
                    sig.stop();
                }
            }
        }
    }

    fn spawn_edge_trigger(&mut self, output_id: u32) {
        let lw = ((crate::ui::COLLAPSED.0 + 2.0 * crate::ui::EAR + 48.0).round() as u32).max(1);
        let lh = 2u32;
        let frame = Frame { logical_size: (lw, lh), buffer_size: (lw, lh), pixels: vec![0u8; (lw * lh * 4) as usize] };
        let mut m = self.create_managed(Some(output_id), "sky-island-edge", KeyboardInteractivity::None, (lw, lh));
        m.pending = Some(frame);
        if let Some(entry) = self.mirrors.get_mut(&output_id) {
            entry.surface = Some(m);
        }
    }

    fn present_managed(&mut self, m: &mut ManagedSurface, frame: Frame) {
        let target_logical = (frame.logical_size.0.max(1), frame.logical_size.1.max(1));
        if m.configured == Some(target_logical) {
            self.commit_frame(m, &frame);
            m.pending = None;
        } else {
            if m.requested != Some(target_logical) {
                m.layer.set_size(target_logical.0, target_logical.1);
                m.layer.commit();
                m.requested = Some(target_logical);
            }
            m.pending = Some(frame);
        }
    }

    fn commit_frame(&mut self, m: &ManagedSurface, frame: &Frame) {
        let lw = frame.logical_size.0.max(1);
        let lh = frame.logical_size.1.max(1);
        let wl_surf = m.layer.wl_surface();

        if let Some(vp) = &m.viewport {
            let (bw, bh) = (frame.buffer_size.0.max(1), frame.buffer_size.1.max(1));
            let Ok((buffer, canvas)) = self.pool.create_buffer(bw as i32, bh as i32, bw as i32 * 4, wl_shm::Format::Argb8888) else {
                return;
            };
            let n = canvas.len().min(frame.pixels.len());
            canvas[..n].copy_from_slice(&frame.pixels[..n]);
            wl_surf.set_buffer_scale(1);
            vp.set_destination(lw as i32, lh as i32);
            wl_surf.damage_buffer(0, 0, bw as i32, bh as i32);
            if buffer.attach_to(wl_surf).is_ok() {
                m.layer.commit();
            }
        } else {
            // Without wp_viewporter, buffer size must be an exact multiple of buffer_scale.
            let bs = m.buffer_scale.max(1) as u32;
            let (bw, bh) = (lw * bs, lh * bs);
            let Ok((buffer, canvas)) = self.pool.create_buffer(bw as i32, bh as i32, bw as i32 * 4, wl_shm::Format::Argb8888) else {
                return;
            };
            if frame.buffer_size == (bw, bh) {
                let n = canvas.len().min(frame.pixels.len());
                canvas[..n].copy_from_slice(&frame.pixels[..n]);
            } else {
                blit_scaled_argb(&frame.pixels, frame.buffer_size, canvas, (bw, bh));
            }
            wl_surf.set_buffer_scale(bs as i32);
            wl_surf.damage_buffer(0, 0, bw as i32, bh as i32);
            if buffer.attach_to(wl_surf).is_ok() {
                m.layer.commit();
            }
        }
    }

    fn refresh_shared_outputs(&self) {
        let mut out = Vec::new();
        for wl_out in self.output_state.outputs() {
            let Some(info) = self.output_state.info(&wl_out) else { continue };
            let logical_pos = info.logical_position.unwrap_or(info.location);
            let current_mode = info.modes.iter().find(|m| m.current).or_else(|| info.modes.first());
            let scale = info.scale_factor.max(1) as f64;
            let logical_size = info.logical_size.unwrap_or_else(|| {
                let raw = current_mode.map_or((1920, 1080), |m| m.dimensions);
                ((raw.0 as f64 / scale).round() as i32, (raw.1 as f64 / scale).round() as i32)
            });
            let refresh_millihertz = current_mode.map_or(60_000, |m| m.refresh_rate.max(0) as u32);
            let name = info.name.clone().unwrap_or_else(|| format!("output-{}", info.id));
            out.push(OutputInfo { id: info.id, name, logical_pos, logical_size, scale, refresh_millihertz });
        }
        if let Ok(mut guard) = self.shared_outputs.lock() {
            *guard = out;
        }
    }

    fn identify_surface(&self, surface: &wl_surface::WlSurface) -> Option<(Option<u32>, bool)> {
        if let Some(p) = &self.primary
            && p.layer.wl_surface() == surface
        {
            return Some((p.output_id, false));
        }
        for (&id, m) in &self.mirrors {
            if let Some(ms) = &m.surface
                && ms.layer.wl_surface() == surface
            {
                return Some((Some(id), true));
            }
        }
        None
    }
}

/// Pack premultiplied RGBA (`tiny_skia::Pixmap`) into ARGB8888 little-endian bytes for `wl_shm`,
/// cropping `tuck_px` transparent/off-screen pixel rows from the top while tucked.
pub fn pack_shm_pixels(pixmap: &Pixmap, tuck_px: u32) -> (u32, u32, Vec<u8>) {
    let w = pixmap.width();
    let h = pixmap.height();
    let crop = tuck_px.min(h.saturating_sub(1));
    let out_h = h - crop;
    let src_pixels = &pixmap.pixels()[(crop as usize * w as usize)..];
    let mut bytes = Vec::with_capacity(src_pixels.len() * 4);
    for p in src_pixels {
        bytes.extend_from_slice(&[p.blue(), p.green(), p.red(), p.alpha()]);
    }
    (w, out_h, bytes)
}

/// Blit/stretch ARGB8888 pixels when `wp_viewporter` is unavailable and buffer scale demands exact integer dimensions.
fn blit_scaled_argb(src: &[u8], src_size: (u32, u32), dst: &mut [u8], dst_size: (u32, u32)) {
    dst.fill(0);
    let (sw, sh) = (src_size.0 as usize, src_size.1 as usize);
    let (dw, dh) = (dst_size.0 as usize, dst_size.1 as usize);
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 {
        return;
    }
    for y in 0..dh {
        let sy = (y * sh / dh).min(sh - 1);
        for x in 0..dw {
            let sx = (x * sw / dw).min(sw - 1);
            let si = (sy * sw + sx) * 4;
            let di = (y * dw + x) * 4;
            if si + 4 <= src.len() && di + 4 <= dst.len() {
                dst[di..di + 4].copy_from_slice(&src[si..si + 4]);
            }
        }
    }
}

/// Pick the target Wayland output matching the user's `Screen` setting.
pub fn select_output(outputs: &[OutputInfo], screen: settings::Screen, follow: Option<u32>) -> Option<&OutputInfo> {
    if outputs.is_empty() {
        return None;
    }
    match screen {
        settings::Screen::Main => outputs.iter().find(|o| o.logical_pos == (0, 0)).or_else(|| outputs.first()),
        settings::Screen::Display(n) => {
            let mut ordered: Vec<&OutputInfo> = outputs.iter().collect();
            ordered.sort_by_key(|o| (o.logical_pos.0, o.logical_pos.1, o.id));
            (n as usize).checked_sub(1).and_then(|idx| ordered.get(idx).copied()).or_else(|| outputs.first())
        }
        settings::Screen::All | settings::Screen::Every => {
            follow.and_then(|id| outputs.iter().find(|o| o.id == id)).or_else(|| outputs.iter().find(|o| o.logical_pos == (0, 0))).or_else(|| outputs.first())
        }
    }
}

/// Convert Wayland `wl_pointer` vertical scroll values into `winit` delta units.
/// Wayland positive vertical scroll moves content downward (scroll down), opposite `winit` `LineDelta`/`PixelDelta`.
pub fn wayland_scroll_delta(discrete: i32, absolute: f64) -> Option<MouseScrollDelta> {
    if discrete != 0 {
        Some(MouseScrollDelta::LineDelta(0.0, -(discrete as f32)))
    } else if absolute.abs() > f64::EPSILON {
        Some(MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(0.0, -absolute)))
    } else {
        None
    }
}

/// Map Linux evdev keycodes (from raw `wl_keyboard::Event::Key`) into `winit` keys + basic text.
pub fn evdev_to_key(code: u32, shift: bool) -> Option<(Key, Option<String>)> {
    let ch = |lower: &str, upper: &str| -> (Key, Option<String>) {
        let s = if shift { upper } else { lower };
        (Key::Character(s.into()), Some(s.to_string()))
    };
    Some(match code {
        1 => (Key::Named(NamedKey::Escape), None),
        14 => (Key::Named(NamedKey::Backspace), None),
        15 => (Key::Named(NamedKey::Tab), None),
        28 | 96 => (Key::Named(NamedKey::Enter), None),
        103 => (Key::Named(NamedKey::ArrowUp), None),
        105 => (Key::Named(NamedKey::ArrowLeft), None),
        106 => (Key::Named(NamedKey::ArrowRight), None),
        108 => (Key::Named(NamedKey::ArrowDown), None),
        57 => (Key::Named(NamedKey::Space), Some(" ".into())),
        2 => ch("1", "!"),
        3 => ch("2", "@"),
        4 => ch("3", "#"),
        5 => ch("4", "$"),
        6 => ch("5", "%"),
        7 => ch("6", "^"),
        8 => ch("7", "&"),
        9 => ch("8", "*"),
        10 => ch("9", "("),
        11 => ch("0", ")"),
        12 => ch("-", "_"),
        13 => ch("=", "+"),
        16 => ch("q", "Q"),
        17 => ch("w", "W"),
        18 => ch("e", "E"),
        19 => ch("r", "R"),
        20 => ch("t", "T"),
        21 => ch("y", "Y"),
        22 => ch("u", "U"),
        23 => ch("i", "I"),
        24 => ch("o", "O"),
        25 => ch("p", "P"),
        26 => ch("[", "{"),
        27 => ch("]", "}"),
        30 => ch("a", "A"),
        31 => ch("s", "S"),
        32 => ch("d", "D"),
        33 => ch("f", "F"),
        34 => ch("g", "G"),
        35 => ch("h", "H"),
        36 => ch("j", "J"),
        37 => ch("k", "K"),
        38 => ch("l", "L"),
        39 => ch(";", ":"),
        40 => ch("'", "\""),
        41 => ch("`", "~"),
        43 => ch("\\", "|"),
        44 => ch("z", "Z"),
        45 => ch("x", "X"),
        46 => ch("c", "C"),
        47 => ch("v", "V"),
        48 => ch("b", "B"),
        49 => ch("n", "N"),
        50 => ch("m", "M"),
        51 => ch(",", "<"),
        52 => ch(".", ">"),
        53 => ch("/", "?"),
        // Numpad digits
        71 => ch("7", "7"),
        72 => ch("8", "8"),
        73 => ch("9", "9"),
        75 => ch("4", "4"),
        76 => ch("5", "5"),
        77 => ch("6", "6"),
        79 => ch("1", "1"),
        80 => ch("2", "2"),
        81 => ch("3", "3"),
        82 => ch("0", "0"),
        83 => ch(".", "."),
        _ => return None,
    })
}

impl CompositorHandler for WaylandState {
    fn scale_factor_changed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _surface: &wl_surface::WlSurface, _new_factor: i32) {}
    fn transform_changed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _surface: &wl_surface::WlSurface, _new_transform: wl_output::Transform) {}
    fn frame(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _surface: &wl_surface::WlSurface, _time: u32) {}
    fn surface_enter(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _surface: &wl_surface::WlSurface, _output: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _surface: &wl_surface::WlSurface, _output: &wl_output::WlOutput) {}
}

impl OutputHandler for WaylandState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _output: wl_output::WlOutput) {
        self.refresh_shared_outputs();
        self.emit(Event::OutputsChanged);
    }

    fn update_output(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _output: wl_output::WlOutput) {
        self.refresh_shared_outputs();
        self.emit(Event::OutputsChanged);
    }

    fn output_destroyed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _output: wl_output::WlOutput) {
        self.refresh_shared_outputs();
        self.emit(Event::OutputsChanged);
    }
}

impl LayerShellHandler for WaylandState {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, layer: &LayerSurface) {
        if self.primary.as_ref().is_some_and(|p| p.layer.wl_surface() == layer.wl_surface()) {
            self.primary = None;
            self.closed = true;
            self.emit(Event::Closed);
        } else {
            self.mirrors.retain(|_, m| m.surface.as_ref().is_none_or(|ms| ms.layer.wl_surface() != layer.wl_surface()));
        }
    }

    fn configure(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, layer: &LayerSurface, configure: LayerSurfaceConfigure, _serial: u32) {
        let cfg = (configure.new_size.0.max(1), configure.new_size.1.max(1));
        if self.primary.as_ref().is_some_and(|p| p.layer.wl_surface() == layer.wl_surface()) {
            let mut m = self.primary.take().unwrap();
            m.configured = Some(cfg);
            if let Some(frame) = m.pending.take() {
                if (frame.logical_size.0.max(1), frame.logical_size.1.max(1)) == cfg {
                    self.commit_frame(&m, &frame);
                } else {
                    self.present_managed(&mut m, frame);
                }
            }
            self.primary = Some(m);
            let _ = self.conn.flush();
            return;
        }
        let target_id = self.mirrors.iter().find_map(|(&id, m)| m.surface.as_ref().filter(|ms| ms.layer.wl_surface() == layer.wl_surface()).map(|_| id));
        if let Some(id) = target_id
            && let Some(mut m) = self.mirrors.get_mut(&id).and_then(|e| e.surface.take())
        {
            m.configured = Some(cfg);
            if let Some(frame) = m.pending.take() {
                if (frame.logical_size.0.max(1), frame.logical_size.1.max(1)) == cfg {
                    self.commit_frame(&m, &frame);
                } else {
                    self.present_managed(&mut m, frame);
                }
            }
            if let Some(entry) = self.mirrors.get_mut(&id) {
                entry.surface = Some(m);
            }
            let _ = self.conn.flush();
        }
    }
}

impl SeatHandler for WaylandState {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {}

    fn new_capability(&mut self, _conn: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seat_state.get_pointer(qh, &seat).ok();
        }
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            self.keyboard = Some(seat.get_keyboard(qh, ()));
        }
    }

    fn remove_capability(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Pointer
            && let Some(p) = self.pointer.take()
        {
            p.release();
        }
        if capability == Capability::Keyboard
            && let Some(k) = self.keyboard.take()
        {
            k.release();
        }
    }

    fn remove_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {}
}

impl PointerHandler for WaylandState {
    fn pointer_frame(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _pointer: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        for ev in events {
            let Some((output_id, mirror)) = self.identify_surface(&ev.surface) else { continue };
            match &ev.kind {
                PointerEventKind::Enter { .. } => self.emit(Event::CursorEntered { output_id, mirror, x: ev.position.0, y: ev.position.1 }),
                PointerEventKind::Leave { .. } => self.emit(Event::CursorLeft { output_id, mirror }),
                PointerEventKind::Motion { .. } => self.emit(Event::CursorMoved { output_id, mirror, x: ev.position.0, y: ev.position.1 }),
                PointerEventKind::Press { button, .. } if *button == BTN_LEFT && !mirror => self.emit(Event::MouseInput { pressed: true }),
                PointerEventKind::Release { button, .. } if *button == BTN_LEFT && !mirror => self.emit(Event::MouseInput { pressed: false }),
                PointerEventKind::Axis { vertical, .. } if !mirror => {
                    if let Some(delta) = wayland_scroll_delta(vertical.discrete, vertical.absolute) {
                        self.emit(Event::MouseWheel { delta });
                    }
                }
                _ => {}
            }
        }
    }
}

impl ShmHandler for WaylandState {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for WaylandState {
    fn event(state: &mut Self, _proxy: &wl_keyboard::WlKeyboard, event: wl_keyboard::Event, _data: &(), _conn: &Connection, _qh: &QueueHandle<Self>) {
        match event {
            wl_keyboard::Event::Enter { surface, .. } => {
                if state.primary.as_ref().is_some_and(|p| p.layer.wl_surface() == &surface) {
                    state.emit(Event::Focused(true));
                }
            }
            wl_keyboard::Event::Leave { surface, .. } => {
                if state.primary.as_ref().is_some_and(|p| p.layer.wl_surface() == &surface) {
                    state.emit(Event::Focused(false));
                }
            }
            wl_keyboard::Event::Modifiers { mods_depressed, .. } => {
                state.shift_down = (mods_depressed & 1) != 0;
            }
            wl_keyboard::Event::Key { key, state: WEnum::Value(wl_keyboard::KeyState::Pressed), .. } => {
                if let Some((logical_key, text)) = evdev_to_key(key, state.shift_down) {
                    state.emit(Event::KeyPressed { key: logical_key, text });
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<WpFractionalScaleManagerV1, ()> for WaylandState {
    fn event(
        _state: &mut Self,
        _proxy: &WpFractionalScaleManagerV1,
        _event: <WpFractionalScaleManagerV1 as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WpFractionalScaleV1, ObjectId> for WaylandState {
    fn event(state: &mut Self, _proxy: &WpFractionalScaleV1, event: wp_fractional_scale_v1::Event, surface_id: &ObjectId, _conn: &Connection, _qh: &QueueHandle<Self>) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            let factor = (scale as f64 / 120.0).max(0.5);
            if let Some(p) = &mut state.primary
                && p.layer.wl_surface().id() == *surface_id
            {
                p.scale = factor;
                if (state.primary_scale - factor).abs() > 1e-4 {
                    state.primary_scale = factor;
                    state.emit(Event::ScaleChanged(factor));
                }
                return;
            }
            for m in state.mirrors.values_mut() {
                if let Some(ms) = &mut m.surface
                    && ms.layer.wl_surface().id() == *surface_id
                {
                    ms.scale = factor;
                    break;
                }
            }
        }
    }
}

impl Dispatch<WpViewporter, ()> for WaylandState {
    fn event(_state: &mut Self, _proxy: &WpViewporter, _event: <WpViewporter as Proxy>::Event, _data: &(), _conn: &Connection, _qh: &QueueHandle<Self>) {}
}

impl Dispatch<WpViewport, ()> for WaylandState {
    fn event(_state: &mut Self, _proxy: &WpViewport, _event: <WpViewport as Proxy>::Event, _data: &(), _conn: &Connection, _qh: &QueueHandle<Self>) {}
}

delegate_compositor!(WaylandState);
delegate_output!(WaylandState);
delegate_shm!(WaylandState);
delegate_seat!(WaylandState);
delegate_pointer!(WaylandState);
delegate_layer!(WaylandState);
delegate_registry!(WaylandState);

impl ProvidesRegistryState for WaylandState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiny_skia::{Color, PremultipliedColorU8};

    #[test]
    fn pack_shm_pixels_converts_rgba_to_argb_and_crops_tuck_rows() {
        let mut pm = Pixmap::new(2, 4).unwrap();
        let top = PremultipliedColorU8::from_rgba(10, 20, 30, 40).unwrap();
        let bottom = PremultipliedColorU8::from_rgba(100, 150, 200, 255).unwrap();
        pm.fill(Color::TRANSPARENT);
        pm.pixels_mut()[0] = top;
        pm.pixels_mut()[4] = bottom;

        let (w, h, bytes) = pack_shm_pixels(&pm, 2);
        assert_eq!((w, h), (2, 2));
        assert_eq!(bytes.len(), 2 * 2 * 4);
        assert_eq!(&bytes[0..4], &[200, 150, 100, 255]);

        // Cropping beyond height leaves at least one row.
        let (w2, h2, bytes2) = pack_shm_pixels(&pm, 10);
        assert_eq!((w2, h2), (2, 1));
        assert_eq!(bytes2.len(), 8);
    }

    #[test]
    fn select_output_handles_main_display_order_and_follow() {
        let outputs = vec![
            OutputInfo { id: 10, name: "DP-1".into(), logical_pos: (1920, 0), logical_size: (2560, 1440), scale: 1.0, refresh_millihertz: 144_000 },
            OutputInfo { id: 5, name: "eDP-1".into(), logical_pos: (0, 0), logical_size: (1920, 1080), scale: 1.5, refresh_millihertz: 60_000 },
        ];
        assert_eq!(select_output(&outputs, settings::Screen::Main, None).map(|o| o.id), Some(5));
        assert_eq!(select_output(&outputs, settings::Screen::Display(1), None).map(|o| o.id), Some(5));
        assert_eq!(select_output(&outputs, settings::Screen::Display(2), None).map(|o| o.id), Some(10));
        assert_eq!(select_output(&outputs, settings::Screen::All, Some(10)).map(|o| o.id), Some(10));
        assert_eq!(select_output(&outputs, settings::Screen::Every, Some(999)).map(|o| o.id), Some(5));
    }

    #[test]
    fn wayland_scroll_negates_direction_for_winit_compatibility() {
        assert_eq!(wayland_scroll_delta(1, 15.0), Some(MouseScrollDelta::LineDelta(0.0, -1.0)));
        assert_eq!(wayland_scroll_delta(0, -12.5), Some(MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(0.0, 12.5))));
        assert_eq!(wayland_scroll_delta(0, 0.0), None);
    }

    #[test]
    fn evdev_key_maps_typing_and_navigation() {
        assert_eq!(evdev_to_key(1, false), Some((Key::Named(NamedKey::Escape), None)));
        assert_eq!(evdev_to_key(28, false), Some((Key::Named(NamedKey::Enter), None)));
        assert_eq!(evdev_to_key(30, false), Some((Key::Character("a".into()), Some("a".into()))));
        assert_eq!(evdev_to_key(30, true), Some((Key::Character("A".into()), Some("A".into()))));
        assert_eq!(evdev_to_key(2, true), Some((Key::Character("!".into()), Some("!".into()))));
    }
}
