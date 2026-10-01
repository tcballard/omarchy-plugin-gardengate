# Garden Gate

[![CI](https://github.com/tcballard/omarchy-plugin-gardengate/actions/workflows/test.yml/badge.svg)](https://github.com/tcballard/omarchy-plugin-gardengate/actions/workflows/test.yml) [![MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE) [![Built for Omarchy: Plugin](https://raw.githubusercontent.com/tcballard/omarchy-badges/75975e5b5bf75e7ede3764bcd2950046f7abfe2c/badges/v1/omarchy-plugin.svg)](https://github.com/tcballard/omarchy-badges)

Bring files from your iPhone's iCloud Drive to an ordinary folder on your Omarchy desktop.

**0.0.1 developer preview.** Garden Gate is a hosted Quattro bar widget plus a Rust companion. Click the widget to open the Qt management menu. The companion downloads a selected folder, preserves local edits and keeps recovery copies. It never uploads to or deletes from iCloud.

## Install and connect

The plugin manager installs the shell widget. Install its companion separately; plugin add does not build code, install packages or sign into Apple.

```bash
git clone https://github.com/tcballard/omarchy-plugin-gardengate.git
cd omarchy-plugin-gardengate
sudo pacman -S --needed rust rclone libsecret kdialog
bash companion/scripts/install.sh
omarchy plugin add "$(pwd)" --enable
```

Add Garden Gate to your bar through your shell's widget configuration. The widget ID is `io.github.tcballard.gardengate`. It uses a compact `GG` label on vertical bars. Left-click opens management; the button does not indicate sync status.

Create **Omarchy Inbox** under **Files → iCloud Drive** on your iPhone and save a small test document there. Connect on the XPS:

```bash
~/.local/bin/gardengate connect
```

In rclone choose a new remote named `icloud`, storage `iclouddrive`, service `drive`. Complete Apple sign-in and 2FA; retain config encryption. Then:

```bash
~/.local/bin/gardengate add "Omarchy Inbox" "$HOME/iCloud Drive/Omarchy Inbox"
~/.local/bin/gardengate plan
~/.local/bin/gardengate pull --apply
~/.local/bin/gardengate open
```

After checking the first transfer, optionally enable background downloads:

```bash
systemctl --user enable --now gardengate.service
```

The first successful manual download approves background transfers. The watcher checks around 60 seconds after each cycle and backs off on errors. This preview stages the whole selected folder each cycle, with a 512 MiB cap. Start with disposable files in a small folder.

[Companion instructions](companion/README.md) explain account setup, conflicts, pause/resume, limitations and removal.

## Ownership and dependencies

| Component | Responsibility |
| --- | --- |
| `BarWidget.qml` | Hosted launcher; starts `~/.local/bin/gardengate manage` with an argument vector |
| `companion/` | Rust CLI, reconciliation, recovery and watcher |
| rclone 1.75.1+ | Unofficial iCloud Drive transport and Apple authentication |
| Secret Service / libsecret | Encrypted config unlock key in the desktop keyring |
| kdialog | Basic Qt management menu |
| systemd user services | Optional watcher and manual download job |

Plugin code runs unsandboxed in the user's shell. The widget does not read credentials or contact Apple. The companion talks to iCloud through rclone and its dedicated encrypted config. Durable data is under `$XDG_CONFIG_HOME/gardengate` and `$XDG_STATE_HOME/gardengate` (usual XDG defaults); local downloads are at the selected path. Installing does not enable a service or create credentials.

This name introduces separate `gardengate` state, services and keyring identity. An earlier `omarchy-icloud` preview is not automatically migrated. Stop/uninstall that preview before using Garden Gate with the same inbox; retain its recovery files.

## Compatibility and evidence

Intended for Omarchy 4 with Quattro plugin support, x86_64 Linux. **No live Omarchy revision has been tested.** Portable manifest checks and Rust tests are separate from live acceptance. Apple login, iPhone → XPS transfer, keyring, kdialog, widget launch and systemd remain untested on the real desktop. `preview.svg` is a labelled design placeholder, not a screenshot.

```bash
./tests/run
cd companion
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked --release
```

Optional transport tests use a real rclone binary with disposable local endpoints:

```bash
RCLONE_TEST_BIN=/absolute/path/to/rclone cargo test --locked --test transport -- --include-ignored
```

See [verification](companion/VERIFICATION.md) and [design](DESIGN.md). Two-way sync, Apple Notes/Photos, Reflect container support, live status widget and conflict-resolution UI are deferred.

## Update and remove

```bash
omarchy plugin update io.github.tcballard.gardengate
omarchy plugin remove io.github.tcballard.gardengate
```

Removing the widget leaves the companion and its services installed. To remove those too, run `bash companion/scripts/uninstall.sh` from the checkout. Inbox files, encrypted config, keyring entry and recovery copies are retained.

Recommended repository topics: `omarchy`, `omarchy-plugin`, `icloud`, `rust`.

MIT © 2026 Tom Ballard.
