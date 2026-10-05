# Garden Gate implementation record

Accepted identity: Garden Gate, `io.github.tcballard.gardengate`, repository `tcballard/omarchy-plugin-gardengate`. Hosted kind: `bar-widget`, root `BarWidget.qml`. Scope: iPhone iCloud Drive → XPS download-only developer preview.

The widget launches the management menu in the separate Rust companion. It owns no authentication, transfer loop or persistent sync state. No extra Quickshell process is started. Ordinary files, encrypted account configuration, operation locks, ledger and recovery storage belong to the companion. The existing Rust adapter remains rclone-based. No socket IPC exists yet.

The launcher is intentionally static: it does not imply an authenticated, online or syncing state. The management menu and CLI show actual companion state. Installation of dependencies and services is an explicit external operation, never a plugin hook. Credentials are configured interactively outside the shell.

Verification: portable manifest validation, seven reconciliation/state tests and two real-rclone local transport/encryption tests. Live host checks still required: plugin discovery/add, horizontal/vertical sizing, left-click management, reload/removal, keyboard accessibility, Qt imports, keyring, systemd and real Apple/iPhone transfer. No live screenshot exists.

## 0.0.2 — graphical setup and gate button, 5 October 2026

The user's XPS screenshot exposed the oversized tiled kdialog menu and terminal-only setup. Accepted change: a literal theme-coloured gate in the bar, graphical Apple credentials/2FA, cloud-folder browsing, and compact floating dialogs. Preserve the existing download-only scope and account storage.

`setup.rs` uses rclone's supported non-interactive JSON configuration questions. It preselects Drive and skips advanced settings; answers remain in child-process environment variables, never command arguments. Inherited rclone logging overrides are removed. Work happens in an encrypted staging config, with verification before replacement and cancellation preserving the prior account. Native kdialog output is private process I/O. No credentials enter hosted QML. The existing keyring remains the encryption-key owner.

The bar uses a small Canvas gate instead of a font glyph, so theme changes and both orientations use the same shape. Dialog placement addresses only the spawned dialog's PID and follows Omarchy's modern-Lua/legacy-dispatch fallback, without changing persistent Hyprland configuration. Updates can explicitly refresh a clean installer-owned plugin with `--update-plugin`.

Evidence: Rust/provider adapter tests, graphical setup fixtures (including cancellation and passwords containing spaces), real rclone 1.75.1 question negotiation up to password entry without contacting Apple, and installer upgrade fixtures. Gate rendering, focus, sizing and genuine Apple verification still need live XPS acceptance.

Deferred: status service/bar polling, hosted panel, two-way sync, Reflect visibility, Photos, Perch integration, incremental transfer and recovery pruning. Original proposed broader scope remains in companion/docs/SPEC.md.
