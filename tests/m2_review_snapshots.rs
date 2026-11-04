use keelhaul::{
    backend::{Backend, ObjectName, local::LocalBackend},
    crypto::envelope::Label,
    ops::backup::backup,
    repo::Repository,
    secret::Secret,
};
use std::{
    path::Path,
    process::{Command, Output},
};
fn command(root: &Path, args: &[&str]) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_keelhaul"));
    for key in [
        "KEELHAUL_REPO",
        "KEELHAUL_KEY_FILE",
        "KEELHAUL_PASSWORD_FILE",
        "KEELHAUL_PASSWORD_COMMAND",
    ] {
        c.env_remove(key);
    }
    c.env("HOME", root)
        .env("XDG_CACHE_HOME", root)
        .env("XDG_CONFIG_HOME", root)
        .arg("-r")
        .arg(root.join("repo"))
        .arg("--key-file")
        .arg(root.join("key"))
        .args(args)
        .output()
        .unwrap()
}
fn fixture(root: &Path) -> (Repository<LocalBackend>, String) {
    std::fs::write(root.join("key"), [7; 32]).unwrap();
    std::fs::write(root.join("foo"), b"A").unwrap();
    let repo = Repository::init(
        LocalBackend::new(root.join("repo")).unwrap(),
        &Secret::KeyFile([7; 32]),
    )
    .unwrap();
    let a = backup(&repo, &[root.join("foo")], vec![]).unwrap().snapshot;
    (repo, a)
}
#[test]
fn unrelated_bad_snapshot_does_not_block_id_latest_or_good_list() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let (repo, a) = fixture(root);
    let (_, mut b) = repo.select_snapshot(&a).unwrap();
    b.time = "2099-01-01T00:00:00.000000000Z".into();
    let b = repo
        .write_object(
            "snapshots",
            Label::Snapshot,
            &serde_json::to_vec(&b).unwrap(),
        )
        .unwrap();
    std::fs::write(root.join("repo/snapshots").join(&b), b"damage").unwrap();
    assert_eq!(repo.select_snapshot(&a[..8]).unwrap().0, a);
    assert_eq!(repo.select_snapshot("latest").unwrap().0, a);
    let output = command(
        root,
        &[
            "restore",
            &a,
            "--target",
            root.join("out").to_str().unwrap(),
            "--json",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = command(root, &["snapshots", "--json"]);
    assert_eq!(output.status.code(), Some(3));
    let rows: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["id"], a);
    assert!(String::from_utf8_lossy(&output.stderr).contains(&format!("snapshots/{b}")));
    assert!(repo.select_snapshot(&b).is_err());
}
#[test]
fn latest_is_host_scoped_and_override_and_missing_host_are_explicit() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let (repo, a) = fixture(root);
    let (_, mut peer) = repo.select_snapshot(&a).unwrap();
    peer.host = "remote-host".into();
    peer.time = "2099-01-01T00:00:00.000000000Z".into();
    let peer = repo
        .write_object(
            "snapshots",
            Label::Snapshot,
            &serde_json::to_vec(&peer).unwrap(),
        )
        .unwrap();
    assert_eq!(repo.select_snapshot("latest").unwrap().0, a);
    assert_eq!(
        repo.select_snapshot_for_host("latest", Some("remote-host"))
            .unwrap()
            .0,
        peer
    );
    assert_eq!(repo.select_snapshot(&peer[..8]).unwrap().0, peer);
    let error = repo
        .select_snapshot_for_host("latest", Some("missing-host"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("remote-host") && error.contains(&keelhaul::host::hostname()));
    let output = command(
        root,
        &[
            "restore",
            "latest",
            "--host",
            "remote-host",
            "--target",
            root.join("peer-out").to_str().unwrap(),
            "--json",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = command(root, &["ls", "latest", "--host", "missing-host", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    // Removing the local snapshot must not silently select another host's backup.
    repo.backend
        .delete(&ObjectName::new(&format!("snapshots/{a}")).unwrap())
        .unwrap();
    assert!(repo.select_snapshot("latest").is_err());
}
