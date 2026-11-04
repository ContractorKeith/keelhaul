//! Real binary and scratch-only repository helpers for crash tests.
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::OnceLock,
};
pub fn binary() -> &'static Path {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let mut command = Command::new(env!("CARGO"));
        command.args([
            "build",
            "--locked",
            "--features",
            "fault-injection",
            "--bin",
            "keelhaul",
        ]);
        command.arg("--release");
        assert!(command.status().unwrap().success());
        Path::new(env!("CARGO_BIN_EXE_keelhaul"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("release/keelhaul")
    })
}
pub fn command(root: &Path) -> Command {
    let mut command = Command::new(binary());
    for name in [
        "KEELHAUL_REPO",
        "KEELHAUL_PASSWORD_FILE",
        "KEELHAUL_PASSWORD_COMMAND",
        "KEELHAUL_KEY_FILE",
        "KEELHAUL_FAULT",
    ] {
        command.env_remove(name);
    }
    command
        .env("HOME", root)
        .env("XDG_CONFIG_HOME", root)
        .env("XDG_CACHE_HOME", root)
        .arg("-r")
        .arg(root.join("repo"))
        .arg("--key-file")
        .arg(root.join("key"))
        .arg("--json");
    command
}
pub fn ok(output: Output) -> Value {
    assert!(
        output.status.success(),
        "status {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
pub fn backup(root: &Path) -> Value {
    ok(command(root)
        .arg("backup")
        .arg(root.join("source"))
        .output()
        .unwrap())
}
pub fn check(root: &Path) {
    // M3 supplies structural check after the required rebase. Until then the
    // restore assertion below proves every committed snapshot is readable.
    let output = command(root).arg("check").output().unwrap();
    if output.status.code() != Some(2) {
        ok(output);
    } else {
        let snapshots = ok(command(root).arg("snapshots").output().unwrap());
        for snapshot in snapshots.as_array().unwrap() {
            let target = root.join("check-out");
            ok(command(root)
                .arg("restore")
                .arg(snapshot["id"].as_str().unwrap())
                .arg("--target")
                .arg(&target)
                .output()
                .unwrap());
            fs::remove_dir_all(target).unwrap();
        }
    }
}
pub fn restored(root: &Path) {
    let target = root.join("out");
    ok(command(root)
        .args(["restore", "latest", "--target"])
        .arg(&target)
        .output()
        .unwrap());
    let source = root.join("source");
    let output = target.join(source.strip_prefix("/").unwrap());
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        assert_eq!(
            fs::read(entry.path()).unwrap(),
            fs::read(output.join(entry.file_name())).unwrap()
        );
    }
    fs::remove_dir_all(target).unwrap();
}
pub fn fixture(root: &Path) {
    fixture_size(root, 4 * 1024 * 1024);
}
pub fn fixture_size(root: &Path, size: usize) {
    fixture_sparse(root, size, size);
}
pub fn fixture_sparse(root: &Path, size: usize, entropy: usize) {
    fs::write(root.join("key"), [17; 32]).unwrap();
    ok(command(root).arg("init").output().unwrap());
    fs::create_dir(root.join("source")).unwrap();
    let mut bytes = vec![0; entropy];
    for n in 0..4u64 {
        blake3::Hasher::new()
            .update(&n.to_le_bytes())
            .finalize_xof()
            .fill(&mut bytes);
        let path = root.join(format!("source/{n}"));
        fs::write(&path, &bytes).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_len(size as u64)
            .unwrap();
    }
}
