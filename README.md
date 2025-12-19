# witcher

Alt+Tab window switcher for Wayland with a daemon and compositor keybinds.

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
witcher --daemon --backend niri
```

Supported backends: `niri`, `hyprland`

Trigger the switcher from your compositor keybinding:

```bash
witcher --show
```

Reverse cycle:

```bash
witcher --show-prev
```

Example keybinds:

Niri (`~/.config/niri/config.kdl`):

```
binds {
    Alt+Tab { spawn "witcher" "--show" }
    Alt+Shift+Tab { spawn "witcher" "--show-prev" }
}
```

Hyprland (`~/.config/hypr/hyprland.conf`):

```
bind = ALT, Tab, exec, witcher --show
bind = ALT SHIFT, Tab, exec, witcher --show-prev
```

## Notes

- For niri, ensure Alt+Tab binds run `witcher --show` so the compositor consumes the key.
- The daemon must be running before Alt+Tab will work.

## Niri autostart example

In `~/.config/niri/config.kdl`:

```
spawn-at-startup "witcher" "--daemon" "--backend" "niri"
```
