use serde_json::Value;
use std::{
    fs,
    process::{Command, Output},
};
use tempfile::TempDir;

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_keelhaul"));
    for env in [
        "KEELHAUL_REPO",
        "KEELHAUL_PASSWORD_FILE",
        "KEELHAUL_PASSWORD_COMMAND",
        "KEELHAUL_KEY_FILE",
        "KEELHAUL_NEW_PASSWORD_FILE",
    ] {
        command.env_remove(env);
    }
    command
}

fn assert_json_error(output: Output, expected: i32) -> Value {
    assert_eq!(output.status.code(), Some(expected));
    let v: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(v["error"]["exit"], expected);
    assert!(
        v["error"]["message"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
    );
    assert!(!output.stderr.is_empty());
    v
}

#[test]
fn help_hides_all_secret_environment_values() {
    let dir = TempDir::new().unwrap();
    for args in [vec!["--help"], vec!["key", "add", "--help"]] {
        let output = command()
            .env("HOME", dir.path())
            .env("KEELHAUL_PASSWORD_COMMAND", "echo PRIVATE_SECRET_MARKER")
            .env("KEELHAUL_PASSWORD_FILE", "PRIVATE_SECRET_MARKER")
            .env("KEELHAUL_KEY_FILE", "PRIVATE_SECRET_MARKER")
            .env("KEELHAUL_NEW_PASSWORD_FILE", "PRIVATE_SECRET_MARKER")
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(!String::from_utf8_lossy(&output.stdout).contains("PRIVATE_SECRET_MARKER"));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("PRIVATE_SECRET_MARKER"));
    }
}

#[test]
fn wrong_password_unreadable_slot_and_unknown_id_have_useful_errors() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    let repo = root.join("repo");
    let pw = root.join("password");
    fs::write(&pw, "correct\n").unwrap();
    let run = |args: &[&str]| {
        command()
            .env("HOME", root)
            .arg("-r")
            .arg(&repo)
            .arg("--password-file")
            .arg(&pw)
            .args(args)
            .output()
            .unwrap()
    };
    assert!(run(&["init"]).status.success());
    let v = assert_json_error(run(&["key", "remove", "deadbeef", "--json"]), 1);
    assert_eq!(v["error"]["message"], "key slot deadbeef not found");
    fs::write(&pw, "wrong\n").unwrap();
    let v = assert_json_error(run(&["key", "list", "--json"]), 1);
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no key slot matches")
    );
    fs::write(&pw, "correct\n").unwrap();
    let slot = fs::read_dir(repo.join("keys"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::write(&slot, b"{").unwrap();
    let v = assert_json_error(run(&["key", "list", "--json"]), 3);
    let expected = format!("keys/{}", slot.file_name().unwrap().to_str().unwrap());
    let message = v["error"]["message"].as_str().unwrap();
    assert!(message.contains(&expected));
    assert!(message.contains("bad-format"));
    assert!(!message.contains("wrong secret"));
}

#[test]
fn new_password_flag_env_conflicts_and_json_usage() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    let repo = root.join("repo");
    let key = root.join("key");
    let pw = root.join("pw");
    fs::write(&key, [7; 32]).unwrap();
    fs::write(&pw, "new\n").unwrap();
    let run = |args: &[&str], env_new: bool| {
        let mut c = command();
        c.env("HOME", root)
            .arg("-r")
            .arg(&repo)
            .arg("--key-file")
            .arg(&key)
            .args(args);
        if env_new {
            c.env("KEELHAUL_NEW_PASSWORD_FILE", &pw);
        }
        c.output().unwrap()
    };
    assert!(run(&["init"], false).status.success());
    assert!(run(&["key", "add", "--json"], true).status.success());
    let output = run(
        &[
            "key",
            "add",
            "--new-key-file",
            root.join("new-key").to_str().unwrap(),
            "--json",
        ],
        true,
    );
    assert_json_error(output, 2);
    let output = run(
        &[
            "key",
            "add",
            "--new-password-file",
            pw.to_str().unwrap(),
            "--new-key-file",
            root.join("new-key").to_str().unwrap(),
            "--json",
        ],
        false,
    );
    assert_json_error(output, 2);
    assert!(!root.join("new-key").exists());
}
