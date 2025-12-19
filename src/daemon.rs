use anyhow::{Context, Result};
use evdev::{Device, InputEventKind, Key};
use std::{
    io::{Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
        Arc,
    },
    thread,
};

use crate::icon::IconCache;
use crate::mru::MruState;
use crate::switcher::run_switcher;
use crate::types::BackendKind;

pub fn run_daemon(backend: BackendKind) -> Result<()> {
    let socket_path = runtime_socket_path("witcher.sock")?;
    let _listener = match bind_listener(&socket_path) {
        Ok(listener) => listener,
        Err(err) => return Err(err),
    };

    let (tx, rx) = mpsc::channel::<DaemonMsg>();
    let listener = _listener.try_clone().context("clone listener")?;
    let tx_listener = tx.clone();
    thread::spawn(move || {
        loop {
            let Ok((mut stream, _)) = listener.accept() else {
                continue;
            };
            let mut buf = [0u8; 32];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(b"ok");
            let _ = tx_listener.send(DaemonMsg::Show);
        }
    });

    let active = Arc::new(AtomicBool::new(false));
    spawn_evdev_listener(tx.clone(), active.clone());

    let mut icon_cache = IconCache::default();
    let mut mru = MruState::default();
    loop {
        let Ok(msg) = rx.recv() else {
            continue;
        };
        if matches!(msg, DaemonMsg::Show) {
            while rx.try_recv().is_ok() {}
            active.store(true, Ordering::SeqCst);
            match run_switcher(backend, &mut icon_cache, &mut mru) {
                Ok(Some(id)) => mru.update_on_focus(id),
                Ok(None) => {}
                Err(err) => eprintln!("witcher: switcher error: {err:#}"),
            }
            active.store(false, Ordering::SeqCst);
            while rx.try_recv().is_ok() {}
        }
    }
}

fn runtime_socket_path(name: &str) -> Result<PathBuf> {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    Ok(runtime_dir.join(name))
}

fn bind_listener(path: &PathBuf) -> Result<UnixListener> {
    if let Ok(mut stream) = UnixStream::connect(path) {
        let mut buf = [0u8; 8];
        let _ = stream.write_all(b"ping");
        let _ = stream.read(&mut buf);
        return Err(anyhow::anyhow!("witcher daemon already running"));
    }
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path)
        .with_context(|| format!("bind {}", path.display()))?;
    Ok(listener)
}

#[derive(Clone, Copy)]
enum DaemonMsg {
    Show,
}

fn spawn_evdev_listener(tx: mpsc::Sender<DaemonMsg>, active: Arc<AtomicBool>) {
    let devices = match enumerate_keyboards() {
        Ok(devices) => devices,
        Err(err) => {
            eprintln!("witcher: evdev enumerate error: {err:#}");
            return;
        }
    };
    if devices.is_empty() {
        eprintln!("witcher: no keyboard devices with Alt+Tab found");
        return;
    }
    for mut device in devices {
        let tx = tx.clone();
        let active = active.clone();
        thread::spawn(move || {
            let mut alt_down = false;
            loop {
                let events = match device.fetch_events() {
                    Ok(events) => events,
                    Err(_) => continue,
                };
                for ev in events {
                    if let InputEventKind::Key(key) = ev.kind() {
                        let value = ev.value();
                        match key {
                            Key::KEY_LEFTALT | Key::KEY_RIGHTALT => {
                                alt_down = value != 0;
                            }
                            Key::KEY_TAB => {
                                if alt_down && value != 0 && !active.load(Ordering::SeqCst) {
                                    let _ = tx.send(DaemonMsg::Show);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        });
    }
}

fn enumerate_keyboards() -> Result<Vec<Device>> {
    let mut devices = Vec::new();
    for (_path, device) in evdev::enumerate() {
        if let Some(keys) = device.supported_keys() {
            let has_alt = keys.contains(Key::KEY_LEFTALT) || keys.contains(Key::KEY_RIGHTALT);
            let has_tab = keys.contains(Key::KEY_TAB);
            if has_alt && has_tab {
                devices.push(device);
            }
        }
    }
    Ok(devices)
}
