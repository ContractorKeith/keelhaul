use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;

fn run(root: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_keelhaul"));
    for name in [
        "KEELHAUL_REPO",
        "KEELHAUL_PASSWORD_FILE",
        "KEELHAUL_PASSWORD_COMMAND",
        "KEELHAUL_KEY_FILE",
        "KEELHAUL_NEW_PASSWORD_FILE",
    ] {
        command.env_remove(name);
    }
    command
        .env("HOME", root)
        .arg("-r")
        .arg(root.join("repo"))
        .args(args)
        .output()
        .unwrap()
}

fn init(root: &Path) {
    fs::write(root.join("key"), [3; 32]).unwrap();
    assert!(
        run(
            root,
            &["--key-file", root.join("key").to_str().unwrap(), "init"]
        )
        .status
        .success()
    );
}

#[test]
fn generated_key_must_be_outside_local_repository() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init(root);
    let repo = root.join("repo");
    let source = root.join("key");
    let mut forbidden = vec![repo.join("raw-key"), repo.join("keys/x")];
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&repo, root.join("alias")).unwrap();
        forbidden.push(root.join("alias/raw-key"));
    }
    for destination in forbidden {
        let output = run(
            root,
            &[
                "--key-file",
                source.to_str().unwrap(),
                "key",
                "add",
                "--new-key-file",
                destination.to_str().unwrap(),
                "--json",
            ],
        );
        assert_eq!(
            output.status.code(),
            Some(2),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("outside the repository"));
        assert!(!destination.exists());
    }
    fs::create_dir(root.join("sibling")).unwrap();
    let destination = root.join("sibling/raw-key");
    assert!(
        run(
            root,
            &[
                "--key-file",
                source.to_str().unwrap(),
                "key",
                "add",
                "--new-key-file",
                destination.to_str().unwrap()
            ]
        )
        .status
        .success()
    );
    assert_eq!(fs::read(destination).unwrap().len(), 32);
}

#[test]
fn missing_config_in_existing_repository_is_damage() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init(root);
    fs::remove_file(root.join("repo/config")).unwrap();
    let output = run(
        root,
        &[
            "--key-file",
            root.join("key").to_str().unwrap(),
            "key",
            "list",
            "--json",
        ],
    );
    assert_eq!(output.status.code(), Some(3));
    let v: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(v["error"]["exit"], 3);
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("config: missing")
    );
}

#[test]
fn every_repository_marker_with_missing_config_reaches_damage() {
    for marker in ["keys", "data", "index", "snapshots"] {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("repo").join(marker)).unwrap();
        let output = run(
            root,
            &["--password-command", "printf pw", "key", "list", "--json"],
        );
        assert_eq!(output.status.code(), Some(3));
    }
}

#[test]
fn empty_new_passwords_are_usage_errors() {
    for bytes in [b"".as_slice(), b"\n", b"\r\n"] {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let password = root.join("empty-password");
        fs::write(&password, bytes).unwrap();
        let output = run(
            root,
            &["--password-file", password.to_str().unwrap(), "init"],
        );
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).contains("password is empty"));
        assert!(!root.join("repo/config").exists());
        init(root);
        let output = run(
            root,
            &[
                "--key-file",
                root.join("key").to_str().unwrap(),
                "key",
                "add",
                "--new-password-file",
                password.to_str().unwrap(),
            ],
        );
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).contains("password is empty"));
        assert_eq!(fs::read_dir(root.join("repo/keys")).unwrap().count(), 1);
    }
    let dir = TempDir::new().unwrap();
    let output = run(dir.path(), &["--password-command", "true", "init"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("password is empty"));
}

#[test]
fn password_command_limit_and_help_contract() {
    let dir = TempDir::new().unwrap();
    let output = run(
        dir.path(),
        &["--password-command", "head -c 1048577 /dev/zero", "init"],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("exceeds 1 MiB"));
    let output = run(dir.path(), &["--help"]);
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("sh -c"));
    assert!(help.contains("visible in ps"));
}
