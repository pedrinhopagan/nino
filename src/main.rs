mod capture;
mod clipboard;
mod draw;
mod editor;
mod overlay;
mod record;
mod session;
mod shortcuts;

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, FrameCallbackData},
    delegate_registry,
    dispatch2::Dispatch2,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{EventLoop, channel},
        calloop_wayland_source::WaylandSource,
        client::{
            Connection, Proxy, QueueHandle,
            globals::registry_queue_init,
            protocol::{wl_keyboard, wl_output, wl_pointer, wl_region, wl_seat, wl_surface},
        },
        protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::{Shape, WpCursorShapeDeviceV1},
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers},
        pointer::{PointerEvent, PointerEventKind, PointerHandler, cursor_shape::CursorShapeManager},
    },
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use wayland_protocols::wp::viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter};

use clipboard::{Clipboard, Payload};
use draw::{Sel, Tool};
use overlay::{Mode, Outcome, Overlay};
use record::Recording;
use session::Session;
use shortcuts::Action;

pub enum Msg {
    Shortcut(Action),
    RecordingDone(Result<PathBuf, String>),
    Note(usize, String),
    Fatal(String),
}

const BTN_LEFT: u32 = 0x110;
const NAMESPACE: &str = "utility";

pub struct App {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    wayland: Connection,
    viewporter: WpViewporter,
    cursor_shape: Option<CursorShapeManager>,
    pool: SlotPool,
    pub clipboard: Option<Clipboard>,
    dbus: zbus::Connection,
    tx: channel::Sender<Msg>,
    qh: QueueHandle<App>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    cursor_device: Option<WpCursorShapeDeviceV1>,
    modifiers: Modifiers,
    pointer_serial: u32,
    cursor: Option<Shape>,
    overlay: Option<Overlay>,
    ring: Option<Overlay>,
    opened_at: Option<(Instant, Duration)>,
    pending_record: Option<(i32, i32, u32, u32)>,
    recording: Option<Recording>,
    session: Option<Session>,
    shot_at: Option<Duration>,
    stopping: bool,
    fatal: Option<String>,
}

pub struct Ud;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let result = match args.get(1).map(String::as_str) {
        Some("editor") => editor::run(
            args.get(2).map_or("Anotação", String::as_str),
            args.get(3).map_or("cancela", String::as_str),
        ),
        _ => run(),
    };
    if let Err(e) = result {
        eprintln!("nino: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("wayland: {e}"))?;
    let (globals, queue) = registry_queue_init::<App>(&conn).map_err(|e| format!("registry: {e}"))?;
    let qh = queue.handle();

    let mut event_loop: EventLoop<App> = EventLoop::try_new().map_err(|e| e.to_string())?;
    WaylandSource::new(conn.clone(), queue)
        .insert(event_loop.handle())
        .map_err(|e| e.to_string())?;

    let (tx, rx) = channel::channel::<Msg>();
    event_loop
        .handle()
        .insert_source(rx, |ev, _, app: &mut App| {
            if let channel::Event::Msg(msg) = ev {
                app.handle(msg);
            }
        })
        .map_err(|e| e.to_string())?;

    let shm = Shm::bind(&globals, &qh).map_err(|e| format!("wl_shm: {e}"))?;
    let pool = SlotPool::new(1920 * 1080 * 4 * 2, &shm).map_err(|e| format!("shm pool: {e}"))?;

    let mut app = App {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        compositor: CompositorState::bind(&globals, &qh).map_err(|e| format!("wl_compositor: {e}"))?,
        layer_shell: LayerShell::bind(&globals, &qh).map_err(|e| format!("layer shell: {e}"))?,
        viewporter: globals
            .bind(&qh, 1..=1, Ud)
            .map_err(|e| format!("wp_viewporter: {e}"))?,
        cursor_shape: CursorShapeManager::bind(&globals, &qh).ok(),
        clipboard: Clipboard::bind(&globals, &qh).map_err(|e| eprintln!("nino: {e}")).ok(),
        shm,
        wayland: conn.clone(),
        pool,
        dbus: zbus::block_on(zbus::Connection::session()).map_err(|e| format!("dbus: {e}"))?,
        tx: tx.clone(),
        qh: qh.clone(),
        keyboard: None,
        pointer: None,
        cursor_device: None,
        modifiers: Modifiers::default(),
        pointer_serial: 0,
        cursor: None,
        overlay: None,
        ring: None,
        opened_at: None,
        pending_record: None,
        recording: None,
        session: None,
        shot_at: None,
        stopping: false,
        fatal: None,
    };

    shortcuts::spawn(tx);
    eprintln!("nino: ready");

    loop {
        event_loop.dispatch(None, &mut app).map_err(|e| e.to_string())?;
        if let Some(e) = app.fatal.take() {
            return Err(e);
        }
    }
}

impl App {
    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Shortcut(Action::Screenshot) => {
                if self.overlay.is_none() && !self.editing() {
                    self.open_shot();
                }
            }
            Msg::Shortcut(Action::Record) => {
                if let Some(rec) = &self.recording {
                    if !self.stopping {
                        self.stopping = true;
                        rec.stop();
                    }
                } else if self.overlay.as_ref().is_some_and(|o| !o.is_shot()) {
                    self.close_overlay();
                } else if self.overlay.is_none() && self.session.is_none() {
                    self.open_record();
                }
            }
            Msg::Shortcut(Action::Note) => self.open_note(),
            Msg::RecordingDone(result) => {
                self.recording = None;
                self.stopping = false;
                self.close_ring();
                match result {
                    Ok(path) => {
                        eprintln!("nino: saved {}", path.display());
                        if let Some(session) = self.session.as_mut() {
                            session.video = Some(path);
                        }
                    }
                    Err(e) => {
                        eprintln!("nino: recording failed: {e}");
                        self.notify("Falha na gravação", &e);
                    }
                }
                self.finish_session();
            }
            Msg::Note(id, text) => {
                if let Some(session) = self.session.as_mut() {
                    session.editing = false;
                    session.set_text(id, text);
                }
                self.finish_session();
            }
            Msg::Fatal(e) => self.fatal = Some(e),
        }
    }

    fn editing(&self) -> bool {
        self.session.as_ref().is_some_and(|s| s.editing)
    }

    fn open_note(&mut self) {
        if self.recording.is_none() || self.overlay.is_some() || self.editing() {
            return;
        }
        let Some(session) = self.session.as_mut() else { return };
        let (id, title) = session.add_note(session.elapsed());
        self.open_editor(id, &title, "cancela");
    }

    fn open_editor(&mut self, id: usize, title: &str, esc: &str) {
        let Some(session) = self.session.as_mut() else { return };
        let child = std::env::current_exe().and_then(|exe| {
            Command::new(exe)
                .args(["editor", title, esc])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .spawn()
        });
        let mut child = match child {
            Ok(child) => child,
            Err(e) => {
                eprintln!("nino: editor: {e}");
                self.notify("Falha ao abrir a anotação", &e.to_string());
                return;
            }
        };
        session.editing = true;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let mut text = String::new();
            if let Some(mut out) = child.stdout.take() {
                let _ = out.read_to_string(&mut text);
            }
            let _ = child.wait();
            let _ = tx.send(Msg::Note(id, text.trim().to_string()));
        });
    }

    fn finish_session(&mut self) {
        if self.recording.is_some() || self.editing() {
            return;
        }
        let Some(session) = self.session.take() else { return };
        let finished = match session.finish() {
            Ok(finished) => finished,
            Err(e) => {
                eprintln!("nino: session: {e}");
                self.notify("Falha ao salvar a sessão", &e);
                return;
            }
        };
        let uris: Vec<String> = finished
            .files
            .iter()
            .map(|p| format!("file://{}", encode_path(p)))
            .collect();
        if uris.is_empty() {
            return;
        }
        let mut entries = vec![
            ("text/uri-list", format!("{}\r\n", uris.join("\r\n")).into_bytes()),
            (
                "x-special/gnome-copied-files",
                format!("copy\n{}", uris.join("\n")).into_bytes(),
            ),
        ];
        if let Some(notes) = finished.notes {
            entries.push(("text/plain;charset=utf-8", notes.clone().into_bytes()));
            entries.push(("text/plain", notes.into_bytes()));
        }
        self.set_clipboard(Payload { entries });
        eprintln!("nino: copied {} files", uris.len());
    }

    fn open_shot(&mut self) {
        self.shot_at = self.session.as_ref().map(Session::elapsed);
        let started = Instant::now();
        let (layer, viewport) = self.new_layer();
        if let Err(e) = self.wayland.flush() {
            eprintln!("nino: wayland flush: {e}");
        }
        match capture::capture_active_screen(&self.dbus) {
            Ok(shot) => {
                self.opened_at = Some((started, started.elapsed()));
                self.overlay = Some(Overlay::new(layer, viewport, Mode::Shot { shot }));
            }
            Err(e) => {
                viewport.destroy();
                drop(layer);
                eprintln!("nino: capture failed: {e}");
                self.notify("Falha no print", &e);
            }
        }
    }

    fn open_record(&mut self) {
        let started = Instant::now();
        let (layer, viewport) = self.new_layer();
        self.opened_at = Some((started, Duration::ZERO));
        self.overlay = Some(Overlay::new(layer, viewport, Mode::Record));
    }

    fn new_layer(&self) -> (LayerSurface, WpViewport) {
        let qh = &self.qh;
        let surface = self.compositor.create_surface(qh);
        let viewport = self.viewporter.get_viewport(&surface, qh, Ud);
        let layer = self
            .layer_shell
            .create_layer_surface(qh, surface, Layer::Overlay, Some(NAMESPACE), None);
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        layer.commit();
        (layer, viewport)
    }

    fn close_overlay(&mut self) {
        self.pending_record = None;
        if let Some(ov) = self.overlay.take() {
            ov.viewport.destroy();
            drop(ov.layer);
        }
    }

    fn close_ring(&mut self) {
        if let Some(ring) = self.ring.take() {
            ring.viewport.destroy();
            drop(ring.layer);
        }
    }

    fn redraw(&mut self, qh: &QueueHandle<Self>) {
        let Some(ov) = self.overlay.as_mut() else { return };
        if ov.size.is_none() {
            return;
        }
        if ov.frame_pending {
            ov.dirty = true;
            return;
        }
        let render_started = Instant::now();
        if let Err(e) = ov.present(&mut self.pool) {
            eprintln!("nino: present: {e}");
            return;
        }
        let surface = ov.layer.wl_surface();
        surface.frame(qh, FrameCallbackData(surface.clone()));
        ov.layer.commit();
        ov.dirty = false;
        ov.frame_pending = true;
        if let Some((started, capture)) = self.opened_at.take() {
            let mode = if ov.is_shot() { "shot" } else { "record" };
            eprintln!(
                "nino: {mode} overlay shown in {:?} (capture {:?}, configure wait {:?}, render {:?})",
                started.elapsed(),
                capture,
                render_started.duration_since(started).saturating_sub(capture),
                render_started.elapsed()
            );
        }
    }

    fn apply(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Nothing => {}
            Outcome::Redraw => {
                let qh = self.qh.clone();
                self.redraw(&qh);
            }
            Outcome::Close => self.close_overlay(),
            Outcome::Copy => self.copy_shot(),
            Outcome::Record(sel) => self.start_recording(sel),
        }
    }

    fn copy_shot(&mut self) {
        let started = Instant::now();
        let Some(image) = self.overlay.as_ref().and_then(Overlay::export) else {
            return;
        };
        self.close_overlay();
        let png = match draw::encode_png(&image) {
            Ok(png) => png,
            Err(e) => {
                eprintln!("nino: png: {e}");
                return;
            }
        };
        if let Some(session) = self.session.as_mut() {
            let at = self.shot_at.take().unwrap_or_else(|| session.elapsed());
            match session.add_print(at, &png) {
                Ok((id, title)) => self.open_editor(id, &title, "pula"),
                Err(e) => {
                    eprintln!("nino: print: {e}");
                    self.notify("Falha ao salvar o print", &e);
                }
            }
            return;
        }
        let size = png.len();
        self.set_clipboard(Payload {
            entries: vec![("image/png", png)],
        });
        eprintln!(
            "nino: copied {}x{} png ({} KiB) in {:?}",
            image.width(),
            image.height(),
            size / 1024,
            started.elapsed()
        );
    }

    fn set_clipboard(&mut self, payload: Payload) {
        let qh = self.qh.clone();
        match self.clipboard.as_mut().map(|c| c.set(payload, &qh)) {
            Some(Ok(())) => {}
            Some(Err(e)) => eprintln!("nino: clipboard: {e}"),
            None => eprintln!("nino: clipboard unavailable"),
        }
    }

    fn start_recording(&mut self, sel: Sel) {
        let Some(ov) = self.overlay.as_mut() else { return };
        let output = ov.output.clone().or_else(|| self.output_state.outputs().next());
        let info = output.as_ref().and_then(|o| self.output_state.info(o));
        let pos = info.as_ref().and_then(|i| i.logical_position).unwrap_or((0, 0));
        let screen = (ov.logical.0 as i32, ov.logical.1 as i32);

        let x0 = ((sel.x / ov.scale).floor() as i32).clamp(0, screen.0) & !1;
        let y0 = ((sel.y / ov.scale).floor() as i32).clamp(0, screen.1) & !1;
        let x1 = ((sel.x + sel.w) / ov.scale).ceil() as i32;
        let y1 = ((sel.y + sel.h) / ov.scale).ceil() as i32;
        let w = align_up(x1 - x0, 4).min((screen.0 - x0) & !3);
        let h = align_up(y1 - y0, 2).min((screen.1 - y0) & !1);
        if w < 4 || h < 2 {
            self.close_overlay();
            return;
        }

        ov.arm(Sel {
            x: x0 as f32 * ov.scale,
            y: y0 as f32 * ov.scale,
            w: w as f32 * ov.scale,
            h: h as f32 * ov.scale,
        });
        let region = self.compositor.wl_compositor().create_region(&self.qh, Ud);
        ov.layer.wl_surface().set_input_region(Some(&region));
        region.destroy();
        ov.layer.set_keyboard_interactivity(KeyboardInteractivity::None);

        self.pending_record = Some((pos.0 + x0, pos.1 + y0, w as u32, h as u32));
        let qh = self.qh.clone();
        self.redraw(&qh);
    }

    fn begin_recording(&mut self, (x, y, w, h): (i32, i32, u32, u32)) {
        let started = Session::new().and_then(|session| {
            let rec = Recording::start(session.video_path(), x, y, w, h, self.tx.clone())
                .inspect_err(|_| _ = std::fs::remove_dir(&session.dir))?;
            Ok((session, rec))
        });
        match started {
            Ok((session, rec)) => {
                eprintln!("nino: recording {w}x{h}+{x}+{y} -> {}", rec.path.display());
                self.session = Some(session);
                self.recording = Some(rec);
                self.ring = self.overlay.take();
            }
            Err(e) => {
                eprintln!("nino: {e}");
                self.close_overlay();
                self.notify("Falha na gravação", &e);
            }
        }
    }

    fn notify(&self, summary: &str, body: &str) {
        let hints = std::collections::HashMap::from([("desktop-entry", zbus::Variant::from("nino"))]);
        let _ = zbus::block_on(self.dbus.call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &("Nino", 0u32, "nino", summary, body, Vec::<&str>::new(), hints, 4000i32),
        ));
    }
}

fn encode_path(path: &std::path::Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::new();
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

impl CompositorHandler for App {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(&mut self, _: &Connection, qh: &QueueHandle<Self>, surface: &wl_surface::WlSurface, _: u32) {
        let Some(ov) = self.overlay.as_mut() else { return };
        if ov.layer.wl_surface() != surface {
            return;
        }
        ov.frame_pending = false;
        if ov.dirty {
            self.redraw(qh);
        } else if let Some(region) = self.pending_record.take() {
            self.begin_recording(region);
        }
    }

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        output: &wl_output::WlOutput,
    ) {
        if let Some(ov) = self.overlay.as_mut().filter(|o| o.layer.wl_surface() == surface) {
            ov.output = Some(output.clone());
        }
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        if self.overlay.as_ref().is_some_and(|o| &o.layer == layer) {
            self.close_overlay();
        }
        if self.ring.as_ref().is_some_and(|o| &o.layer == layer) {
            self.close_ring();
        }
    }

    fn configure(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        cfg: LayerSurfaceConfigure,
        _: u32,
    ) {
        let Some(ov) = self.overlay.as_mut().filter(|o| &o.layer == layer) else {
            return;
        };
        let (w, h) = cfg.new_size;
        if w == 0 || h == 0 {
            return;
        }
        ov.configure(w, h);
        ov.frame_pending = false;
        self.redraw(qh);
    }
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        if let Some(clip) = self.clipboard.as_mut() {
            clip.set_seat(&seat, qh);
        }
    }

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if let Some(clip) = self.clipboard.as_mut() {
            clip.set_seat(&seat, qh);
        }
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            self.keyboard = self.seat_state.get_keyboard(qh, &seat, None).ok();
        }
        if capability == Capability::Pointer
            && self.pointer.is_none()
            && let Ok(pointer) = self.seat_state.get_pointer(qh, &seat)
        {
            self.cursor_device = self.cursor_shape.as_ref().map(|m| m.get_shape_device(&pointer, qh));
            self.pointer = Some(pointer);
        }
    }

    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Keyboard
            && let Some(k) = self.keyboard.take()
        {
            k.release();
        }
        if capability == Capability::Pointer {
            if let Some(d) = self.cursor_device.take() {
                d.destroy();
            }
            if let Some(p) = self.pointer.take() {
                p.release();
            }
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl KeyboardHandler for App {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
    }
    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
    ) {
    }

    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        let Some(ov) = self.overlay.as_mut() else { return };
        let ctrl = self.modifiers.ctrl;
        let outcome = match event.keysym {
            Keysym::Escape => Outcome::Close,
            Keysym::c | Keysym::C if ctrl && ov.is_shot() => Outcome::Copy,
            Keysym::Return | Keysym::KP_Enter if ov.is_shot() => Outcome::Copy,
            Keysym::z | Keysym::Z if ctrl => ov.undo(),
            Keysym::a | Keysym::A if !ctrl => ov.set_tool(Tool::Draw),
            Keysym::d | Keysym::D if !ctrl => ov.set_tool(Tool::Arrow),
            Keysym::q | Keysym::Q if !ctrl => ov.set_tool(Tool::Rect),
            Keysym::e | Keysym::E if !ctrl => ov.set_tool(Tool::Ellipse),
            k if !ctrl && ov.is_shot() && (Keysym::_1.raw()..=Keysym::_9.raw()).contains(&k.raw()) => {
                ov.number((k.raw() - Keysym::_0.raw()) as usize)
            }
            Keysym::c | Keysym::C if !ctrl && ov.is_shot() => ov.next_color(),
            _ => Outcome::Nothing,
        };
        self.apply(outcome);
    }

    fn repeat_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {}
    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {
    }

    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        modifiers: Modifiers,
        _: RawModifiers,
        _: u32,
    ) {
        self.modifiers = modifiers;
    }
}

impl PointerHandler for App {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            let Some(ov) = self.overlay.as_mut().filter(|o| o.layer.wl_surface() == &event.surface) else {
                continue;
            };
            let p = ov.to_buffer(event.position);
            let outcome = match event.kind {
                PointerEventKind::Enter { serial } => {
                    self.pointer_serial = serial;
                    self.cursor = None;
                    Outcome::Nothing
                }
                PointerEventKind::Motion { .. } => ov.motion(p, self.modifiers.shift),
                PointerEventKind::Press { button: BTN_LEFT, .. } => ov.press(p),
                PointerEventKind::Release { button: BTN_LEFT, .. } => ov.release(),
                _ => Outcome::Nothing,
            };
            let shape = ov.cursor_at(p);
            if self.cursor != Some(shape) {
                if let Some(dev) = &self.cursor_device {
                    dev.set_shape(self.pointer_serial, shape);
                }
                self.cursor = Some(shape);
            }
            self.apply(outcome);
        }
    }
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl Dispatch2<WpViewporter, App> for Ud {
    fn event(
        &self,
        _: &mut App,
        _: &WpViewporter,
        _: <WpViewporter as Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
    }
}

impl Dispatch2<WpViewport, App> for Ud {
    fn event(
        &self,
        _: &mut App,
        _: &WpViewport,
        _: <WpViewport as Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
    }
}

impl Dispatch2<wl_region::WlRegion, App> for Ud {
    fn event(
        &self,
        _: &mut App,
        _: &wl_region::WlRegion,
        _: <wl_region::WlRegion as Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
    }
}

delegate_registry!(App);

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

smithay_client_toolkit::delegate_dispatch2!(App);

fn align_up(v: i32, to: i32) -> i32 {
    (v + to - 1) / to * to
}
