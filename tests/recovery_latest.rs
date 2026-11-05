use keelhaul::{
    backend::{Backend, Kind, memory::MemoryBackend},
    ops::{backup::backup, restore::restore},
    repo::{Error, Repository},
    secret::Secret,
};
#[test]
fn latest_names_all_damaged_snapshots_and_counts_only_readable_hosts() {
    let dir = tempfile::tempdir().unwrap();
    let source = std::fs::canonicalize(dir.path()).unwrap().join("file");
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([7; 32])).unwrap();
    for bytes in [b"first", b"other"] {
        std::fs::write(&source, bytes).unwrap();
        backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
    }
    let snapshots = repo.backend.list(Kind::Snapshots).unwrap();
    let a = repo.backend.read(&snapshots[0].name).unwrap();
    let b = repo.backend.read(&snapshots[1].name).unwrap();
    repo.backend
        .plant_raw(snapshots[0].name.as_str(), &b)
        .unwrap();
    repo.backend
        .plant_raw(snapshots[1].name.as_str(), &a)
        .unwrap();
    let error = restore(&repo, "latest", &dir.path().join("out"), &[]).unwrap_err();
    assert!(matches!(&error, Error::Damage { .. }));
    let text = error.to_string();
    for snapshot in snapshots {
        assert!(text.contains(snapshot.name.as_str()), "{text}");
    }
    assert!(
        text.contains("0 readable snapshots, 2 damaged snapshots"),
        "{text}"
    );
}
