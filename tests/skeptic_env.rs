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

#[test]
fn secret_flags_override_different_environment_sources_but_peers_conflict() {
    let scratch = tempfile::tempdir().unwrap();
    let key = scratch.path().join("key");
    fs::write(&key, [7; 32]).unwrap();
    for (n, flags, envs, expected) in [
        (
            0,
            vec!["--key-file", key.to_str().unwrap()],
            vec![("KEELHAUL_PASSWORD_FILE", "/not-read")],
            0,
        ),
        (
            1,
            vec!["--password-command", "printf pw"],
            vec![("KEELHAUL_KEY_FILE", "/not-read")],
            0,
        ),
        (
            2,
            vec!["--password-command", "printf pw", "--key-file", "/not-read"],
            vec![],
            2,
        ),
        (
            3,
            vec![],
            vec![
                ("KEELHAUL_PASSWORD_FILE", "/not-read"),
                ("KEELHAUL_KEY_FILE", "/not-read"),
            ],
            2,
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_keelhaul"))
            .env_clear()
            .env("HOME", scratch.path())
            .env("PATH", std::env::var_os("PATH").unwrap())
            .envs(envs)
            .arg("-r")
            .arg(scratch.path().join(format!("repo{n}")))
            .args(flags)
            .arg("init")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(expected), "{output:?}");
    }
}
