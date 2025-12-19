use anyhow::{Context, Result};
use std::{
    io::{Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    sync::{
        mpsc,
        Arc,
        Mutex,
    },
    thread,
};

use crate::icon::IconCache;
use crate::mru::MruState;
use crate::switcher::{run_switcher, SwitcherControl};
use crate::types::BackendKind;

pub fn send_show() -> Result<()> {
    send_command(b"show")
}

pub fn send_show_prev() -> Result<()> {
    send_command(b"prev")
}

fn send_command(cmd: &[u8]) -> Result<()> {
    let socket_path = runtime_socket_path("witcher.sock")?;
    let mut stream = UnixStream::connect(&socket_path)
        .with_context(|| format!("connect {}", socket_path.display()))?;
    let _ = stream.write_all(cmd);
    let mut buf = [0u8; 8];
    let _ = stream.read(&mut buf);
    Ok(())
}

struct SwitcherControlSender {
    tx: mpsc::Sender<SwitcherControl>,
    wake: UnixStream,
}

impl SwitcherControlSender {
    fn send(&mut self, msg: SwitcherControl) {
        let _ = self.tx.send(msg);
        let _ = self.wake.write_all(b"x");
    }
}

pub fn run_daemon(backend: BackendKind) -> Result<()> {
    let socket_path = runtime_socket_path("witcher.sock")?;
    let _listener = match bind_listener(&socket_path) {
        Ok(listener) => listener,
        Err(err) => return Err(err),
    };

    let (tx, rx) = mpsc::channel::<DaemonMsg>();
    let switcher_sender: Arc<Mutex<Option<SwitcherControlSender>>> = Arc::new(Mutex::new(None));
    let listener = _listener.try_clone().context("clone listener")?;
    let tx_listener = tx.clone();
    let sender_listener = switcher_sender.clone();
    thread::spawn(move || {
        loop {
            let Ok((mut stream, _)) = listener.accept() else {
                continue;
            };
            let mut buf = [0u8; 32];
            let read_len = match stream.read(&mut buf) {
                Ok(len) => len,
                Err(_) => 0,
            };
            let _ = stream.write_all(b"ok");
            let msg = parse_socket_msg(&buf[..read_len]);
            if !try_send_control(&sender_listener, &msg) {
                let _ = tx_listener.send(msg);
            }
        }
    });

    let mut icon_cache = IconCache::default();
    let mut mru = MruState::default();
    loop {
        let Ok(msg) = rx.recv() else {
            continue;
        };
        if matches!(msg, DaemonMsg::Show | DaemonMsg::ShowPrev) {
            while rx.try_recv().is_ok() {}
            let (control_tx, control_rx) = mpsc::channel();
            let (wake_write, wake_read) = UnixStream::pair().context("create wake pipe")?;
            {
                let mut guard = switcher_sender.lock().unwrap();
                *guard = Some(SwitcherControlSender {
                    tx: control_tx,
                    wake: wake_write,
                });
            }
            let result = run_switcher(backend, &mut icon_cache, &mut mru, control_rx, wake_read);
            {
                let mut guard = switcher_sender.lock().unwrap();
                *guard = None;
            }
            match result {
                Ok(Some(id)) => mru.update_on_focus(id),
                Ok(None) => {}
                Err(err) => eprintln!("witcher: switcher error: {err:#}"),
            }
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

fn parse_socket_msg(buf: &[u8]) -> DaemonMsg {
    let text = std::str::from_utf8(buf).unwrap_or("").trim();
    if text.eq_ignore_ascii_case("prev") {
        DaemonMsg::ShowPrev
    } else {
        DaemonMsg::Show
    }
}

fn try_send_control(
    sender: &Arc<Mutex<Option<SwitcherControlSender>>>,
    msg: &DaemonMsg,
) -> bool {
    let mut guard = sender.lock().unwrap();
    let Some(sender) = guard.as_mut() else {
        return false;
    };
    let control = match msg {
        DaemonMsg::Show => SwitcherControl::CycleNext,
        DaemonMsg::ShowPrev => SwitcherControl::CyclePrev,
    };
    sender.send(control);
    true
}

#[derive(Clone, Copy)]
enum DaemonMsg {
    Show,
    ShowPrev,
}
