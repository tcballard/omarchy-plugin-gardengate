# Omarchy Garden Gate — product and technical specification

Version: 0.1 · 30 September 2026 · Status: proposed design; implementation and account testing have not started.

## 1. Product decision

Give Omarchy users a dependable local copy of selected iCloud Drive folders, with background two-way sync and a clear recovery experience. Files remain ordinary files that any application can open. The first use case is sharing notes and documents with a Mac and iPhone; Reflect is a compatibility target subject to a separate access test.

Working title: **Garden Gate for Omarchy**. This is an unofficial community product; a final public name and repository are not selected. Do not imply Apple or Reflect endorsement.

Use rclone as the cloud transport and initial folder-sync engine, supervised by a Rust user service. Deliver one installable application package with a management window and CLI, plus an optional thin Omarchy bar/panel plugin. Perch can expose the same status and actions through an adapter once its integration contract is verified.

## 2. Scope and boundaries

### First usable release

- One Apple Account with password, 2FA and trusted-device approval where required.
- Browse accessible iCloud Drive folders and map selected folders to local directories.
- Keep selected content locally, usable without network access.
- Two-way folder sync, manual refresh, pause/resume and startup recovery.
- First-sync preview, preserved conflict versions, deletion recovery and meaningful errors.
- Management window, CLI, desktop notifications and optional themed bar/panel entry.
- An evidence report that distinguishes general Drive support from Reflect compatibility.

### Deferred

iCloud Photos, Contacts, Calendar, Keychain/password sync, Find My, Mail, multi-account support, whole-drive default syncing, shared-folder guarantees, on-demand FUSE mounts, collaboration, automatic Markdown merging, and a Reflect application port. None are implied by the first release.

## 3. Evidence and unresolved dependencies

Rclone documents an iCloud Drive backend, added in v1.69. Its setup uses the regular Apple Account password and 2FA, not an app-specific password. It documents a 30-day trust-token lifetime. Current documentation supports Advanced Data Protection with web access enabled and possible trusted-device approval. These are upstream claims requiring live verification with the packaged version. [S1]

Bisync supports two-way transfers, conflict copies, recovery and backup directories. It is an advanced command with concurrent-edit limitations; it does not implement document collaboration or semantic Markdown merging. Do not advertise lossless simultaneous editing merely because bisync completes. [S2]

Reflect documents an app iCloud container and Apple-specific conflict handling. Its container visibility through rclone is **unknown**. Another sync client explicitly ignores app-library objects, illustrating why ordinary Drive access is insufficient evidence. [S4–S5]

**Release gate:** select and pin a tested rclone version rather than assuming v1.69 provides every required feature. Verify its licence and distribution notices, authentication integration, folder visibility, conflict naming and recovery semantics before committing the adapter contract.

## 4. Primary workflows

### Connect

1. Explain that this is an unofficial iCloud connection using rclone, requiring the normal Apple Account password and 2FA.
2. Enter account credentials through a native setup surface. Keep secrets out of shell arguments, URLs, logs and shell-plugin state.
3. Complete 2FA or trusted-device approval. Present waiting, rejected, expired and successful states separately.
4. Test folder listing without changing cloud files.
5. Show only capabilities demonstrated by the account and installed backend. Explain web-access or approval requirements without instructing users to disable Advanced Data Protection.

The setup adapter must use a tested structured rclone configuration flow where available. Do not implement a fragile parser of interactive English prompts. If this cannot be made reliable, a documented terminal setup is acceptable for the developer preview, but blocks a consumer-ready release.

### Add a folder

Choose a remote folder and local destination. Default to a new dedicated directory beneath ~/iCloud Drive/. Never auto-map the entire home directory or entire cloud drive. Reject overlapping mappings, filesystem roots, symlink escapes, service-state directories and paths already managed by another sync profile.

Preview the number and total size of discovered files, available local disk space, unsupported names and existing destination content. A new empty destination is the default. An existing destination requires a plan that preserves both sides; never silently select “newer wins.” Show transfers, replacements and deletions separately. First bootstrap performs no deletions. The user accepts the initial plan before transfers begin.

### Daily use

Edit a file with an ordinary application. Local changes schedule a sync after a short settling period; cloud changes are discovered through polling. Show last successful reconciliation and pending work. Closing the management window or restarting the shell does not stop the sync service.

“Up to date” means both sides agreed at the last successful check, not that Apple has delivered the file to every other device.

### Resolve a conflict

Show file, device/side labels, detection time and both preserved versions. Actions: keep local, keep iCloud, keep both, or open copies to merge manually. Text preview is available for small plain-text files; binary files use version details and external open actions. A resolution is invalidated if either version changes before application. Do not overwrite a fresh edit based on stale review state.

### Pause, disconnect and remove

Pause prevents new jobs and safely stops the active job at a recoverable boundary; report any already-completed transfers. Disconnect stops work, removes managed credentials and retains downloaded files. Plugin removal only removes the shell surface; daemon/package removal stops sync and retains user files and recovery copies. Data erasure is a distinct explicit operation.

## 5. Surfaces and interaction

### Management window

Rust backend; proposed Qt Quick/QML frontend for Linux/Wayland, subject to a small toolkit and accessibility validation spike. Main navigation: Folders, Activity, Needs attention, Settings. The default view shows folder cards with status, last check, local location, queued work and pause/open actions. A detail view shows recent transfers and recovery versions.

Mouse-complete flows are mandatory. Provide keyboard navigation, visible focus, accessible names and text alongside status icons. Colour is never the sole error signal. Read Omarchy theme inputs through a dedicated adapter; apply readable fallback colours when inputs are unavailable. Toolkit versions and bindings are selected during the spike, not asserted here as tested.

### Bar and panel

Proposed plugin ID: io.github.tcballard.gardengate, pending repository/public identity selection. Kinds: bar-widget and panel, with hosted QML Item entry points. No independent ShellRoot or second Quickshell instance. No cloud polling in each widget instance.

Bar: compact status icon with optional pending count. Panel: overall state, recent check, folder summaries, Sync now, Pause, Open local folder and Manage. Multiple monitors share one service. Only transient selection/expanded rows are per-instance. Shell configuration contains presentation preferences only.

Perch: use the same status snapshot and actions; do not fork scheduling or authentication logic. Until its extension contract is inspected, this is an integration target, not a promised API.

## 6. Architecture and ownership

| Component | Responsibility |
| --- | --- |
| Rust domain modules | Profile validation, states, scheduling policy, recovery records and user commands |
| Rust daemon | Own jobs, profile locks, subprocesses, durable state and notifications |
| rclone adapter | Authenticate, list, transfer and invoke tested bisync operations; classify errors |
| Management UI / CLI | Issue typed commands and present service state |
| Omarchy plugin / Perch adapter | Read sanitised state and invoke allowlisted actions |

Use a systemd user service, tentatively gardengate.service, running as the signed-in user with no sudo during ordinary use. Shell restart leaves it running. Do not enable systemd lingering by default.

IPC: a Unix socket under $XDG_RUNTIME_DIR/gardengate/, restricted to the user. Versioned request/response protocol plus event subscription. Commands include status, profiles.list, sync.now, sync.pause, sync.resume, conflicts.list and conflicts.resolve. Profile CRUD and authentication are restricted to the management client paths by explicit API design; never expose credentials to hosted shell UI. This separation reduces exposure but is not a security boundary against arbitrary processes running as the same user.

Use opaque profile IDs; clients cannot submit arbitrary command text or rclone flags. Resolve open-folder actions from validated stored paths. Execute tools using argument arrays, never shell interpolation. Bound event queues and list pagination; a disconnected UI cannot block transfers.

Durable settings: $XDG_CONFIG_HOME/gardengate/. Job journal, bisync state and recovery records: $XDG_STATE_HOME/gardengate/. Temporary downloads: private local staging. Keep all state outside synced roots. For a tested Reflect profile exclude .reflect/ and .git/; ordinary folder exclusions remain explicit and reviewable.

## 7. Sync engine and recovery rules

Use rclone bisync for the initial engine, behind an adapter so it can be replaced without rewriting the product. It is file-level sync. Do not claim that it calls Apple's native NSFileVersion conflict interface.

- Start with one active reconciliation per account; coalesce additional requests. This reduces overlapping writes and rate pressure.
- Proposed remote polling interval: 60 seconds while enabled, adjustable with a conservative minimum. Remote changes may arrive later because Apple propagation and listing speed vary. Back off on errors/rate limits; do not equate the timer with a delivery guarantee.
- Local events trigger a debounced cycle; a periodic scan catches missed watcher events. Event triggers never bypass profile locks.
- Keep a durable operation ID and last completed state. Cancellation reaps child processes before releasing locks. Crash recovery uses tested engine recovery; never automatically issue --resync after an unexplained error.
- Default conflicts preserve both versions. Translate verified engine outputs into a conflict record; do not choose a winner from timestamps alone.
- Preserve replaced/deleted versions outside the sync root, with configurable retention and an initial 30-day policy. Notify before storage pressure forces cleanup; never silently discard unresolved conflicts.
- Block deletion when a root is unavailable, auth fails, listings are incomplete, or profile identity changes. Require access probes and previous-state checks. Pause abnormal deletion bursts for review; establish thresholds from test workloads and make normal large intentional deletions reviewable.
- Do not assume remote compare-and-swap or transactional writes. Establish what rclone/iCloud actually guarantees. If races can silently overwrite intervening changes, preserve versions where possible and document the residual limit; this blocks a strong data-safety guarantee.
- Validate case and Unicode collisions before syncing; explain files Linux can represent but iCloud cannot. Reject symlinks and special files initially. Treat cross-profile moves as separate changes.
- Disk-full, quota-full, partial upload and network interruption must preserve the local source and resumable state. Never report success for partial completion.

Avoid mixing a second independent reconciliation algorithm with bisync's private state. Reflect conflict integration and three-way Markdown merging are later work, requiring an explicit engine handoff and stable base-version storage.

## 8. Credentials and network

Own a dedicated rclone configuration; do not silently edit the user's existing remotes. Optional import is explicit and must avoid unexpectedly moving credentials.

Encrypt managed rclone configuration at rest. Store its unlock secret through Linux Secret Service; rclone “obscure” alone is not secure encryption. Validate secret delivery without command-line exposure and support locked/unavailable keyring as a user-visible state. Never silently fall back to plaintext. Token refresh persists encrypted configuration atomically. Full account password/session persistence follows the tested backend requirements and is explained during setup. [S3]

Network traffic goes through rclone to Apple authentication and iCloud service endpoints discovered by the backend. No project-hosted proxy, telemetry or account service. Do not pin an incomplete hostname list as a compatibility promise. Keep TLS verification enabled. Diagnostics redact passwords, tokens, cookies, Apple Account identifiers and file contents; filenames/paths are removed from shareable diagnostics by default. No privileged operations beyond explicit package installation.

## 9. States and performance

Profile states: unconfigured, connecting, approval-required, scanning, syncing, up-to-date, paused, offline, sign-in-required, needs-review, unsupported and failed. Preserve structured reasons such as quota-full, disk-full, dependency-missing and recovery-required. Aggregate status prioritises action-needed states without hiding healthy folders.

UI remains responsive during listing and transfers; process/network work runs outside the UI thread. Set measured budgets during the spike using representative small notes and large document folders. Stream/paginate activity; do not load file bodies or the entire drive tree into the shell. Idle service sleeps between events/checks. Record memory, CPU, scan duration and transfer throughput before setting release claims.

## 10. Implementation milestones

| Milestone | Deliverable and exit evidence |
| --- | --- |
| M0: feasibility | Tested rclone version, account setup, ordinary folder CRUD, ADP assessment and Reflect container visibility report using disposable data |
| M1: vertical slice | Rust service + CLI, one empty local folder, download/upload, manual sync, offline editing, restart recovery and preserved conflicts |
| M2: usable product | Management setup, folder preview, credential storage, activity, pause, recovery and desktop integration |
| M3: Omarchy integration | Theme adaptation, multi-monitor bar/panel, service lifecycle, deterministic demo; Perch adapter if contract is available |
| M4: release candidate | Arch package, clean install/update/remove tests, live Mac/iPhone interoperability and full failure matrix |

If Reflect's folder is inaccessible, M0 may still approve general Drive development. Record Reflect support as blocked; investigate upstream folder selection or backend app-library access independently. Never redirect a user's live Reflect folder without a reviewed migration plan.

## 11. Acceptance and evidence

Automated tests cover path containment/overlap, state transitions, job coalescing, secret redaction, crash journals, subprocess cancellation and conflict-resolution freshness. Engine integration uses disposable local endpoints where possible; those tests do not prove iCloud compatibility.

Live account acceptance uses an isolated cloud folder and Mac/iPhone files:

1. Initial download with matching contents; initial local/cloud collision preserves both.
2. Linux edit arrives on Apple device; Apple edit arrives locally.
3. Simultaneous and offline edits preserve competing versions; resolution round-trips.
4. Rename, delete, edit-vs-delete, empty directories and Unicode/case collisions.
5. Network interruption during upload/download; restart midway; expired auth and ADP approval timeout.
6. Local disk full, cloud quota full, missing root, incomplete listing and deletion burst.
7. Shell restart, window close, multi-monitor widgets and service update without duplicate jobs.
8. Disconnect and uninstall retain local files and documented recovery data.
9. Reflect folder listing and a disposable graph round-trip, if accessible; native conflict-version behaviour assessed separately.

Each result is PASS, FAIL, BLOCKED or NOT RUN, with rclone/package versions and date. Current status: **all implementation and live tests NOT RUN**. Never use synthetic fixtures as evidence of actual Apple interoperability.

Demo fixture states: up-to-date, syncing, offline, sign-in-required and two-version conflict. Fixtures make no cloud requests. Capture management, panel and conflict screenshots on a live Omarchy desktop before release.

## 12. Sources checked 30 September 2026

- S1: https://rclone.org/iclouddrive/ — authentication, session lifetime, ADP and backend capabilities.
- S2: https://rclone.org/bisync/ — conflicts, recovery, backups and concurrency limitations.
- S3: https://rclone.org/docs/#configuration-encryption and https://rclone.org/rc/ — configuration protection and integration surfaces; exact adapter calls still require implementation validation.
- S4: https://github.com/team-reflect/reflect-open/blob/master/docs/plans/21-icloud-drive-sync.md — app container and native conflict design.
- S5: https://github.com/gordonaspin/icloudds — app-library exclusions.
- S6: https://rclone.org/licence/ — review required when packaging/distributing dependencies.

These sources establish candidate mechanisms. The architecture, UI, scheduling defaults and release gates above are proposed product decisions, not claims of already-working integration.
