use keelhaul::{
    backend::{Backend, Kind, local::LocalBackend, memory::MemoryBackend},
    crypto::envelope::Label,
    format::{
        pack::BlobKind,
        snapshot::{Snapshot, Summary},
        tree::Tree,
    },
    ops::{
        backup::backup,
        check::check,
        forget::{Options, forget},
        prune::prune,
        restore::restore,
        verify::verify,
    },
    repo::{
        Repository,
        blobs::{BlobStore, pack_name},
    },
    secret::Secret,
};
use std::collections::HashSet;
const TIME: &str = "2024-01-01T00:00:00.000000000Z";
fn fixture<B: Backend>(backend: B) -> (Repository<B>, String, Vec<Vec<u8>>) {
    let repo = Repository::init(backend, &Secret::KeyFile([13; 32])).unwrap();
    let mut store = BlobStore::load(&repo).unwrap();
    let mut ids = Vec::new();
    let mut contents = Vec::new();
    for n in 0..10u64 {
        let mut bytes = vec![0; 1000];
        blake3::Hasher::new()
            .update(&n.to_le_bytes())
            .finalize_xof()
            .fill(&mut bytes);
        ids.push(store.add(BlobKind::Data, &bytes).unwrap());
        contents.push(bytes);
    }
    store.finish().unwrap();
    let tree = serde_json::json!({"version":1,"entries":[{"name":"%2Ffile","mode":420,"mtime":TIME,"ctime":TIME,"uid":0,"gid":0,"inode":1,"device":1,"links":1,"type":"file","size":7000,"content":ids[..7]}]});
    let raw = serde_json::to_vec(&tree).unwrap();
    Tree::parse(&raw).unwrap();
    let tree = store.add(BlobKind::Tree, &raw).unwrap();
    store.finish().unwrap();
    let snapshot = Snapshot {
        version: 1,
        time: TIME.into(),
        host: keelhaul::host::hostname(),
        username: "test".into(),
        paths: vec!["/file".into()],
        tags: vec![],
        excludes: vec![],
        tree,
        parent: None,
        program: "test".into(),
        summary: Summary::default(),
    };
    let id = repo
        .write_object(
            "snapshots",
            Label::Snapshot,
            &serde_json::to_vec(&snapshot).unwrap(),
        )
        .unwrap();
    (repo, id, contents)
}
#[test]
fn thirty_percent_unused_obeys_threshold_and_restores_verified_bytes() {
    let (repo, id, contents) = fixture(MemoryBackend::new());
    assert!(check(&repo).unwrap().damage.is_empty());
    let before = repo.backend.list(Kind::Data).unwrap();
    let high = prune(&repo, 40, false).unwrap();
    assert_eq!(high.packs_repacked, 0);
    assert_eq!(before, repo.backend.list(Kind::Data).unwrap());
    let indexes = repo.backend.list(Kind::Index).unwrap();
    let dry = prune(&repo, 20, true).unwrap();
    assert_eq!(dry.packs_repacked, 1);
    assert_eq!(indexes, repo.backend.list(Kind::Index).unwrap());
    assert_eq!(before, repo.backend.list(Kind::Data).unwrap());
    let low = prune(&repo, 20, false).unwrap();
    assert_eq!(low.packs_repacked, 1);
    assert!(low.bytes_freed > 0);
    assert!(verify(&repo, None).unwrap().damage.is_empty());
    let scratch = tempfile::tempdir().unwrap();
    restore(&repo, &id, &scratch.path().join("restore"), &[]).unwrap();
    assert_eq!(
        std::fs::read(scratch.path().join("restore/file")).unwrap(),
        contents[..7].concat()
    );
}
#[test]
fn duplicate_blobs_collapse_to_one_verified_copy() {
    let (repo, _, contents) = fixture(MemoryBackend::new());
    let mut writer = BlobStore::repack_writer(&repo).unwrap();
    for bytes in &contents[..7] {
        writer.add(BlobKind::Data, bytes).unwrap();
    }
    writer.finish().unwrap();
    prune(&repo, 100, false).unwrap();
    let mut seen = HashSet::new();
    for pack in repo.backend.list(Kind::Data).unwrap() {
        let bytes = repo.backend.read(&pack.name).unwrap();
        let len =
            keelhaul::format::pack::header_length(&bytes[bytes.len() - 4..], bytes.len() as u64)
                .unwrap() as usize;
        let key = BlobStore::load(&repo).unwrap().worker_keys().1;
        let payload = keelhaul::crypto::envelope::open(
            &key,
            Label::PackHeader,
            &bytes[bytes.len() - 4 - len..bytes.len() - 4],
        )
        .unwrap();
        let header =
            keelhaul::format::pack::Header::parse(&payload, (bytes.len() - 4 - len) as u64)
                .unwrap();
        for blob in header.blobs {
            assert!(seen.insert((blob.kind, blob.id)), "duplicate survived");
        }
    }
    assert!(verify(&repo, None).unwrap().damage.is_empty());
}
#[test]
fn young_orphan_survives_old_orphan_and_temp_are_deleted() {
    let scratch = tempfile::tempdir().unwrap();
    let (repo, _, _) = fixture(LocalBackend::new(scratch.path()).unwrap());
    let old_indexes = repo
        .backend
        .list(Kind::Index)
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect::<HashSet<_>>();
    let mut writer = BlobStore::load(&repo).unwrap();
    writer
        .add(BlobKind::Data, b"unreferenced young data")
        .unwrap();
    writer.finish().unwrap();
    for index in repo.backend.list(Kind::Index).unwrap() {
        if !old_indexes.contains(&index.name) {
            repo.backend.delete(&index.name).unwrap();
        }
    }
    let orphan = repo
        .backend
        .list(Kind::Data)
        .unwrap()
        .into_iter()
        .find(|e| {
            !BlobStore::load(&repo)
                .unwrap()
                .index
                .values()
                .any(|l| pack_name(&l.pack).unwrap() == e.name)
        })
        .unwrap();
    prune(&repo, 40, false).unwrap();
    assert!(repo.backend.stat(&orphan.name).is_ok());
    let old = filetime::FileTime::from_unix_time(
        time::OffsetDateTime::now_utc().unix_timestamp() - 1801,
        0,
    );
    filetime::set_file_mtime(scratch.path().join(orphan.name.as_str()), old).unwrap();
    let raw = format!("{}.tmp-0123456789abcdef", orphan.name);
    std::fs::write(scratch.path().join(&raw), b"partial").unwrap();
    filetime::set_file_mtime(scratch.path().join(&raw), old).unwrap();
    let summary = prune(&repo, 40, false).unwrap();
    assert_eq!(summary.packs_removed, 1);
    assert_eq!(summary.temporary_removed, 1);
    assert!(repo.backend.stat(&orphan.name).is_err());
    assert!(!scratch.path().join(raw).exists());
    assert!(verify(&repo, None).unwrap().damage.is_empty());
}
#[test]
fn unreadable_snapshot_or_tree_aborts_without_deleting_packs_or_indexes() {
    for corrupt_tree in [false, true] {
        let (repo, _, _) = fixture(MemoryBackend::new());
        let object = if corrupt_tree {
            repo.backend
                .list(Kind::Data)
                .unwrap()
                .into_iter()
                .find(|e| e.size < 10000)
                .unwrap()
        } else {
            repo.backend.list(Kind::Snapshots).unwrap().remove(0)
        };
        repo.backend
            .plant_raw(object.name.as_str(), b"broken")
            .unwrap();
        let data = repo.backend.list(Kind::Data).unwrap();
        let indexes = repo.backend.list(Kind::Index).unwrap();
        assert!(matches!(
            prune(&repo, 20, false),
            Err(keelhaul::repo::Error::Damage { .. })
        ));
        assert_eq!(data, repo.backend.list(Kind::Data).unwrap());
        assert_eq!(indexes, repo.backend.list(Kind::Index).unwrap());
    }
}
#[test]
fn forget_prune_frees_space_and_kept_snapshot_restores() {
    let scratch = tempfile::tempdir().unwrap();
    let source = scratch.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([13; 32])).unwrap();
    let mut bytes = vec![0; 600_000];
    blake3::Hasher::new().finalize_xof().fill(&mut bytes);
    std::fs::write(source.join("old"), &bytes).unwrap();
    backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
    std::fs::remove_file(source.join("old")).unwrap();
    std::fs::write(source.join("kept"), b"keep me").unwrap();
    let kept = backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
    let size = || {
        repo.backend
            .list(Kind::Data)
            .unwrap()
            .iter()
            .map(|e| e.size)
            .sum::<u64>()
    };
    let before = size();
    forget(
        &repo,
        &Options {
            keep_last: Some(1),
            prune: true,
            ..Options::default()
        },
    )
    .unwrap();
    assert!(size() < before);
    assert!(verify(&repo, None).unwrap().damage.is_empty());
    let target = scratch.path().join("restore");
    restore(&repo, &kept.snapshot, &target, &[]).unwrap();
    assert_eq!(
        std::fs::read(target.join(source.strip_prefix("/").unwrap()).join("kept")).unwrap(),
        b"keep me"
    );
}
