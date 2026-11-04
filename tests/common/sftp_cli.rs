use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
fn command(repo: &str, config: &Path, scratch: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_keelhaul"));
    for variable in [
        "KEELHAUL_REPO",
        "KEELHAUL_PASSWORD_FILE",
        "KEELHAUL_PASSWORD_COMMAND",
        "KEELHAUL_KEY_FILE",
        "KEELHAUL_NEW_PASSWORD_FILE",
        "KEELHAUL_SSH_COMMAND",
    ] {
        command.env_remove(variable);
    }
    command
        .env("HOME", scratch)
        .env(
            "KEELHAUL_SSH_ARGS",
            format!("-F {}", shell_words::quote(&config.to_string_lossy())),
        )
        .arg("-r")
        .arg(repo);
    command
}
fn json(output: Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
pub fn run(config: &Path, root: &str, scratch: &Path) {
    let repo = format!("sftp:keelhaul-e2e:{root}-cli");
    let password = scratch.join("password");
    let key = scratch.join("new-key");
    fs::write(&password, b"test-password\n").unwrap();
    let initialized = command(&repo, config, scratch)
        .arg("--password-file")
        .arg(&password)
        .args(["init", "--json"])
        .output()
        .unwrap();
    assert!(json(initialized)["id"].is_string());
    let added = command(&repo, config, scratch)
        .arg("--password-file")
        .arg(&password)
        .args(["key", "add", "--new-key-file"])
        .arg(&key)
        .arg("--json")
        .output()
        .unwrap();
    let new_id = json(added)["id"].as_str().unwrap().to_owned();
    assert_eq!(fs::metadata(&key).unwrap().len(), 32);
    let listing = command(&repo, config, scratch)
        .arg("--key-file")
        .arg(&key)
        .args(["key", "list", "--json"])
        .output()
        .unwrap();
    let listing = json(listing);
    let slots = listing["slots"].as_array().unwrap();
    assert_eq!(slots.len(), 2);
    assert!(
        slots
            .iter()
            .any(|slot| slot["id"] == new_id && slot["kind"] == "keyfile")
    );
    assert!(slots.iter().any(|slot| slot["kind"] == "password"));
    // A non-initializing command must not create a missing remote directory.
    let missing = format!("{root}-missing");
    let output = command(&format!("sftp:keelhaul-e2e:{missing}"), config, scratch)
        .arg("--key-file")
        .arg(&key)
        .args(["key", "list", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let mut observer = super::connect(config, root);
    assert_eq!(
        observer.attrs(&missing).unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
}
