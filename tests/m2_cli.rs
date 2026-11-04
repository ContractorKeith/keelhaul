use serde_json::Value;
use std::{
    path::Path,
    process::{Command, Output},
};
fn cli(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_keelhaul"))
        .env_remove("KEELHAUL_REPO")
        .env_remove("KEELHAUL_PASSWORD_FILE")
        .env_remove("KEELHAUL_PASSWORD_COMMAND")
        .env_remove("KEELHAUL_KEY_FILE")
        .env("HOME", root)
        .env("XDG_CONFIG_HOME", root)
        .env("XDG_CACHE_HOME", root)
        .arg("-r")
        .arg(root.join("repo"))
        .arg("--key-file")
        .arg(root.join("key"))
        .args(args)
        .output()
        .unwrap()
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
fn json_contract_filters_ls_include_conflict_and_damage_exit() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path();
    std::fs::write(root.join("key"), [1; 32]).unwrap();
    json(cli(root, &["init", "--json"]));
    let source = root.join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("file"), b"hello").unwrap();
    let s = source.to_str().unwrap();
    let report = json(cli(root, &["backup", s, "--tag", "test", "--json"]));
    assert_eq!(
        report
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        ["skipped", "snapshot", "summary"]
    );
    assert_eq!(report["summary"]["data_blobs_added"], 1);
    assert_eq!(report["skipped"], serde_json::json!([]));
    let again = json(cli(root, &["backup", s, "--tag", "test", "--json"]));
    assert_eq!(again["summary"]["data_blobs_added"], 0);
    let snapshots = json(cli(
        root,
        &["snapshots", "--tag", "test", "--path", s, "--json"],
    ));
    assert_eq!(snapshots.as_array().unwrap().len(), 2);
    assert!(
        json(cli(root, &["snapshots", "--host", "absent", "--json"]))
            .as_array()
            .unwrap()
            .is_empty()
    );
    let listing = json(cli(root, &["ls", "latest", "--json"]));
    assert_eq!(listing.as_array().unwrap().len(), 2);
    let target = root.join("out");
    let output = json(cli(
        root,
        &[
            "restore",
            "latest",
            "--target",
            target.to_str().unwrap(),
            "--json",
        ],
    ));
    assert_eq!(output, serde_json::json!({"restored":2,"damaged":[]}));
    let recorded = snapshots[0]["paths"][0].as_str().unwrap();
    assert_eq!(
        std::fs::read(target.join(recorded.trim_start_matches('/')).join("file")).unwrap(),
        b"hello"
    );
    let conflict = cli(
        root,
        &[
            "restore",
            "latest",
            "--target",
            target.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(conflict.status.code(), Some(1));
    let _: Value = serde_json::from_slice(&conflict.stdout).unwrap();
    let data = std::fs::read_dir(root.join("repo/data"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let pack = std::fs::read_dir(data)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut bytes = std::fs::read(&pack).unwrap();
    bytes[24] ^= 1;
    std::fs::write(pack, bytes).unwrap();
    let damaged = cli(
        root,
        &[
            "restore",
            "latest",
            "--target",
            root.join("damaged").to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(
        damaged.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&damaged.stderr)
    );
    let report: Value = serde_json::from_slice(&damaged.stdout).unwrap();
    assert_eq!(report["damaged"].as_array().unwrap().len(), 1);
    assert!(
        !root
            .join("damaged")
            .join(recorded.trim_start_matches('/'))
            .join("file")
            .exists()
    );
}

#[test]
fn snapshots_path_filter_uses_backup_normalization_through_a_symlinked_parent() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path();
    std::fs::write(root.join("key"), [1; 32]).unwrap();
    json(cli(root, &["init", "--json"]));
    let actual = root.join("actual");
    std::fs::create_dir_all(actual.join("source")).unwrap();
    std::fs::write(actual.join("source/file"), b"bytes").unwrap();
    std::os::unix::fs::symlink(&actual, root.join("alias")).unwrap();
    let alias = root.join("alias/source");
    let source = alias.to_str().unwrap();
    json(cli(root, &["backup", source, "--json"]));
    let snapshots = json(cli(root, &["snapshots", "--path", source, "--json"]));
    assert_eq!(snapshots.as_array().unwrap().len(), 1);
}

#[test]
fn snapshots_path_filter_keeps_working_after_the_source_parent_is_removed() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path();
    std::fs::write(root.join("key"), [1; 32]).unwrap();
    json(cli(root, &["init", "--json"]));
    let parent = std::fs::canonicalize(root).unwrap().join("gone");
    std::fs::create_dir(&parent).unwrap();
    let source = parent.join("file");
    std::fs::write(&source, b"bytes").unwrap();
    json(cli(root, &["backup", source.to_str().unwrap(), "--json"]));
    std::fs::remove_file(&source).unwrap();
    std::fs::remove_dir(&parent).unwrap();
    let snapshots = json(cli(
        root,
        &["snapshots", "--path", source.to_str().unwrap(), "--json"],
    ));
    assert_eq!(snapshots.as_array().unwrap().len(), 1);
}

#[test]
fn missing_source_does_not_back_up_a_live_file_at_its_lexical_fallback() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path();
    std::fs::write(root.join("key"), [1; 32]).unwrap();
    json(cli(root, &["init", "--json"]));
    std::fs::write(root.join("victim"), b"not the requested path").unwrap();
    let missing = root.join("missing/../victim");
    let failed = cli(root, &["backup", missing.to_str().unwrap(), "--json"]);
    assert_eq!(failed.status.code(), Some(1));
    assert!(
        json(cli(root, &["snapshots", "--json"]))
            .as_array()
            .unwrap()
            .is_empty()
    );
    let mixed = json(cli(
        root,
        &[
            "backup",
            missing.to_str().unwrap(),
            root.join("victim").to_str().unwrap(),
            "--json",
        ],
    ));
    assert_eq!(mixed["summary"]["files_new"], 1);
    assert_eq!(mixed["summary"]["files_skipped"], 1);
}
