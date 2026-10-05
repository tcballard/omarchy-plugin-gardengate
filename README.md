# Garden Gate

[![Built for Omarchy: Plugin](https://raw.githubusercontent.com/tcballard/omarchy-badges/75975e5b5bf75e7ede3764bcd2950046f7abfe2c/badges/v1/omarchy-plugin.svg)](https://github.com/tcballard/omarchy-badges)

Bring files from your iPhone's iCloud Drive to an ordinary folder on your Omarchy desktop.

**0.0.2 developer preview.** Garden Gate is a hosted Quattro bar widget plus a Rust companion. Click the widget to open the Qt management menu. The companion downloads a selected folder, preserves local edits and keeps recovery copies. It never uploads to or deletes from iCloud.

## Features

- Open management from a theme-coloured gate icon on horizontal or vertical bars.
- Connect Apple and complete verification in desktop dialogs, then browse iCloud folders.
- Download one selected iCloud Drive folder into a local inbox.
- Preserve local edits, replaced versions and competing cloud copies.
- Pause/resume optional background downloads and open the inbox.

Two-way sync, Apple Notes/Photos, Reflect container support, live bar status and conflict-resolution UI are deferred.

## Requirements

- Omarchy 4 with Quattro shell-plugin support, on x86_64 Linux.
- rclone 1.75.1+ (transport tests used 1.75.1).
- libsecret with a running, unlocked desktop Secret Service.
- kdialog for the Qt management menu and systemd user services for background downloads.
- Python 3 (included with Omarchy) to run the prebuilt installer. Rust/Cargo are only needed by developers building from source.
- An Apple Account with iCloud Drive and access to its sign-in/2FA flow.

## Installation

### Prebuilt developer preview (no compiler needed)

Download `gardengate-0.0.2-linux-x86_64.run` from the **Prebuilt installer** workflow artifact or the provided test download. In a terminal, run it as your normal desktop user:

```bash
bash ~/Downloads/gardengate-0.0.2-linux-x86_64.run
```

The installer includes the compiled companion and the exact plugin source. It asks sudo to install missing runtime packages (`rclone`, `libsecret`, `kdialog`, `git`, `xdg-utils`), then installs per-user files and adds/enables the bar plugin through Omarchy's plugin manager. It never downloads a compiler. Python 3, systemd user services, glibc 2.39+ and x86_64 Linux are required. Source and embedded-file hashes are recorded in the build manifest; `--check` verifies the embedded payload without installing.

An existing bar plugin is left unchanged unless you pass `--update-plugin`. That option updates only a clean checkout at a revision recorded by the prebuilt installer; local changes, extra files and unowned revisions are refused. Unmanaged or locally modified companion files also block installation. If you installed an earlier source preview, remove that companion with its original uninstaller first; your inbox, credentials and recovery copies are retained. On upgrade, stop both `gardengate.service` and `gardengate-pull.service` first. The installer leaves them stopped. Use `--no-plugin` to install only the companion.

The installer does not sign into Apple or enable background downloads. Continue with **Connect your iPhone** below. This is a test artifact, not a live-validated public release.

### Build from source

The plugin manager installs the shell widget. Install its companion separately; plugin add does not build code, install packages or sign into Apple.

```bash
git clone https://github.com/tcballard/omarchy-plugin-gardengate.git
cd omarchy-plugin-gardengate
sudo pacman -S --needed rust rclone libsecret kdialog
bash companion/scripts/install.sh
omarchy plugin add "$(pwd)" --enable
```

## Usage

Add Garden Gate to your bar through your shell's widget configuration. The widget ID is `io.github.tcballard.gardengate`. A drawn gate replaces the text label in both orientations and follows the bar's foreground colour. Left-click opens management; the icon does not indicate sync status.

### Connect your iPhone

Create **Omarchy Inbox** under **Files → iCloud Drive** on your iPhone and save a small test document there. Click the gate icon on your XPS, or run:

```bash
~/.local/bin/gardengate manage
```

First launch opens the setup wizard. Choose **Connect Apple Account**, enter your Apple email/password, and approve sign-in on your iPhone. Enter the verification code in the next dialog. Remote naming, iCloud Drive selection and advanced defaults are handled automatically. Choose **Already connected — choose a folder** if you completed sign-in earlier.

Browse to **Omarchy Inbox**, choose **Download this folder**, and accept or change the local destination. Review the plan, then choose **Download now** from management. The new dialogs request a compact floating position on the focused dialog's monitor, with modern and legacy Hyprland dispatch support. These window requests still need an XPS visual check.

Setup edits an encrypted staging configuration and commits it only after verifying folder access. Cancelling keeps the previous connection. Passwords and answers travel to the rclone child through its environment, not process arguments or shell history; setup disables inherited rclone debug/log overrides. The shell widget never handles credentials.

The terminal workflow remains available for troubleshooting:

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

### Background downloads

After checking the first transfer, optionally enable background downloads:

```bash
systemctl --user enable --now gardengate.service
```

The first successful manual download approves background transfers. The watcher checks around 60 seconds after each cycle and backs off on errors. This preview stages the whole selected folder each cycle, with a 512 MiB cap. Start with disposable files in a small folder.

[Companion instructions](companion/README.md) explain account setup, conflicts, pause/resume, limitations and removal.

## Architecture

| Component | Responsibility |
| --- | --- |
| `BarWidget.qml` | Hosted launcher; starts `~/.local/bin/gardengate manage` with an argument vector |
| `companion/` | Rust CLI, reconciliation, recovery and watcher |
| rclone 1.75.1+ | Unofficial iCloud Drive transport and Apple authentication |
| Secret Service / libsecret | Encrypted config unlock key in the desktop keyring |
| kdialog | Guided sign-in, folder browser and management dialogs |
| systemd user services | Optional watcher and manual download job |

## Security and data

Plugin code runs unsandboxed in the user's shell. The widget does not read credentials or contact Apple. The companion talks to iCloud through rclone and its dedicated encrypted config. Durable data is under `$XDG_CONFIG_HOME/gardengate` and `$XDG_STATE_HOME/gardengate` (usual XDG defaults); local downloads are at the selected path. Installing does not enable a service or create credentials.

This name introduces separate `gardengate` state, services and keyring identity. An earlier `omarchy-icloud` preview is not automatically migrated. Stop/uninstall that preview before using Garden Gate with the same inbox; retain its recovery files.

## Compatibility

Intended for Omarchy 4 with Quattro plugin support, x86_64 Linux. The user's XPS screenshot confirms the earlier management menu launched, but was tiled too large. No exact live Omarchy revision is recorded. The new wizard, gate icon and floating-window behaviour still need live acceptance. Real Apple login, iPhone → XPS transfer, keyring and systemd are not yet verified. `preview.svg` is a labelled design placeholder, not a screenshot.

## Development and validation

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

See [verification](companion/VERIFICATION.md) for reproduced and unrun checks, and [design](DESIGN.md) for the implementation record.

## Update

For a prebuilt installation, download and run the new installer after stopping the two Garden Gate services. It replaces only unchanged files recorded by the previous prebuilt installer. Restart the background service afterwards only if you had already enabled it.

Update the shell widget:

```bash
omarchy plugin update io.github.tcballard.gardengate
```

To update the companion, pull the source checkout and rerun its installer:

```bash
git pull --ff-only
bash companion/scripts/install.sh
```

Restart `gardengate.service` afterwards if you have enabled it.

## Removal

```bash
omarchy plugin remove io.github.tcballard.gardengate
```

Removing the widget leaves the companion and its services installed. To remove those too, run `bash companion/scripts/uninstall.sh` from the checkout. Inbox files, encrypted config, keyring entry and recovery copies are retained.

For a **prebuilt** installation, remove the companion using its ownership-aware uninstaller instead:

```bash
python3 ~/.local/share/gardengate/installer/uninstall.py --uninstall
```

This refuses to delete modified installed files. The bar plugin is removed separately with the command above. Inbox files, credentials and recovery copies are retained.

## License

[MIT](LICENSE) © 2026 Tom Ballard. [Third-party notices](companion/THIRD-PARTY-NOTICES.md).

Recommended repository topics: `omarchy`, `omarchy-plugin`, `icloud`, `rust`.
