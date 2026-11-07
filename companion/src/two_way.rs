//! Opt-in bisync orchestration. Callers hold Paths::lock for the entire operation.
use crate::{
    atomic_json, hash, now, private_dir, read_optional_json, reject_symlinks, validate_folder,
    walk, Paths, Profile, Rclone, Report, SyncMode,
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};
use unicode_normalization::UnicodeNormalization;

pub const ACCESS_FILE: &str = ".gardengate-access";
const VERSION: &str = "rclone v1.75.1";
const MAX_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    Ready,
    Running,
    RecoveryRequired,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SyncState {
    pub schema: u32,
    pub id: String,
    pub generation: String,
    pub local: PathBuf,
    pub remote: String,
    pub initialized: bool,
    pub phase: Phase,
    pub conflicts: Vec<String>,
    pub last_completed_at: Option<u64>,
    pub last_error: Option<String>,
}

impl SyncState {
    pub fn read(paths: &Paths) -> Result<Option<Self>> {
        let state: Option<Self> = read_optional_json(&paths.state.join("two-way.json"))?;
        if let Some(s) = &state {
            if s.schema != 1 || !valid_id(&s.id) || !valid_id(&s.generation) {
                bail!("Unsupported or invalid two-way state; recovery files are unchanged");
            }
        }
        Ok(state)
    }

    fn save(&self, paths: &Paths) -> Result<()> {
        atomic_json(&paths.state.join("two-way.json"), self)
    }

    pub fn remote_recovery(&self) -> String {
        format!("{}.gardengate-recovery-{}", self.remote, self.id)
    }
}

fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

fn identifier() -> Result<String> {
    let mut bytes = [0u8; 16];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Enable,
    Sync,
    Recover,
}

pub struct Engine<'a> {
    paths: &'a Paths,
    profile: &'a Profile,
    rclone: &'a Rclone,
    remote: String,
}

impl<'a> Engine<'a> {
    pub fn new(paths: &'a Paths, profile: &'a Profile, rclone: &'a Rclone) -> Self {
        Self::with_remote(
            paths,
            profile,
            rclone,
            format!("icloud:{}", profile.remote_folder),
        )
    }

    /// An explicit transport endpoint allows disposable local-backend integration tests.
    pub fn with_remote(
        paths: &'a Paths,
        profile: &'a Profile,
        rclone: &'a Rclone,
        remote: String,
    ) -> Self {
        Self {
            paths,
            profile,
            rclone,
            remote,
        }
    }

    fn run(&self, args: &[&str]) -> Result<Vec<u8>> {
        self.rclone
            .run(args, &self.paths.state.join("two-way-output.tmp"))
    }

    fn state(&self) -> Result<Option<SyncState>> {
        let s = SyncState::read(self.paths)?;
        if s.as_ref()
            .is_some_and(|s| s.local != self.profile.local || s.remote != self.remote)
        {
            bail!("Two-way state belongs to a different folder; refusing to reuse its history");
        }
        Ok(s)
    }

    fn validate(&self) -> Result<()> {
        validate_folder(&self.profile.remote_folder)?;
        let local = &self.profile.local;
        if !local.is_absolute() || local.parent() == Some(Path::new("/")) || !local.is_dir() {
            bail!("The configured local folder is missing or invalid; restore it before syncing");
        }
        for owned in [&self.paths.config, &self.paths.state] {
            if local.starts_with(owned) || owned.starts_with(local) {
                bail!("Sync folder overlaps application state");
            }
        }
        reject_symlinks(local)?;
        let version = self.run(&["version"])?;
        if String::from_utf8_lossy(&version).lines().next() != Some(VERSION) {
            bail!("Two-way sync requires the tested rclone 1.75.1; download-only mode remains available");
        }
        Ok(())
    }

    fn preflight(&self, action: Action) -> Result<()> {
        self.validate()?;
        let local_files = walk(&self.profile.local)?;
        check_size(&self.profile.local, &local_files)?;
        let stage = self.paths.state.join("two-way-preflight");
        reject_symlinks(&stage)?;
        if stage.exists() {
            fs::remove_dir_all(&stage)?;
        }
        private_dir(&stage)?;
        let result: Result<()> = (|| {
            self.rclone.download(
                &self.remote,
                &stage,
                &self.paths.state.join("two-way-output.tmp"),
            )?;
            let remote_files = walk(&stage)?;
            check_size(&stage, &remote_files)?;
            validate_names(local_files.iter().chain(&remote_files))?;
            if action != Action::Sync {
                for rel in &remote_files {
                    if rel == Path::new(ACCESS_FILE)
                        || rel.file_name().is_some_and(|n| n == ".DS_Store")
                    {
                        continue;
                    }
                    let local = self.profile.local.join(rel);
                    if local.is_file() && hash(&local)? != hash(&stage.join(rel))? {
                        bail!("Versions differ at {}. Before enabling or rebuilding two-way sync, rename one version or make both copies identical. Neither version has been changed.", rel.display());
                    }
                }
            }
            Ok(())
        })();
        let _ = fs::remove_dir_all(stage);
        result
    }

    pub fn preview(&self) -> Result<String> {
        let s = self.state()?;
        let action = if self.profile.mode == SyncMode::TwoWay
            && s.as_ref().is_some_and(|s| s.phase == Phase::Ready)
        {
            Action::Sync
        } else {
            Action::Enable
        };
        self.preflight(action)?;
        Ok(format!("iCloud folder: {}\nLocal folder: {}\n\nTwo-way mode uploads and downloads edits and propagates deletions. Conflicts keep both versions. Replaced/deleted files go to local recovery storage and a sibling iCloud recovery folder. Deletion bursts above 50% stop for review.\n\n{}\nPause editing on all devices during setup/recovery. This checks folder access, names, size and initial conflicts; it is not an exact transaction preview. No cloud files were changed.", self.profile.remote_folder, self.profile.local.display(), if action == Action::Sync { "The next run uses the existing sync history." } else { "Initialization/recovery copies the union of both folders, so previously deleted files may return." }))
    }

    pub fn execute(&self, action: Action) -> Result<Report> {
        let result = self.execute_inner(action);
        if let Err(error) = &result {
            let report = Report {
                schema: 1,
                state: "failed".into(),
                checked_at: now(),
                error: Some(error.to_string()),
                ..Default::default()
            };
            let _ = atomic_json(&self.paths.state.join("status.json"), &report);
        }
        result
    }

    fn execute_inner(&self, action: Action) -> Result<Report> {
        let existing = self.state()?;
        match action {
            Action::Enable if self.profile.mode == SyncMode::TwoWay => bail!("Two-way mode is already selected. Use sync --apply, or two-way recover --apply after reviewing an interrupted run."),
            Action::Enable if existing.is_some() => bail!("Two-way history already exists. Use two-way recover --apply to rebuild it after reviewing both folders."),
            Action::Sync if self.profile.mode != SyncMode::TwoWay => bail!("Enable two-way sync explicitly first"),
            _ => (),
        }
        let mut s = match existing {
            Some(s) => s,
            None if action == Action::Enable => SyncState {
                schema: 1, id: identifier()?, generation: identifier()?,
                local: self.profile.local.clone(), remote: self.remote.clone(), initialized: false,
                phase: Phase::Running, conflicts: Vec::new(), last_completed_at: None, last_error: None,
            },
            None => bail!("Two-way history is missing; restore two-way.json and the bisync directory from backup. Automatic initialization is refused."),
        };
        if action == Action::Sync && s.phase != Phase::Ready {
            bail!("The previous sync did not complete. Review both folders, then use two-way recover --apply. No automatic resync was attempted.");
        }
        self.preflight(action)?;
        if s.initialized {
            self.check_access(&s)?;
        }
        if action == Action::Recover {
            s.generation = identifier()?;
        }
        s.phase = Phase::Running;
        s.last_error = None;
        s.save(self.paths)?;
        let mut p = self.profile.clone();
        if action != Action::Sync {
            // Schema 2 makes older download-only binaries fail closed after the handoff.
            p.schema = 2;
            p.mode = SyncMode::TwoWay;
            p.approved = false;
            p.paused = true;
            self.paths.save_profile(&p)?;
        }
        let result: Result<Report> = (|| {
            if !s.initialized {
                self.initialize_access(&s)?;
            }
            self.check_access(&s)?;
            let work = self.paths.state.join("bisync").join(&s.generation);
            private_dir(&work)?;
            let run = identifier()?;
            let backup = self.paths.state.join("recovery").join(&s.id).join(&run);
            private_dir(&backup)?;
            let remote_backup = format!("{}/{}", s.remote_recovery(), run);
            let mut args = vec![
                "bisync",
                utf8(&p.local)?,
                &self.remote,
                "--workdir",
                utf8(&work)?,
                "--backup-dir1",
                utf8(&backup)?,
                "--backup-dir2",
                &remote_backup,
                "--check-access",
                "--check-filename",
                ACCESS_FILE,
                "--check-sync",
                "true",
                "--compare",
                "size,checksum",
                "--download-hash",
                "--conflict-resolve",
                "none",
                "--conflict-loser",
                "num",
                "--conflict-suffix",
                "gardengate-local,gardengate-cloud",
                "--max-delete",
                "50",
                "--max-delete-size",
                "128Mi",
                "--transfers",
                "1",
                "--checkers",
                "1",
                "--max-duration",
                "4m",
                "--max-transfer",
                "512Mi",
                "--exclude",
                ".reflect/**",
                "--exclude",
                ".git/**",
                "--exclude",
                ".DS_Store",
            ];
            if action != Action::Sync {
                args.push("--resync");
            }
            self.run(&args)?;
            self.check_access(&s)?;
            s.conflicts = walk(&p.local)?
                .into_iter()
                .filter(|p| {
                    p.file_name().is_some_and(|n| {
                        let n = n.to_string_lossy();
                        n.contains(".gardengate-local") || n.contains(".gardengate-cloud")
                    })
                })
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            s.initialized = true;
            s.phase = Phase::Ready;
            s.last_completed_at = Some(now());
            s.save(self.paths)?;
            if action != Action::Sync {
                p.approved = true;
                self.paths.save_profile(&p)?;
            }
            let report = Report {
                schema: 1,
                state: if s.conflicts.is_empty() {
                    "checked"
                } else {
                    "needs-review"
                }
                .into(),
                checked_at: now(),
                conflicts: s.conflicts.len() as u64,
                ..Default::default()
            };
            atomic_json(&self.paths.state.join("status.json"), &report)?;
            Ok(report)
        })();
        if let Err(error) = &result {
            s.phase = Phase::RecoveryRequired;
            s.last_error = Some(format!(
                "{error}. Review both folders before two-way recover --apply."
            ));
            let _ = s.save(self.paths);
        }
        result
    }

    fn initialize_access(&self, s: &SyncState) -> Result<()> {
        let marker = self.profile.local.join(ACCESS_FILE);
        reject_symlinks(&marker)?;
        if marker.exists() {
            if fs::read(&marker)? != s.id.as_bytes() {
                bail!("Local access marker belongs to another sync mapping");
            }
        } else {
            let mut f = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&marker)?;
            f.write_all(s.id.as_bytes())?;
            f.sync_all()?;
        }
        let target = format!("{}/{ACCESS_FILE}", self.remote);
        self.run(&[
            "copyto",
            utf8(&marker)?,
            &target,
            "--immutable",
            "--checksum",
        ])?;
        Ok(())
    }

    fn check_access(&self, s: &SyncState) -> Result<()> {
        let marker = self.profile.local.join(ACCESS_FILE);
        reject_symlinks(&marker)?;
        if fs::read(&marker).context("Local access marker is missing; restore it before syncing")?
            != s.id.as_bytes()
        {
            bail!("Local access marker mismatch; refusing to sync");
        }
        let remote_marker = self.run(&[
            "cat",
            &format!("{}/{ACCESS_FILE}", self.remote),
            "--count",
            "33",
        ])?;
        if remote_marker != s.id.as_bytes() {
            bail!("Cloud access marker mismatch; check the connected account and folder");
        }
        Ok(())
    }
}

fn utf8(path: &Path) -> Result<&str> {
    path.to_str().context("Non-UTF8 path unsupported")
}

fn check_size(root: &Path, files: &[PathBuf]) -> Result<()> {
    let mut total = 0u64;
    for rel in files {
        total = total
            .checked_add(fs::metadata(root.join(rel))?.len())
            .context("Folder size overflow")?;
        if total > MAX_BYTES {
            bail!("Two-way preview supports folders up to 512 MiB per side");
        }
    }
    Ok(())
}

fn validate_names<'a>(files: impl Iterator<Item = &'a PathBuf>) -> Result<()> {
    let mut names = BTreeMap::new();
    for file in files {
        let mut path = file.as_path();
        let mut directory = false;
        while !path.as_os_str().is_empty() {
            let name = utf8(path)?;
            let folded = name.nfc().collect::<String>().to_lowercase();
            if let Some((previous, was_directory)) =
                names.insert(folded, (name.to_owned(), directory))
            {
                if previous != name || was_directory != directory {
                    bail!("Case, Unicode or file/directory collision: {previous} and {name}");
                }
            }
            path = path.parent().context("Invalid relative file path")?;
            directory = true;
        }
    }
    Ok(())
}
