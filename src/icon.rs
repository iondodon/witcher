use anyhow::{Context, Result};
use freedesktop_icons::lookup;
use image::{imageops::FilterType, DynamicImage};
use resvg::usvg;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use tiny_skia::{Color, IntSize, Paint, Pixmap, Transform};

use crate::config::ICON_SIZE;

#[derive(Default)]
pub struct IconCache {
    icons: std::collections::HashMap<String, Arc<Pixmap>>,
}

impl IconCache {
    pub fn icon_for(&mut self, app_id: &str) -> Arc<Pixmap> {
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
    let transform = tiny_skia::Transform::from_scale(scale, scale).post_translate(dx, dy);
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

    for base in &paths {
        for name in &candidates {
            let file = if name.ends_with(".desktop") {
                base.join(name)
            } else {
                base.join(format!("{name}.desktop"))
            };
            if let Ok(info) = parse_desktop_entry(&file) {
                if let Some(info) = info {
                    if let Some(icon) = info.icon {
                        return Some(icon);
                    }
                }
            }
        }
    }

    let mut candidates_lower = std::collections::HashSet::new();
    for name in &candidates {
        candidates_lower.insert(name.to_ascii_lowercase());
    }

    for base in &paths {
        let entries = match fs::read_dir(&base) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("desktop") {
                continue;
            }
            let info = match parse_desktop_entry(&path) {
                Ok(Some(info)) => info,
                _ => continue,
            };
            let startup = match info.startup_wm_class {
                Some(startup) => startup,
                None => continue,
            };
            if candidates_lower.contains(&startup.to_ascii_lowercase()) {
                if let Some(icon) = info.icon {
                    return Some(icon);
                }
            }
        }
    }

    None
}

struct DesktopEntryInfo {
    icon: Option<String>,
    startup_wm_class: Option<String>,
}

fn parse_desktop_entry(path: &Path) -> Result<Option<DesktopEntryInfo>> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(_) => return Ok(None),
    };
    let mut in_entry = false;
    let mut icon = None;
    let mut startup_wm_class = None;
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
            if !value.is_empty() {
                icon = Some(value.to_string());
            }
            continue;
        }
        if let Some(value) = line.strip_prefix("StartupWMClass=") {
            let value = value.trim();
            if !value.is_empty() {
                startup_wm_class = Some(value.to_string());
            }
            continue;
        }
    }
    if icon.is_none() && startup_wm_class.is_none() {
        return Ok(None);
    }
    Ok(Some(DesktopEntryInfo {
        icon,
        startup_wm_class,
    }))
}
