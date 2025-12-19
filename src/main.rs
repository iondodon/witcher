mod backend;
mod config;
mod daemon;
mod icon;
mod mru;
mod switcher;
mod types;

use anyhow::Result;

use crate::daemon::{run_daemon, send_show, send_show_prev};
use crate::types::BackendKind;

fn parse_backend_required(args: &[String]) -> Result<BackendKind> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--backend" {
            if let Some(value) = iter.next() {
                return match value.as_str() {
                    "niri" => Ok(BackendKind::Niri),
                    "sway" => Ok(BackendKind::Sway),
                    "hyprland" => Ok(BackendKind::Hyprland),
                    "kwin" => Ok(BackendKind::Kwin),
                    "gnome" => Ok(BackendKind::Gnome),
                    _ => Err(anyhow::anyhow!("unknown backend: {value}")),
                };
            }
        }
    }
    Err(anyhow::anyhow!("missing --backend (niri|hyprland)"))
}

fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--show") {
        send_show()?;
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--show-prev") {
        send_show_prev()?;
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--daemon") {
        let backend = parse_backend_required(&args)?;
        run_daemon(backend)?;
        return Ok(());
    }

    eprintln!(
        "Usage: witcher --daemon --backend <name>\n       witcher --show\n       witcher --show-prev\nSupported backends: niri, hyprland"
    );
    Ok(())
}
