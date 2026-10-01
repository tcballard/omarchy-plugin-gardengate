# Architecture and implementation scope

One Rust executable owns setup, profile validation, local reconciliation, process lifetime and persistence. rclone is an external adapter; no Apple API is reimplemented. The prototype deliberately implements the immediate iPhone → XPS workflow as a download-only slice rather than prematurely enabling bisync.

All setup, mapping and download mutations take a process-wide advisory operation lock. A second lock prevents duplicate watchers. The service owns its rclone child, bounds transfers to five minutes, handles SIGINT/SIGTERM, kills/reaps before releasing locks, and backs off failures. systemd KillMode=control-group handles forced termination. There is no Unix socket/event stream yet: management invokes typed CLI commands, and manual background downloads use a systemd oneshot service.

Config is schema-versioned JSON plus a dedicated encrypted rclone config under XDG_CONFIG_HOME/gardengate. Its encryption key lives in Secret Service. Rclone receives a static password-command referring to secret-tool; secret values do not enter argument lists. The shell surface never reads credentials. rclone's interactive account setup owns authentication. Native setup and fully classified authentication errors remain future work.

XDG_STATE_HOME/gardengate contains status, ledger, locks, staging, backups and conflict copies. JSON is written to a same-directory private temporary file, fsynced, renamed and followed by parent-directory fsync. Corrupt/newer schemas stop mutating operations; they are not silently rebuilt. Interrupted downloads cannot feed a partial staging tree into reconciliation. A crash during local reconciliation can leave already-applied files with an older ledger; the next scan recognises equal hashes or preserves a conflict instead of discarding local content.

The inbox is user-owned data outside application state. It is never deleted on uninstall. Local version hashes permit preservation of edits; competing cloud versions are saved privately. A final check detects most concurrent editor writes but there is no filesystem-level compare-and-swap for replacement, so a narrow concurrent-write race remains. This is a developer-preview limit, not a lossless-sync claim.

Qt kdialog provides a small mouse-friendly management menu, not the planned full Qt Quick application. Its use and systemd actions are documented but not visually/live-tested on Omarchy. The root shell plugin ships a static bar launcher for this menu. Live toolkit and service integration must be validated before adding a hosted panel, sync-status widget or Perch adapter.

Next: real account sign-in + phone download evidence; incremental cloud staging; native setup; structured conflict resolution; then deliberate two-way engine integration and live conflict tests. M0 remains blocked on account evidence; M1 is only partially implemented. The proposed spec is retained unchanged in docs/SPEC.md.
