use keelhaul::{
    backend::{local::LocalBackend, memory::MemoryBackend},
    ops::{backup::backup, restore::restore},
    repo::Repository,
    secret::Secret,
};
use std::{fs, os::unix::fs::symlink, path::PathBuf};
#[path = "common/backup_fixture.rs"]
mod backup_fixture;
use backup_fixture::run;
#[test]
fn memory_roundtrip() {
    run(MemoryBackend::new());
}
#[test]
fn local_roundtrip() {
    let d = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(d.path()).unwrap();
    run(LocalBackend::new(root.as_path()).unwrap());
}
#[test]
fn sparse_64_mib_uses_one_repeated_chunk() {
    let d = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(d.path()).unwrap();
    let file = root.as_path().join("zeros");
    fs::File::create(&file)
        .unwrap()
        .set_len(64 * 1024 * 1024)
        .unwrap();
    // Pin gear so this assertion never depends on a randomly generated master key.
    use keelhaul::{
        backend::{Backend, ObjectName},
        crypto::{
            envelope::{self, Label},
            keys::MasterKey,
            slot,
        },
        format::config::Config,
    };
    let master = MasterKey::from_bytes([3; 32]);
    let backend = MemoryBackend::new();
    backend
        .write(
            &ObjectName::new("config").unwrap(),
            &envelope::seal(
                &master.subkeys().encryption,
                Label::Config,
                &serde_json::to_vec(&Config::new().unwrap()).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    backend
        .write(
            &ObjectName::new(&format!("keys/{}", "01".repeat(32))).unwrap(),
            &serde_json::to_vec(&slot::create(&master, &Secret::KeyFile([3; 32])).unwrap())
                .unwrap(),
        )
        .unwrap();
    let repo = Repository::open(backend, &Secret::KeyFile([3; 32])).unwrap();
    let report = backup(&repo, std::slice::from_ref(&file), vec![]).unwrap();
    assert_eq!(report.summary.data_blobs_added, 1);
    let entries = keelhaul::ops::listing::ls(&repo, "latest", None).unwrap();
    let keelhaul::format::tree::Node::File { content, .. } = &entries[0].entry.node else {
        panic!()
    };
    assert_eq!(content.len(), 8);
    assert!(content.iter().all(|id| id == &content[0]));
}
#[test]
fn skips_socket_and_missing_root_and_never_follows_input_symlink() {
    let d = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(d.path()).unwrap();
    let socket = root.as_path().join("socket");
    let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    symlink("/etc", root.as_path().join("link")).unwrap();
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([4; 32])).unwrap();
    let report = backup(
        &repo,
        &[root.as_path().to_path_buf(), root.as_path().join("missing")],
        vec![],
    )
    .unwrap();
    assert_eq!(report.summary.files_skipped, 2);
    assert_eq!(report.skipped.len(), 2);
    let entries = keelhaul::ops::listing::ls(&repo, "latest", None).unwrap();
    assert_eq!(entries.len(), 2);
}
#[test]
fn preexisting_symlink_conflict_is_refused_before_writes() {
    let d = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(d.path()).unwrap();
    let source = root.as_path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("file"), b"secret").unwrap();
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([4; 32])).unwrap();
    backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
    let target = root.as_path().join("target");
    fs::create_dir(&target).unwrap();
    let outside = root.as_path().join("outside");
    fs::create_dir(&outside).unwrap();
    let first = source
        .strip_prefix("/")
        .unwrap()
        .components()
        .next()
        .unwrap();
    symlink(&outside, target.join(first.as_os_str())).unwrap();
    assert!(
        !restore(&repo, "latest", &target, &[])
            .unwrap()
            .failed
            .is_empty()
    );
    assert!(fs::read_dir(outside).unwrap().next().is_none());
    assert!(
        restore(
            &repo,
            "latest",
            &root.as_path().join("other"),
            &[PathBuf::from("../escape")]
        )
        .is_err()
    );
}
