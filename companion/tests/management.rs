//! Exercise the actual companion with disposable state and desktop/provider stubs.
use anyhow::Result;
use gardengate::*;
use serde_json::{json, Value};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Output},
};

struct Fixture {
    temp: tempfile::TempDir,
    paths: Paths,
}

impl Fixture {
    fn new() -> Result<Self> {
        let temp = tempfile::tempdir()?;
        let root = temp.path();
        let paths = Paths {
            config: root.join("config/gardengate"),
            state: root.join("state/gardengate"),
        };
        paths.init()?;
        fs::create_dir(root.join("bin"))?;
        for (name, source) in [
            (
                "systemctl",
                r#"#!/bin/sh
printf '%s\n' "$*" >> "$FIXTURE/systemctl.log"
if [ "$2" = show ]; then
  if [ "$3" = gardengate-pull.service ]; then
    printf 'LoadState=loaded\nActiveState=%s\nUnitFileState=static\n' "${MANUAL_STATE:-inactive}"
  elif [ -e "$FIXTURE/enabled" ]; then
    printf 'LoadState=loaded\nActiveState=active\nUnitFileState=enabled\n'
  else
    printf 'LoadState=loaded\nActiveState=inactive\nUnitFileState=disabled\n'
  fi
elif [ "$2" = enable ]; then
  [ ! -e "$FIXTURE/fail-service" ] || exit 1
  touch "$FIXTURE/enabled"
elif [ "$2" = disable ]; then
  [ ! -e "$FIXTURE/fail-service" ] || exit 1
  rm -f "$FIXTURE/enabled"
else
  exit 1
fi
"#,
            ),
            (
                "rclone",
                "#!/bin/sh\nprintf 'provider called\\n' >> \"$FIXTURE/provider.log\"\nexit 9\n",
            ),
            (
                "kdialog",
                r#"#!/bin/sh
printf '%s\n' "$*" >> "$FIXTURE/dialog.log"
case "$*" in
  *--menu*)
    if [ -e "$FIXTURE/open-status" ]; then
      rm "$FIXTURE/open-status"
      printf 'status\n'
    else exit 1; fi ;;
  *--msgbox*|*--error*) exit 0 ;;
  *) exit 2 ;;
esac
"#,
            ),
        ] {
            let file = root.join("bin").join(name);
            fs::write(&file, source)?;
            fs::set_permissions(file, fs::Permissions::from_mode(0o755))?;
        }
        Ok(Self { temp, paths })
    }

    fn command(&self, args: &[&str]) -> Command {
        let root = self.temp.path();
        let mut command = Command::new(env!("CARGO_BIN_EXE_gardengate"));
        command
            .args(args)
            .env("HOME", root)
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("XDG_STATE_HOME", root.join("state"))
            .env("FIXTURE", root)
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", root.join("bin").display()),
            )
            .env_remove("HYPRLAND_INSTANCE_SIGNATURE");
        command
    }

    fn run(&self, args: &[&str]) -> Result<Output> {
        Ok(self.command(args).output()?)
    }

    fn profile(&self, approved: bool, paused: bool) -> Result<()> {
        self.paths.save_profile(&Profile {
            schema: 1,
            remote_folder: "Inbox".into(),
            local: self.temp.path().join("inbox"),
            approved,
            paused,
        })?;
        fs::write(
            self.paths.config.join("rclone.conf"),
            "RCLONE_ENCRYPT_V0:\nfixture\n",
        )?;
        Ok(())
    }

    fn status(&self) -> Result<Value> {
        let out = self.run(&["status"])?;
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(serde_json::from_slice(&out.stdout)?)
    }
}

#[test]
fn failed_download_is_visible_without_contacting_the_provider() -> Result<()> {
    let f = Fixture::new()?;
    f.profile(true, false)?;
    atomic_json(
        &f.paths.state.join("status.json"),
        &Report {
            schema: 1,
            state: "failed".into(),
            checked_at: now(),
            error: Some("Connection unavailable".into()),
            ..Default::default()
        },
    )?;
    assert_eq!(f.status()?["state"], "failed");
    fs::write(f.temp.path().join("open-status"), "")?;
    assert!(f.run(&["manage"])?.status.success());
    let text = fs::read_to_string(f.temp.path().join("dialog.log"))?;
    assert!(text.contains("Last download failed"));
    assert!(text.contains("Connection unavailable"));
    assert!(!text.contains("Ready"));
    assert!(!f.temp.path().join("provider.log").exists());
    Ok(())
}

#[test]
fn background_requires_first_transfer_and_controls_the_actual_service() -> Result<()> {
    let f = Fixture::new()?;
    f.profile(false, true)?;
    assert!(!f.run(&["background", "on"])?.status.success());
    assert!(!f.temp.path().join("systemctl.log").exists());
    f.profile(true, true)?;
    fs::write(f.temp.path().join("fail-service"), "")?;
    assert!(!f.run(&["background", "on"])?.status.success());
    assert!(
        f.paths.profile()?.paused,
        "Failed start must preserve pause"
    );
    fs::remove_file(f.temp.path().join("fail-service"))?;
    assert!(f.run(&["background", "on"])?.status.success());
    assert!(!f.paths.profile()?.paused);
    assert_eq!(f.status()?["background"]["active_state"], "active");
    // Turning off must remain usable while a transfer owns the operation lock.
    let _lock = f.paths.lock()?;
    assert!(f.run(&["background", "off"])?.status.success());
    assert_eq!(f.status()?["background"]["unit_file_state"], "disabled");
    let calls = fs::read_to_string(f.temp.path().join("systemctl.log"))?;
    assert!(calls.contains("--user enable --now gardengate.service"));
    assert!(calls.contains("--user disable --now gardengate.service"));
    assert!(!f.temp.path().join("provider.log").exists());
    Ok(())
}

#[test]
fn status_distinguishes_setup_approval_busy_and_conflicts() -> Result<()> {
    let f = Fixture::new()?;
    assert_eq!(f.status()?["state"], "unconfigured");
    f.profile(false, false)?;
    assert_eq!(f.status()?["state"], "approval-required");
    let lock = f.paths.lock()?;
    assert_eq!(f.status()?["state"], "busy");
    drop(lock);
    let running = f
        .command(&["status"])
        .env("MANUAL_STATE", "activating")
        .output()?;
    assert_eq!(
        serde_json::from_slice::<Value>(&running.stdout)?["state"],
        "downloading"
    );
    f.profile(true, true)?;
    assert_eq!(f.status()?["state"], "paused");
    atomic_json(
        &f.paths.state.join("ledger.json"),
        &json!({"schema": 1, "files": {}, "conflicts": {"note.md": "/fixture/copy"}}),
    )?;
    assert_eq!(f.status()?["state"], "needs-review");
    // Service availability must not hide the local state or contact Apple.
    fs::write(f.temp.path().join("bin/systemctl"), "#!/bin/sh\nexit 1\n")?;
    assert_eq!(f.status()?["background"]["active_state"], "unknown");
    assert_eq!(f.status()?["state"], "needs-review");
    assert!(!f.temp.path().join("provider.log").exists());
    Ok(())
}

#[test]
fn unreadable_or_future_profile_never_reopens_setup() -> Result<()> {
    let f = Fixture::new()?;
    for content in ["broken JSON".to_string(), json!({"schema": 2, "remote_folder": "Inbox", "local": "/fixture/inbox", "approved": true, "paused": false}).to_string()] {
        let profile = f.paths.config.join("profile.json");
        fs::write(&profile, &content)?;
        assert!(!f.run(&["manage"])?.status.success());
        assert_eq!(fs::read_to_string(profile)?, content);
    }
    let dialogs = fs::read_to_string(f.temp.path().join("dialog.log"))?;
    assert!(dialogs.contains("--error"));
    assert!(!dialogs.contains("--menu"));
    assert!(!f.temp.path().join("provider.log").exists());
    Ok(())
}

#[test]
fn early_transfer_failure_replaces_stale_success_without_approving_downloads() -> Result<()> {
    let f = Fixture::new()?;
    f.profile(false, false)?;
    atomic_json(
        &f.paths.state.join("status.json"),
        &Report {
            schema: 1,
            state: "checked".into(),
            ..Default::default()
        },
    )?;
    fs::remove_file(f.paths.config.join("rclone.conf"))?;
    assert!(!f.run(&["pull", "--apply"])?.status.success());
    assert!(!f.paths.profile()?.approved);
    assert_eq!(f.status()?["last_check"]["state"], "failed");
    assert!(!f.temp.path().join("provider.log").exists());
    assert!(!Path::new(&f.paths.state.join("staging")).exists());
    Ok(())
}
