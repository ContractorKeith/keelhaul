use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;

fn cli(root: &Path, source: &[&str], args: &[&str], new_password: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_keelhaul"));
    for name in [
        "KEELHAUL_REPO",
        "KEELHAUL_PASSWORD_FILE",
        "KEELHAUL_PASSWORD_COMMAND",
        "KEELHAUL_KEY_FILE",
        "KEELHAUL_NEW_PASSWORD_FILE",
        "HOSTNAME",
        "USER",
    ] {
        command.env_remove(name);
    }
    command
        .env("HOME", root)
        .arg("-r")
        .arg(root.join("repo"))
        .args(source)
        .args(args);
    if let Some(path) = new_password {
        command.env("KEELHAUL_NEW_PASSWORD_FILE", path);
    }
    command.output().unwrap()
}

fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn password_to_keyfile_rotation_and_json_contract() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    let pw = root.join("pw");
    fs::write(&pw, "pw1\r\n").unwrap();
    let source = ["--password-file", pw.to_str().unwrap()];
    let initialized = success(cli(root, &source, &["init", "--json"], None));
    assert_eq!(initialized.as_object().unwrap().len(), 1);
    assert_eq!(initialized["id"].as_str().unwrap().len(), 64);
    let listed = success(cli(root, &source, &["key", "list", "--json"], None));
    let slot = &listed["slots"][0];
    let original_id = slot["id"].as_str().unwrap();
    assert_eq!(listed["slots"].as_array().unwrap().len(), 1);
    assert!(!slot["username"].as_str().unwrap().is_empty());
    let created = slot["created"].as_str().unwrap();
    assert_eq!(created.len(), 30);
    assert_eq!(
        listed,
        json!({"slots":[{"id":original_id,"kind":"password","created":created,"host":gethostname::gethostname().to_string_lossy(),"username":keelhaul::host::username()}]})
    );
    let key = root.join("new-key");
    let added = success(cli(
        root,
        &source,
        &[
            "key",
            "add",
            "--new-key-file",
            key.to_str().unwrap(),
            "--json",
        ],
        None,
    ));
    assert_eq!(added.as_object().unwrap().len(), 1);
    assert_eq!(fs::read(&key).unwrap().len(), 32);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&key).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let existing = cli(
        root,
        &source,
        &["key", "add", "--new-key-file", key.to_str().unwrap()],
        None,
    );
    assert_eq!(existing.status.code(), Some(1));
    let removed = success(cli(
        root,
        &source,
        &["key", "remove", original_id, "--json"],
        None,
    ));
    assert_eq!(removed, json!({"removed":original_id}));
    assert_eq!(
        cli(root, &source, &["key", "list"], None).status.code(),
        Some(1)
    );
    let key_source = ["--key-file", key.to_str().unwrap()];
    let listed = success(cli(root, &key_source, &["key", "list", "--json"], None));
    assert_eq!(listed["slots"].as_array().unwrap().len(), 1);
    assert_eq!(listed["slots"][0]["kind"], "keyfile");
    assert_eq!(listed["slots"][0]["id"], added["id"]);
    assert_eq!(
        cli(
            root,
            &key_source,
            &["key", "remove", added["id"].as_str().unwrap()],
            None
        )
        .status
        .code(),
        Some(1)
    );
    fs::write(root.join("repo/config"), b"corrupt").unwrap();
    assert_eq!(
        cli(root, &key_source, &["key", "list"], None).status.code(),
        Some(3)
    );
}

#[test]
fn command_source_new_password_and_conflicting_sources() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    let source = ["--password-command", "printf 'pw1\n'"];
    success(cli(root, &source, &["init", "--json"], None));
    let pw = root.join("new-pw");
    fs::write(&pw, "pw2\n").unwrap();
    success(cli(root, &source, &["key", "add", "--json"], Some(&pw)));
    success(cli(
        root,
        &["--password-file", pw.to_str().unwrap()],
        &["key", "list", "--json"],
        None,
    ));
    let conflict = cli(
        root,
        &["--password-file", "/not-read", "--key-file", "/not-read"],
        &["init"],
        None,
    );
    assert_eq!(conflict.status.code(), Some(2));
    assert!(!String::from_utf8_lossy(&conflict.stderr).contains("pw1"));
    assert_eq!(
        cli(
            root,
            &["--password-command", "exit 1"],
            &["key", "list"],
            None
        )
        .status
        .code(),
        Some(1)
    );
    assert_eq!(
        cli(root, &[], &["key", "list"], None).status.code(),
        Some(1)
    );
}

#[test]
fn keyfile_init_and_invalid_secret_files() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    let key = root.join("key");
    fs::write(&key, [8; 32]).unwrap();
    let source = ["--key-file", key.to_str().unwrap()];
    success(cli(root, &source, &["init", "--json"], None));
    success(cli(root, &source, &["key", "list", "--json"], None));
    fs::write(&key, [8; 31]).unwrap();
    assert_eq!(
        cli(root, &source, &["key", "list"], None).status.code(),
        Some(1)
    );
    fs::write(&key, [0xff]).unwrap();
    assert_eq!(
        cli(
            root,
            &["--password-file", key.to_str().unwrap()],
            &["key", "list"],
            None
        )
        .status
        .code(),
        Some(1)
    );
}

#[test]
fn missing_repository_is_reported_without_creating_directory() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    for exists in [false, true] {
        if exists {
            fs::create_dir(root.join("repo")).unwrap();
        }
        let output = cli(
            root,
            &["--password-command", "printf pw"],
            &["key", "list"],
            None,
        );
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stderr).contains(&format!(
            "repository not found: {}",
            root.join("repo").display()
        )));
        assert_eq!(root.join("repo").exists(), exists);
    }
}
