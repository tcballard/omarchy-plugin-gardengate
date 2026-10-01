# Garden Gate implementation record

Accepted identity: Garden Gate, `io.github.tcballard.gardengate`, repository `tcballard/omarchy-plugin-gardengate`. Hosted kind: `bar-widget`, root `BarWidget.qml`. Scope: iPhone iCloud Drive → XPS download-only developer preview.

The widget launches the management menu in the separate Rust companion. It owns no authentication, transfer loop or persistent sync state. No extra Quickshell process is started. Ordinary files, encrypted account configuration, operation locks, ledger and recovery storage belong to the companion. The existing Rust adapter remains rclone-based. No socket IPC exists yet.

The launcher is intentionally static: it does not imply an authenticated, online or syncing state. The management menu and CLI show actual companion state. Installation of dependencies and services is an explicit external operation, never a plugin hook. Credentials are configured interactively outside the shell.

Verification: portable manifest validation, seven reconciliation/state tests and two real-rclone local transport/encryption tests. Live host checks still required: plugin discovery/add, horizontal/vertical sizing, left-click management, reload/removal, keyboard accessibility, Qt imports, keyring, systemd and real Apple/iPhone transfer. No live screenshot exists.

Deferred: status service/bar polling, hosted panel, graphical Apple sign-in, two-way sync, Reflect visibility, Photos, Perch integration, incremental transfer and recovery pruning. Original proposed broader scope remains in companion/docs/SPEC.md.
