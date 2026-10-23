# 2026-10-23 — First version: viewer, status icons and settings panel

The plugin opens no ports and makes no network requests: it runs local programs and writes two
user configuration files.

## Surface

- **Processes run:** `herdr`, `git`, `glow` and `bat`, always with separate arguments and no shell.
  The paths they receive are absolute (they cannot start with `-`); a herdr workspace name goes inside
  `status=…` after `--token`, so it cannot be taken as an option.
- **Files written:** `settings.toml` (the plugin's config folder) and herdr's `config.toml`. In the
  latter only `ui.sidebar.agents.rows`, `ui.sidebar.spaces.rows` and the `[[keys.command]]` entries
  whose `command` starts with `jeg-herdr-toolkit.` are touched (recognised by id, not by description).
  The rest of the file is kept as it is.
- **Files read:** the ones previewed in the viewer (up to 5 MB; binaries are not read in full).
- **Clipboard (paste action):** only read, never written. A file list is typed as paths; an image is
  saved as `clipboard-<ms>.png` in the plugin's state folder and its path typed. Paths are passed to
  `herdr pane send-text` as a single argument (no shell), quoted if they contain spaces or shell-special
  characters; text is typed, never run (no Enter is sent). Saved images older than 7 days are removed,
  and only files named `clipboard-*.png` in that folder.

## Safeguards

- A `config.toml` that exists but cannot be read (not UTF-8, locked) is **not touched**: the change is
  cancelled instead of overwriting it.
- Atomic writes (temporary file + rename) and a `config.toml.bak` copy on disk while the change lasts;
  if herdr rejects the new configuration, both files are restored.
- Missing tables are created as regular tables, so the file can still be edited by hand.
- Before a shortcut is saved, it is checked that neither herdr nor another plugin uses it (herdr would
  accept it without warning).

## Open items (not security)

- Shortcuts without prefix such as `enter` or the arrow keys can be assigned, which would take the
  key away from the terminal.
- CRLF line endings in `config.toml` become LF when it is rewritten.
- `[keys]` written as an inline table and `[keys.indexed]` are not considered when looking for clashes.

## Not covered

Not tested with a `config.toml` that uses an inline `[keys]` table or `[keys.indexed]`.
