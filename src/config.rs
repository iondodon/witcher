use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::OnceLock;

pub const ICON_SIZE: u32 = 77;
pub const ICON_SPACING: u32 = 22;
pub const PANEL_PADDING: u32 = 14;
pub const HIGHLIGHT_PADDING: u32 = 24;
pub const CORNER_RADIUS: f32 = 19.2;
pub const BORDER_WIDTH: f32 = 2.0;
pub const INDICATOR_BORDER_WIDTH: f32 = 2.0;
pub const PANEL_OPACITY: f32 = 0.55;
pub const SELECTED_INDICATOR_OPACITY: f32 = 0.45;
pub const PANEL_BORDER_OPACITY: f32 = 0.65;
pub const SELECTED_INDICATOR_BORDER_OPACITY: f32 = 0.65;

#[derive(Clone, Copy, Debug, Serialize)]
pub struct AppConfig {
    pub icon_size: u32,
    pub icon_spacing: u32,
    pub panel_padding: u32,
    pub highlight_padding: u32,
    pub corner_radius: f32,
    pub border_width: f32,
    pub indicator_border_width: f32,
    pub panel_opacity: f32,
    pub selected_indicator_opacity: f32,
    pub panel_border_opacity: f32,
    pub selected_indicator_border_opacity: f32,
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    icon_size: Option<u32>,
    icon_spacing: Option<u32>,
    panel_padding: Option<u32>,
    highlight_padding: Option<u32>,
    corner_radius: Option<f32>,
    border_width: Option<f32>,
    indicator_border_width: Option<f32>,
    panel_opacity: Option<f32>,
    selected_indicator_opacity: Option<f32>,
    panel_border_opacity: Option<f32>,
    selected_indicator_border_opacity: Option<f32>,
}

static CONFIG: OnceLock<AppConfig> = OnceLock::new();

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            icon_size: ICON_SIZE,
            icon_spacing: ICON_SPACING,
            panel_padding: PANEL_PADDING,
            highlight_padding: HIGHLIGHT_PADDING,
            corner_radius: CORNER_RADIUS,
            border_width: BORDER_WIDTH,
            indicator_border_width: INDICATOR_BORDER_WIDTH,
            panel_opacity: PANEL_OPACITY,
            selected_indicator_opacity: SELECTED_INDICATOR_OPACITY,
            panel_border_opacity: PANEL_BORDER_OPACITY,
            selected_indicator_border_opacity: SELECTED_INDICATOR_BORDER_OPACITY,
        }
    }
}

impl AppConfig {
    fn merge_file(mut self, file: FileConfig) -> Self {
        if let Some(value) = file.icon_size {
            self.icon_size = value;
        }
        if let Some(value) = file.icon_spacing {
            self.icon_spacing = value;
        }
        if let Some(value) = file.panel_padding {
            self.panel_padding = value;
        }
        if let Some(value) = file.highlight_padding {
            self.highlight_padding = value;
        }
        if let Some(value) = file.corner_radius {
            self.corner_radius = value;
        }
        if let Some(value) = file.border_width {
            self.border_width = value;
        }
        if let Some(value) = file.indicator_border_width {
            self.indicator_border_width = value;
        }
        if let Some(value) = file.panel_opacity {
            self.panel_opacity = value;
        }
        if let Some(value) = file.selected_indicator_opacity {
            self.selected_indicator_opacity = value;
        }
        if let Some(value) = file.panel_border_opacity {
            self.panel_border_opacity = value;
        }
        if let Some(value) = file.selected_indicator_border_opacity {
            self.selected_indicator_border_opacity = value;
        }
        self
    }
}

pub fn init() {
    let _ = CONFIG.set(load());
}

pub fn app_config() -> &'static AppConfig {
    CONFIG.get_or_init(load)
}

pub fn icon_size() -> u32 {
    app_config().icon_size
}

fn load() -> AppConfig {
    let Some(path) = config_path() else {
        return AppConfig::default();
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return AppConfig::default(),
        Err(err) => {
            eprintln!("witcher: failed to read {}: {err}", path.display());
            return AppConfig::default();
        }
    };

    match serde_json::from_str::<FileConfig>(&text) {
        Ok(file_config) => AppConfig::default().merge_file(file_config),
        Err(err) => {
            eprintln!("witcher: failed to parse {}: {err}", path.display());
            AppConfig::default()
        }
    }
}

fn config_path() -> Option<PathBuf> {
    if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(
            PathBuf::from(config_home)
                .join("witcher")
                .join("config.json"),
        );
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".config").join("witcher").join("config.json"))
}

pub fn opacity_alpha(value: f32) -> u8 {
    let clamped = if value < 0.0 {
        0.0
    } else if value > 1.0 {
        1.0
    } else {
        value
    };
    (clamped * 255.0 + 0.5) as u8
}

pub fn panel_opacity_alpha() -> u8 {
    opacity_alpha(app_config().panel_opacity)
}

pub fn selected_indicator_alpha() -> u8 {
    opacity_alpha(app_config().selected_indicator_opacity)
}

pub fn panel_border_alpha() -> u8 {
    opacity_alpha(app_config().panel_border_opacity)
}

pub fn selected_indicator_border_alpha() -> u8 {
    opacity_alpha(app_config().selected_indicator_border_opacity)
}
