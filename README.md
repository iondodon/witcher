# witcher

Alt+Tab window switcher for Wayland with a daemon + evdev capture.

## Build

```bash
cargo build --release
```

Binary: `/home/ion/nwitcher/target/release/witcher`

## Run

Start the daemon (required):

```bash
/home/ion/nwitcher/target/release/witcher --daemon --backend niri
```

Supported backends: `niri`, `hyprland`

## Permissions (evdev)

The daemon reads `/dev/input/event*` to detect Alt+Tab globally.

Add your user to the input group and re-login:

```bash
sudo usermod -aG input $USER
```

## Notes

- For niri, remove any Alt+Tab binds so the compositor does not consume Tab events.
- The daemon must be running before Alt+Tab will work.

## Niri autostart example

In `~/.config/niri/config.kdl`:

```
spawn-at-startup "/home/ion/nwitcher/target/release/witcher" "--daemon" "--backend" "niri"
```
