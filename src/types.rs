use std::sync::Arc;
use tiny_skia::Pixmap;

#[derive(Clone, Copy, Debug)]
pub enum BackendKind {
    Niri,
    Sway,
    Hyprland,
    Kwin,
    Gnome,
}

#[derive(Clone)]
pub struct WindowEntry {
    pub id: u64,
    pub is_focused: bool,
    pub icon: Arc<Pixmap>,
}
