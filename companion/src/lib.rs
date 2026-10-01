use anyhow::{bail, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub static STOP: AtomicBool = AtomicBool::new(false);
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub schema: u32,
    pub remote_folder: String,
    pub local: PathBuf,
    pub approved: bool,
    pub paused: bool,
}
#[derive(Default, Debug, Serialize, Deserialize)]
pub struct Ledger {
    pub schema: u32,
    pub files: BTreeMap<String, String>,
    pub conflicts: BTreeMap<String, String>,
}
#[derive(Default, Debug, Serialize, Deserialize)]
pub struct Report {
    pub schema: u32,
    pub state: String,
    pub checked_at: u64,
    pub downloaded: u64,
    pub updated: u64,
    pub conflicts: u64,
    pub error: Option<String>,
}
#[derive(Clone)]
pub struct Paths {
    pub config: PathBuf,
    pub state: PathBuf,
}
impl Paths {
    pub fn discover() -> Result<Self> {
        let home = PathBuf::from(std::env::var("HOME").context("HOME unavailable")?);
        let xdg = |name: &str, default: &str| -> Result<PathBuf> {
            let p = std::env::var_os(name)
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(default));
            if !p.is_absolute() {
                bail!("XDG paths must be absolute")
            }
            Ok(p.join("gardengate"))
        };
        Ok(Self {
            config: xdg("XDG_CONFIG_HOME", ".config")?,
            state: xdg("XDG_STATE_HOME", ".local/state")?,
        })
    }
    pub fn init(&self) -> Result<()> {
        private_dir(&self.config)?;
        private_dir(&self.state)?;
        Ok(())
    }
    pub fn lock(&self) -> Result<File> {
        self.init()?;
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(self.state.join("operation.lock"))?;
        f.try_lock_exclusive()
            .context("Another setup or transfer is already running")?;
        Ok(f)
    }
    pub fn profile(&self) -> Result<Profile> {
        let p: Profile = read_json(&self.config.join("profile.json"))?;
        if p.schema != 1 {
            bail!("Unsupported profile schema")
        }
        Ok(p)
    }
    pub fn save_profile(&self, p: &Profile) -> Result<()> {
        atomic_json(&self.config.join("profile.json"), p)
    }
    pub fn encrypted_config(&self) -> Result<PathBuf> {
        let p = self.config.join("rclone.conf");
        reject_symlinks(&p)?;
        let mut header = [0u8; 128];
        let n = File::open(&p)
            .context("Run gardengate connect first")?
            .read(&mut header)?;
        if !String::from_utf8_lossy(&header[..n]).contains("RCLONE_ENCRYPT_V0:") {
            bail!("Refusing unencrypted rclone configuration; run connect")
        }
        Ok(p)
    }
}
pub fn private_dir(path: &Path) -> Result<()> {
    reject_symlinks(path)?;
    fs::create_dir_all(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}
pub fn reject_symlinks(path: &Path) -> Result<()> {
    let mut cur = PathBuf::new();
    for c in path.components() {
        if matches!(c, Component::ParentDir) {
            bail!("Parent path traversal is not allowed")
        }
        cur.push(c);
        match fs::symlink_metadata(&cur) {
            Ok(m) if m.file_type().is_symlink() => bail!("Symlink paths are not supported"),
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
pub fn validate_folder(s: &str) -> Result<()> {
    if s.is_empty()
        || s.starts_with('/')
        || s.contains(':')
        || s.chars().any(char::is_control)
        || s.split('/').any(|p| p == ".." || p == "." || p.is_empty())
    {
        bail!("Choose a named relative iCloud folder, such as Omarchy Inbox")
    }
    Ok(())
}
pub fn validate_destination(local: &Path, paths: &Paths) -> Result<()> {
    if !local.is_absolute() || local.parent() == Some(Path::new("/")) || local == Path::new("/") {
        bail!("Choose a dedicated folder below your home directory")
    }
    reject_symlinks(local)?;
    for forbidden in [&paths.config, &paths.state] {
        if local.starts_with(forbidden) || forbidden.starts_with(local) {
            bail!("Destination overlaps application state")
        }
    }
    if local.exists() && fs::read_dir(local)?.next().is_some() {
        bail!("Initial destination must be empty; existing files are never silently merged")
    }
    Ok(())
}
pub fn atomic_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    reject_symlinks(path)?;
    let parent = path.parent().context("Missing parent")?;
    private_dir(parent)?;
    let tmp = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap().to_string_lossy(),
        std::process::id()
    ));
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)
        .context("Stale temporary state exists; inspect before retrying")?;
    let result = (|| {
        serde_json::to_writer_pretty(&mut f, value)?;
        f.write_all(b"\n")?;
        f.sync_all()?;
        fs::rename(&tmp, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}
pub fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    reject_symlinks(path)?;
    Ok(serde_json::from_reader(File::open(path)?)?)
}
pub fn hash(path: &Path) -> Result<String> {
    let mut f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    if !f.metadata()?.is_file() {
        bail!("Not a regular file")
    }
    let mut h = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}
pub fn walk(root: &Path) -> Result<Vec<PathBuf>> {
    fn rec(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let t = entry.file_type()?;
            if t.is_symlink() {
                bail!("Symlink in transfer tree")
            }
            let name = entry.file_name();
            let name = name.to_str().context("Non-UTF8 file name unsupported")?;
            if name.chars().any(char::is_control) {
                bail!("Control characters in file name")
            }
            if name == ".reflect" || name == ".git" {
                continue;
            }
            if t.is_dir() {
                rec(root, &entry.path(), out)?
            } else if t.is_file() {
                out.push(entry.path().strip_prefix(root)?.to_path_buf())
            } else {
                bail!("Special file unsupported")
            }
        }
        Ok(())
    }
    reject_symlinks(root)?;
    let mut out = Vec::new();
    rec(root, root, &mut out)?;
    out.sort();
    Ok(out)
}
/// File-level download reconciliation. Never deletes or writes to the remote.
/// Local edits are retained; competing cloud content is saved in private recovery storage.
pub fn reconcile(stage: &Path, local: &Path, state: &Path, ledger: &mut Ledger) -> Result<Report> {
    reject_symlinks(stage)?;
    reject_symlinks(local)?;
    private_dir(state)?;
    fs::create_dir_all(local)?;
    let mut report = Report {
        schema: 1,
        state: "checked".into(),
        checked_at: now(),
        ..Default::default()
    };
    // Validate every staged/local entry before changing any local document.
    let incoming = walk(stage)?;
    walk(local)?;
    let mut folded = BTreeMap::new();
    for rel in &incoming {
        let key = rel.to_string_lossy().to_lowercase();
        if folded.insert(key, rel).is_some() {
            bail!("Case collision in incoming files")
        }
    }
    for rel in incoming {
        let key = rel.to_str().context("Invalid file name")?.to_string();
        let source = stage.join(&rel);
        let dest = local.join(&rel);
        reject_symlinks(&dest)?;
        let cloud_hash = hash(&source)?;
        let current = if dest.exists() {
            Some(hash(&dest)?)
        } else {
            None
        };
        let baseline = ledger.files.get(&key);
        if current.as_ref() == Some(&cloud_hash) {
            ledger.files.insert(key.clone(), cloud_hash);
            ledger.conflicts.remove(&key);
            continue;
        }
        if current.is_some() && current.as_ref() != baseline {
            if baseline == Some(&cloud_hash) {
                continue;
            } // Cloud unchanged; preserve local edit.
            let conflict_dir = state.join("conflicts").join(&cloud_hash);
            private_dir(&conflict_dir)?;
            let copy = conflict_dir.join(&rel);
            reject_symlinks(&copy)?;
            private_dir(copy.parent().unwrap())?;
            if !copy.exists() {
                fs::copy(&source, &copy)?;
                File::open(&copy)?.sync_all()?;
            }
            ledger
                .conflicts
                .insert(key, copy.to_string_lossy().into_owned());
            continue;
        }
        fs::create_dir_all(dest.parent().unwrap())?;
        reject_symlinks(dest.parent().unwrap())?;
        let temp = dest
            .parent()
            .unwrap()
            .join(format!(".gardengate-{}.tmp", std::process::id()));
        let mut tf = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temp)?;
        let result = (|| -> Result<()> {
            let mut sf = File::open(&source)?;
            std::io::copy(&mut sf, &mut tf)?;
            tf.sync_all()?;
            if let Some(before) = &current {
                if hash(&dest)? != *before {
                    bail!("Local file changed during transfer; retry after editing")
                }
                let backup = state.join("backups").join(before).join(&rel);
                reject_symlinks(&backup)?;
                private_dir(backup.parent().unwrap())?;
                if !backup.exists() {
                    fs::copy(&dest, &backup)?;
                    File::open(&backup)?.sync_all()?;
                }
                if hash(&dest)? != *before {
                    bail!("Local file changed during backup; retry")
                }
                fs::rename(&temp, &dest)?;
                report.updated += 1;
            } else {
                // Atomic create-if-absent: never overwrite a newly created local file.
                fs::hard_link(&temp, &dest).context("Destination changed during download")?;
                fs::remove_file(&temp)?;
                report.downloaded += 1;
            }
            File::open(dest.parent().unwrap())?.sync_all()?;
            Ok(())
        })();
        let _ = fs::remove_file(&temp);
        result?;
        ledger.files.insert(key.clone(), cloud_hash);
        ledger.conflicts.remove(&key);
    }
    report.conflicts = ledger.conflicts.len() as u64;
    if report.conflicts > 0 {
        report.state = "needs-review".into()
    };
    Ok(report)
}

pub struct Rclone {
    pub binary: PathBuf,
    pub config: PathBuf,
    pub password_command: Option<String>,
}
impl Rclone {
    pub fn managed(paths: &Paths) -> Result<Self> {
        Ok(Self {
            binary: PathBuf::from("rclone"),
            config: paths.encrypted_config()?,
            password_command: Some("secret-tool lookup application gardengate".into()),
        })
    }
    pub fn command(&self) -> Command {
        let mut c = Command::new(&self.binary);
        c.arg("--config")
            .arg(&self.config)
            .env_remove("RCLONE_CONFIG_PASS")
            .env_remove("RCLONE_PASSWORD_COMMAND");
        if let Some(p) = &self.password_command {
            c.arg("--password-command").arg(p);
        }
        c
    }
    pub fn run(&self, args: &[&str], output_path: &Path) -> Result<Vec<u8>> {
        reject_symlinks(output_path)?;
        if output_path.exists() {
            fs::remove_file(output_path)?;
        }
        let out = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(output_path)?;
        let mut child = self
            .command()
            .args(args)
            .args(["--contimeout", "15s", "--timeout", "60s", "--retries", "1"])
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(Stdio::null())
            .spawn()
            .context("rclone could not start")?;
        let start = Instant::now();
        let result = loop {
            if STOP.load(Ordering::Relaxed) || start.elapsed() > Duration::from_secs(300) {
                let _ = child.kill();
                let _ = child.wait();
                break Err(anyhow::anyhow!(
                    "Transfer interrupted or timed out; local files retained"
                ));
            }
            if let Some(status) = child.try_wait()? {
                break if status.success() {
                    fs::read(output_path).map_err(Into::into)
                } else {
                    Err(anyhow::anyhow!("Cloud operation failed (exit {}). Check connection, sign-in, folder access and storage; no raw credential logs are exposed",status.code().unwrap_or(-1)))
                };
            }
            thread::sleep(Duration::from_millis(100));
        };
        let _ = fs::remove_file(output_path);
        result
    }
    pub fn download(&self, remote: &str, stage: &Path, output_path: &Path) -> Result<()> {
        let stage = stage.to_str().context("Invalid staging path")?;
        self.run(
            &[
                "copy",
                remote,
                stage,
                "--transfers",
                "1",
                "--exclude",
                ".reflect/**",
                "--exclude",
                ".git/**",
                "--exclude",
                ".DS_Store",
                "--max-duration",
                "4m",
                "--max-transfer",
                "512Mi",
            ],
            output_path,
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_remote_paths() {
        for p in ["", "/", "../notes", "a/../b", "a:b", "a\nb", "a//b"] {
            assert!(validate_folder(p).is_err());
        }
        assert!(validate_folder("Omarchy Inbox/Notes").is_ok());
    }
    #[test]
    fn preserves_edit_conflict_and_cloud_deletion() -> Result<()> {
        let t = tempfile::tempdir()?;
        let stage = t.path().join("stage");
        let local = t.path().join("local");
        let state = t.path().join("state");
        fs::create_dir(&stage)?;
        fs::create_dir(&local)?;
        let mut l = Ledger {
            schema: 1,
            ..Default::default()
        };
        fs::write(stage.join("note.md"), "one")?;
        assert_eq!(reconcile(&stage, &local, &state, &mut l)?.downloaded, 1);
        fs::write(stage.join("note.md"), "two")?;
        assert_eq!(reconcile(&stage, &local, &state, &mut l)?.updated, 1);
        assert_eq!(
            fs::read_to_string(
                state
                    .join("backups")
                    .join(format!("{:x}", Sha256::digest(b"one")))
                    .join("note.md")
            )?,
            "one"
        );
        fs::write(local.join("note.md"), "local edit")?;
        fs::write(stage.join("note.md"), "cloud edit")?;
        assert_eq!(reconcile(&stage, &local, &state, &mut l)?.conflicts, 1);
        assert_eq!(fs::read_to_string(local.join("note.md"))?, "local edit");
        assert_eq!(
            fs::read_to_string(l.conflicts.get("note.md").unwrap())?,
            "cloud edit"
        );
        fs::remove_file(stage.join("note.md"))?;
        reconcile(&stage, &local, &state, &mut l)?;
        assert!(local.join("note.md").exists());
        Ok(())
    }
    #[test]
    fn local_edit_unchanged_cloud_is_not_conflict() -> Result<()> {
        let t = tempfile::tempdir()?;
        let s = t.path().join("s");
        let l = t.path().join("l");
        let d = t.path().join("d");
        fs::create_dir(&s)?;
        fs::create_dir(&l)?;
        fs::write(s.join("n"), "base")?;
        let mut ledger = Ledger::default();
        reconcile(&s, &l, &d, &mut ledger)?;
        fs::write(l.join("n"), "edited")?;
        assert_eq!(reconcile(&s, &l, &d, &mut ledger)?.conflicts, 0);
        assert_eq!(fs::read_to_string(l.join("n"))?, "edited");
        Ok(())
    }
    #[test]
    fn symlink_escape_rejected() -> Result<()> {
        let t = tempfile::tempdir()?;
        std::os::unix::fs::symlink("/tmp", t.path().join("link"))?;
        assert!(reject_symlinks(&t.path().join("link/file")).is_err());
        Ok(())
    }
    #[test]
    fn state_roundtrip_and_new_schema() -> Result<()> {
        let t = tempfile::tempdir()?;
        let p = Profile {
            schema: 1,
            remote_folder: "Inbox".into(),
            local: t.path().join("l"),
            approved: false,
            paused: false,
        };
        let paths = Paths {
            config: t.path().join("c"),
            state: t.path().join("s"),
        };
        paths.init()?;
        paths.save_profile(&p)?;
        assert_eq!(paths.profile()?.remote_folder, "Inbox");
        let mut p = p;
        p.schema = 2;
        paths.save_profile(&p)?;
        assert!(paths.profile().is_err());
        Ok(())
    }
    #[test]
    fn operation_lock_excludes_second_process() -> Result<()> {
        let t = tempfile::tempdir()?;
        let p = Paths {
            config: t.path().join("c"),
            state: t.path().join("s"),
        };
        let a = p.lock()?;
        assert!(p.lock().is_err());
        drop(a);
        assert!(p.lock().is_ok());
        Ok(())
    }
    #[test]
    fn collisions_rejected_before_write() -> Result<()> {
        let t = tempfile::tempdir()?;
        let s = t.path().join("s");
        let l = t.path().join("l");
        fs::create_dir(&s)?;
        fs::create_dir(&l)?;
        fs::write(s.join("A"), "a")?;
        fs::write(s.join("a"), "b")?;
        assert!(reconcile(&s, &l, &t.path().join("state"), &mut Ledger::default()).is_err());
        assert_eq!(fs::read_dir(l)?.count(), 0);
        Ok(())
    }
}
