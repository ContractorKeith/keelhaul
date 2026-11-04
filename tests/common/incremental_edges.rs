use super::*;

#[test]
fn report_counter_does_not_change_snapshot_format() {
    let scratch = tempfile::tempdir().unwrap();
    fs::write(scratch.path().join("a"), b"a").unwrap();
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([2; 32])).unwrap();
    let report = backup(&repo, &[scratch.path().into()], vec![]).unwrap();
    assert_eq!(
        serde_json::to_value(&report).unwrap()["summary"]["files_read"],
        1
    );
    let entry = repo.backend.list(Kind::Snapshots).unwrap().pop().unwrap();
    let value: serde_json::Value =
        serde_json::from_slice(&repo.read_object(&entry.name, Label::Snapshot).unwrap()).unwrap();
    assert!(value["summary"].get("files_read").is_none());
}

#[test]
fn overlapping_roots_keep_independent_root_names() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("source");
    fs::create_dir_all(root.join("nested")).unwrap();
    fs::write(root.join("nested/file"), b"overlap").unwrap();
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([3; 32])).unwrap();
    let report = backup(&repo, &[root.clone(), root.join("nested")], vec![]).unwrap();
    let (_, snapshot) = repo.select_snapshot(&report.snapshot).unwrap();
    let mut store = keelhaul::repo::blobs::BlobStore::load(&repo).unwrap();
    let tree = keelhaul::format::tree::Tree::parse(
        &store
            .read(keelhaul::format::pack::BlobKind::Tree, &snapshot.tree)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(tree.entries.len(), 2);
    assert_eq!(
        tree.entries[0].name.as_deref(),
        Some(keelhaul::format::snapshot::root_name(root.to_str().unwrap()).as_str())
    );
    let out = scratch.path().join("out");
    assert!(
        keelhaul::ops::restore::restore(&repo, &report.snapshot, &out, &[])
            .unwrap()
            .damaged
            .is_empty()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn one_file_system_prunes_a_different_device_but_allows_it_as_its_own_root() {
    use std::os::unix::fs::MetadataExt;
    // /dev is a separate mount in Linux CI. Whitelist only the empty /dev
    // directory: no device, home, or user file is opened by this fixture.
    if fs::metadata("/").unwrap().dev() == fs::metadata("/dev").unwrap().dev() {
        return;
    }
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([6; 32])).unwrap();
    let mut policy = options(1);
    policy.excludes = vec!["*".into(), "!/dev".into()];
    assert_eq!(run(&repo, Path::new("/"), policy).summary.dirs, 2);
    let mut policy = options(1);
    policy.excludes = vec!["*".into(), "!/dev".into()];
    policy.one_file_system = true;
    assert_eq!(run(&repo, Path::new("/"), policy).summary.dirs, 1);
    let mut policy = options(1);
    policy.excludes = vec!["*".into()];
    policy.one_file_system = true;
    assert_eq!(run(&repo, Path::new("/dev"), policy).summary.dirs, 1);
}
