//! Desktop onboarding over rclone's supported non-interactive configuration protocol.
use super::{dialog, prepare_connection};
use anyhow::{bail, Context, Result};
use gardengate::*;
use serde_json::Value;
use std::{
    fs,
    io::Read,
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub fn float_dialog(child: &mut Child) {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_none() {
        return;
    }
    let pid = child.id();
    for _ in 0..20 {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        let Ok(out) = Command::new("hyprctl").args(["-j", "clients"]).output() else {
            return;
        };
        let Ok(clients) = serde_json::from_slice::<Value>(&out.stdout) else {
            return;
        };
        let client = clients
            .as_array()
            .and_then(|xs| xs.iter().find(|x| x["pid"].as_u64() == Some(pid as u64)));
        if let Some(client) = client {
            let monitor = Command::new("hyprctl")
                .args(["-j", "monitors"])
                .output()
                .ok()
                .and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok())
                .and_then(|v| {
                    v.as_array()
                        .and_then(|xs| xs.iter().find(|x| x["id"] == client["monitor"]).cloned())
                });
            let dispatch = |lua: String, name: &str, value: String| {
                let modern = Command::new("hyprctl")
                    .args(["dispatch", &lua])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
                if modern.is_ok_and(|s| s.success()) {
                    return;
                }
                let _ = Command::new("hyprctl")
                    .args(["dispatch", name, &value])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            };
            let (mut width, mut height) = (560, 380);
            let mut position = None;
            if let Some(m) = monitor {
                let scale = m["scale"].as_f64().unwrap_or(1.0).max(0.25);
                let mw = (m["width"].as_f64().unwrap_or(1920.0) / scale) as i64;
                let mh = (m["height"].as_f64().unwrap_or(1080.0) / scale) as i64;
                width = width.min((mw - 64).max(240));
                height = height.min((mh - 64).max(180));
                position = Some((
                    m["x"].as_i64().unwrap_or(0) + (mw - width) / 2,
                    m["y"].as_i64().unwrap_or(0) + (mh - height) / 2,
                ));
            }
            dispatch(
                format!("hl.dsp.window.float({{ window = \"pid:{pid}\", action = \"on\" }})"),
                "setfloating",
                format!("pid:{pid}"),
            );
            dispatch(
                format!(
                    "hl.dsp.window.resize({{ window = \"pid:{pid}\", x = {width}, y = {height} }})"
                ),
                "resizewindowpixel",
                format!("exact {width} {height},pid:{pid}"),
            );
            if let Some((x, y)) = position {
                dispatch(
                    format!("hl.dsp.window.move({{ window = \"pid:{pid}\", x = {x}, y = {y} }})"),
                    "movewindowpixel",
                    format!("exact {x} {y},pid:{pid}"),
                );
            }
            return;
        }
        thread::sleep(Duration::from_millis(40));
    }
}

struct StagedConfig(PathBuf);
impl Drop for StagedConfig {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn config_command(r: &Rclone, continuation: Option<(&str, &str)>) -> Command {
    let mut command = r.command();
    // No inherited rclone debug/dump overrides can log setup credentials.
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("RCLONE_") {
            command.env_remove(name);
        }
    }
    command.args([
        "--log-level",
        "ERROR",
        "--log-file",
        "/dev/null",
        "--contimeout",
        "15s",
        "--timeout",
        "60s",
        "--retries",
        "1",
    ]);
    if let Some((state, answer)) = continuation {
        command
            .args([
                "config",
                "update",
                "icloud",
                "--continue",
                "--non-interactive",
            ])
            .env("RCLONE_STATE", state)
            .env("RCLONE_RESULT", answer);
    } else {
        command.args([
            "config",
            "create",
            "icloud",
            "iclouddrive",
            "--all",
            "--non-interactive",
        ]);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command
}

fn config_step(r: &Rclone, continuation: Option<(&str, &str)>) -> Result<Value> {
    let mut child = config_command(r, continuation)
        .spawn()
        .context("Could not start iCloud setup")?;
    let stdout = child.stdout.take().context("Missing setup response pipe")?;
    let reader = thread::spawn(move || {
        let mut output = Vec::new();
        stdout
            .take(1024 * 1024)
            .read_to_end(&mut output)
            .map(|_| output)
    });
    let start = Instant::now();
    let result = loop {
        if STOP.load(std::sync::atomic::Ordering::Relaxed)
            || start.elapsed() > Duration::from_secs(120)
        {
            let _ = child.kill();
            let _ = child.wait();
            break Err(anyhow::anyhow!("Apple sign-in timed out or was interrupted. Your previous connection is unchanged."));
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                break if status.success() {
                    Ok(())
                } else {
                    Err(anyhow::anyhow!("Apple sign-in did not finish. Check your account details, internet connection and trusted-device prompts, then try again."))
                }
            }
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(anyhow::anyhow!("Could not wait for Apple sign-in."));
            }
        }
    };
    let bytes = reader
        .join()
        .map_err(|_| anyhow::anyhow!("Could not read setup response"))??;
    result?;
    serde_json::from_slice(&bytes).context("Unsupported iCloud setup response; update rclone")
}

fn automatic_answer(name: &str) -> Option<&'static str> {
    match name {
        "service" => Some("drive"),
        "config_fs_advanced" => Some("false"),
        _ => None,
    }
}

fn answer_question(response: &Value) -> Result<Option<String>> {
    let option = &response["Option"];
    let name = option["Name"]
        .as_str()
        .context("Unsupported setup question")?;
    if let Some(answer) = automatic_answer(name) {
        return Ok(Some(answer.into()));
    }
    let failed = response["Error"].as_str().is_some_and(|s| !s.is_empty());
    let prompt = match name {
        "apple_id" => "Apple Account email address".to_owned(),
        "password" => "Apple Account password\n\nUse your usual Apple password. Garden Gate keeps this in your encrypted connection configuration.".to_owned(),
        "config_2fa" => "Approve the sign-in on your iPhone, then enter its verification code.\n\nYou can type sms to request a text message instead.".to_owned(),
        "config_2fa_sms" => "Enter the verification code from Apple's text message.".to_owned(),
        _ => option["Help"].as_str().unwrap_or("Continue Apple sign-in").to_owned(),
    };
    let prompt = if failed {
        format!("That response was not accepted. Please try again.\n\n{prompt}")
    } else {
        prompt
    };
    // Never pass stored/default passwords or a previous answer in argv.
    let secret = option["IsPassword"].as_bool().unwrap_or(false) || name.contains("password");
    let result = if secret {
        dialog(&["--password", &prompt])?
    } else if let Some(examples) = option["Examples"].as_array().filter(|x| !x.is_empty()) {
        if option["Exclusive"].as_bool().unwrap_or(false) {
            let mut args = vec!["--menu".to_string(), prompt];
            for (i, example) in examples.iter().enumerate() {
                args.push(i.to_string());
                args.push(example["Help"].as_str().unwrap_or("Continue").to_string());
            }
            match dialog(&args.iter().map(String::as_str).collect::<Vec<_>>())? {
                Some(choice) => {
                    let index: usize = choice.parse().context("Invalid sign-in choice")?;
                    Some(
                        examples
                            .get(index)
                            .and_then(|e| e["Value"].as_str())
                            .context("Invalid sign-in choice")?
                            .to_string(),
                    )
                }
                None => None,
            }
        } else {
            dialog(&["--inputbox", &prompt])?
        }
    } else {
        dialog(&["--inputbox", &prompt])?
    };
    if result.as_ref().is_some_and(|s| s.is_empty())
        && option["Required"].as_bool().unwrap_or(false)
    {
        bail!("This field is required. Setup stopped; your previous connection is unchanged.")
    }
    Ok(result)
}

pub fn connect_gui(paths: &Paths) -> Result<bool> {
    if dialog(&["--yes-label", "Continue", "--no-label", "Cancel", "--yesno",
        "Connect iCloud Drive\n\nHave your iPhone ready to approve Apple's sign-in request. Garden Gate only downloads files; it never uploads or deletes from iCloud.\n\nThe connection uses rclone and your desktop keyring."])?.is_none() { return Ok(false); }
    let _lock = paths.lock()?;
    prepare_connection(paths)?;
    let current = paths.encrypted_config()?;
    let stage = StagedConfig(paths.config.join("setup.rclone.conf"));
    reject_symlinks(&stage.0)?;
    fs::copy(&current, &stage.0)?;
    let mut r = Rclone::managed(paths)?;
    r.config = stage.0.clone();
    let mut response = config_step(&r, None)?;
    for _ in 0..40 {
        let state = response["State"]
            .as_str()
            .context("Unsupported setup state")?
            .to_owned();
        if state.is_empty() {
            r.run(
                &["lsjson", "icloud:", "--dirs-only"],
                &paths.state.join("setup-listing.tmp"),
            )?;
            // rclone must preserve encryption throughout every continuation.
            let data = fs::read(&stage.0)?;
            if !String::from_utf8_lossy(&data[..data.len().min(128)]).contains("RCLONE_ENCRYPT_V0:")
            {
                bail!("Connection was not encrypted; your previous connection is unchanged.");
            }
            fs::File::open(&stage.0)?.sync_all()?;
            fs::rename(&stage.0, current)?;
            dialog(&[
                "--msgbox",
                "Connected to iCloud Drive.\n\nYou can now choose the folder to download.",
            ])?;
            return Ok(true);
        }
        let Some(answer) = answer_question(&response)? else {
            return Ok(false);
        };
        response = config_step(&r, Some((&state, &answer)))?;
    }
    bail!("Sign-in requested too many steps; your previous connection is unchanged.")
}

pub fn choose_folder(paths: &Paths) -> Result<bool> {
    if paths.profile().is_ok() {
        dialog(&["--msgbox", "An inbox is already configured. Use Open local inbox to view it. This preview supports one folder."])?;
        return Ok(true);
    }
    let mut folder = String::new();
    loop {
        let bytes = {
            let _lock = paths.lock()?;
            Rclone::managed(paths)?.run(
                &["lsjson", &format!("icloud:{folder}"), "--dirs-only"],
                &paths.state.join("setup-listing.tmp"),
            )?
        };
        let listing: Value = serde_json::from_slice(&bytes)?;
        let names: Vec<&str> = listing
            .as_array()
            .context("Invalid folder listing")?
            .iter()
            .filter_map(|x| x["Name"].as_str())
            .collect();
        let title = if folder.is_empty() {
            "iCloud Drive"
        } else {
            &folder
        };
        let mut args = vec!["--menu".to_string(), format!("Choose your iCloud folder\n{title}\n\nCreate Omarchy Inbox in iPhone Files if you need a test folder.")];
        if !folder.is_empty() {
            args.extend([
                "use".into(),
                "Download this folder".into(),
                "up".into(),
                "Back to parent folder".into(),
            ]);
        }
        args.extend(["refresh".into(), "Refresh folders".into()]);
        for (index, name) in names.iter().enumerate() {
            args.extend([index.to_string(), (*name).to_string()]);
        }
        let Some(choice) = dialog(&args.iter().map(String::as_str).collect::<Vec<_>>())? else {
            return Ok(false);
        };
        match choice.as_str() {
            "use" if !folder.is_empty() => break,
            "up" => {
                folder = folder
                    .rsplit_once('/')
                    .map(|(parent, _)| parent)
                    .unwrap_or("")
                    .to_string()
            }
            "refresh" => (),
            _ => {
                let index: usize = choice.parse().context("Invalid folder choice")?;
                let name = names.get(index).context("Invalid folder choice")?;
                let next = if folder.is_empty() {
                    name.to_string()
                } else {
                    format!("{folder}/{name}")
                };
                validate_folder(&next)?;
                folder = next;
            }
        }
    }
    let home = std::env::var("HOME")?;
    let default = format!(
        "{home}/iCloud Drive/{}",
        folder.rsplit('/').next().unwrap_or("Omarchy Inbox")
    );
    let Some(local) = dialog(&["--inputbox", "Where should downloaded files go?\n\nGarden Gate creates this folder if needed. It must be empty and inside your home.", &default])? else { return Ok(false); };
    let output = Command::new(std::env::current_exe()?)
        .args(["add", &folder, &local])
        .output()?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr));
    }
    let plan = Command::new(std::env::current_exe()?)
        .arg("plan")
        .output()?;
    if !plan.status.success() {
        bail!("{}", String::from_utf8_lossy(&plan.stderr));
    }
    let message = format!("Your inbox is ready.\n\n{}\n\nOpen Garden Gate and choose Download now when you are ready.", String::from_utf8_lossy(&plan.stdout));
    dialog(&["--msgbox", &message])?;
    Ok(true)
}

pub fn wizard(paths: &Paths) -> Result<bool> {
    let mut args = vec!["--menu", "Bring an iCloud folder to your desktop\n\nConnect your Apple Account, then choose a folder. Files download one way and local edits are preserved.",
                        "connect", "Connect Apple Account"];
    if paths.encrypted_config().is_ok() {
        args.extend(["folder", "Already connected — choose a folder"]);
    }
    let Some(action) = dialog(&args)? else {
        return Ok(false);
    };
    if action == "connect" && !connect_gui(paths)? {
        return Ok(false);
    }
    choose_folder(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn setup_keeps_credentials_out_of_process_arguments() {
        let r = Rclone {
            binary: "rclone".into(),
            config: "encrypted.conf".into(),
            password_command: None,
        };
        let c = config_command(&r, Some(("state-fixture", "secret-fixture")));
        let args: Vec<_> = c.get_args().map(|x| x.to_string_lossy()).collect();
        assert!(!args
            .iter()
            .any(|x| x.contains("secret-fixture") || x.contains("state-fixture")));
        assert!(c.get_envs().any(
            |(k, v)| k == "RCLONE_RESULT" && v == Some(std::ffi::OsStr::new("secret-fixture"))
        ));
    }
    #[test]
    fn setup_only_automates_drive_and_skips_advanced_options() {
        assert_eq!(automatic_answer("service"), Some("drive"));
        assert_eq!(automatic_answer("config_fs_advanced"), Some("false"));
        assert_eq!(automatic_answer("password"), None);
        assert_eq!(automatic_answer("config_2fa"), None);
    }
}
