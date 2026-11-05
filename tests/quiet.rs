use std::{
    path::Path,
    process::{Command, Output},
};
fn run(repo: &Path, key: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_keelhaul"))
        .arg("-q")
        .arg("-r")
        .arg(repo)
        .arg("--key-file")
        .arg(key)
        .args(args)
        .output()
        .unwrap()
}
#[test]
fn every_happy_command_is_quiet_but_damage_and_json_survive() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let key = dir.path().join("key");
    std::fs::write(&key, [7; 32]).unwrap();
    let source = dir.path().join("file");
    std::fs::write(&source, b"content").unwrap();
    let target = dir.path().join("out");
    for args in [
        vec!["version"],
        vec!["init"],
        vec!["backup", source.to_str().unwrap()],
        vec!["snapshots"],
        vec!["ls", "latest"],
        vec!["restore", "latest", "--target", target.to_str().unwrap()],
        vec!["check"],
        vec!["verify"],
        vec!["key", "list"],
        vec!["unlock"],
        vec!["rebuild-index"],
        vec!["forget", "--keep-last", "1"],
        vec!["prune"],
        vec!["--no-lock", "snapshots"],
    ] {
        let out = run(&repo, &key, &args);
        assert!(out.status.success(), "{args:?}: {out:?}");
        assert!(
            out.stdout.is_empty() && out.stderr.is_empty(),
            "{args:?}: {out:?}"
        );
    }
    let second = dir.path().join("second-key");
    let added = run(
        &repo,
        &key,
        &["key", "add", "--new-key-file", second.to_str().unwrap()],
    );
    assert!(added.status.success() && added.stdout.is_empty() && added.stderr.is_empty());
    let slots = run(&repo, &key, &["--json", "key", "list"]);
    let slots: serde_json::Value = serde_json::from_slice(&slots.stdout).unwrap();
    let id = slots["slots"][0]["id"].as_str().unwrap();
    let removed = run(&repo, &key, &["key", "remove", id]);
    assert!(removed.status.success() && removed.stdout.is_empty() && removed.stderr.is_empty());
    // Continue with whichever secret remains after removing a slot in sorted order.
    let key = if run(&repo, &key, &["snapshots"]).status.success() {
        key
    } else {
        second
    };
    let unreadable = repo.join("keys").join("ff".repeat(32));
    std::fs::write(&unreadable, b"invalid slot").unwrap();
    let listed = run(&repo, &key, &["key", "list"]);
    assert!(String::from_utf8_lossy(&listed.stderr).contains("unreadable key slot"));
    std::fs::remove_file(unreadable).unwrap();
    let conflict = run(
        &repo,
        &key,
        &["restore", "latest", "--target", target.to_str().unwrap()],
    );
    assert_eq!(conflict.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&conflict.stderr).contains("restore conflict"));
    let out = run(&repo, &key, &["--json", "snapshots"]);
    assert!(!out.stdout.is_empty());
    std::fs::write(repo.join("config"), b"damaged").unwrap();
    let out = run(&repo, &key, &["check"]);
    assert_eq!(out.status.code(), Some(3));
    assert!(!out.stderr.is_empty() || !out.stdout.is_empty());
}
