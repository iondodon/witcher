⚠️ This app is Vibe Coded.

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

## Config

Witcher reads configuration from:

- `$XDG_CONFIG_HOME/witcher/config`
- or `~/.config/witcher/config` if `XDG_CONFIG_HOME` is unset

If the file does not exist, Witcher uses the built-in defaults.

Format:

```text
# comments are allowed
icon_size = 77
icon_spacing = 22
panel_padding = 14
highlight_padding = 24
corner_radius = 28.0
border_width = 2.0
indicator_border_width = 2.0
panel_opacity = 0.33
selected_indicator_opacity = 0.28
panel_border_opacity = 0.45
panel_shadow_size = 2.0
selected_indicator_border_opacity = 0.24
selected_indicator_shadow_size = 0.0
panel_background_color = 111111
panel_border_color = 242424
panel_shadow_color = 000000
hover_border_color = 484848
selected_indicator_color = ffffff
selected_indicator_border_color = ffffff
placeholder_icon_color = 5a5a5a
```

## Notes

- Ensure Alt+Tab binds run `~/.cargo/bin/witcher --cycle-next` so the compositor consumes the key.
- The daemon must be running before Alt+Tab will work.

## Niri autostart example

In `~/.config/niri/config.kdl`:

```
spawn-at-startup "~/.cargo/bin/witcher" "--daemon" "--backend" "niri"
```
