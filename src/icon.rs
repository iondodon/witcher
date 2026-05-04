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
    pub fn icon_for(&mut self, app_id: &str, title: Option<&str>) -> Arc<Pixmap> {
        let cache_key = icon_cache_key(app_id, title);
        if let Some(icon) = self.icons.get(&cache_key) {
            return icon.clone();
        }
        let icon = load_icon(app_id, title).unwrap_or_else(|_| placeholder_icon(icon_size()));
        let icon = Arc::new(icon);
        self.icons.insert(cache_key, icon.clone());
        icon
    }
}

fn icon_cache_key(app_id: &str, title: Option<&str>) -> String {
    format!("{}\t{}", app_id, title.unwrap_or(""))
}

fn load_icon(app_id: &str, title: Option<&str>) -> Result<Pixmap> {
    let icon_size = icon_size();
    let mut candidates = icon_name_candidates(app_id);
    if let Some(title) = title {
        let mut seen = candidates.iter().cloned().collect::<HashSet<_>>();
        for candidate in icon_name_candidates(title) {
            push_icon_candidate(&mut candidates, &mut seen, &candidate);
        }
    }

    if let Some(icon_name) = desktop_icon_name(&candidates) {
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
    push_semantic_icon_candidates(&mut candidates, &mut seen, app_id);

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

fn push_semantic_icon_candidates(
    candidates: &mut Vec<String>,
    seen: &mut HashSet<String>,
    text: &str,
) {
    let lower = text.to_ascii_lowercase();
    if !contains_password_related_term(&lower) {
        return;
    }
    for name in [
        "dialog-password",
        "password-manager",
        "password",
        "preferences-desktop-user-password",
    ] {
        push_icon_candidate(candidates, seen, name);
    }
}

fn contains_password_related_term(text: &str) -> bool {
    ["password", "passphrase", "keyring", "wallet", "secret"]
        .iter()
        .any(|term| text.contains(term))
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

fn desktop_icon_name(candidates: &[String]) -> Option<String> {
    let paths = application_dirs();

    for base in &paths {
        for name in candidates {
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
    for name in candidates {
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
            let matches_name = info
                .names
                .iter()
                .any(|name| candidates_lower.contains(&name.to_ascii_lowercase()));
            let matches_startup = info
                .startup_wm_class
                .as_ref()
                .map(|startup| candidates_lower.contains(&startup.to_ascii_lowercase()))
                .unwrap_or(false);
            let matches_exec = info
                .exec_names
                .iter()
                .any(|exec| candidates_lower.contains(&exec.to_ascii_lowercase()));
            if matches_name || matches_startup || matches_exec {
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
    names: Vec<String>,
    startup_wm_class: Option<String>,
    exec_names: Vec<String>,
}

fn parse_desktop_entry(path: &Path) -> Result<Option<DesktopEntryInfo>> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(_) => return Ok(None),
    };
    let mut in_entry = false;
    let mut icon = None;
    let mut names = Vec::new();
    let mut startup_wm_class = None;
    let mut exec_names = Vec::new();
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
        if let Some(value) = desktop_name_value(line) {
            names.push(value.to_string());
            continue;
        }
        if let Some(value) = line.strip_prefix("StartupWMClass=") {
            let value = value.trim();
            if !value.is_empty() {
                startup_wm_class = Some(value.to_string());
            }
            continue;
        }
        if let Some(value) = line.strip_prefix("Exec=") {
            if let Some(exec_name) = desktop_exec_name(value) {
                exec_names.push(exec_name);
            }
            continue;
        }
    }
    if icon.is_none() && names.is_empty() && startup_wm_class.is_none() && exec_names.is_empty() {
        return Ok(None);
    }
    Ok(Some(DesktopEntryInfo {
        icon,
        names,
        startup_wm_class,
        exec_names,
    }))
}

fn desktop_name_value(line: &str) -> Option<&str> {
    if let Some(value) = line.strip_prefix("Name=") {
        let value = value.trim();
        return (!value.is_empty()).then_some(value);
    }
    if !line.starts_with("Name[") {
        return None;
    }
    let (_key, value) = line.split_once('=')?;
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn desktop_exec_name(value: &str) -> Option<String> {
    let command = value.split_whitespace().next()?.trim_matches('"');
    let name = Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(command)
        .trim();
    (!name.is_empty()).then(|| name.to_string())
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

    #[test]
    fn desktop_name_value_handles_localized_names() {
        assert_eq!(desktop_name_value("Name=Counter-Strike 2"), Some("Counter-Strike 2"));
        assert_eq!(
            desktop_name_value("Name[en_US]=Counter-Strike 2"),
            Some("Counter-Strike 2")
        );
        assert_eq!(desktop_name_value("Icon=steam_icon_730"), None);
    }

    #[test]
    fn password_related_candidates_use_generic_password_icons() {
        let candidates = icon_name_candidates("org.kde.ksecretd");

        assert!(candidates.iter().any(|name| name == "dialog-password"));
        assert!(candidates.iter().any(|name| name == "password-manager"));
    }
}
