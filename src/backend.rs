use anyhow::{Context, Result};
use niri_ipc::{socket::Socket, Action, Request, Response};
use serde::Deserialize;
use std::process::Command;

use crate::types::BackendKind;

pub struct BackendWindow {
    pub id: u64,
    pub app_id: Option<String>,
    pub is_focused: bool,
}

#[derive(Deserialize)]
struct HyprClient {
    address: Option<String>,
    class: Option<String>,
    #[serde(rename = "initialClass")]
    initial_class: Option<String>,
    focus: Option<bool>,
    mapped: Option<bool>,
    hidden: Option<bool>,
}

#[derive(Deserialize)]
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

fn hyprctl_json<T: for<'de> Deserialize<'de>>(args: &[&str]) -> Result<T> {
    let text = hyprctl(args)?;
    let value = serde_json::from_str(&text).context("parse hyprctl json")?;
    Ok(value)
}

pub fn focus_window(backend: BackendKind, id: u64) -> Result<()> {
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

pub fn focused_output_info(backend: BackendKind) -> Result<(Option<(i32, i32)>, u32)> {
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

pub fn backend_windows(backend: BackendKind) -> Result<Vec<BackendWindow>> {
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
                let app_id = client.initial_class.clone().or(client.class.clone());
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
