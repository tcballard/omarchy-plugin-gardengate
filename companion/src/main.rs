use anyhow::{bail, Context, Result};
mod setup;
use gardengate::*;
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::Ordering,
    thread,
    time::Duration,
};
extern "C" fn stop(_: libc::c_int) {
    STOP.store(true, Ordering::Relaxed);
}
fn main() {
    unsafe {
        libc::umask(0o077);
        libc::signal(libc::SIGTERM, stop as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, stop as *const () as libc::sighandler_t);
    }
    if let Err(e) = run() {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    if cmd == "--version" {
        println!("gardengate 0.0.2 (download-only developer preview)");
        return Ok(());
    }
    if cmd == "help" || cmd == "--help" {
        println!("Garden Gate for Omarchy 0.0.2 — developer preview\n\nDownload-only: iCloud Drive → local inbox. No cloud writes or deletions.\n\n  setup               Connect Apple and choose a folder with desktop dialogs\n  manage              Open the Garden Gate management menu\n  connect             Configure encrypted account connection (interactive)\n  folders [path]      List accessible iCloud Drive folders\n  add FOLDER LOCAL    Select one folder and an empty absolute local directory\n  plan                Show cloud folder size and current local contents\n  pull --apply        Download now and approve background downloading\n  watch               Download every 60s; used by systemd\n  pause | resume      Control background downloads\n  status              Print JSON state and preserved conflict-copy paths\n  open                Open the local folder\n\nCreate 'Omarchy Inbox' in iPhone Files → iCloud Drive, then use that folder.\nSetup currently uses rclone's terminal UI. Full management UI, two-way sync\nand Reflect compatibility are not implemented in this preview.");
        return Ok(());
    }
    let paths = Paths::discover()?;
    paths.init()?;
    match cmd {
        "connect" => connect(&paths),
        "setup" => setup::wizard(&paths).map(|_| ()),
        "manage" => manage(&paths),
        "folders" => {
            let _lock = paths.lock()?;
            let r = Rclone::managed(&paths)?;
            let folder = args.get(1).cloned().unwrap_or_default();
            if !folder.is_empty() {
                validate_folder(&folder)?
            }
            let bytes = r.run(
                &["lsjson", &format!("icloud:{folder}"), "--dirs-only"],
                &paths.state.join("listing.tmp"),
            )?;
            let value: serde_json::Value = serde_json::from_slice(&bytes)?;
            for x in value.as_array().context("Invalid cloud listing")? {
                if let Some(name) = x.get("Name").and_then(|v| v.as_str()) {
                    println!("{name:?}");
                }
            }
            Ok(())
        }
        "add" => {
            if args.len() != 3 {
                bail!("Usage: gardengate add FOLDER /absolute/empty/local/folder")
            };
            let _lock = paths.lock()?;
            if paths.config.join("profile.json").exists() {
                bail!("A folder is already configured; this preview supports one mapping")
            };
            validate_folder(&args[1])?;
            let local = PathBuf::from(&args[2]);
            let home = PathBuf::from(std::env::var("HOME")?);
            if !local.starts_with(&home) || local == home {
                bail!("Choose a dedicated folder inside your home directory")
            };
            validate_destination(&local, &paths)?;
            let r = Rclone::managed(&paths)?;
            r.run(
                &["lsjson", &format!("icloud:{}", args[1]), "--dirs-only"],
                &paths.state.join("listing.tmp"),
            )?;
            fs::create_dir_all(&local)?;
            paths.save_profile(&Profile {
                schema: 1,
                remote_folder: args[1].clone(),
                local,
                approved: false,
                paused: false,
            })?;
            println!("Folder selected. Run plan, then pull --apply when ready.");
            Ok(())
        }
        "plan" => {
            let _lock = paths.lock()?;
            let p = paths.profile()?;
            validate_folder(&p.remote_folder)?;
            reject_symlinks(&p.local)?;
            let r = Rclone::managed(&paths)?;
            let bytes = r.run(
                &[
                    "size",
                    &format!("icloud:{}", p.remote_folder),
                    "--json",
                    "--exclude",
                    ".reflect/**",
                    "--exclude",
                    ".git/**",
                    "--exclude",
                    ".DS_Store",
                ],
                &paths.state.join("listing.tmp"),
            )?;
            let v: serde_json::Value = serde_json::from_slice(&bytes)?;
            println!("iCloud folder: {:?}\nLocal directory: {:?}\nCloud files: {}\nCloud bytes: {}\nLocal files: {}\nNo cloud writes. No local/cloud deletions.\nChanged local files are preserved; conflicting cloud versions go to recovery storage.\nThis preview stages the whole cloud folder each cycle; choose a SMALL folder.\nRun pull --apply to approve downloading.",p.remote_folder,p.local,v["count"],v["bytes"],walk(&p.local)?.len());
            Ok(())
        }
        "pull" => {
            if args.get(1).map(String::as_str) != Some("--apply") {
                bail!("Use plan first, then pull --apply")
            };
            let _lock = paths.lock()?;
            let mut p = paths.profile()?;
            let report = pull(&paths, &p)?;
            p.approved = true;
            paths.save_profile(&p)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        "watch" => watch(&paths),
        "pause" | "resume" => {
            let _lock = paths.lock()?;
            let mut p = paths.profile()?;
            p.paused = cmd == "pause";
            paths.save_profile(&p)?;
            println!(
                "{}",
                if p.paused {
                    "Paused; local files retained."
                } else {
                    "Resumed."
                }
            );
            Ok(())
        }
        "status" => {
            let p = if paths.config.join("profile.json").exists() {
                Some(paths.profile()?)
            } else {
                None
            };
            let report = if paths.state.join("status.json").exists() {
                Some(read_json::<Report>(&paths.state.join("status.json"))?)
            } else {
                None
            };
            let ledger = if paths.state.join("ledger.json").exists() {
                Some(read_json::<Ledger>(&paths.state.join("ledger.json"))?)
            } else {
                None
            };
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"schema":1,"mode":"download-only","profile":p,"last_check":report,"conflict_copies":ledger.map(|l|l.conflicts)})
                )?
            );
            Ok(())
        }
        "open" => {
            let p = paths.profile()?;
            reject_symlinks(&p.local)?;
            let status = Command::new("xdg-open").arg(p.local).status()?;
            if !status.success() {
                bail!("Could not open folder")
            };
            Ok(())
        }
        _ => bail!("Unknown command; use --help"),
    }
}
fn connect(paths: &Paths) -> Result<()> {
    let _lock = paths.lock()?;
    prepare_connection(paths)?;
    println!("Configure a remote named icloud, type iclouddrive, service drive.\nUse your Apple Account password and complete 2FA on your iPhone.\nDo not remove configuration encryption. This is an unofficial connection.\nIn rclone select n to create, or e to edit the existing icloud remote.\n");
    let r = Rclone::managed(paths)?;
    if !r.command().arg("config").status()?.success() {
        bail!("Account setup did not finish")
    };
    paths.encrypted_config()?;
    r.run(
        &["lsjson", "icloud:", "--dirs-only"],
        &paths.state.join("listing.tmp"),
    )?;
    println!("Connection verified. Open Garden Gate to choose your iCloud folder.");
    Ok(())
}
fn prepare_connection(paths: &Paths) -> Result<()> {
    let config = paths.config.join("rclone.conf");
    reject_symlinks(&config)?;
    let password_command = "secret-tool lookup application gardengate";
    if !config.exists() {
        let mut random = [0u8; 32];
        fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
        let secret: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let mut store = Command::new("secret-tool")
            .args([
                "store",
                "--label=Garden Gate configuration key",
                "application",
                "gardengate",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("Install libsecret and unlock your desktop keyring first")?;
        store.stdin.take().unwrap().write_all(secret.as_bytes())?;
        if !store.wait()?.success() {
            bail!("Could not save encryption key to the desktop keyring; setup stopped")
        }
        fs::write(&config, b"# Managed Garden Gate configuration\n")?;
        let status = Command::new("rclone")
            .arg("--config")
            .arg(&config)
            .args([
                "--password-command",
                password_command,
                "config",
                "encryption",
                "set",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        if !status.success() {
            let _ = fs::remove_file(&config);
            bail!("Could not encrypt account configuration; no login attempted")
        }
    }
    paths.encrypted_config()?;
    Ok(())
}
fn pull(paths: &Paths, p: &Profile) -> Result<Report> {
    validate_folder(&p.remote_folder)?;
    reject_symlinks(&p.local)?;
    let ledger_path = paths.state.join("ledger.json");
    let mut ledger = if ledger_path.exists() {
        read_json::<Ledger>(&ledger_path)?
    } else {
        Ledger {
            schema: 1,
            ..Default::default()
        }
    };
    if ledger.schema != 1 {
        bail!("Unsupported ledger schema; refusing to discard state")
    }
    let stage = paths.state.join("staging");
    if stage.exists() {
        reject_symlinks(&stage)?;
        fs::remove_dir_all(&stage)?
    };
    private_dir(&stage)?;
    let r = Rclone::managed(paths)?;
    let result: Result<Report> = (|| {
        r.download(
            &format!("icloud:{}", p.remote_folder),
            &stage,
            &paths.state.join("transfer.tmp"),
        )?;
        let report = reconcile(&stage, &p.local, &paths.state, &mut ledger)?;
        atomic_json(&ledger_path, &ledger)?;
        atomic_json(&paths.state.join("status.json"), &report)?;
        Ok(report)
    })();
    let _ = fs::remove_dir_all(stage);
    if let Err(e) = &result {
        let report = Report {
            schema: 1,
            state: "failed".into(),
            checked_at: now(),
            error: Some(e.to_string()),
            ..Default::default()
        };
        let _ = atomic_json(&paths.state.join("status.json"), &report);
    }
    result
}
fn watch(paths: &Paths) -> Result<()> {
    // Separate service lock prevents duplicate watchers; operation lock also protects manual commands.
    let service_lock = paths.state.join("watch.lock");
    use fs2::FileExt;
    use std::os::unix::fs::OpenOptionsExt;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(service_lock)?;
    lock.try_lock_exclusive()
        .context("Watcher already running")?;
    let mut backoff = 60;
    while !STOP.load(Ordering::Relaxed) {
        let success = match paths.lock() {
            Ok(_operation) => {
                let p = paths.profile()?;
                if p.approved && !p.paused {
                    pull(paths, &p).is_ok()
                } else {
                    true
                }
            }
            Err(_) => true,
        };
        backoff = if success { 60 } else { (backoff * 2).min(900) };
        for _ in 0..backoff {
            if STOP.load(Ordering::Relaxed) {
                break;
            };
            thread::sleep(Duration::from_secs(1));
        }
    }
    Ok(())
}

fn dialog(args: &[&str]) -> Result<Option<String>> {
    let mut child = Command::new("kdialog")
        .args(["--title", "Garden Gate", "--geometry", "560x380"])
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("Install kdialog for the management window")?;
    setup::float_dialog(&mut child);
    let output = child.wait_with_output()?;
    if !output.status.success() {
        if output.status.code() == Some(1) {
            return Ok(None);
        }
        bail!("Garden Gate could not open its dialog. Check your desktop session.");
    }
    Ok(Some(
        String::from_utf8(output.stdout)?
            .trim_end_matches(['\r', '\n'])
            .to_string(),
    ))
}
fn manage(paths: &Paths) -> Result<()> {
    if paths.profile().is_err() {
        match setup::wizard(paths) {
            Ok(true) => (),
            Ok(false) => return Ok(()),
            Err(error) => {
                dialog(&["--error", &error.to_string()])?;
                return Ok(());
            }
        }
    }
    loop {
        let state = if let Ok(p) = paths.profile() {
            if p.paused {
                "Paused"
            } else if let Ok(r) = read_json::<Report>(&paths.state.join("status.json")) {
                if r.conflicts > 0 {
                    "Needs review"
                } else {
                    "Ready — see last check"
                }
            } else {
                "Folder selected"
            }
        } else {
            "Setup required"
        };
        let title =
            format!("Garden Gate · developer preview\n{state}\nDownload-only: iPhone → XPS");
        let Some(action) = dialog(&[
            "--title",
            "Garden Gate",
            "--menu",
            &title,
            "setup",
            "Connect or reconnect Apple Account",
            "add",
            "Choose iCloud folder",
            "preview",
            "Preview download",
            "pull",
            "Download now",
            "status",
            "Status and conflict copies",
            "pause",
            "Pause background downloads",
            "resume",
            "Resume background downloads",
            "open",
            "Open local inbox",
        ])?
        else {
            break;
        };
        if action == "setup" {
            if let Err(error) = setup::connect_gui(paths) {
                dialog(&["--error", &error.to_string()])?;
            }
            continue;
        }
        if action == "add" {
            if let Err(error) = setup::choose_folder(paths) {
                dialog(&["--error", &error.to_string()])?;
            }
            continue;
        }
        if action == "pull" {
            if dialog(&["--yesno","Download this folder now? Cloud files are copied locally, changed local files are preserved, and competing cloud copies go to recovery storage. No cloud writes or propagated deletions. This also approves future background downloads."] )?.is_none(){continue}
            let output = Command::new("systemctl")
                .args(["--user", "start", "--no-block", "gardengate-pull.service"])
                .output()?;
            if output.status.success() {
                dialog(&[
                    "--msgbox",
                    "Download queued. Use Status to see completion or errors.",
                ])?;
            } else {
                dialog(&[
                    "--error",
                    "Could not queue download. In a terminal run: gardengate pull --apply",
                ])?;
            }
            continue;
        }
        let command = match action.as_str() {
            "preview" => "plan",
            "status" => "status",
            "pause" => "pause",
            "resume" => "resume",
            "open" => "open",
            _ => continue,
        };
        show_command(&[command])?;
    }
    Ok(())
}
fn show_command(args: &[&str]) -> Result<()> {
    let out = Command::new(std::env::current_exe()?).args(args).output()?;
    let text = if out.status.success() {
        String::from_utf8_lossy(&out.stdout)
    } else {
        String::from_utf8_lossy(&out.stderr)
    };
    dialog(&[
        if out.status.success() {
            "--msgbox"
        } else {
            "--error"
        },
        &text,
    ])?;
    Ok(())
}
