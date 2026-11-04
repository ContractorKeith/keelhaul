use super::{Fixture, docker};
use std::{
    path::Path,
    process::{Command, Output},
};

fn command(repo: &str, root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_keelhaul"));
    for variable in [
        "KEELHAUL_REPO",
        "KEELHAUL_PASSWORD_FILE",
        "KEELHAUL_PASSWORD_COMMAND",
        "KEELHAUL_KEY_FILE",
        "KEELHAUL_NEW_PASSWORD_FILE",
        "KEELHAUL_S3_TIMEOUT_SECS",
        "AWS_SESSION_TOKEN",
    ] {
        command.env_remove(variable);
    }
    command
        .env("HOME", root)
        .env("AWS_ACCESS_KEY_ID", "keelhaul-test")
        .env("AWS_SECRET_ACCESS_KEY", "keelhaul-test-password")
        .env("AWS_REGION", "us-east-1")
        .arg("-r")
        .arg(repo);
    command
}

fn json(output: Output, exit: i32) -> serde_json::Value {
    assert_eq!(
        output.status.code(),
        Some(exit),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn s3_real_cli_init_add_key_and_list_json() {
    if !docker::enabled() {
        return;
    }
    let fixture = Fixture::new();
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let repo = format!(
        "s3:http://127.0.0.1:7070/{}/cli/repo",
        fixture.bucket.name()
    );
    let password = root.join("password");
    let key = root.join("key");
    std::fs::write(&password, "test-password\n").unwrap();
    for timeout in ["0", "3601", "18446744073709551615"] {
        let output = command(&repo, root)
            .env("KEELHAUL_S3_TIMEOUT_SECS", timeout)
            .arg("--password-file")
            .arg(&password)
            .args(["init", "--json"])
            .output()
            .unwrap();
        let error = json(output, 2);
        assert!(
            error["error"]["message"]
                .as_str()
                .unwrap()
                .contains("1..=3600")
        );
    }
    let initialized = command(&repo, root)
        .arg("--password-file")
        .arg(&password)
        .args(["init", "--json", "-v"])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&initialized.stderr).contains("S3 create-once: If-None-Match: *")
    );
    assert!(json(initialized, 0)["id"].is_string());
    let added = command(&repo, root)
        .arg("--password-file")
        .arg(&password)
        .args(["key", "add", "--new-key-file"])
        .arg(&key)
        .arg("--json")
        .output()
        .unwrap();
    let new_id = json(added, 0)["id"].as_str().unwrap().to_owned();
    assert_eq!(std::fs::metadata(&key).unwrap().len(), 32);
    // Opening with the new key proves the new slot can unlock the S3 repository.
    let listing = command(&repo, root)
        .arg("--key-file")
        .arg(&key)
        .args(["key", "list", "--json"])
        .output()
        .unwrap();
    assert!(listing.stderr.is_empty());
    let listing = json(listing, 0);
    let slots = listing["slots"].as_array().unwrap();
    assert_eq!(slots.len(), 2);
    assert!(
        slots
            .iter()
            .any(|slot| slot["id"] == new_id && slot["kind"] == "keyfile")
    );
}

#[test]
fn s3_real_cli_missing_bucket_exits_one() {
    if !docker::enabled() {
        return;
    }
    docker::require_s3();
    let scratch = tempfile::tempdir().unwrap();
    let password = scratch.path().join("password");
    std::fs::write(&password, "test-password\n").unwrap();
    let mut nonce = [0; 8];
    getrandom::fill(&mut nonce).unwrap();
    let bucket = format!("keelhaul-missing-{}", hex::encode(nonce));
    let repo = format!("s3:http://127.0.0.1:7070/{bucket}");
    let output = command(&repo, scratch.path())
        .arg("--password-file")
        .arg(&password)
        .args(["key", "list", "--json"])
        .output()
        .unwrap();
    let error = json(output, 1);
    assert_eq!(
        error["error"]["message"],
        format!("bucket not found: {bucket}")
    );
}
