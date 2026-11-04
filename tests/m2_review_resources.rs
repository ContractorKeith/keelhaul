use keelhaul::{
    backend::local::LocalBackend, ops::backup::backup, repo::Repository, secret::Secret,
};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    process::Command,
};
#[test]
fn three_hundred_reused_directories_restore_under_64_descriptors() {
    let scratch = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(scratch.path()).unwrap();
    let source = root.join("source");
    fs::create_dir(&source).unwrap();
    for i in 0..300 {
        let dir = source.join(format!("dir-{i:03}"));
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("file"), b"content").unwrap();
    }
    let repo_path = root.join("repo");
    let repo = Repository::init(
        LocalBackend::new(&repo_path).unwrap(),
        &Secret::KeyFile([7; 32]),
    )
    .unwrap();
    backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
    fs::write(root.join("key"), [7; 32]).unwrap();
    let target = root.join("out");
    let existing = target.join(source.strip_prefix("/").unwrap());
    fs::create_dir_all(&existing).unwrap();
    let time = filetime::FileTime::from_unix_time(123456, 789);
    for i in 0..300 {
        let dir = existing.join(format!("dir-{i:03}"));
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o751)).unwrap();
        filetime::set_file_mtime(dir, time).unwrap();
    }
    let mut command = Command::new("sh");
    for key in [
        "KEELHAUL_REPO",
        "KEELHAUL_KEY_FILE",
        "KEELHAUL_PASSWORD_FILE",
        "KEELHAUL_PASSWORD_COMMAND",
    ] {
        command.env_remove(key);
    }
    let output = command
        .env("HOME", &root)
        .env("XDG_CACHE_HOME", &root)
        .env("XDG_CONFIG_HOME", &root)
        .args(["-c", "ulimit -n 64; exec \"$@\"", "keelhaul-low-fd"])
        .arg(env!("CARGO_BIN_EXE_keelhaul"))
        .arg("-r")
        .arg(&repo_path)
        .arg("--key-file")
        .arg(root.join("key"))
        .args(["restore", "latest", "--target"])
        .arg(&target)
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for i in 0..300 {
        let dir = existing.join(format!("dir-{i:03}"));
        assert_eq!(fs::read(dir.join("file")).unwrap(), b"content");
        let m = fs::metadata(dir).unwrap();
        assert_eq!(m.mode() & 0o7777, 0o751);
        assert_eq!((m.mtime(), m.mtime_nsec()), (123456, 789));
    }
}
