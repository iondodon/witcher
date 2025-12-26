pub const ICON_SIZE: u32 = 77;
pub const ICON_SPACING: u32 = 22;
pub const PANEL_PADDING: u32 = 14;
pub const HIGHLIGHT_PADDING: u32 = 24;
pub const CORNER_RADIUS: f32 = 19.2;
pub const BORDER_WIDTH: f32 = 1.0;
pub const PANEL_OPACITY: f32 = 1.0;

pub const fn panel_opacity_alpha() -> u8 {
    let clamped = if PANEL_OPACITY < 0.0 {
        0.0
    } else if PANEL_OPACITY > 1.0 {
        1.0
    } else {
        PANEL_OPACITY
    };
    (clamped * 255.0 + 0.5) as u8
}
