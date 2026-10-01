use anyhow::Result;
use gardengate::*;
use std::{fs, path::PathBuf, process::Command};
#[test]
#[ignore = "Requires RCLONE_TEST_BIN; run explicitly with pinned real rclone"]
fn real_rclone_download_and_conflict() -> Result<()> {
    let binary = PathBuf::from(std::env::var("RCLONE_TEST_BIN")?);
    let t = tempfile::tempdir()?;
    let cloud = t.path().join("cloud");
    let stage = t.path().join("stage");
    let local = t.path().join("local");
    let state = t.path().join("state");
    for d in [&cloud, &stage, &local, &state] {
        fs::create_dir(d)?;
    }
    fs::write(cloud.join("iPhone note.md"), "hello from phone")?;
    fs::create_dir(cloud.join(".reflect"))?;
    fs::write(cloud.join(".reflect/index.db"), "private state")?;
    let r = Rclone {
        binary,
        config: t.path().join("rclone.conf"),
        password_command: None,
    };
    fs::write(&r.config, "[test]\ntype = local\n")?;
    let remote = format!("test:{}", cloud.display());
    let mut ledger = Ledger {
        schema: 1,
        ..Default::default()
    };
    r.download(&remote, &stage, &state.join("output.tmp"))?;
    let report = reconcile(&stage, &local, &state, &mut ledger)?;
    assert_eq!(report.downloaded, 1);
    assert!(!local.join(".reflect").exists());
    assert_eq!(
        fs::read_to_string(local.join("iPhone note.md"))?,
        "hello from phone"
    );
    fs::write(local.join("iPhone note.md"), "XPS edit")?;
    fs::write(cloud.join("iPhone note.md"), "new phone edit")?;
    fs::remove_dir_all(&stage)?;
    fs::create_dir(&stage)?;
    r.download(&remote, &stage, &state.join("output.tmp"))?;
    assert_eq!(reconcile(&stage, &local, &state, &mut ledger)?.conflicts, 1);
    assert_eq!(
        fs::read_to_string(local.join("iPhone note.md"))?,
        "XPS edit"
    );
    assert_eq!(
        fs::read_to_string(cloud.join("iPhone note.md"))?,
        "new phone edit"
    );
    // Failed remote read must never reconcile an empty/partial staging tree.
    assert!(r
        .download(
            "test:/does-not-exist-icloud-test",
            &stage,
            &state.join("output.tmp")
        )
        .is_err());
    assert_eq!(
        fs::read_to_string(local.join("iPhone note.md"))?,
        "XPS edit"
    );
    Ok(())
}
#[test]
#[ignore = "Requires pinned real rclone"]
fn config_encryption_command_contract() -> Result<()> {
    let binary = std::env::var("RCLONE_TEST_BIN")?;
    let t = tempfile::tempdir()?;
    let config = t.path().join("rclone.conf");
    fs::write(&config, "[test]\ntype = local\n")?;
    let status = Command::new(&binary)
        .arg("--config")
        .arg(&config)
        .args([
            "--password-command",
            "printf fixture-only-encryption-key",
            "config",
            "encryption",
            "set",
        ])
        .status()?;
    assert!(status.success());
    assert!(fs::read_to_string(&config)?.contains("RCLONE_ENCRYPT_V0:"));
    assert!(Command::new(&binary)
        .arg("--config")
        .arg(&config)
        .args([
            "--password-command",
            "printf fixture-only-encryption-key",
            "config",
            "encryption",
            "check"
        ])
        .status()?
        .success());
    Ok(())
}
