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

#[test]
fn latest_retains_damage_when_an_older_snapshot_is_readable() {
    let dir = tempfile::tempdir().unwrap();
    let source = std::fs::canonicalize(dir.path()).unwrap().join("file");
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([7; 32])).unwrap();
    std::fs::write(&source, b"first").unwrap();
    let first = backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
    std::fs::write(&source, b"other").unwrap();
    let newest = backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
    let name =
        keelhaul::backend::ObjectName::new(&format!("snapshots/{}", newest.snapshot)).unwrap();
    let mut bytes = repo.backend.read(&name).unwrap();
    bytes[24] ^= 1;
    repo.backend.plant_raw(name.as_str(), &bytes).unwrap();
    let target = dir.path().join("out");
    let report = restore(&repo, "latest", &target, &[]).unwrap();
    assert_eq!(report.snapshot, first.snapshot);
    assert!(report.damaged.iter().any(|d| d.object == name.as_str()));
    assert_eq!(
        std::fs::read(target.join(source.strip_prefix("/").unwrap())).unwrap(),
        b"first"
    );
    let listed = keelhaul::ops::listing::ls_report_for_host(&repo, "latest", None, None).unwrap();
    assert_eq!(listed.snapshot, first.snapshot);
    assert!(listed.damage.iter().any(|d| d.object == name.as_str()));
}
