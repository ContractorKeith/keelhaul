use std::{fs, os::unix::fs::symlink, process::Command};
#[test]
fn symlink_root_warns_and_trailing_slash_follows_target() {
    let scratch = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(scratch.path()).unwrap();
    fs::create_dir(root.join("source")).unwrap();
    fs::write(root.join("source/file"), b"bytes").unwrap();
    fs::write(root.join("key"), [7; 32]).unwrap();
    symlink("source", root.join("sl")).unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_keelhaul"))
            .env("HOME", &root)
            .env_remove("KEELHAUL_PASSWORD_FILE")
            .env_remove("KEELHAUL_PASSWORD_COMMAND")
            .env_remove("KEELHAUL_KEY_FILE")
            .arg("-r")
            .arg(root.join("repo"))
            .arg("--key-file")
            .arg(root.join("key"))
            .args(args)
            .output()
            .unwrap()
    };
    assert!(run(&["init"]).status.success());
    let output = run(&["backup", root.join("sl").to_str().unwrap(), "--json"]);
    assert!(output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("is a symlink; storing the link only (add a trailing /")
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["summary"]["files_new"], 0);
    let id = value["snapshot"].as_str().unwrap();
    assert!(
        run(&[
            "restore",
            id,
            "--target",
            root.join("out-link").to_str().unwrap()
        ])
        .status
        .success()
    );
    assert_eq!(
        fs::read_link(
            root.join("out-link")
                .join(root.strip_prefix("/").unwrap())
                .join("sl")
        )
        .unwrap(),
        std::path::Path::new("source")
    );
    let output = run(&[
        "backup",
        &format!("{}/", root.join("sl").display()),
        "--json",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains("is a symlink"));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["summary"]["files_new"], 1);
    let id = value["snapshot"].as_str().unwrap();
    assert!(
        run(&[
            "restore",
            id,
            "--target",
            root.join("out-dir").to_str().unwrap()
        ])
        .status
        .success()
    );
    assert_eq!(
        fs::read(
            root.join("out-dir")
                .join(root.strip_prefix("/").unwrap())
                .join("source/file")
        )
        .unwrap(),
        b"bytes"
    );
}
