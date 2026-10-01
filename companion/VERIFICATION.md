# Verification — 1 October 2026

Scope: download-only developer preview 0.0.1. Source identity: the Git commit containing this file; no compiled binary is committed. Spec remains proposed broader scope, not an implementation claim.

## Garden Gate adaptation

Renamed companion binary, XDG state, keyring identity, services and desktop launcher to gardengate. Re-ran formatting, strict Clippy, all nine tests and the release build after the rename. Root plugin passes generated checks and toolkit manifest validation. Qt/QML imports and live widget activation remain NOT RUN. Advisory security scanning reports remote-git-execution-unpinned for the documented source clone/build flow; installer and service capabilities are explicit external operations, never plugin-add hooks. This is not a marketplace approval or security certification.

## Reproduced now

Environment: x86_64 Ubuntu 24.04 container; Linux 6.18.44. Rust 1.98.1, Cargo stable toolchain. rclone v1.75.1 downloaded from downloads.rclone.org; ZIP SHA-256 matched that release's upstream SHA256SUMS. No Apple Account was connected.

| Check | Result |
| --- | --- |
| cargo clippy --locked --all-targets -- -D warnings | PASS |
| cargo fmt --check | PASS |
| cargo test --locked (7 core tests) | PASS |
| RCLONE_TEST_BIN=... cargo test --locked --test transport -- --include-ignored (2 integration tests) | PASS |
| cargo build --locked --release | PASS |
| Release binary --version / --help | PASS |
| bash -n scripts/install.sh scripts/uninstall.sh | PASS |

Core tests reproduce: safe remote path validation; symlink escape rejection; initial download, cloud update and backup; local edit preservation; competing cloud version retention; cloud deletion retaining local file; case-collision preflight; durable schema round-trip and newer-schema rejection; advisory operation locking.

Real-rclone integration reproduces: downloads between disposable local endpoints, excluded app state, conflicting changes retained without cloud writes, nonexistent-remote failure leaving local data intact. Encryption command test creates an encrypted fixture configuration and verifies decryption using a password command. It does not test a real desktop keyring, Apple password, 2FA or ADP.

## Failed during development, then corrected

Initial build needed an explicit Result type annotation. Strict Clippy flagged a redundant match, replaced by is_ok(). Upstream checksum verification initially failed on a blank checksum-file line, corrected in the verification script; ZIP checksum subsequently matched. These were development checks, not released failures.

## Not run / blocked

- Apple Account sign-in, 2FA, ADP, token refresh: BLOCKED pending user-side authentication.
- iPhone iCloud Drive → XPS transfer: NOT RUN.
- Reflect app-container visibility: NOT RUN.
- Omarchy/Hyprland live acceptance, installed Omarchy version: NOT RUN / none tested.
- kdialog management menu: NOT RUN visually or on a live desktop.
- Secret Service store/lookup/locked-keyring lifecycle: NOT RUN against a live keyring.
- systemd user enable/start/stop and actual installer/uninstaller: NOT RUN live; shell syntax checked only.
- Disk/quota-full, kill during reconcile, pathological Unicode names and active editor races: NOT RUN live.
- Two-way transfers, conflict resolution UI, socket IPC and Perch: NOT IMPLEMENTED. Root bar widget launches management; live behavior NOT RUN.

## Known limits

A small selected folder is downloaded to staging each cycle; this is not incremental sync. Download-only means local changes are never sent to the phone. Cloud deletions/renames are not mirrored destructively. Full recovery-copy retention has no automatic pruning. Status errors from rclone are generic and intentionally omit raw provider logs. Source replacement has a residual concurrent-editor race; use disposable notes/documents for the first test.

## Next live acceptance

Create Omarchy Inbox in iPhone Files/iCloud Drive and save one test document. Connect the account on the XPS, list folders, choose an empty local inbox, review plan, and run pull --apply. Check contents. Change the file on iPhone, download again and check the old local version was backed up. Change both sides and verify local content plus separate cloud recovery copy. Only then enable the background watcher for continued testing.
