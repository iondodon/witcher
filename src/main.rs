use anyhow::{Context, Result};
use evdev::{Device, InputEventKind, Key};
use freedesktop_icons::lookup;
use image::{imageops::FilterType, DynamicImage};
use niri_ipc::{socket::Socket, Action, Request, Response};
use resvg::usvg;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_registry,
    delegate_seat, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc,
    sync::Arc,
    sync::atomic::{AtomicBool, Ordering},
    thread,
};
use tiny_skia::{Color, IntSize, Paint, PathBuilder, Pixmap, PixmapMut, PixmapPaint, Transform};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_seat, wl_shm, wl_surface},
    Connection, QueueHandle,
};

const ICON_SIZE: u32 = 64;
const ICON_SPACING: u32 = 18;
const PANEL_PADDING: u32 = 22;
const HIGHLIGHT_PADDING: u32 = 10;
const CORNER_RADIUS: f32 = 16.0;

#[derive(Clone, Copy, Debug)]
enum BackendKind {
    Niri,
    Sway,
    Hyprland,
    Kwin,
    Gnome,
}

fn parse_backend_required(args: &[String]) -> Result<BackendKind> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--backend" {
            if let Some(value) = iter.next() {
                return match value.as_str() {
                    "niri" => Ok(BackendKind::Niri),
                    "sway" => Ok(BackendKind::Sway),
                    "hyprland" => Ok(BackendKind::Hyprland),
                    "kwin" => Ok(BackendKind::Kwin),
                    "gnome" => Ok(BackendKind::Gnome),
                    _ => Err(anyhow::anyhow!("unknown backend: {value}")),
                };
            }
        }
    }
    Err(anyhow::anyhow!("missing --backend (niri|hyprland)"))
}

fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--daemon") {
        let backend = parse_backend_required(&args)?;
        run_daemon(backend)?;
        return Ok(());
    }

    eprintln!(
        "Usage: witcher --daemon --backend <name>\nSupported backends: niri, hyprland\nNote: evdev requires /dev/input access; add your user to the input group and re-login:\n  sudo usermod -aG input $USER"
    );
    Ok(())
}

fn run_daemon(backend: BackendKind) -> Result<()> {
    let socket_path = runtime_socket_path("witcher.sock")?;
    let _listener = match bind_listener(&socket_path) {
        Ok(listener) => listener,
        Err(err) => return Err(err),
    };

    let (tx, rx) = mpsc::channel::<DaemonMsg>();
    let listener = _listener.try_clone().context("clone listener")?;
    let tx_listener = tx.clone();
    thread::spawn(move || {
        loop {
            let Ok((mut stream, _)) = listener.accept() else {
                continue;
            };
            let mut buf = [0u8; 32];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(b"ok");
            let _ = tx_listener.send(DaemonMsg::Show);
        }
    });

    let active = Arc::new(AtomicBool::new(false));
    spawn_evdev_listener(tx.clone(), active.clone());

    let mut icon_cache = IconCache::default();
    let mut mru = MruState::default();
    loop {
        let Ok(msg) = rx.recv() else {
            continue;
        };
        if matches!(msg, DaemonMsg::Show) {
            while rx.try_recv().is_ok() {}
            active.store(true, Ordering::SeqCst);
            match run_switcher(backend, &mut icon_cache, &mut mru) {
                Ok(Some(id)) => mru.update_on_focus(id),
                Ok(None) => {}
            Err(err) => eprintln!("witcher: switcher error: {err:#}"),
            }
            active.store(false, Ordering::SeqCst);
            while rx.try_recv().is_ok() {}
        }
    }
}

fn runtime_socket_path(name: &str) -> Result<PathBuf> {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    Ok(runtime_dir.join(name))
}

fn bind_listener(path: &PathBuf) -> Result<UnixListener> {
    if let Ok(mut stream) = UnixStream::connect(path) {
        let mut buf = [0u8; 8];
        let _ = stream.write_all(b"ping");
        let _ = stream.read(&mut buf);
        return Err(anyhow::anyhow!("witcher daemon already running"));
    }
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path)
        .with_context(|| format!("bind {}", path.display()))?;
    Ok(listener)
}

#[derive(Clone, Copy)]
enum DaemonMsg {
    Show,
}

fn spawn_evdev_listener(tx: mpsc::Sender<DaemonMsg>, active: Arc<AtomicBool>) {
    let devices = match enumerate_keyboards() {
        Ok(devices) => devices,
        Err(err) => {
            eprintln!("witcher: evdev enumerate error: {err:#}");
            return;
        }
    };
    if devices.is_empty() {
        eprintln!("witcher: no keyboard devices with Alt+Tab found");
        return;
    }
    for mut device in devices {
        let tx = tx.clone();
        let active = active.clone();
        thread::spawn(move || {
            let mut alt_down = false;
            loop {
                let events = match device.fetch_events() {
                    Ok(events) => events,
                    Err(_) => continue,
                };
                for ev in events {
                    if let InputEventKind::Key(key) = ev.kind() {
                        let value = ev.value();
                        match key {
                            Key::KEY_LEFTALT | Key::KEY_RIGHTALT => {
                                alt_down = value != 0;
                            }
                            Key::KEY_TAB => {
                                if alt_down && value != 0 && !active.load(Ordering::SeqCst) {
                                    let _ = tx.send(DaemonMsg::Show);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        });
    }
}

fn enumerate_keyboards() -> Result<Vec<Device>> {
    let mut devices = Vec::new();
    for (_path, device) in evdev::enumerate() {
        if let Some(keys) = device.supported_keys() {
            let has_alt = keys.contains(Key::KEY_LEFTALT) || keys.contains(Key::KEY_RIGHTALT);
            let has_tab = keys.contains(Key::KEY_TAB);
            if has_alt && has_tab {
                devices.push(device);
            }
        }
    }
    Ok(devices)
}

fn run_switcher(
    backend: BackendKind,
    icon_cache: &mut IconCache,
    mru: &mut MruState,
) -> Result<Option<u64>> {
    let mut windows = load_windows(backend, icon_cache).context("load windows via backend")?;
    if windows.is_empty() {
        return Ok(None);
    }

    let focused_id = windows.iter().find(|w| w.is_focused).map(|w| w.id);
    if let Some(id) = focused_id {
        mru.update_on_focus(id);
    }
    windows = mru.order_windows(windows);

    let selected = if windows.len() > 1 { 1 } else { 0 };
    let icon_size = ICON_SIZE;
    let (desired_width, desired_height) = layout_size(windows.len(), icon_size);
    let (initial_output_size, initial_scale) =
        focused_output_info(backend).unwrap_or((None, 1));

    let conn = Connection::connect_to_env().context("connect to Wayland")?;
    let (globals, mut event_queue) =
        registry_queue_init::<Switcher>(&conn).context("init registry")?;
    let qh = event_queue.handle();

    let compositor =
        CompositorState::bind(&globals, &qh).context("wl_compositor not available")?;
    let layer_shell = LayerShell::bind(&globals, &qh).context("layer shell not available")?;
    let shm = Shm::bind(&globals, &qh).context("wl_shm not available")?;

    let surface = compositor.create_surface(&qh);
    let layer = layer_shell.create_layer_surface(&qh, surface, Layer::Overlay, Some("witcher"), None);
    layer.set_anchor(Anchor::TOP | Anchor::LEFT);
    layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
    layer.set_exclusive_zone(-1);
    layer.set_size(desired_width, desired_height);
    if initial_scale > 1 {
        layer.wl_surface().set_buffer_scale(initial_scale as i32);
    }
    layer.commit();

    let pool = SlotPool::new((desired_width * desired_height * 4) as usize, &shm)
        .context("create shm pool")?;

    let mut app = Switcher {
        backend,
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        layer,
        pool,
        width: desired_width,
        height: desired_height,
        buffer_scale: initial_scale,
        output_logical_size: initial_output_size,
        first_configure: true,
        exit: false,
        keyboard: None,
        modifiers: Modifiers::default(),
        windows,
        selected,
        redraw: true,
        finalized: false,
    };

    loop {
        if app.exit {
            break;
        }
        event_queue
            .blocking_dispatch(&mut app)
            .context("dispatch events")?;
    }

    Ok(app.windows.get(app.selected).map(|w| w.id))
}

#[derive(Clone)]
struct WindowEntry {
    id: u64,
    is_focused: bool,
    icon: Arc<Pixmap>,
}

struct BackendWindow {
    id: u64,
    app_id: Option<String>,
    is_focused: bool,
}

struct Switcher {
    backend: BackendKind,
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    layer: LayerSurface,
    pool: SlotPool,
    width: u32,
    height: u32,
    buffer_scale: u32,
    output_logical_size: Option<(i32, i32)>,
    first_configure: bool,
    exit: bool,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    modifiers: Modifiers,
    windows: Vec<WindowEntry>,
    selected: usize,
    redraw: bool,
    finalized: bool,
}

impl Switcher {
    fn draw(&mut self, qh: &QueueHandle<Self>) {
        let buffer_width = self.width * self.buffer_scale;
        let buffer_height = self.height * self.buffer_scale;
        let stride = buffer_width as i32 * 4;

        let needed = (buffer_width * buffer_height * 4) as usize;
        if self.pool.len() < needed {
            self.pool.resize(needed).expect("resize shm pool");
        }

        let (buffer, canvas) = self
            .pool
            .create_buffer(
                buffer_width as i32,
                buffer_height as i32,
                stride,
                wl_shm::Format::Argb8888,
            )
            .expect("create buffer");

        {
            let mut pixmap =
                PixmapMut::from_bytes(canvas.as_mut(), buffer_width, buffer_height)
                    .expect("pixmap from buffer");
            pixmap.fill(Color::from_rgba8(0, 0, 0, 0));

            let transform = Transform::from_scale(self.buffer_scale as f32, self.buffer_scale as f32);
            let background = rounded_rect_path(
                0.0,
                0.0,
                self.width as f32,
                self.height as f32,
                CORNER_RADIUS,
            );
            let mut paint = Paint::default();
            paint.set_color(Color::from_rgba8(20, 20, 20, 220));
            pixmap.fill_path(&background, &paint, tiny_skia::FillRule::Winding, transform, None);

            let item_size = ICON_SIZE + HIGHLIGHT_PADDING * 2;
            let y = self.height as i32 / 2 - (ICON_SIZE / 2) as i32;
            for (idx, window) in self.windows.iter().enumerate() {
                let x = PANEL_PADDING as i32 + idx as i32 * (item_size + ICON_SPACING) as i32;
                if idx == self.selected {
                    let highlight = rounded_rect_path(
                        (x - HIGHLIGHT_PADDING as i32) as f32,
                        (y - HIGHLIGHT_PADDING as i32) as f32,
                        item_size as f32,
                        item_size as f32,
                        CORNER_RADIUS * 0.7,
                    );
                    let mut paint = Paint::default();
                    paint.set_color(Color::from_rgba8(255, 255, 255, 28));
                    pixmap.fill_path(
                        &highlight,
                        &paint,
                        tiny_skia::FillRule::Winding,
                        transform,
                        None,
                    );
                }

                let icon_x = x as i32;
                let icon_y = y as i32;
                let paint = PixmapPaint::default();
                pixmap.draw_pixmap(
                    icon_x,
                    icon_y,
                    window.icon.as_ref().as_ref(),
                    &paint,
                    transform,
                    None,
                );
            }
        }

        swizzle_rgba_to_bgra(canvas.as_mut());

        self.layer
            .wl_surface()
            .damage_buffer(0, 0, buffer_width as i32, buffer_height as i32);
        self.layer
            .wl_surface()
            .frame(qh, self.layer.wl_surface().clone());
        buffer.attach_to(self.layer.wl_surface()).expect("buffer attach");
        self.layer.commit();
        self.redraw = false;
    }

    fn cycle(&mut self, delta: i32, qh: &QueueHandle<Self>) {
        if self.windows.is_empty() {
            return;
        }
        let len = self.windows.len() as i32;
        let next = (self.selected as i32 + delta).rem_euclid(len) as usize;
        if next != self.selected {
            self.selected = next;
            self.redraw = true;
            self.draw(qh);
        }
    }

    fn finalize(&mut self) {
        if self.finalized {
            return;
        }
        self.finalized = true;
        if let Some(window) = self.windows.get(self.selected) {
            let _ = focus_window(self.backend, window.id);
        }
        self.exit = true;
    }

    fn apply_layout(&mut self) {
        if let Some((output_w, output_h)) = self.output_logical_size {
            let left = ((output_w - self.width as i32) / 2).max(0);
            let top = ((output_h - self.height as i32) / 2).max(0);
            self.layer.set_margin(top, 0, 0, left);
        }
    }
}

impl CompositorHandler for Switcher {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        if self.redraw {
            self.draw(qh);
        }
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for Switcher {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _output: wl_output::WlOutput) {}

    fn update_output(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        if let Some(info) = self.output_state.info(&output) {
            if let Some(size) = info.logical_size {
                self.output_logical_size = Some(size);
                let scale = info.scale_factor.max(1) as u32;
                if scale != self.buffer_scale {
                    self.buffer_scale = scale;
                    self.layer.wl_surface().set_buffer_scale(scale as i32);
                    self.redraw = true;
                }
                self.apply_layout();
                self.layer.commit();
            }
        }
    }

    fn output_destroyed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _output: wl_output::WlOutput) {}
}

impl LayerShellHandler for Switcher {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        self.exit = true;
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        if configure.new_size.0 != 0 && configure.new_size.1 != 0 {
            self.width = configure.new_size.0;
            self.height = configure.new_size.1;
        }

        self.apply_layout();

        if self.first_configure {
            self.first_configure = false;
            self.redraw = true;
            self.draw(qh);
        }
    }
}

impl SeatHandler for Switcher {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            let keyboard = self
                .seat_state
                .get_keyboard(qh, &seat, None)
                .expect("create keyboard");
            self.keyboard = Some(keyboard);
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard {
            if let Some(keyboard) = self.keyboard.take() {
                keyboard.release();
            }
        }
    }

    fn remove_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {}
}

impl KeyboardHandler for Switcher {
    fn enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _surface: &wl_surface::WlSurface,
        _serial: u32,
        _raw: &[u32],
        _keysyms: &[Keysym],
    ) {
    }

    fn leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _surface: &wl_surface::WlSurface,
        _serial: u32,
    ) {
    }

    fn press_key(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        match event.keysym {
            Keysym::Tab => {
                self.cycle(1, qh);
            }
            Keysym::ISO_Left_Tab => {
                self.cycle(-1, qh);
            }
            Keysym::Escape => {
                self.exit = true;
            }
            Keysym::Return | Keysym::KP_Enter => {
                self.finalize();
            }
            _ => {}
        }
    }

    fn release_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        if matches!(event.keysym, Keysym::Alt_L | Keysym::Alt_R) {
            self.finalize();
        }
    }

    fn update_modifiers(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        modifiers: Modifiers,
        _layout: u32,
    ) {
        let was_alt = self.modifiers.alt;
        self.modifiers = modifiers;
        if was_alt && !modifiers.alt {
            self.finalize();
        }
    }
}

impl ShmHandler for Switcher {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

#[derive(Default)]
struct MruState {
    order: Vec<u64>,
}

impl MruState {
    fn update_on_focus(&mut self, id: u64) {
        self.order.retain(|&existing| existing != id);
        self.order.insert(0, id);
        if self.order.len() > 256 {
            self.order.truncate(256);
        }
    }

    fn order_windows(&self, windows: Vec<WindowEntry>) -> Vec<WindowEntry> {
        let focused = windows.iter().find(|w| w.is_focused).map(|w| w.id);
        let mut order_index = std::collections::HashMap::new();
        for (idx, id) in self.order.iter().enumerate() {
            order_index.insert(*id, idx);
        }

        let mut ranked = Vec::with_capacity(windows.len());
        for (idx, window) in windows.into_iter().enumerate() {
            let rank = if Some(window.id) == focused {
                0usize
            } else if let Some(mru_idx) = order_index.get(&window.id) {
                1 + *mru_idx
            } else {
                1 + order_index.len() + idx
            };
            ranked.push((rank, idx, window));
        }

        ranked.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
        ranked.into_iter().map(|(_, _, window)| window).collect()
    }
}

delegate_compositor!(Switcher);
delegate_output!(Switcher);
delegate_shm!(Switcher);
delegate_seat!(Switcher);
delegate_keyboard!(Switcher);
delegate_layer!(Switcher);
delegate_registry!(Switcher);

impl ProvidesRegistryState for Switcher {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState, SeatState];
}

fn layout_size(count: usize, icon_size: u32) -> (u32, u32) {
    if count == 0 {
        return (0, 0);
    }
    let width = PANEL_PADDING * 2 + count as u32 * icon_size + (count as u32 - 1) * ICON_SPACING;
    let height = PANEL_PADDING * 2 + icon_size;
    (width, height)
}

fn load_windows(backend: BackendKind, icon_cache: &mut IconCache) -> Result<Vec<WindowEntry>> {
    let windows = backend_windows(backend)?;
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for window in windows {
        let app_id = window
            .app_id
            .unwrap_or_else(|| "application-x-executable".to_string());
        if !seen.insert(window.id) {
            continue;
        }
        let icon = icon_cache.icon_for(&app_id);
        entries.push(WindowEntry {
            id: window.id,
            is_focused: window.is_focused,
            icon,
        });
    }
    Ok(entries)
}

#[derive(Default)]
struct IconCache {
    icons: std::collections::HashMap<String, Arc<Pixmap>>,
}

impl IconCache {
    fn icon_for(&mut self, app_id: &str) -> Arc<Pixmap> {
        if let Some(icon) = self.icons.get(app_id) {
            return icon.clone();
        }
        let icon = load_icon(app_id).unwrap_or_else(|_| placeholder_icon(ICON_SIZE));
        let icon = Arc::new(icon);
        self.icons.insert(app_id.to_string(), icon.clone());
        icon
    }
}

fn load_icon(app_id: &str) -> Result<Pixmap> {
    let icon_size = ICON_SIZE;
    let mut candidates = Vec::new();
    candidates.push(app_id.to_string());
    if let Some(trimmed) = app_id.strip_suffix(".desktop") {
        candidates.push(trimmed.to_string());
    }
    if let Some(last) = app_id.rsplit('.').next() {
        candidates.push(last.to_string());
    }

    if let Some(icon_name) = desktop_icon_name(app_id) {
        candidates.push(icon_name);
    }

    let path = candidates
        .into_iter()
        .find_map(|name| lookup(&name).with_size(icon_size as u16).find())
        .or_else(|| lookup("application-x-executable").with_size(icon_size as u16).find())
        .context("no icon found")?;

    if path.extension().and_then(|ext| ext.to_str()) == Some("svg") {
        return render_svg(&path, icon_size);
    }

    let image = image::open(&path).with_context(|| format!("open icon {}", path.display()))?;
    let resized = image.resize_exact(icon_size, icon_size, FilterType::Lanczos3);
    Ok(pixmap_from_image(resized))
}

fn pixmap_from_image(image: DynamicImage) -> Pixmap {
    let rgba = image.to_rgba8();
    let size = IntSize::from_wh(rgba.width(), rgba.height()).expect("icon size");
    Pixmap::from_vec(rgba.into_raw(), size).expect("pixmap from image")
}

fn placeholder_icon(size: u32) -> Pixmap {
    let mut pixmap = Pixmap::new(size, size).expect("placeholder pixmap");
    let mut paint = Paint::default();
    paint.set_color(Color::from_rgba8(90, 90, 90, 255));
    pixmap.fill_rect(
        tiny_skia::Rect::from_xywh(0.0, 0.0, size as f32, size as f32).unwrap(),
        &paint,
        Transform::identity(),
        None,
    );
    pixmap
}

fn render_svg(path: &Path, size: u32) -> Result<Pixmap> {
    let data = fs::read(path).with_context(|| format!("read svg {}", path.display()))?;
    let options = usvg::Options::default();
    let tree = usvg::Tree::from_data(&data, &options)
        .with_context(|| format!("parse svg {}", path.display()))?;
    let mut pixmap = Pixmap::new(size, size).context("create svg pixmap")?;
    let tree_size = tree.size();
    let scale_x = size as f32 / tree_size.width();
    let scale_y = size as f32 / tree_size.height();
    let scale = scale_x.min(scale_y);
    let scaled_w = tree_size.width() * scale;
    let scaled_h = tree_size.height() * scale;
    let dx = (size as f32 - scaled_w) * 0.5;
    let dy = (size as f32 - scaled_h) * 0.5;
    let transform = tiny_skia::Transform::from_scale(scale, scale)
        .post_translate(dx, dy);
    let mut pixmap_mut = pixmap.as_mut();
    resvg::render(&tree, transform, &mut pixmap_mut);
    Ok(pixmap)
}

fn desktop_icon_name(app_id: &str) -> Option<String> {
    let mut candidates = Vec::new();
    candidates.push(app_id.to_string());
    if let Some(trimmed) = app_id.strip_suffix(".desktop") {
        candidates.push(trimmed.to_string());
    }
    if let Some(last) = app_id.rsplit('.').next() {
        candidates.push(last.to_string());
    }

    let mut paths = Vec::new();
    paths.push(PathBuf::from("/usr/share/applications"));
    paths.push(PathBuf::from("/usr/local/share/applications"));
    if let Some(home) = std::env::var_os("HOME") {
        paths.push(PathBuf::from(home).join(".local/share/applications"));
    }
    if let Ok(xdg_dirs) = std::env::var("XDG_DATA_DIRS") {
        for dir in xdg_dirs.split(':') {
            if !dir.is_empty() {
                paths.push(PathBuf::from(dir).join("applications"));
            }
        }
    }

    for base in paths {
        for name in &candidates {
            let file = if name.ends_with(".desktop") {
                base.join(name)
            } else {
                base.join(format!("{name}.desktop"))
            };
            if let Ok(icon) = read_desktop_icon(&file) {
                if let Some(icon) = icon {
                    return Some(icon);
                }
            }
        }
    }
    None
}

fn read_desktop_icon(path: &Path) -> Result<Option<String>> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(_) => return Ok(None),
    };
    let mut in_entry = false;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        if let Some(value) = line.strip_prefix("Icon=") {
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            return Ok(Some(value.to_string()));
        }
    }
    Ok(None)
}

fn swizzle_rgba_to_bgra(bytes: &mut [u8]) {
    for pixel in bytes.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
}

#[derive(serde::Deserialize)]
struct HyprClient {
    address: Option<String>,
    class: Option<String>,
    #[serde(rename = "initialClass")]
    initial_class: Option<String>,
    focus: Option<bool>,
    mapped: Option<bool>,
    hidden: Option<bool>,
}

#[derive(serde::Deserialize)]
struct HyprMonitor {
    focused: Option<bool>,
    width: Option<u32>,
    height: Option<u32>,
    scale: Option<f64>,
}

fn parse_hypr_address(value: &str) -> Option<u64> {
    let trimmed = value.trim().trim_start_matches("0x");
    u64::from_str_radix(trimmed, 16).ok()
}

fn hyprctl(args: &[&str]) -> Result<String> {
    let output = Command::new("hyprctl")
        .args(args)
        .output()
        .context("spawn hyprctl")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("hyprctl failed: {stderr}"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn hyprctl_json<T: for<'de> serde::Deserialize<'de>>(args: &[&str]) -> Result<T> {
    let text = hyprctl(args)?;
    let value = serde_json::from_str(&text).context("parse hyprctl json")?;
    Ok(value)
}

fn focus_window(backend: BackendKind, id: u64) -> Result<()> {
    match backend {
        BackendKind::Niri => {
            let socket = Socket::connect().context("connect to niri socket")?;
            let (reply, _events) = socket
                .send(Request::Action(Action::FocusWindow { id }))
                .context("send focus request")?;
            match reply {
                Ok(Response::Handled) => Ok(()),
                Ok(_) => Ok(()),
                Err(message) => Err(anyhow::anyhow!(message)),
            }
        }
        BackendKind::Hyprland => {
            let addr = format!("address:0x{id:x}");
            hyprctl(&["dispatch", "focuswindow", &addr])?;
            Ok(())
        }
        _ => Err(anyhow::anyhow!("backend not supported")),
    }
}

fn focused_output_info(backend: BackendKind) -> Result<(Option<(i32, i32)>, u32)> {
    match backend {
        BackendKind::Niri => {
            let socket = Socket::connect().context("connect to niri socket")?;
            let (reply, _events) =
                socket.send(Request::FocusedOutput).context("send focused output request")?;
            let output = match reply {
                Ok(Response::FocusedOutput(output)) => output,
                Ok(_) => None,
                Err(message) => return Err(anyhow::anyhow!(message)),
            };
            if let Some(output) = output {
                if let Some(logical) = output.logical {
                    let scale = logical.scale.max(1.0).round() as u32;
                    return Ok((Some((logical.width as i32, logical.height as i32)), scale.max(1)));
                }
            }
            Ok((None, 1))
        }
        BackendKind::Hyprland => {
            let output = hyprctl_json::<Vec<HyprMonitor>>(&["-j", "monitors"])?;
            if let Some(monitor) = output.into_iter().find(|m| m.focused.unwrap_or(false)) {
                let scale = monitor.scale.unwrap_or(1.0).max(1.0);
                return Ok((
                    Some((monitor.width.unwrap_or(0) as i32, monitor.height.unwrap_or(0) as i32)),
                    scale.round() as u32,
                ));
            }
            Ok((None, 1))
        }
        _ => Ok((None, 1)),
    }
}

fn backend_windows(backend: BackendKind) -> Result<Vec<BackendWindow>> {
    match backend {
        BackendKind::Niri => {
            let socket = Socket::connect().context("connect to niri socket")?;
            let (reply, _events) = socket.send(Request::Windows).context("send windows request")?;
            let windows = match reply {
                Ok(Response::Windows(windows)) => windows,
                Ok(_) => return Ok(Vec::new()),
                Err(message) => return Err(anyhow::anyhow!(message)),
            };
            Ok(windows
                .into_iter()
                .map(|window| BackendWindow {
                    id: window.id,
                    app_id: window.app_id,
                    is_focused: window.is_focused,
                })
                .collect())
        }
        BackendKind::Hyprland => {
            let clients = hyprctl_json::<Vec<HyprClient>>(&["-j", "clients"])?;
            let mut windows = Vec::new();
            for client in clients {
                if client.mapped == Some(false) || client.hidden == Some(true) {
                    continue;
                }
                let addr = match client.address.as_deref() {
                    Some(addr) => addr,
                    None => continue,
                };
                let id = match parse_hypr_address(addr) {
                    Some(id) => id,
                    None => continue,
                };
                let app_id = client
                    .initial_class
                    .clone()
                    .or(client.class.clone());
                windows.push(BackendWindow {
                    id,
                    app_id,
                    is_focused: client.focus.unwrap_or(false),
                });
            }
            Ok(windows)
        }
        _ => Err(anyhow::anyhow!("backend not supported")),
    }
}

fn rounded_rect_path(x: f32, y: f32, width: f32, height: f32, radius: f32) -> tiny_skia::Path {
    let r = radius.min(width / 2.0).min(height / 2.0);
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + width - r, y);
    pb.quad_to(x + width, y, x + width, y + r);
    pb.line_to(x + width, y + height - r);
    pb.quad_to(x + width, y + height, x + width - r, y + height);
    pb.line_to(x + r, y + height);
    pb.quad_to(x, y + height, x, y + height - r);
    pb.line_to(x, y + r);
    pb.quad_to(x, y, x + r, y);
    pb.close();
    pb.finish().expect("rounded rect path")
}
