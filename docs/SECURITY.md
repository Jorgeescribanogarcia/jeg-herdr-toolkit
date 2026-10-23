# Security — review index

One entry per review (`security-logs/YYYY-MM-DD_desc.md`).

| Date | Review | Summary |
|---|---|---|
| 2026-10-23 | [First version](security-logs/2026-10-23_first-version.md) | Local plugin: writes herdr's `config.toml` atomically and with a backup; no command injection. |
| 2026-10-23 | [Plugin marketplace](security-logs/2026-10-23_marketplace.md) | HTTPS to herdr's index and raw manifests; index names validated before any command or URL; third-party text cleaned and never run. |
