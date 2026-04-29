use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

pub const ICON_SIZE: u32 = 77;
pub const ICON_SPACING: u32 = 22;
pub const PANEL_PADDING: u32 = 14;
pub const HIGHLIGHT_PADDING: u32 = 24;
pub const CORNER_RADIUS: f32 = 19.2;
pub const BORDER_WIDTH: f32 = 2.0;
pub const INDICATOR_BORDER_WIDTH: f32 = 2.0;
pub const PANEL_OPACITY: f32 = 0.33;
pub const SELECTED_INDICATOR_OPACITY: f32 = 0.60;
pub const PANEL_BORDER_OPACITY: f32 = 0.45;
pub const SELECTED_INDICATOR_BORDER_OPACITY: f32 = 0.45;
pub const PANEL_BACKGROUND_COLOR: Rgb = Rgb {
    r: 17,
    g: 17,
    b: 17,
};
pub const PANEL_BORDER_COLOR: Rgb = Rgb {
    r: 36,
    g: 36,
    b: 36,
};
pub const HOVER_BORDER_COLOR: Rgb = Rgb {
    r: 72,
    g: 72,
    b: 72,
};
pub const SELECTED_INDICATOR_COLOR: Rgb = Rgb {
    r: 44,
    g: 44,
    b: 44,
};
pub const SELECTED_INDICATOR_BORDER_COLOR: Rgb = Rgb {
    r: 54,
    g: 54,
    b: 54,
};
pub const PLACEHOLDER_ICON_COLOR: Rgb = Rgb {
    r: 90,
    g: 90,
    b: 90,
};

#[derive(Clone, Copy, Debug)]
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
    pub panel_background_color: Rgb,
    pub panel_border_color: Rgb,
    pub hover_border_color: Rgb,
    pub selected_indicator_color: Rgb,
    pub selected_indicator_border_color: Rgb,
    pub placeholder_icon_color: Rgb,
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
            panel_background_color: PANEL_BACKGROUND_COLOR,
            panel_border_color: PANEL_BORDER_COLOR,
            hover_border_color: HOVER_BORDER_COLOR,
            selected_indicator_color: SELECTED_INDICATOR_COLOR,
            selected_indicator_border_color: SELECTED_INDICATOR_BORDER_COLOR,
            placeholder_icon_color: PLACEHOLDER_ICON_COLOR,
        }
    }
}

impl AppConfig {
    fn apply(&mut self, key: &str, value: &str) -> Result<(), String> {
        match key {
            "icon_size" => self.icon_size = parse_u32(key, value)?,
            "icon_spacing" => self.icon_spacing = parse_u32(key, value)?,
            "panel_padding" => self.panel_padding = parse_u32(key, value)?,
            "highlight_padding" => self.highlight_padding = parse_u32(key, value)?,
            "corner_radius" => self.corner_radius = parse_f32(key, value)?,
            "border_width" => self.border_width = parse_f32(key, value)?,
            "indicator_border_width" => self.indicator_border_width = parse_f32(key, value)?,
            "panel_opacity" => self.panel_opacity = parse_f32(key, value)?,
            "selected_indicator_opacity" => {
                self.selected_indicator_opacity = parse_f32(key, value)?
            }
            "panel_border_opacity" => self.panel_border_opacity = parse_f32(key, value)?,
            "selected_indicator_border_opacity" => {
                self.selected_indicator_border_opacity = parse_f32(key, value)?
            }
            "panel_background_color" => self.panel_background_color = parse_rgb(key, value)?,
            "panel_border_color" => self.panel_border_color = parse_rgb(key, value)?,
            "hover_border_color" => self.hover_border_color = parse_rgb(key, value)?,
            "selected_indicator_color" => {
                self.selected_indicator_color = parse_rgb(key, value)?
            }
            "selected_indicator_border_color" => {
                self.selected_indicator_border_color = parse_rgb(key, value)?
            }
            "placeholder_icon_color" => self.placeholder_icon_color = parse_rgb(key, value)?,
            _ => return Err(format!("unknown key `{key}`")),
        }
        Ok(())
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

    let mut config = AppConfig::default();
    for (idx, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            eprintln!(
                "witcher: failed to parse {}:{}: expected `key = value`",
                path.display(),
                idx + 1
            );
            continue;
        };

        if let Err(err) = config.apply(key.trim(), value.trim()) {
            eprintln!(
                "witcher: failed to parse {}:{}: {}",
                path.display(),
                idx + 1,
                err
            );
        }
    }
    config
}

fn config_path() -> Option<PathBuf> {
    if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(config_home).join("witcher").join("config"));
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".config").join("witcher").join("config"))
}

fn parse_u32(key: &str, value: &str) -> Result<u32, String> {
    value
        .parse::<u32>()
        .map_err(|err| format!("invalid value for `{key}`: {err}"))
}

fn parse_f32(key: &str, value: &str) -> Result<f32, String> {
    value
        .parse::<f32>()
        .map_err(|err| format!("invalid value for `{key}`: {err}"))
}

fn parse_rgb(key: &str, value: &str) -> Result<Rgb, String> {
    let hex = value.trim().strip_prefix('#').unwrap_or(value.trim());
    if hex.len() != 6 {
        return Err(format!(
            "invalid value for `{key}`: expected 6-digit hex color"
        ));
    }

    let parse = |range: std::ops::Range<usize>| {
        u8::from_str_radix(&hex[range], 16)
            .map_err(|err| format!("invalid value for `{key}`: {err}"))
    };

    Ok(Rgb {
        r: parse(0..2)?,
        g: parse(2..4)?,
        b: parse(4..6)?,
    })
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
