# jeg-herdr-toolkit

A [herdr](https://herdr.dev) plugin with three pieces that live inside herdr:

- **File viewer** in a herdr pane: a tree with Material Icon Theme icons and VS Code–style git
  status, plus a Markdown and code preview.
- **Agent status icons** in the sidebar, with several icon sets and colours to choose from.
- **Settings panel** to change all of the above and the shortcuts, with keyboard or mouse.

## Requirements

- herdr ≥ 0.9.5, on **Windows** (the plugin declares `platforms = ["windows"]`).
- [Rust](https://rustup.rs) (`cargo`): herdr builds the plugin when installing it.
- A [Nerd Font](https://www.nerdfonts.com) 3.x in the terminal, for the icons (the “Text” status
  icon set works without one).
- Optional, for the preview: [`glow`](https://github.com/charmbracelet/glow) (Markdown) and
  [`bat`](https://github.com/sharkdp/bat) (code). Without them the plain text is shown.

## Install

```powershell
herdr plugin install Jorgeescribanogarcia/jeg-herdr-toolkit
herdr plugin action invoke jeg-herdr-toolkit.settings
```

The second command opens the settings panel, which on opening writes the shortcuts and the sidebar
layout into herdr's `config.toml`. Just close it (`q`).

To work on the code: `cargo build --release` and `herdr plugin link <repo folder>` (`plugin link`
does not build). Windows locks the `.exe` while the viewer or the panel is open: close them before
rebuilding.

## Layout

A Cargo workspace:

| Crate | What it is |
|---|---|
| `common/` | `settings.toml` format, agent states, icon sets and status publishing. |
| `viewer/` | File viewer (`herdr-viewer`). |
| `status/` | Agent status icons in the sidebar (`herdr-status`). |
| `settings/` | Settings panel (`herdr-settings`). |

## Shortcuts

| Shortcut | Action |
|---|---|
| `prefix+Ctrl+F` | Viewer on the right (the same shortcut closes it) |
| `prefix+f` | Viewer popup (`q` or `Esc` close it) |
| `prefix+.` | Settings panel |

All three can be changed from the panel itself.

## File viewer

Opens the active pane's folder (the repo root if it is inside one), on the right or as a popup.

- **Tree** like VS Code's explorer: folders first, Material Icon Theme icons, and git status with its
  colours — a letter on each file (M, U, A, D, R, `!` for conflicts), a dot on folders that contain
  changes, ignored files greyed out (or hidden, depending on the settings).
- **Preview:** Markdown with `glow`, everything else with `bat` (Visual Studio Dark+ theme). Without
  them the plain text is shown. Images and binaries only say what they are.
- **Width:** from 80 columns, tree and preview side by side; below that they take turns (Enter opens,
  Esc goes back).
- Re-reads the disk and git every few seconds (2 by default): whatever an agent changes shows up on
  its own.

Keys: `↑↓`/`jk` move · `Enter` open · `←→`/`hl` fold and unfold · `Tab` tree ↔ preview ·
`PgUp`/`PgDn` · `r` reload · `q` close. Mouse: click to open, wheel to scroll.

**File icons.** The colours and which icon each file gets come from Material Icon Theme itself
(`viewer/src/icons.tsv`); the drawing is the closest Nerd Font glyph (a Nerd Font 3.x is needed in the
terminal). To regenerate the table after updating Material: `node scripts/generate-icons.mjs`. The
real SVGs can only be drawn with the Kitty graphics protocol, which Windows Terminal lacks (WezTerm has
it).

**How the viewer is recognised.** herdr does not say which plugin opened each pane, so the shortcut
looks in the tab for a pane whose process is this very executable (`herdr pane process-info`), not one
titled “Viewer”.

## Agent status icons

herdr only lets you choose between its own dots and symbols. The plugin publishes its own `$status`
value on every pane with an agent and on every workspace: the state's icon plus the workspace name
(`󰁪 Dev`). The sidebar shows it instead of `state_icon` and `workspace`, and colours it by the icon it
starts with.

Sets: **Material** (default), **Codicons**, **Circles** and **Text** (no Nerd Font needed). By default,
Material with the One Dark palette: working 󰁪 `#e5c07b` · waiting for you 󰂞 `#e06c75` bold · done 󰗠
`#98c379` · idle 󰗡 `#5c6370` (the done ✓ as an outline: “done” means it finished and you have not
looked yet) · unknown 󰘥 `#4b5263`. Within a set the five icons must differ, because the colour goes by
icon.

- **When it updates:** at herdr startup (the values do not survive a restart) and on each event:
  agent detected, state changed, pane closed and workspace renamed. Nothing runs in between. Each time
  it goes over everything with what herdr reports *at that moment*, and tags the report with the read
  time (`--seq`) so an older one arriving late does not overwrite the good one.
- **Why icon and name go together:** herdr separates each value of a row with « · », except its own
  icon. Together they read well; the price is that the name takes the state's colour.
- **If the plugin is disabled**, the sidebar rows lose their name: remove the `[ui.sidebar.agents]`
  and `[ui.sidebar.spaces]` blocks from `config.toml`.

## Settings panel

A popup with four sections: **Icons** (set and icon per state), **Colours**, **Viewer** (popup size,
tree width, git-ignored files, refresh interval) and **Shortcuts**. Changes show in the sidebar at
once; `u` goes back to what there was when the panel was opened.

With the mouse: click a section; click an option to select it and click again to change it (right
click changes backwards; on the icon set, click `◀`); wheel to move. With the keyboard: `1`-`4` or
`Tab` for the section, `↑↓` the option, `←→` the value, `Enter` to edit or capture a shortcut.

- **`settings.toml`** (in `herdr plugin config-dir jeg-herdr-toolkit`) is the single source of truth.
  If it is missing or wrong, the viewer and the status icons use the defaults and say so (viewer
  footer, plugin log).
- **herdr's `config.toml`:** the panel only rewrites `ui.sidebar.agents.rows`,
  `ui.sidebar.spaces.rows` and the `[[keys.command]]` entries whose `command` starts with
  `jeg-herdr-toolkit.`; everything else is kept as it is, comments included. Writes are atomic, with a
  `config.toml.bak` copy while the change lasts; if herdr does not accept the new configuration, both
  files are restored.
- **Repeated shortcuts:** herdr accepts a shortcut another action already uses without warning, so
  the panel checks first: against herdr's own bindings (read from `herdr --default-config`, the only
  way to get them) with whatever your `config.toml` changes, and against other plugins' commands.

## Troubleshooting

`herdr plugin log list --plugin jeg-herdr-toolkit` shows the output of each run of the plugin.

## License

MIT (see [`LICENSE`](LICENSE)). It includes data derived from other projects: see
[`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
