use anyhow::{Context, Result};
use freedesktop_icons::lookup;
use image::{DynamicImage, imageops::FilterType};
use resvg::usvg;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use tiny_skia::{Color, IntSize, Paint, Pixmap, Transform};

use crate::config::{app_config, icon_size};

#[derive(Default)]
pub struct IconCache {
    icons: std::collections::HashMap<String, Arc<Pixmap>>,
}

impl IconCache {
    pub fn icon_for(&mut self, app_id: &str) -> Arc<Pixmap> {
        if let Some(icon) = self.icons.get(app_id) {
            return icon.clone();
        }
        let icon = load_icon(app_id).unwrap_or_else(|_| placeholder_icon(icon_size()));
        let icon = Arc::new(icon);
        self.icons.insert(app_id.to_string(), icon.clone());
        icon
    }
}

fn load_icon(app_id: &str) -> Result<Pixmap> {
    let icon_size = icon_size();
    let mut candidates = icon_name_candidates(app_id);

    if let Some(icon_name) = desktop_icon_name(app_id) {
        candidates.push(icon_name);
    }

    let path = candidates
        .into_iter()
        .find_map(|name| resolve_icon_path(&name, icon_size))
        .or_else(|| {
            lookup("application-x-executable")
                .with_size(icon_size as u16)
                .find()
        })
        .context("no icon found")?;

    if path.extension().and_then(|ext| ext.to_str()) == Some("svg") {
        return render_svg(&path, icon_size);
    }

    let image = image::open(&path).with_context(|| format!("open icon {}", path.display()))?;
    let resized = image.resize_exact(icon_size, icon_size, FilterType::Lanczos3);
    Ok(pixmap_from_image(resized))
}

fn resolve_icon_path(name: &str, icon_size: u32) -> Option<PathBuf> {
    let path = Path::new(name);
    if path.is_absolute() && path.is_file() {
        return Some(path.to_path_buf());
    }
    lookup(name)
        .with_size(icon_size as u16)
        .find()
        .or_else(|| find_installed_icon(name, icon_size))
}

fn icon_name_candidates(app_id: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();
    let app_id = app_id.trim();
    let normalized = app_id.strip_suffix(".desktop").unwrap_or(app_id);

    push_icon_candidate(&mut candidates, &mut seen, app_id);
    if normalized != app_id {
        push_icon_candidate(&mut candidates, &mut seen, normalized);
    }
    if let Some(last) = normalized.rsplit('.').next() {
        push_icon_candidate(&mut candidates, &mut seen, last);
    }

    candidates
}

fn push_icon_candidate(candidates: &mut Vec<String>, seen: &mut HashSet<String>, name: &str) {
    let name = name.trim();
    if name.is_empty() {
        return;
    }
    if seen.insert(name.to_string()) {
        candidates.push(name.to_string());
    }

    let lower = name.to_ascii_lowercase();
    if lower != name && seen.insert(lower.clone()) {
        candidates.push(lower);
    }
}

fn pixmap_from_image(image: DynamicImage) -> Pixmap {
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    let mut bytes = rgba.into_raw();
    premultiply_alpha(&mut bytes);
    let size = IntSize::from_wh(width, height).expect("icon size");
    Pixmap::from_vec(bytes, size).expect("pixmap from image")
}

fn placeholder_icon(size: u32) -> Pixmap {
    let mut pixmap = Pixmap::new(size, size).expect("placeholder pixmap");
    let mut paint = Paint::default();
    let color = app_config().placeholder_icon_color;
    paint.set_color(Color::from_rgba8(color.r, color.g, color.b, 255));
    pixmap.fill_rect(
        tiny_skia::Rect::from_xywh(0.0, 0.0, size as f32, size as f32).unwrap(),
        &paint,
        Transform::identity(),
        None,
    );
    pixmap
}

fn premultiply_alpha(bytes: &mut [u8]) {
    for pixel in bytes.chunks_exact_mut(4) {
        let a = pixel[3] as u16;
        if a == 0 {
            pixel[0] = 0;
            pixel[1] = 0;
            pixel[2] = 0;
            continue;
        }
        if a == 255 {
            continue;
        }
        let r = (pixel[0] as u16 * a + 127) / 255;
        let g = (pixel[1] as u16 * a + 127) / 255;
        let b = (pixel[2] as u16 * a + 127) / 255;
        pixel[0] = r as u8;
        pixel[1] = g as u8;
        pixel[2] = b as u8;
    }
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
    let candidates = icon_name_candidates(app_id);
    let paths = application_dirs();

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

    let mut candidates_lower = HashSet::new();
    for name in &candidates {
        candidates_lower.insert(name.to_ascii_lowercase());
    }

    for base in &paths {
        let entries = match fs::read_dir(base) {
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

fn application_dirs() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    push_path(
        &mut paths,
        &mut seen,
        PathBuf::from("/usr/share/applications"),
    );
    push_path(
        &mut paths,
        &mut seen,
        PathBuf::from("/usr/local/share/applications"),
    );
    push_path(
        &mut paths,
        &mut seen,
        PathBuf::from("/var/lib/flatpak/exports/share/applications"),
    );
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        push_path(
            &mut paths,
            &mut seen,
            home.join(".local/share/applications"),
        );
        push_path(
            &mut paths,
            &mut seen,
            home.join(".local/share/flatpak/exports/share/applications"),
        );
    }
    if let Some(data_home) = std::env::var_os("XDG_DATA_HOME") {
        push_path(
            &mut paths,
            &mut seen,
            PathBuf::from(data_home).join("applications"),
        );
    }
    if let Ok(xdg_dirs) = std::env::var("XDG_DATA_DIRS") {
        for dir in xdg_dirs.split(':') {
            if !dir.is_empty() {
                push_path(
                    &mut paths,
                    &mut seen,
                    PathBuf::from(dir).join("applications"),
                );
            }
        }
    }
    paths
}

fn icon_dirs() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    if let Some(data_home) = std::env::var_os("XDG_DATA_HOME") {
        push_path(
            &mut paths,
            &mut seen,
            PathBuf::from(data_home).join("icons"),
        );
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        push_path(&mut paths, &mut seen, home.join(".local/share/icons"));
        push_path(&mut paths, &mut seen, home.join(".icons"));
    }
    if let Ok(xdg_dirs) = std::env::var("XDG_DATA_DIRS") {
        for dir in xdg_dirs.split(':') {
            if !dir.is_empty() {
                push_path(&mut paths, &mut seen, PathBuf::from(dir).join("icons"));
            }
        }
    }
    push_path(&mut paths, &mut seen, PathBuf::from("/usr/share/icons"));
    push_path(
        &mut paths,
        &mut seen,
        PathBuf::from("/usr/local/share/icons"),
    );
    paths
}

fn find_installed_icon(name: &str, icon_size: u32) -> Option<PathBuf> {
    let mut best = None;
    for base in icon_dirs() {
        collect_best_icon(&base, name, icon_size, &mut best);
    }
    best.map(|candidate| candidate.path)
}

struct IconPathCandidate {
    path: PathBuf,
    score: u32,
}

fn collect_best_icon(dir: &Path, name: &str, icon_size: u32, best: &mut Option<IconPathCandidate>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_best_icon(&path, name, icon_size, best);
            continue;
        }
        if !is_supported_icon_file(&path)
            || path.file_stem().and_then(|stem| stem.to_str()) != Some(name)
        {
            continue;
        }

        let score = icon_path_score(&path, icon_size);
        let replace = best
            .as_ref()
            .map(|candidate| score < candidate.score)
            .unwrap_or(true);
        if replace {
            *best = Some(IconPathCandidate { path, score });
        }
    }
}

fn is_supported_icon_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("png" | "jpg" | "jpeg" | "svg")
    )
}

fn icon_path_score(path: &Path, icon_size: u32) -> u32 {
    let size_delta = path
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .filter_map(directory_icon_size)
        .map(|size| size.abs_diff(icon_size))
        .min()
        .unwrap_or(icon_size);
    let symbolic_penalty = path
        .components()
        .any(|component| component.as_os_str().to_string_lossy().contains("symbolic"))
        .then_some(1)
        .unwrap_or(0);

    size_delta * 2 + symbolic_penalty
}

fn directory_icon_size(value: &str) -> Option<u32> {
    let value = value.strip_suffix("@2x").unwrap_or(value);
    let first = value.split('x').next()?;
    first.parse().ok()
}

fn push_path(paths: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>, path: PathBuf) {
    if seen.insert(path.clone()) {
        paths.push(path);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_id_candidates_include_normalized_forms() {
        let candidates = icon_name_candidates("org.chromium.Chromium");

        assert!(
            candidates
                .iter()
                .any(|name| name == "org.chromium.Chromium")
        );
        assert!(
            candidates
                .iter()
                .any(|name| name == "org.chromium.chromium")
        );
        assert!(candidates.iter().any(|name| name == "Chromium"));
        assert!(candidates.iter().any(|name| name == "chromium"));
        assert!(!candidates.iter().any(|name| name == "google-chrome"));
    }

    #[test]
    fn desktop_suffix_is_optional_for_candidates() {
        let candidates = icon_name_candidates("com.example.App.desktop");

        assert!(
            candidates
                .iter()
                .any(|name| name == "com.example.App.desktop")
        );
        assert!(candidates.iter().any(|name| name == "com.example.App"));
        assert!(candidates.iter().any(|name| name == "app"));
    }
}
