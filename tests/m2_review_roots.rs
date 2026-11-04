use keelhaul::{backend::local::LocalBackend, repo::Repository, secret::Secret};
use serde_json::Value;
use std::{
    fs,
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, Output},
};
struct Unblock(PathBuf);
impl Drop for Unblock {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o700));
    }
}
fn command(root: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_keelhaul"));
    for name in [
        "KEELHAUL_REPO",
        "KEELHAUL_KEY_FILE",
        "KEELHAUL_PASSWORD_FILE",
        "KEELHAUL_PASSWORD_COMMAND",
    ] {
        command.env_remove(name);
    }
    command
        .env("HOME", root)
        .env("XDG_CACHE_HOME", root)
        .env("XDG_CONFIG_HOME", root)
        .arg("-r")
        .arg(root.join("repo"))
        .arg("--key-file")
        .arg(root.join("key"))
        .args(args);
    if uzers::get_effective_uid() == 0 {
        command.gid(65534).uid(65534);
    }
    command.output().unwrap()
}
fn json(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
#[test]
fn inaccessible_ancestor_is_skipped_while_good_roots_succeed_and_all_failed_writes_no_snapshot() {
    let scratch = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(scratch.path()).unwrap();
    fs::write(root.join("key"), [7; 32]).unwrap();
    let _repo = Repository::init(
        LocalBackend::new(root.join("repo")).unwrap(),
        &Secret::KeyFile([7; 32]),
    )
    .unwrap();
    let good = root.join("good");
    fs::write(&good, b"good bytes").unwrap();
    let blocked = root.join("denied");
    fs::create_dir_all(blocked.join("inner")).unwrap();
    let denied = blocked.join("inner/file");
    fs::write(&denied, b"unreadable").unwrap();
    // Root-run harnesses drop only the CLI subprocess so permissions are real.
    if uzers::get_effective_uid() == 0 {
        let mut pending = vec![root.clone()];
        while let Some(path) = pending.pop() {
            if path.is_dir() {
                for child in fs::read_dir(&path).unwrap() {
                    pending.push(child.unwrap().path());
                }
            }
            std::os::unix::fs::chown(path, Some(65534), Some(65534)).unwrap();
        }
    }
    let _unblock = Unblock(blocked.clone());
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o000)).unwrap();
    let failed = command(&root, &["backup", denied.to_str().unwrap(), "--json"]);
    assert_eq!(failed.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&failed.stderr).contains(denied.to_str().unwrap()));
    assert!(
        json(command(&root, &["snapshots", "--json"]))
            .as_array()
            .unwrap()
            .is_empty()
    );
    let output = command(
        &root,
        &[
            "backup",
            denied.to_str().unwrap(),
            good.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("warning:"));
    let report = json(output);
    assert_eq!(report["summary"]["files_skipped"], 1);
    assert_eq!(report["summary"]["files_new"], 1);
    assert_eq!(report["skipped"].as_array().unwrap().len(), 1);
    assert_eq!(report["skipped"][0]["path"], denied.to_str().unwrap());
    assert!(
        report["skipped"][0]["reason"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("permission denied")
    );
    let snapshots = json(command(&root, &["snapshots", "--json"]));
    assert_eq!(snapshots.as_array().unwrap().len(), 1);
    assert_eq!(snapshots[0]["paths"], serde_json::json!([good]));
}
