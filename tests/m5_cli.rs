use keelhaul::{
    backend::{Backend, Kind, ObjectName, local::LocalBackend},
    crypto::envelope::Label,
    ops::backup::backup,
    repo::{Repository, lock::Lock},
    secret::Secret,
};
use std::{fs, process::Command, time::SystemTime};
fn command(root: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_keelhaul"));
    for name in [
        "KEELHAUL_PASSWORD_FILE",
        "KEELHAUL_PASSWORD_COMMAND",
        "KEELHAUL_KEY_FILE",
        "KEELHAUL_FAULT",
    ] {
        command.env_remove(name);
    }
    command
        .env("HOME", root)
        .arg("-r")
        .arg(root.join("repo"))
        .arg("--key-file")
        .arg(root.join("key"))
        .arg("--json");
    command
}
#[test]
fn exclusive_lock_blocks_every_locked_command_and_unlock_preserves_it() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    fs::write(root.join("key"), [19; 32]).unwrap();
    let repo = Repository::init(
        LocalBackend::new(root.join("repo")).unwrap(),
        &Secret::KeyFile([19; 32]),
    )
    .unwrap();
    let source = root.join("source");
    fs::write(&source, b"data").unwrap();
    let snap = backup(&repo, std::slice::from_ref(&source), vec![])
        .unwrap()
        .snapshot;
    let slot = repo.list_keys().unwrap()[0].0.clone();
    let lock = Lock::new(true, SystemTime::now()).unwrap();
    let id = repo
        .write_object("locks", Label::Lock, &serde_json::to_vec(&lock).unwrap())
        .unwrap();
    let target = root.join("restore");
    let target = target.to_str().unwrap();
    let source = source.to_str().unwrap();
    for args in [
        vec!["backup", source],
        vec!["restore", &snap, "--target", target],
        vec!["check"],
        vec!["verify"],
        vec!["snapshots"],
        vec!["ls", &snap],
        vec!["forget", "--keep-last", "1"],
        vec!["prune"],
        vec!["key", "remove", &slot],
    ] {
        let output = command(root).args(&args).output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(4),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["error"]["exit"], 4);
        assert!(
            value["error"]["message"]
                .as_str()
                .unwrap()
                .contains("keelhaul unlock")
        );
    }
    let output = command(root).arg("unlock").output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
        serde_json::json!({"removed":[]})
    );
    assert_eq!(repo.backend.list(Kind::Locks).unwrap().len(), 1);
    assert!(
        command(root)
            .args(["key", "list"])
            .output()
            .unwrap()
            .status
            .success()
    );
    repo.backend
        .delete(&ObjectName::new(&format!("locks/{id}")).unwrap())
        .unwrap();
}
#[test]
fn forget_usage_and_explicit_dry_run_have_stable_json_shape() {
    let scratch = tempfile::tempdir().unwrap();
    let real = fs::canonicalize(scratch.path()).unwrap();
    let alias = real.join("alias");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    let root = alias.as_path();
    fs::write(root.join("key"), [19; 32]).unwrap();
    let repo = Repository::init(
        LocalBackend::new(root.join("repo")).unwrap(),
        &Secret::KeyFile([19; 32]),
    )
    .unwrap();
    let source = root.join("source");
    fs::write(&source, b"data").unwrap();
    let stored_path = fs::canonicalize(&source).unwrap();
    assert_ne!(
        source, stored_path,
        "fixture must exercise path normalization"
    );
    let snap = backup(&repo, std::slice::from_ref(&source), vec![])
        .unwrap()
        .snapshot;
    for args in [
        vec!["forget"],
        vec!["forget", &snap, "--keep-last", "1"],
        vec!["prune", "--max-unused", "101"],
    ] {
        assert_eq!(
            command(root).args(args).output().unwrap().status.code(),
            Some(2)
        );
    }
    let output = command(root)
        .args(["forget", &snap, "--dry-run"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        value,
        serde_json::json!({"groups":[{"key":{"host":keelhaul::host::hostname(),"paths":[stored_path.to_str().unwrap()]},"keep":[],"remove":[snap]}]})
    );
    assert_eq!(repo.backend.list(Kind::Snapshots).unwrap().len(), 1);
}
