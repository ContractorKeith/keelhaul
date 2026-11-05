use std::{
    fs,
    os::unix::{fs::PermissionsExt, process::CommandExt},
    process::Command,
};
#[test]
fn unreadable_file_writes_snapshot_with_exit_five_but_fifo_is_expected() {
    let scratch = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(scratch.path()).unwrap();
    fs::write(root.join("key"), [7; 32]).unwrap();
    fs::create_dir(root.join("source")).unwrap();
    let denied = root.join("source/denied");
    fs::write(&denied, b"private").unwrap();
    if uzers::get_effective_uid() == 0 {
        for path in [&root, &root.join("key"), &root.join("source"), &denied] {
            std::os::unix::fs::chown(path, Some(65534), Some(65534)).unwrap();
        }
    }
    let run = |args: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_keelhaul"));
        command
            .env("HOME", &root)
            .env_remove("KEELHAUL_PASSWORD_FILE")
            .env_remove("KEELHAUL_PASSWORD_COMMAND")
            .env_remove("KEELHAUL_KEY_FILE")
            .arg("-r")
            .arg(root.join("repo"))
            .arg("--key-file")
            .arg(root.join("key"))
            .args(args);
        if uzers::get_effective_uid() == 0 {
            command.gid(65534).uid(65534);
        }
        command.output().unwrap()
    };
    assert!(run(&["init"]).status.success());
    fs::set_permissions(&denied, fs::Permissions::from_mode(0o000)).unwrap();
    let output = run(&["backup", root.join("source").to_str().unwrap(), "--json"]);
    fs::set_permissions(&denied, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        output.status.code(),
        Some(5),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["incomplete"], true);
    assert_eq!(value["summary"]["files_skipped"], 1);
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("snapshot written; 1 skipped (0 unsupported types)")
    );
    let output = run(&["snapshots", "--json"]);
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value.as_array().unwrap().len(), 1);
    fs::remove_file(&denied).unwrap();
    assert!(
        Command::new("mkfifo")
            .arg(root.join("source/fifo"))
            .status()
            .unwrap()
            .success()
    );
    let output = run(&["backup", root.join("source").to_str().unwrap(), "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["incomplete"], false);
    assert_eq!(value["skipped"].as_array().unwrap().len(), 1);
}
