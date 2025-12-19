use anyhow::{Context, Result};
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
use std::collections::HashSet;
use tiny_skia::{Color, Paint, PathBuilder, PixmapMut, PixmapPaint, Transform};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_seat, wl_shm, wl_surface},
    Connection, QueueHandle,
};

use crate::backend::{backend_windows, focus_window, focused_output_info};
use crate::config::{
    BORDER_WIDTH, CORNER_RADIUS, HIGHLIGHT_PADDING, ICON_SIZE, ICON_SPACING, PANEL_PADDING,
};
use crate::icon::IconCache;
use crate::mru::MruState;
use crate::types::{BackendKind, WindowEntry};

pub fn run_switcher(
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
    let (initial_output_size, initial_scale) = focused_output_info(backend).unwrap_or((None, 1));

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
            let outer = rounded_rect_path(
                0.0,
                0.0,
                self.width as f32,
                self.height as f32,
                CORNER_RADIUS,
            );
            let mut paint = Paint::default();
            paint.set_color(Color::from_rgba8(255, 255, 255, 36));
            pixmap.fill_path(&outer, &paint, tiny_skia::FillRule::Winding, transform, None);

            let inset = BORDER_WIDTH.max(0.0);
            let inner_width = (self.width as f32 - inset * 2.0).max(0.0);
            let inner_height = (self.height as f32 - inset * 2.0).max(0.0);
            let inner = rounded_rect_path(
                inset,
                inset,
                inner_width,
                inner_height,
                (CORNER_RADIUS - inset).max(0.0),
            );
            paint.set_color(Color::from_rgba8(20, 20, 20, 220));
            pixmap.fill_path(&inner, &paint, tiny_skia::FillRule::Winding, transform, None);

            let item_size = ICON_SIZE + HIGHLIGHT_PADDING * 2;
            let total_width = self.windows.len() as i32 * item_size as i32
                + (self.windows.len().saturating_sub(1) as i32 * ICON_SPACING as i32);
            let available = self.width as i32 - (PANEL_PADDING as i32 * 2);
            let start_x = (PANEL_PADDING as i32 + ((available - total_width) / 2)).max(0);
            let y = self.height as i32 / 2 - (ICON_SIZE / 2) as i32;
            for (idx, window) in self.windows.iter().enumerate() {
                let item_x = start_x + idx as i32 * (item_size + ICON_SPACING) as i32;
                let icon_x = item_x + HIGHLIGHT_PADDING as i32;
                if idx == self.selected {
                    let highlight = rounded_rect_path(
                        item_x as f32,
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
        _configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
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
    let item_size = icon_size + HIGHLIGHT_PADDING * 2;
    let width = PANEL_PADDING * 2 + count as u32 * item_size + (count as u32 - 1) * ICON_SPACING;
    let height = PANEL_PADDING * 2 + item_size;
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

fn swizzle_rgba_to_bgra(bytes: &mut [u8]) {
    for pixel in bytes.chunks_exact_mut(4) {
        pixel.swap(0, 2);
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
