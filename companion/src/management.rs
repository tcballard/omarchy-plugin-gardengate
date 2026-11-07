//! Management uses local state and systemd properties; status never contacts Apple.
use super::{dialog, setup, show_command};
use anyhow::{bail, Context, Result};
use gardengate::two_way::{Phase, SyncState};
use gardengate::*;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    process::{Command, Stdio},
};

const WATCH: &str = "gardengate.service";
const MANUAL: &str = "gardengate-pull.service";

#[derive(Serialize)]
pub struct ServiceState {
    load_state: String,
    active_state: String,
    unit_file_state: String,
}

impl ServiceState {
    fn read(unit: &str) -> Self {
        let output = Command::new("systemctl")
            .args([
                "--user",
                "show",
                unit,
                "--property=LoadState,ActiveState,UnitFileState",
                "--no-pager",
            ])
            .stdin(Stdio::null())
            .output();
        let mut state = Self {
            load_state: "unavailable".into(),
            active_state: "unknown".into(),
            unit_file_state: "unknown".into(),
        };
        if let Ok(output) = output {
            if output.status.success() {
                for line in String::from_utf8_lossy(&output.stdout).lines() {
                    if let Some((key, value)) = line.split_once('=') {
                        match key {
                            "LoadState" => state.load_state = value.into(),
                            "ActiveState" => state.active_state = value.into(),
                            "UnitFileState" => state.unit_file_state = value.into(),
                            _ => (),
                        }
                    }
                }
            }
        }
        state
    }

    fn running(&self) -> bool {
        matches!(
            self.active_state.as_str(),
            "active" | "activating" | "reloading"
        )
    }

    fn label(&self) -> &'static str {
        if self.load_state != "loaded" {
            return "Unavailable — check the companion installation and desktop session";
        }
        match self.active_state.as_str() {
            "failed" => "Failed — try starting it again",
            "deactivating" => "Stopping",
            "active" | "activating" | "reloading" if self.unit_file_state == "enabled" => {
                "On — starts when you sign in"
            }
            "active" | "activating" | "reloading" => "Running for this session",
            "inactive" if self.unit_file_state == "enabled" => "Stopped — still enabled at sign-in",
            "inactive" => "Off",
            _ => "Unknown",
        }
    }
}

#[derive(Serialize)]
pub struct Snapshot {
    schema: u32,
    mode: &'static str,
    state: &'static str,
    profile: Option<Profile>,
    last_check: Option<Report>,
    conflict_copies: BTreeMap<String, String>,
    background: ServiceState,
    manual_download: ServiceState,
    operation_busy: bool,
    two_way: Option<SyncState>,
    local_recovery: String,
    cloud_recovery: Option<String>,
}

impl Snapshot {
    pub fn read(paths: &Paths) -> Result<Self> {
        let profile = paths.optional_profile()?;
        let last_check: Option<Report> = read_optional_json(&paths.state.join("status.json"))?;
        let ledger: Option<Ledger> = read_optional_json(&paths.state.join("ledger.json"))?;
        if last_check.as_ref().is_some_and(|r| r.schema != 1)
            || ledger.as_ref().is_some_and(|l| l.schema != 1)
        {
            bail!("Unsupported saved status schema; existing data is unchanged");
        }
        let operation_busy = match paths.lock() {
            Ok(_lock) => false,
            Err(e)
                if e.downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::WouldBlock) =>
            {
                true
            }
            Err(e) => return Err(e),
        };
        let two_way = SyncState::read(paths)?;
        let mode = profile.as_ref().map(|p| p.mode).unwrap_or_default();
        let cloud_recovery = two_way.as_ref().map(SyncState::remote_recovery);
        let mut snapshot = Self {
            schema: 1,
            mode: mode.label(),
            state: "unconfigured",
            profile,
            last_check,
            conflict_copies: ledger.map(|l| l.conflicts).unwrap_or_default(),
            background: ServiceState::read(WATCH),
            manual_download: ServiceState::read(MANUAL),
            operation_busy,
            two_way,
            local_recovery: paths.state.to_string_lossy().into_owned(),
            cloud_recovery,
        };
        snapshot.state = if snapshot.manual_download.running() {
            if mode == SyncMode::TwoWay {
                "syncing"
            } else {
                "downloading"
            }
        } else if snapshot.operation_busy {
            "busy"
        } else if snapshot.profile.is_none() {
            "unconfigured"
        } else if mode == SyncMode::TwoWay
            && snapshot
                .two_way
                .as_ref()
                .is_none_or(|s| s.phase != Phase::Ready)
        {
            "recovery-required"
        } else if snapshot
            .last_check
            .as_ref()
            .is_some_and(|r| r.state == "failed")
        {
            "failed"
        } else if (mode == SyncMode::DownloadOnly && !snapshot.conflict_copies.is_empty())
            || (mode == SyncMode::TwoWay
                && snapshot
                    .two_way
                    .as_ref()
                    .is_some_and(|s| !s.conflicts.is_empty()))
        {
            "needs-review"
        } else if !snapshot.profile.as_ref().unwrap().approved {
            "approval-required"
        } else if snapshot.profile.as_ref().unwrap().paused {
            "paused"
        } else {
            "idle"
        };
        Ok(snapshot)
    }

    fn summary(&self) -> String {
        let state = match self.state {
            "downloading" => "Download running or queued",
            "busy" => "Setup or download in progress",
            "unconfigured" => "Setup required",
            "failed" => "Last download failed — open Status for details",
            "needs-review" => "Competing file versions need review",
            "approval-required" => "First download required — choose Download now",
            "paused" => "Background transfers paused",
            _ => "Waiting for the next download",
        };
        let state = match self.state {
            "syncing" => "Two-way sync running or queued",
            "recovery-required" => {
                "Sync stopped — review both folders, then choose Rebuild two-way history"
            }
            _ => state,
        };
        let state = if self.mode == "two-way" {
            state
                .replace("download", "sync")
                .replace("Download", "Sync")
        } else {
            state.to_owned()
        };
        format!("{state}\nBackground service: {}", self.background.label())
    }

    fn details(&self) -> String {
        let mut text = self.summary();
        if let Some(p) = &self.profile {
            text.push_str(&format!(
                "\n\niCloud folder: {}\nLocal inbox: {}",
                p.remote_folder,
                p.local.display()
            ));
        }
        if let Some(r) = &self.last_check {
            let elapsed = now().saturating_sub(r.checked_at);
            text.push_str(&format!(
                "\n\nLast {} attempt: {} minutes ago",
                if self.mode == "two-way" {
                    "sync"
                } else {
                    "download"
                },
                elapsed / 60
            ));
            if self.mode == "download-only" {
                text.push_str(&format!(
                    "\nNew files: {} · Updated: {}",
                    r.downloaded, r.updated
                ));
            }
            if let Some(error) = &r.error {
                text.push_str(&format!("\n\n{error}"));
            }
        } else {
            text.push_str("\n\nNo completed download has been recorded.");
        }
        if self.manual_download.active_state == "failed" {
            text.push_str("\n\nThe manual download job failed. Check the connection and choose Download now to retry.");
        }
        if self.mode == "two-way" {
            text.push_str("\n\nMode: two-way. Edits and deletions propagate in both directions.");
            if let Some(s) = &self.two_way {
                if !s.conflicts.is_empty() {
                    text.push_str(
                        "\n\nBoth conflicting versions are in the local/iCloud folder:\n",
                    );
                    text.push_str(&s.conflicts.join("\n"));
                }
                if let Some(error) = &s.last_error {
                    text.push_str(&format!("\n\n{error}"));
                }
            }
            if let Some(cloud) = &self.cloud_recovery {
                text.push_str(&format!("\n\niCloud recovery folder: {cloud}"));
            }
            text = text.replace("download", "sync").replace("Download", "Sync");
        }
        if self.mode == "download-only" && !self.conflict_copies.is_empty() {
            text.push_str(&format!("\n\n{} competing cloud copies are preserved. Local edits are unchanged.\nChoose Open recovery copies to inspect them.", self.conflict_copies.len()));
        }
        text
    }
}

fn service_action(args: &[&str]) -> Result<()> {
    let output = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .context("Could not control background transfers; check your desktop session")?;
    if !output.status.success() {
        bail!("Could not change the download service. Check that the companion is installed and your desktop session is running. Open Status to see its current state.");
    }
    Ok(())
}

pub fn background(paths: &Paths, enabled: bool) -> Result<()> {
    if !enabled {
        // Stop first: the watcher may hold the operation lock during a transfer.
        service_action(&["disable", "--now", WATCH])?;
        println!("Background transfers stopped and disabled at sign-in. Manual transfers remain available.");
        return Ok(());
    }
    let _lock = paths.lock()?;
    let mut p = paths.profile()?;
    if !p.approved {
        bail!("Complete the first transfer in the selected mode before enabling background transfers.");
    }
    if p.mode == SyncMode::TwoWay && SyncState::read(paths)?.is_none_or(|s| s.phase != Phase::Ready)
    {
        bail!(
            "Review and recover the interrupted two-way sync before enabling background transfers."
        );
    }
    paths.encrypted_config()?;
    let was_paused = p.paused;
    p.paused = false;
    paths.save_profile(&p)?;
    if let Err(error) = service_action(&["enable", "--now", WATCH]) {
        p.paused = was_paused;
        paths.save_profile(&p)?;
        return Err(error);
    }
    println!("Background transfers enabled. They also start when you sign in.");
    Ok(())
}

pub fn manage(paths: &Paths) -> Result<()> {
    if paths.optional_profile()?.is_none() && !setup::wizard(paths)? {
        return Ok(());
    }
    loop {
        let snapshot = Snapshot::read(paths)?;
        let title = format!(
            "Garden Gate · developer preview\n{}\n{}",
            snapshot.summary(),
            if snapshot.mode == "two-way" {
                "Two-way: iCloud ↔ XPS (including deletions)"
            } else {
                "Download-only: iPhone → XPS"
            }
        );
        let Some(action) = dialog(&[
            "--menu",
            &title,
            "pull",
            if snapshot.mode == "two-way" {
                "Sync now"
            } else {
                "Download now"
            },
            "status",
            "Status and last transfer",
            "open",
            "Open local inbox",
            "recovery",
            "Open recovery copies",
            "background-on",
            "Enable or resume background transfers",
            "background-off",
            "Turn off background transfers",
            "preview",
            "Check current sync plan",
            "mode",
            "Sync mode and recovery…",
            "setup",
            "Connect or reconnect Apple Account",
            "add",
            "Choose iCloud folder",
        ])?
        else {
            break;
        };
        let result = match action.as_str() {
            "setup" => setup::connect_gui(paths).map(|_| ()),
            "add" => setup::choose_folder(paths).map(|_| ()),
            "status" => Snapshot::read(paths)
                .and_then(|current| dialog(&["--msgbox", &current.details()]).map(|_| ())),
            "preview" => show_command(&["plan"]),
            "mode" => manage_mode(),
            "open" => show_command(&["open"]),
            "recovery" => show_command(&["recovery"]),
            "background-on" => {
                if dialog(&["--yesno", "Enable background transfers?\n\nGarden Gate will check this folder about every minute and start again when you sign in. The first transfer in the selected mode must have completed successfully."])?.is_none() { continue; }
                show_command(&["background", "on"])
            }
            "background-off" => show_command(&["background", "off"]),
            "pull" => {
                let snapshot = Snapshot::read(paths)?;
                if snapshot.operation_busy || snapshot.manual_download.running() {
                    dialog(&[
                        "--msgbox",
                        "Setup or a download is already running. Open Status to check it.",
                    ])?;
                    continue;
                }
                if dialog(&["--yesno", if snapshot.mode == "two-way" {
                    "Sync this folder now?\n\nEdits and deletions propagate in both directions. Conflicting versions and recovery copies are retained."
                } else {
                    "Download this folder now?\n\nLocal edits are preserved and competing cloud copies go to recovery storage. No cloud writes or deletions. A successful download lets you enable background transfers separately."
                }])?.is_none() { continue; }
                service_action(&["start", "--no-block", MANUAL]).and_then(|_| {
                    dialog(&[
                        "--msgbox",
                        "Transfer queued. Open Status to check completion or errors.",
                    ])
                    .map(|_| ())
                })
            }
            _ => continue,
        };
        if let Err(error) = result {
            dialog(&["--error", &error.to_string()])?;
        }
    }
    Ok(())
}

fn manage_mode() -> Result<()> {
    let Some(action) = dialog(&[
        "--menu",
        "Sync mode and recovery",
        "two-way-plan",
        "Check two-way setup",
        "two-way-enable",
        "Enable two-way sync…",
        "two-way-recover",
        "Rebuild two-way history…",
        "two-way-disable",
        "Switch to download-only…",
    ])?
    else {
        return Ok(());
    };
    match action.as_str() {
        "two-way-plan" => show_command(&["two-way", "plan"]),
        "two-way-enable" | "two-way-recover" => {
            let recover = action == "two-way-recover";
            if dialog(&["--yesno", if recover {
                    "Rebuild two-way history?\n\nReview both folders first and stop editing on all devices. Files present on either side will be copied to the other, so deleted files may return. Different versions with the same name block rebuilding. Old history and recovery copies are retained. Background transfers remain paused afterwards."
                } else {
                    "Enable two-way sync?\n\nLocal edits will upload to iCloud. Edits and deletions will propagate in both directions. Conflicts preserve both versions; replaced/deleted files go to recovery folders. Stop editing during setup. Different versions with the same name block setup. Background transfers remain paused until you resume them."
                }])?.is_none() { return Ok(()); }
            show_command(&[
                "two-way",
                if recover { "recover" } else { "enable" },
                "--apply",
            ])
        }
        "two-way-disable" => {
            if dialog(&["--yesno", "Switch to download-only and pause background transfers?\n\nLocal files, two-way history and recovery copies are retained. Re-enabling two-way mode requires an explicit history rebuild."])?.is_none() { return Ok(()); }
            show_command(&["two-way", "disable"])
        }

        _ => Ok(()),
    }
}
