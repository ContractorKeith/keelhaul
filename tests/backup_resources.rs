use std::{fs, process::Command};

#[test]
fn exhausted_descriptors_abort_without_publishing_a_snapshot() {
    let scratch = tempfile::tempdir().unwrap();
    let source = scratch.path().join("source");
    fs::create_dir(&source).unwrap();
    let mut bytes = vec![0; 32768];
    blake3::Hasher::new()
        .update(b"resource fixture")
        .finalize_xof()
        .fill(&mut bytes);
    for n in 0..2000u64 {
        bytes[..8].copy_from_slice(&n.to_le_bytes());
        fs::write(source.join(format!("{n:04}")), &bytes).unwrap();
    }
    let key = scratch.path().join("key");
    fs::write(&key, [17; 32]).unwrap();
    let repository = scratch.path().join("repo");
    let run = |limit: bool, args: &[&str]| {
        let mut command = Command::new(if limit {
            "sh"
        } else {
            env!("CARGO_BIN_EXE_keelhaul")
        });
        if limit {
            command.args([
                "-c",
                "ulimit -n 48; exec \"$@\"",
                "keelhaul-resource-test",
                env!("CARGO_BIN_EXE_keelhaul"),
            ]);
        }
        command
            .env("HOME", scratch.path())
            .env_remove("KEELHAUL_PASSWORD_FILE")
            .env_remove("KEELHAUL_PASSWORD_COMMAND")
            .env_remove("KEELHAUL_FAULT")
            .arg("-r")
            .arg(&repository)
            .arg("--key-file")
            .arg(&key)
            .args(args)
            .output()
            .unwrap()
    };
    assert!(run(false, &["init"]).status.success());
    let output = run(
        true,
        &["--jobs", "64", "backup", source.to_str().unwrap(), "--json"],
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let message = String::from_utf8_lossy(&output.stderr);
    assert!(
        message.contains("lower --jobs") && message.contains("ulimit -n"),
        "{message}"
    );
    assert_eq!(
        fs::read_dir(repository.join("snapshots"))
            .map(|entries| entries.count())
            .unwrap_or(0),
        0
    );
    assert!(
        run(false, &["--jobs", "4", "backup", source.to_str().unwrap()])
            .status
            .success()
    );
}

#[cfg(feature = "fault-injection")]
#[test]
fn runtime_emfile_on_nth_file_open_aborts_without_snapshot() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    fs::write(root.join("key"), [17; 32]).unwrap();
    fs::create_dir(root.join("source")).unwrap();
    for n in 0..6 {
        fs::write(root.join("source").join(n.to_string()), b"bytes").unwrap();
    }
    let run = |inject: bool, args: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_keelhaul"));
        command
            .env("HOME", root)
            .env_remove("KEELHAUL_PASSWORD_FILE")
            .env_remove("KEELHAUL_PASSWORD_COMMAND")
            .env_remove("KEELHAUL_KEY_FILE")
            .env_remove("KEELHAUL_FAULT")
            .env_remove("KEELHAUL_TEST_EMFILE_AT_OPEN")
            .arg("-r")
            .arg(root.join("repo"))
            .arg("--key-file")
            .arg(root.join("key"))
            .args(args);
        if inject {
            command.env("KEELHAUL_TEST_EMFILE_AT_OPEN", "3");
        }
        command.output().unwrap()
    };
    assert!(run(false, &["init"]).status.success());
    let output = run(
        true,
        &[
            "--jobs",
            "1",
            "backup",
            root.join("source").to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let message = String::from_utf8_lossy(&output.stderr);
    assert!(
        message.contains("source/2") && message.contains("lower --jobs"),
        "{message}"
    );
    assert_eq!(
        fs::read_dir(root.join("repo/snapshots"))
            .map(|entries| entries.count())
            .unwrap_or(0),
        0
    );
    assert!(
        run(false, &["backup", root.join("source").to_str().unwrap()])
            .status
            .success()
    );
}
