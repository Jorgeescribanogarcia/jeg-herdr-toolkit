# 2026-10-23 — Plugin marketplace and update check

New crate `market/` (`herdr-market`): an explorer of herdr's plugin index that installs, updates and
uninstalls through herdr, and an update check that runs at startup and on focus events.

## Surface

- **Network (HTTPS only, through `ureq` with rustls):** `assets.herdr.dev/plugins/index.json` (herdr's
  public index) and `raw.githubusercontent.com/<owner>/<repo>/<commit>/<path>` (one plugin manifest).
  20 s timeout and a 64 MB cap per response. No credentials are sent; no GitHub API or token is used.
- **Processes run, always with separate arguments and no shell:** `herdr plugin list --json`,
  `herdr plugin install <owner/repo[/dir]> --yes`, `herdr plugin update <id> --yes`,
  `herdr plugin uninstall <id>` (stdin closed), `herdr plugin pane open …`,
  `git -c credential.helper= ls-remote -- https://github.com/<owner>/<repo>` (with
  `GIT_TERMINAL_PROMPT=0` and `GCM_INTERACTIVE=never`, killed after 20 s: a deleted or private repo
  must not open a sign-in window from a background hook, or hang it), and `cmd /c start "" https://github.com/<owner>/<repo>`
  for the "open on GitHub" key.
- **Files written** (plugin state folder, `market/`): the cached index, the time of the last check, a
  lock file, the updates found (`pending.json`), the updates turned down and the time the explorer was
  last opened. Atomic writes.

## Safeguards

- **Names from the index are untrusted.** `owner/repo` must match two segments of
  `[A-Za-z0-9_.-]`, none starting with `-` and none `.` or `..`; a manifest folder must be made of such
  segments too; a commit must be alphanumeric. Anything else is refused before it reaches a URL or a
  command line, so it cannot be taken as an option or climb out of a path. Tests:
  `text::tests::valid_repo_names`, `valid_subdir_paths`, `install::tests::refuses_invalid_repo`.
- **Third-party text is never interpreted:** descriptions, names and manifest commands are drawn after
  removing control characters (no escape sequences reach the terminal) and are never run by the
  plugin. Test: `text::tests::clean_drops_control_characters`.
- **What a plugin runs is shown before it runs:** the explorer's confirmation lists the plugin's
  Windows `[[build]]` and `[[startup]]` commands, read at the commit the default branch points to at
  that moment (not the index's, which can be ~45 minutes old), and **Install is refused until they have
  loaded**. An update whose commands changed is flagged (⚠) and its new commands are shown. Test:
  `ui_browse::tests::install_needs_the_manifest`.
- **Forks reuse plugin ids:** «installed», Update and Uninstall match the installed plugin by id
  **and** repo, so Uninstall on a fork never removes the original. Test:
  `index::tests::fork_with_the_same_id_is_not_installed`.
- **Hooks:** a lock (token written through the creating handle; an empty lock counts as held) keeps
  simultaneous focus events to one check. Only `[marketplace]` of `settings.toml` is read by the hooks,
  so an invalid setting elsewhere cannot switch checks back on.
- **`cmd /c start`** only ever receives `https://github.com/` plus a name that passed the check above
  (no `&`, `|`, quotes or spaces can get through).
- The check never shows an error window: failures go to stderr (the plugin log).
- Uninstalling is done by herdr's own command; the plugin deletes nothing of other plugins.

## Not covered

- A push in the seconds between reading the manifest and `herdr plugin install` would install something
  other than what was shown (installing at a fixed commit would avoid it, but herdr would then never
  update the plugin).
- The integrity of what herdr downloads and builds: that is herdr's (`git` over HTTPS). Installing a
  plugin runs its build commands with your user's rights; the explorer shows them, it does not vet them.
- The index is unreviewed (herdr says so); a listed plugin can be malicious.
- A plugin installed into a sub-folder of its repo: herdr's `source.subdir` field name is assumed (only
  root-level plugins were seen); if it differs, those plugins are skipped by the update check and the
  reason is logged.
- Update failures (a plugin whose build fails) were not reproduced; the window shows the last line herdr
  prints on stderr.
