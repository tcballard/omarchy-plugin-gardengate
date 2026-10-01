# Garden Gate for Omarchy

<img src="https://raw.githubusercontent.com/tcballard/omarchy-badges/75975e5b5bf75e7ede3764bcd2950046f7abfe2c/badges/v1/omarchy-app.svg" alt="Omarchy community app" height="20">

Download files from a selected iCloud Drive folder into an ordinary local inbox on your Linux desktop.

**0.0.1 developer preview: download-only, not the completed spec.** Rust CLI and background watcher; a basic Qt management menu via kdialog. No Python backend. Setup uses rclone's terminal flow. Two-way sync, full management UI, live-tested bar/Perch integration, Reflect app-container compatibility and Photos are deferred. The community badge is an identity label, not official approval.

## Install on the XPS

From the repository checkout, open a terminal inside `companion/`:

```bash
sudo pacman -S --needed rclone libsecret kdialog
bash scripts/install.sh
```

This source checkout builds the companion with Cargo. Install Rust/Cargo before running the installer. rclone 1.75.1+ is required; 1.75.1 was tested here. Your desktop Secret Service must be running and unlocked. No credential or service is created/enabled by the installer. Intended target: Omarchy 4 / Hyprland; **live Omarchy versions tested: none**.

## Connect your iPhone

On the iPhone, open **Files → Browse → iCloud Drive**, and create **Omarchy Inbox**. Put a small test document there. For content from another app use **Share → Save to Files**, then choose this folder. This does not synchronise the Apple Notes database, local “On My iPhone” files, or the Photos library automatically.

On the XPS:

```bash
~/.local/bin/gardengate connect
```

The app creates an encrypted dedicated rclone config. Its unlock key lives in the desktop keyring. In rclone's setup choose:

- `n` for a new remote named `icloud`.
- Storage type `iclouddrive` (type the word, not a guessed menu number).
- Service `drive` (the default).
- Your regular Apple Account credentials; complete 2FA/trusted-device approval.
- Keep config encryption enabled. Do not switch the remote to Photos.
- `q` when finished.

This is an unofficial iCloud connection. Upstream currently documents 30-day trust tokens and support for Advanced Data Protection when web access and trusted-device approval are available. Account behaviour still requires a live test. Do not disable ADP to make the preview work.

List accessible folders, select the folder and preview its size:

```bash
~/.local/bin/gardengate folders
~/.local/bin/gardengate add "Omarchy Inbox" "$HOME/iCloud Drive/Omarchy Inbox"
~/.local/bin/gardengate plan
```

The initial destination must be empty. Inspect the preview, then approve the download:

```bash
~/.local/bin/gardengate pull --apply
~/.local/bin/gardengate open
```

Your test document should now be in the XPS inbox. That first successful transfer approves future background downloads. Enable the watcher only after this succeeds:

```bash
systemctl --user enable --now gardengate.service
```

The preview checks at approximately 60-second intervals, measured after the previous cycle; failures back off. Apple propagation can take longer. It stages the whole selected folder each cycle, with a 512 MiB transfer cap. **Use a small test folder, not your entire Drive.** Optimised incremental transfer is future work.

Open **Garden Gate (Preview)** in the launcher, or run:

```bash
~/.local/bin/gardengate manage
```

The Qt menu offers folder selection, preview, background download, status, pause/resume and open-inbox actions. Apple sign-in instructions lead to the terminal; there is no graphical credential entry yet. The management menu and live systemd/keyring interaction are **NOT RUN on Omarchy**.

## File behaviour

- Never uploads to or deletes from iCloud.
- Cloud deletion leaves downloaded local files intact. Rename appears as a new download; the previous local path remains.
- Changed cloud content updates a previously downloaded local file if it still matches the recorded downloaded version. The replaced version is backed up.
- Local edits stay local. If both sides differ, the cloud version is saved as a separate recovery copy and status becomes `needs-review`.
- `status` shows preserved conflict-copy paths. Open/copy these manually; an integrated conflict resolver is deferred. To converge manually, copy the preserved cloud contents into the local file, or keep the local edit and accept the unresolved record for now.
- `.reflect/`, `.git/` and `.DS_Store` are excluded. Symlinks/special files and incoming case collisions are rejected.
- Local files remain usable offline; failure never reconciles a partially downloaded stage.
- Backups/conflict copies have no automatic cleanup in this preview. Monitor disk usage yourself.

```bash
~/.local/bin/gardengate status
~/.local/bin/gardengate pause
~/.local/bin/gardengate resume
systemctl --user status gardengate.service
```

A pause request waits for an active transfer's operation lock; stop the service to interrupt it immediately. Signal handling kills/reaps an active rclone child and retains local files. No realtime collaboration guarantee: a file editor writing at the exact replacement boundary can still race with reconciliation. Test with disposable data and avoid editing an inbox file during its download.

## Reconnect and remove

If credentials expire, rerun `connect` and edit the existing `icloud` remote. rclone's interactive reconnection behaviour must be tested on the real account; do not reset/delete cloud folders to fix authentication.

Uninstall:

```bash
bash scripts/uninstall.sh
```

Local inbox, encrypted config, keyring entry and recovery files are retained. To forget the account, first stop both services, then deliberately remove the dedicated `rclone.conf` and run `secret-tool clear application gardengate`. This is separate from deleting the inbox or recovery data. No live account data is included in the archive.

## Development

```bash
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release
RCLONE_TEST_BIN=/absolute/path/to/rclone cargo test --locked --test transport -- --include-ignored
```

Portable validation: 7 core tests and 2 real-rclone integration tests run here using rclone 1.75.1. Integration endpoints were disposable local directories, **not iCloud**. See [VERIFICATION.md](VERIFICATION.md), [ARCHITECTURE.md](ARCHITECTURE.md) and the [full proposed spec](docs/SPEC.md).
