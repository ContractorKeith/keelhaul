use keelhaul::{
    backend::local::LocalBackend,
    ops::{backup::backup, listing::ls},
    repo::Repository,
    secret::Secret,
};
#[test]
fn listing_is_preorder() {
    let d = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(d.path()).unwrap();
    let source = root.as_path().join("source");
    std::fs::create_dir_all(source.join("dir")).unwrap();
    std::fs::write(source.join("dir/leaf"), b"bytes").unwrap();
    let repo = Repository::init(
        LocalBackend::new(root.as_path().join("repo")).unwrap(),
        &Secret::KeyFile([7; 32]),
    )
    .unwrap();
    backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
    let entries = ls(&repo, "latest", None).unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].path, source.to_str().unwrap());
    assert!(entries[1].path.ends_with("/dir"));
    assert!(entries[2].path.ends_with("/dir/leaf"));
}
#[cfg(target_os = "linux")]
#[test]
fn json_paths_and_cli_filter_preserve_non_utf8_bytes() {
    use base64::{Engine, prelude::BASE64_STANDARD};
    use std::{
        os::unix::ffi::{OsStrExt, OsStringExt},
        process::Command,
    };
    let d = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(d.path()).unwrap();
    let source = root.as_path().join("source");
    std::fs::create_dir(&source).unwrap();
    let raw = source.join(std::ffi::OsString::from_vec(vec![b'a', 255]));
    let other = source.join(std::ffi::OsString::from_vec(vec![b'a', 254]));
    std::fs::write(&raw, b"first").unwrap();
    std::fs::write(other, b"second").unwrap();
    std::fs::write(root.as_path().join("key"), [7; 32]).unwrap();
    let repo = Repository::init(
        LocalBackend::new(root.as_path().join("repo")).unwrap(),
        &Secret::KeyFile([7; 32]),
    )
    .unwrap();
    backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
    let entries = ls(&repo, "latest", Some(&raw)).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        BASE64_STANDARD
            .decode(entries[0].path_b64.as_ref().unwrap())
            .unwrap(),
        raw.as_os_str().as_bytes()
    );
    let mut command = Command::new(env!("CARGO_BIN_EXE_keelhaul"));
    for key in [
        "KEELHAUL_REPO",
        "KEELHAUL_KEY_FILE",
        "KEELHAUL_PASSWORD_FILE",
        "KEELHAUL_PASSWORD_COMMAND",
    ] {
        command.env_remove(key);
    }
    let output = command
        .env("HOME", root.as_path())
        .arg("-r")
        .arg(root.as_path().join("repo"))
        .arg("--key-file")
        .arg(root.as_path().join("key"))
        .args(["ls", "latest"])
        .arg(&raw)
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rows: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(
        BASE64_STANDARD
            .decode(rows[0]["path_b64"].as_str().unwrap())
            .unwrap(),
        raw.as_os_str().as_bytes()
    );
}
