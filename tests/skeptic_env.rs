use std::{fs, process::Command};
#[test]
fn empty_secret_and_backend_environment_values_are_unset() {
    let root = tempfile::tempdir().unwrap();
    let key = root.path().join("key");
    fs::write(&key, [1; 32]).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_keelhaul"))
        .env_clear()
        .env("HOME", root.path())
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("KEELHAUL_PASSWORD_FILE", "")
        .env("KEELHAUL_PASSWORD_COMMAND", "")
        .env("KEELHAUL_SSH_COMMAND", "")
        .env("KEELHAUL_S3_TIMEOUT_SECS", "")
        .arg("-r")
        .arg(root.path().join("repo"))
        .arg("--key-file")
        .arg(key)
        .arg("init")
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
}
