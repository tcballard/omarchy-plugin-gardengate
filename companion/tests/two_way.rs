use anyhow::Result;
use gardengate::{
    two_way::{Action, Engine, Phase, SyncState, ACCESS_FILE},
    *,
};
use std::{
    fs,
    path::{Path, PathBuf},
};

struct Fixture {
    _temp: tempfile::TempDir,
    paths: Paths,
    profile: Profile,
    rclone: Rclone,
    cloud: PathBuf,
    remote: String,
}

impl Fixture {
    fn new() -> Result<Self> {
        let temp = tempfile::tempdir()?;
        let root = temp.path();
        let paths = Paths {
            config: root.join("config"),
            state: root.join("state"),
        };
        paths.init()?;
        let cloud = root.join("cloud");
        let local = root.join("local");
        fs::create_dir(&cloud)?;
        fs::create_dir(&local)?;
        let profile = Profile {
            schema: 1,
            remote_folder: "Inbox".into(),
            local,
            approved: true,
            paused: false,
            mode: SyncMode::DownloadOnly,
        };
        paths.save_profile(&profile)?;
        let config = paths.config.join("test.conf");
        fs::write(&config, "[test]\ntype = local\n")?;
        let rclone = Rclone {
            binary: PathBuf::from(std::env::var("RCLONE_TEST_BIN")?),
            config,
            password_command: None,
        };
        let remote = format!("test:{}", cloud.display());
        Ok(Self {
            _temp: temp,
            paths,
            profile,
            rclone,
            cloud,
            remote,
        })
    }
    fn engine(&self) -> Engine<'_> {
        Engine::with_remote(
            &self.paths,
            &self.profile,
            &self.rclone,
            self.remote.clone(),
        )
    }
    fn enable(&mut self) -> Result<()> {
        self.engine().execute(Action::Enable)?;
        self.profile = self.paths.profile()?;
        Ok(())
    }
    fn sync(&self) -> Result<Report> {
        self.engine().execute(Action::Sync)
    }
    fn read(&self, root: &Path, file: &str) -> String {
        fs::read_to_string(root.join(file)).unwrap()
    }
    fn seed(&self) -> Result<()> {
        for i in 0..10 {
            fs::write(self.cloud.join(format!("keep{i}")), "keep")?;
        }
        Ok(())
    }
    fn state(&self) -> SyncState {
        SyncState::read(&self.paths).unwrap().unwrap()
    }
    fn recovery_contains(&self, text: &str) -> Result<bool> {
        let local = self.paths.state.join("recovery");
        let cloud = PathBuf::from(
            self.state()
                .remote_recovery()
                .strip_prefix("test:")
                .unwrap(),
        );
        for root in [&local, &cloud] {
            if root.exists() {
                for p in walk(root)? {
                    if fs::read_to_string(root.join(p))? == text {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }
}

#[test]
#[ignore = "Requires pinned RCLONE_TEST_BIN"]
fn round_trip_same_size_edits_deletes_renames_and_backups() -> Result<()> {
    let mut f = Fixture::new()?;
    f.seed()?;
    fs::write(f.cloud.join("phone.txt"), "phone-original")?;
    fs::write(f.profile.local.join("laptop.txt"), "laptop-original")?;
    assert!(f
        .engine()
        .preview()?
        .contains("No cloud files were changed"));
    assert!(!f.cloud.join("laptop.txt").exists());
    assert!(SyncState::read(&f.paths)?.is_none());
    f.enable()?;
    assert!(f.profile.paused);
    assert!(f.profile.approved);
    assert_eq!(f.profile.schema, 2);
    assert_eq!(f.read(&f.cloud, "laptop.txt"), "laptop-original");
    assert_eq!(f.read(&f.profile.local, "phone.txt"), "phone-original");
    fs::write(f.profile.local.join("phone.txt"), "laptop-edited!")?;
    fs::write(f.cloud.join("laptop.txt"), "iphone-edited!!")?;
    f.sync()?;
    assert_eq!(f.read(&f.cloud, "phone.txt"), "laptop-edited!");
    assert_eq!(f.read(&f.profile.local, "laptop.txt"), "iphone-edited!!");
    assert!(f.recovery_contains("phone-original")?);
    assert!(f.recovery_contains("laptop-original")?);
    fs::remove_file(f.profile.local.join("phone.txt"))?;
    fs::rename(f.cloud.join("laptop.txt"), f.cloud.join("renamed.txt"))?;
    f.sync()?;
    assert!(!f.cloud.join("phone.txt").exists());
    assert!(!f.profile.local.join("laptop.txt").exists());
    assert_eq!(f.read(&f.profile.local, "renamed.txt"), "iphone-edited!!");
    assert!(f.recovery_contains("laptop-edited!")?);
    f.sync()?;
    assert_eq!(f.state().phase, Phase::Ready);
    Ok(())
}

#[test]
#[ignore = "Requires pinned RCLONE_TEST_BIN"]
fn conflicting_edits_preserve_both_versions_on_both_sides() -> Result<()> {
    let mut f = Fixture::new()?;
    f.seed()?;
    fs::write(f.cloud.join("note.md"), "base")?;
    f.enable()?;
    fs::write(f.cloud.join("note.md"), "phone edit")?;
    fs::write(f.profile.local.join("note.md"), "local edit")?;
    assert_eq!(f.sync()?.state, "needs-review");
    let state = f.state();
    assert_eq!(state.conflicts.len(), 2);
    for root in [&f.cloud, &f.profile.local] {
        let copies: Vec<_> = state.conflicts.iter().map(|p| f.read(root, p)).collect();
        assert!(copies.contains(&"phone edit".to_string()));
        assert!(copies.contains(&"local edit".to_string()));
    }
    f.sync()?;
    assert_eq!(f.state().conflicts.len(), 2);
    Ok(())
}

#[test]
#[ignore = "Requires pinned RCLONE_TEST_BIN"]
fn initialization_refuses_divergent_versions_and_colliding_names_without_writes() -> Result<()> {
    let mut f = Fixture::new()?;
    fs::write(f.cloud.join("note"), "phone")?;
    fs::write(f.profile.local.join("note"), "local")?;
    assert!(f.enable().is_err());
    assert_eq!(f.read(&f.cloud, "note"), "phone");
    assert_eq!(f.read(&f.profile.local, "note"), "local");
    assert!(SyncState::read(&f.paths)?.is_none());
    fs::remove_file(f.profile.local.join("note"))?;
    fs::write(f.profile.local.join("NOTE"), "local")?;
    assert!(f.enable().unwrap_err().to_string().contains("collision"));
    fs::remove_file(f.profile.local.join("NOTE"))?;
    fs::write(f.cloud.join("é.txt"), "cloud")?;
    fs::write(f.profile.local.join("e\u{301}.txt"), "local")?;
    assert!(f.enable().unwrap_err().to_string().contains("collision"));
    assert_eq!(f.paths.profile()?.mode, SyncMode::DownloadOnly);
    Ok(())
}

#[test]
#[ignore = "Requires pinned RCLONE_TEST_BIN"]
fn missing_or_replaced_access_marker_and_unavailable_root_never_delete_files() -> Result<()> {
    let mut f = Fixture::new()?;
    f.seed()?;
    f.enable()?;
    fs::remove_file(f.cloud.join(ACCESS_FILE))?;
    assert!(f.sync().is_err());
    assert!(f.profile.local.join("keep0").exists());
    fs::write(f.cloud.join(ACCESS_FILE), "different account")?;
    assert!(f
        .sync()
        .unwrap_err()
        .to_string()
        .contains("marker mismatch"));
    fs::remove_dir_all(&f.cloud)?;
    assert!(f.sync().is_err());
    assert!(f.profile.local.join("keep0").exists());
    Ok(())
}

#[test]
#[ignore = "Requires pinned RCLONE_TEST_BIN"]
fn interruption_requires_explicit_recovery_and_retains_old_history() -> Result<()> {
    let mut f = Fixture::new()?;
    f.seed()?;
    f.enable()?;
    let mut s = f.state();
    let generation = s.generation.clone();
    s.phase = Phase::Running;
    atomic_json(&f.paths.state.join("two-way.json"), &s)?;
    fs::write(f.profile.local.join("offline.txt"), "offline work")?;
    assert!(f
        .sync()
        .unwrap_err()
        .to_string()
        .contains("did not complete"));
    assert!(!f.cloud.join("offline.txt").exists());
    f.engine().execute(Action::Recover)?;
    assert!(f.paths.state.join("bisync").join(generation).exists());
    assert_eq!(f.read(&f.cloud, "offline.txt"), "offline work");
    assert_eq!(f.state().phase, Phase::Ready);
    assert!(f.paths.profile()?.paused);
    fs::remove_file(f.paths.state.join("two-way.json"))?;
    assert!(f
        .sync()
        .unwrap_err()
        .to_string()
        .contains("history is missing"));
    Ok(())
}

#[test]
#[ignore = "Requires pinned RCLONE_TEST_BIN"]
fn deletion_burst_stops_and_recovery_restores_the_union() -> Result<()> {
    let mut f = Fixture::new()?;
    f.seed()?;
    f.enable()?;
    for i in 0..9 {
        fs::remove_file(f.profile.local.join(format!("keep{i}")))?;
    }
    assert!(f.sync().is_err());
    for i in 0..10 {
        assert!(f.cloud.join(format!("keep{i}")).exists());
    }
    assert_eq!(f.state().phase, Phase::RecoveryRequired);
    assert!(f.sync().is_err());
    f.engine().execute(Action::Recover)?;
    for i in 0..10 {
        assert!(f.profile.local.join(format!("keep{i}")).exists());
    }
    Ok(())
}

#[test]
#[ignore = "Requires pinned RCLONE_TEST_BIN"]
fn exclusions_symlinks_and_changed_mapping_are_rejected_or_preserved() -> Result<()> {
    let mut f = Fixture::new()?;
    for root in [&f.cloud, &f.profile.local] {
        fs::create_dir(root.join(".git"))?;
        fs::create_dir(root.join(".reflect"))?;
    }
    fs::write(f.profile.local.join(".git/config"), "local-only")?;
    fs::write(f.cloud.join(".reflect/database"), "cloud-only")?;
    std::os::unix::fs::symlink("/tmp", f.profile.local.join("escape"))?;
    assert!(f.enable().is_err());
    fs::remove_file(f.profile.local.join("escape"))?;
    f.enable()?;
    assert!(!f.cloud.join(".git/config").exists());
    assert!(!f.profile.local.join(".reflect/database").exists());
    f.remote.push_str("-other");
    assert!(f
        .sync()
        .unwrap_err()
        .to_string()
        .contains("different folder"));
    Ok(())
}

#[test]
fn legacy_profiles_stay_download_only_and_unknown_modes_fail_closed() -> Result<()> {
    let p: Profile = serde_json::from_str(
        r#"{"schema":1,"remote_folder":"Inbox","local":"/tmp/inbox","approved":true,"paused":false}"#,
    )?;
    assert_eq!(p.mode, SyncMode::DownloadOnly);
    assert!(serde_json::from_str::<Profile>(r#"{"schema":2,"remote_folder":"Inbox","local":"/tmp/inbox","approved":true,"paused":false,"mode":"unknown"}"#).is_err());
    Ok(())
}

#[test]
#[ignore = "Requires pinned RCLONE_TEST_BIN"]
fn checksum_fallback_detects_same_length_edits_without_backend_hashes() -> Result<()> {
    let mut f = Fixture::new()?;
    fs::write(&f.rclone.config, "[test]\ntype = local\nhashes = none\n")?;
    fs::write(f.cloud.join("note"), "aaaa")?;
    f.enable()?;
    fs::write(f.profile.local.join("note"), "bbbb")?;
    f.sync()?;
    assert_eq!(f.read(&f.cloud, "note"), "bbbb");
    fs::write(f.cloud.join("note"), "cccc")?;
    f.sync()?;
    assert_eq!(f.read(&f.profile.local, "note"), "cccc");
    Ok(())
}

#[test]
#[ignore = "Requires pinned RCLONE_TEST_BIN"]
fn delete_versus_edit_preserves_the_edit() -> Result<()> {
    let mut f = Fixture::new()?;
    f.seed()?;
    fs::write(f.cloud.join("note"), "base")?;
    f.enable()?;
    fs::remove_file(f.profile.local.join("note"))?;
    fs::write(f.cloud.join("note"), "cloud survives")?;
    f.sync()?;
    assert_eq!(f.read(&f.profile.local, "note"), "cloud survives");
    fs::remove_file(f.cloud.join("note"))?;
    fs::write(f.profile.local.join("note"), "local survives")?;
    f.sync()?;
    assert_eq!(f.read(&f.cloud, "note"), "local survives");
    Ok(())
}
