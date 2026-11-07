# XPS two-way acceptance (development only)

This runner supplements the existing Rust/real-rclone tests; it does not replace
or reproduce the sync engine. Run as your normal Omarchy desktop user. Never run
as root. No release decision follows from a portable pass.

## Build and reuse existing checks

From a clean checkout of `feat/two-way-sync`:

```bash
git fetch origin
git switch feat/two-way-sync
git pull --ff-only
./tests/run
cd companion
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
RCLONE_TEST_BIN="$(command -v rclone)" cargo test --locked -- --include-ignored
cargo build --locked --release
cd ..
```

Install exactly rclone **1.75.1** before the real transport/live tests. The CI
workflow already specifies the official download and SHA-256; use that pin if
Arch's packaged version differs. The runner and companion both use `rclone` from
PATH. The existing 30 Rust tests (11 real-rclone scenarios) remain the authority
for portable semantics, including deletion bursts, name collisions, cancellation
and management. Python guard tests run in `./tests/run` and require no account.

## Authentication and read-only cloud checks

Use the normal installed Garden Gate connection flow first if not connected.
This is a manual acceptance step: record sign-in, 2FA, ADP/trust approval and
locked/unlocked keyring outcomes without recording credentials. Do not reconnect
or replace a working production connection just for this harness. `new` copies
only the existing encrypted config; it never copies a production profile or sync
history, stores a key, or invokes authentication. It uses the existing keyring
lookup without changing it. Authentication may refresh provider session tokens.

Exact commands from the repository root:

```bash
RUN=$(python3 companion/acceptance/xps.py new --binary "$PWD/companion/target/release/gardengate")
printf 'Acceptance run: %s\n' "$RUN"
python3 companion/acceptance/xps.py preflight --id "$RUN"
python3 companion/acceptance/xps.py desktop --id "$RUN"
```

`new` writes only a fresh private `~/GardenGate-Acceptance/<random-id>` directory.
`preflight` reads provider version and cloud directory names; it does not modify
Drive data. `desktop` only queries native command/service availability. Neither
means the graphical menu or service integration has passed.

There is deliberately no folder override and no option to adopt an existing
folder. The name is `GardenGate-Acceptance-<random-id>`. Initial cloud presence,
nonempty local destination, changed profile mappings, mismatched owner markers,
symlink paths, changed binary and reused runs are refused. Cloud APIs lack atomic
exclusive directory creation: an external actor deliberately creating the exact
random name between checks cannot be excluded transactionally. Keep all other
editors/services away from this disposable namespace.

## Live writes, phone round trips, interruption and recovery

```bash
python3 companion/acceptance/xps.py run --id "$RUN"
```

Review the displayed destination. Type `WRITE <id>` to approve cloud writes,
then follow the iPhone Files instructions. Use a text editor capable of saving
UTF-8 without a trailing newline. The runner checks exact bytes, so a newline is
a failure, not silently normalised. Phone upload delays can cause a failed check;
wait until Files finishes uploading. A failure stops the sequence; do not rerun
`run` on partial state. Inspect evidence and start a new run if appropriate.

The sequence checks:

| Step | Evidence and expectation |
| --- | --- |
| Initial union | Both seed files on both endpoints; background paused |
| Phone and XPS edits | Exact v2 bytes; original contents in recovery storage |
| Renames | New paths on both sides, old paths absent |
| Deletions | Separate `DELETE <id>` approval before rename/deletion phase; removed files absent and contents preserved in recovery |
| Concurrent edits | Two recorded conflict paths, both distinct contents on both sides |
| Interrupted sync | Separate `INTERRUPT <id>` approval; SIGTERM when durable state enters running; nonzero result, non-ready state, next sync refused |

This is a controlled signal interruption, not proof of a dropped network midway
through an Apple transaction. A transfer that finishes too fast is INCONCLUSIVE.
The runner never rewrites the engine state to manufacture a recovery result.

After interruption, inspect `evidence/`, `events.jsonl` and retained state. Close
all editors. Recovery is a separate command and separate approval:

```bash
python3 companion/acceptance/xps.py evidence --id "$RUN"
python3 companion/acceptance/xps.py recover --id "$RUN"
```

Type `RECOVER <id>` only after reviewing both endpoints. Recovery can resurrect
deleted files. It must retain the union, old bisync generation and paused profile.
If divergent names block recovery, preserve both versions manually in the test
folder, record that operation, and inspect again before approving recovery.

## Physical network and provider failures

After successful recovery:

```bash
python3 companion/acceptance/xps.py network --id "$RUN"
```

`NETWORK <id>` approves this scenario. Follow the offline/online prompts. This
checks a real unavailable provider, unchanged local data, unchanged cloud after
reconnection, and records whether explicit recovery is required. A network that
was still reachable is inconclusive. The companion can take several minutes to
time out. Ctrl-C terminates the process group, retains evidence and stops.
No recovery or sync retry follows automatically.

Mid-transfer Wi-Fi loss, expired tokens, Apple throttling, quota exhaustion and
server outages remain separate **NOT RUN** cases until deliberately observed.
Do not fill an Apple account or revoke production credentials to simulate them.
Record the actual cause and phase, not merely a generic failure. A pre-transfer
network failure is not evidence of partial-write recovery.

## Native Omarchy service and menu acceptance

The runner intentionally never starts `gardengate.service`, enables a watcher,
or opens management: those fixed production unit names can operate on a real
profile despite the harness's isolated XDG environment. Read-only `desktop`
results do not validate native integration.

For full installed-unit/menu acceptance use a **separate disposable desktop Unix
user on the XPS**, with its own home, unlocked Secret Service and Garden Gate
installation. Connect Apple interactively in that user, run the harness there,
and use only its generated folder. Do not run the following in the production
user. This portion requires manual review and a fresh explicit approval before
starting any service or accepting a menu action that writes to iCloud.

In that disposable user's terminal, after the runner's recovery succeeds:

```bash
export XDG_CONFIG_HOME="$HOME/GardenGate-Acceptance/$RUN/config"
export XDG_STATE_HOME="$HOME/GardenGate-Acceptance/$RUN/state"
# These values must also reach the user service manager before starting units.
systemctl --user import-environment XDG_CONFIG_HOME XDG_STATE_HOME
companion/target/release/gardengate manage
```

Confirm the menu displays the generated test mapping, paused two-way mode and
correct recovery paths. Cancel each destructive dialog once and verify no file
changes. Record the installed companion hash (it must match the tested binary).
Then explicitly approve a background transfer in the menu, edit a disposable
file, verify it on iPhone, stop background transfers, and confirm both unit state
and file stability. Test manual transfer cancellation and a locked keyring; do
not save or publish journal output containing credentials. Re-run `evidence`
after stopping services. The harness refuses unexpectedly unpaused profiles.

Record: Omarchy/Hyprland version, binary hash, service unit paths, pass/fail,
expected/actual file hashes, cancellation and keyring behaviour, and sanitized
screenshots. A separate user is required because importing XDG values into the
production user manager can redirect unrelated applications. Log out of the
disposable session after testing. This procedure is **manual and unperformed**
until those observations exist.

## Evidence, failure handling and cleanup

Each command has a start/end event. Mutations have before/after snapshots where
possible. Snapshots contain local/cloud file bytes, SHA-256 manifests, conflict
paths, phase and recovery copies. Provider stderr is withheld; do not put Apple
passwords, tokens, encrypted configs or keyring contents in an issue. Share only
reviewed `events.jsonl` and `evidence/`, never the whole run directory. Evidence is
private but can contain anything accidentally placed in the disposable folder.

A failed command, assertion, timeout or declined approval stops immediately.
No retry, rollback, cleanup or automatic resync follows. If the provider is down,
a final cloud snapshot may be unavailable; the missing evidence is not a pass.
The 32 MiB evidence download limit is intentionally below the engine's 512 MiB
per-side limit. Keep fixtures small; recovery storage grows without pruning.

No automatic cleanup is provided. After reviewing and retaining evidence,
manually remove **only** the exact generated folder and its
`.gardengate-recovery-<engine-id>` sibling shown in the saved state. Cleanup is a
separate user-approved deletion, outside acceptance. Never select the Drive root.
Nontransactional provider writes and simultaneous editing remain known risks.
