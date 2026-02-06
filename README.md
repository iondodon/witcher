# witcher

Alt+Tab window switcher for Wayland with a daemon and compositor keybinds.

<p align="center">
  <img src="screenshot.png" alt="Withcer screenshot" width="900">
</p>

## Build

```bash
cargo build --release
```

Binary: `/target/release/witcher`

## Installing

```bash
cargo install --path .
```

## Run

Start the daemon (required):

```bash
~/.cargo/bin/witcher --daemon --backend niri
```

Supported backends: `niri`, `hyprland`

Trigger the switcher from your compositor keybinding:

```bash
~/.cargo/bin/witcher --cycle-next
```

Reverse cycle:

```bash
~/.cargo/bin/witcher --cycle-prev
```

Example keybinds:

Niri (`~/.config/niri/config.kdl`):

```
binds {
    Alt+Tab { spawn "~/.cargo/bin/witcher" "--cycle-next" }
    Alt+Shift+Tab { spawn "~/.cargo/bin/witcher" "--cycle-prev" }
}
```

Hyprland (`~/.config/hypr/hyprland.conf`):

```
bind = ALT, Tab, exec, ~/.cargo/bin/witcher --cycle-next
bind = ALT SHIFT, Tab, exec, ~/.cargo/bin/witcher --cycle-prev
```

## Notes

- Ensure Alt+Tab binds run `~/.cargo/bin/witcher --cycle-next` so the compositor consumes the key.
- The daemon must be running before Alt+Tab will work.

## Niri autostart example

In `~/.config/niri/config.kdl`:

```
spawn-at-startup "~/.cargo/bin/witcher" "--daemon" "--backend" "niri"
```
