use std::{
    fs,
    process::{Command, Output},
};
fn cli(repo: &std::path::Path, key: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_keelhaul"))
        .args(["-r"])
        .arg(repo)
        .args(["--key-file"])
        .arg(key)
        .arg("--json")
        .args(args)
        .env_remove("KEELHAUL_PASSWORD_FILE")
        .env_remove("KEELHAUL_PASSWORD_COMMAND")
        .output()
        .unwrap()
}
#[test]
fn golden_audit_documents_and_usage_errors() {
    let scratch = tempfile::tempdir().unwrap();
    let repo = scratch.path().join("repo");
    let key = scratch.path().join("key");
    fs::write(&key, [7; 32]).unwrap();
    assert!(cli(&repo, &key, &["init"]).status.success());
    for command in ["check", "verify"] {
        let out = cli(&repo, &key, &[command]);
        assert!(out.status.success());
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            "{\"damage\":[],\"notes\":[]}\n"
        );
    }
    fs::create_dir_all(repo.join("index")).unwrap();
    fs::write(repo.join("index").join("0".repeat(64)), b"invalid").unwrap();
    fs::write(repo.join("config.tmp-0123456789abcdef"), b"unfinished").unwrap();
    fs::write(repo.join("config.unknown"), b"foreign").unwrap();
    for command in ["check", "verify"] {
        let out = cli(&repo, &key, &[command]);
        assert_eq!(out.status.code(), Some(3));
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            include_str!("../testdata/golden/audit.json")
        );
    }
    for subset in ["0/3", "4/3", "1/0", "x/3", "1", "1/2/3", "-1/3"] {
        let out = cli(&repo, &key, &["verify", "--subset", subset]);
        assert_eq!(out.status.code(), Some(2), "{subset}");
    }
    fs::write(&key, [9; 32]).unwrap();
    assert_eq!(cli(&repo, &key, &["check"]).status.code(), Some(1));
}
#[test]
fn damaged_config_has_one_audit_document_and_restore_uses_shared_vocabulary() {
    let scratch = tempfile::tempdir().unwrap();
    let repo = scratch.path().join("repo");
    let key = scratch.path().join("key");
    fs::write(&key, [7; 32]).unwrap();
    assert!(cli(&repo, &key, &["init"]).status.success());
    fs::write(repo.join("config"), b"truncated").unwrap();
    fs::write(repo.join("config.unknown"), b"foreign").unwrap();
    for command in ["check", "verify"] {
        let out = cli(&repo, &key, &[command]);
        assert_eq!(out.status.code(), Some(3));
        let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(doc["damage"][0]["object"], "config");
        assert_eq!(doc["damage"][0]["kind"], "config");
        assert_eq!(doc["damage"][0]["problem"], "bad-format");
    }
    let out = cli(
        &repo,
        &key,
        &[
            "restore",
            "latest",
            "--target",
            scratch.path().join("target").to_str().unwrap(),
        ],
    );
    assert_eq!(out.status.code(), Some(3));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["restored"], 0);
    assert_eq!(doc["damage"][0]["object"], "config");
}
