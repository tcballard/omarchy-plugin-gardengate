//! Management uses local state and systemd properties; status never contacts Apple.
use super::{dialog, setup, show_command};
use anyhow::{bail, Context, Result};
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
        let mut snapshot = Self {
            schema: 1,
            mode: "download-only",
            state: "unconfigured",
            profile,
            last_check,
            conflict_copies: ledger.map(|l| l.conflicts).unwrap_or_default(),
            background: ServiceState::read(WATCH),
            manual_download: ServiceState::read(MANUAL),
            operation_busy,
        };
        snapshot.state = if snapshot.manual_download.running() {
            "downloading"
        } else if snapshot.operation_busy {
            "busy"
        } else if snapshot.profile.is_none() {
            "unconfigured"
        } else if snapshot
            .last_check
            .as_ref()
            .is_some_and(|r| r.state == "failed")
        {
            "failed"
        } else if !snapshot.conflict_copies.is_empty() {
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
            "paused" => "Background downloads paused",
            _ => "Waiting for the next download",
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
                "\n\nLast download attempt: {} minutes ago\nNew files: {} · Updated: {}",
                elapsed / 60,
                r.downloaded,
                r.updated
            ));
            if let Some(error) = &r.error {
                text.push_str(&format!("\n\n{error}"));
            }
        } else {
            text.push_str("\n\nNo completed download has been recorded.");
        }
        if self.manual_download.active_state == "failed" {
            text.push_str("\n\nThe manual download job failed. Check the connection and choose Download now to retry.");
        }
        if !self.conflict_copies.is_empty() {
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
        .context("Could not control background downloads; check your desktop session")?;
    if !output.status.success() {
        bail!("Could not change the download service. Check that the companion is installed and your desktop session is running. Open Status to see its current state.");
    }
    Ok(())
}

pub fn background(paths: &Paths, enabled: bool) -> Result<()> {
    if !enabled {
        // Stop first: the watcher may hold the operation lock during a transfer.
        service_action(&["disable", "--now", WATCH])?;
        println!("Background downloads stopped and disabled at sign-in. Manual downloads remain available.");
        return Ok(());
    }
    let _lock = paths.lock()?;
    let mut p = paths.profile()?;
    if !p.approved {
        bail!("Complete Download now successfully before enabling background downloads.");
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
    println!("Background downloads enabled. They also start when you sign in.");
    Ok(())
}

pub fn manage(paths: &Paths) -> Result<()> {
    if paths.optional_profile()?.is_none() && !setup::wizard(paths)? {
        return Ok(());
    }
    loop {
        let snapshot = Snapshot::read(paths)?;
        let title = format!(
            "Garden Gate · developer preview\n{}\nDownload-only: iPhone → XPS",
            snapshot.summary()
        );
        let Some(action) = dialog(&[
            "--menu",
            &title,
            "pull",
            "Download now",
            "status",
            "Status and last download",
            "open",
            "Open local inbox",
            "recovery",
            "Open recovery copies",
            "background-on",
            "Enable or resume background downloads",
            "background-off",
            "Turn off background downloads",
            "preview",
            "Preview download",
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
            "open" => show_command(&["open"]),
            "recovery" => show_command(&["recovery"]),
            "background-on" => {
                if dialog(&["--yesno", "Enable background downloads?\n\nGarden Gate will check this folder about every minute and start again when you sign in. The first Download now must have completed successfully."])?.is_none() { continue; }
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
                if dialog(&["--yesno", "Download this folder now?\n\nLocal edits are preserved and competing cloud copies go to recovery storage. No cloud writes or deletions. A successful download lets you enable background downloads separately."])?.is_none() { continue; }
                service_action(&["start", "--no-block", MANUAL]).and_then(|_| {
                    dialog(&[
                        "--msgbox",
                        "Download queued. Open Status to check completion or errors.",
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
