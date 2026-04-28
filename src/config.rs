pub const ICON_SIZE: u32 = 77;
pub const ICON_SPACING: u32 = 22;
pub const PANEL_PADDING: u32 = 14;
pub const HIGHLIGHT_PADDING: u32 = 24;
pub const CORNER_RADIUS: f32 = 19.2;
pub const BORDER_WIDTH: f32 = 2.0;
pub const INDICATOR_BORDER_WIDTH: f32 = 2.0;
pub const PANEL_OPACITY: f32 = 0.55;
pub const SELECTED_INDICATOR_OPACITY: f32 = 0.45;
pub const SELECTED_INDICATOR_BORDER_OPACITY: f32 = 0.8;

pub const fn opacity_alpha(value: f32) -> u8 {
    let clamped = if value < 0.0 {
        0.0
    } else if value > 1.0 {
        1.0
    } else {
        value
    };
    (clamped * 255.0 + 0.5) as u8
}

pub const fn panel_opacity_alpha() -> u8 {
    opacity_alpha(PANEL_OPACITY)
}

pub const fn selected_indicator_alpha() -> u8 {
    opacity_alpha(SELECTED_INDICATOR_OPACITY)
}

pub const fn selected_indicator_border_alpha() -> u8 {
    opacity_alpha(SELECTED_INDICATOR_BORDER_OPACITY)
}
