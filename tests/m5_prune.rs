use keelhaul::{
    backend::{Backend, Kind, local::LocalBackend, memory::MemoryBackend},
    crypto::envelope::Label,
    format::pack::BlobKind,
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
#[path = "common/m5_prune_fixture.rs"]
mod setup;
use setup::fixture;
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
    let real = std::fs::canonicalize(scratch.path()).unwrap();
    let alias = real.join("alias");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    let source = alias.join("source");
    std::fs::create_dir(&source).unwrap();
    let stored_source = std::fs::canonicalize(&source).unwrap();
    assert_ne!(source, stored_source, "fixture must exercise a path alias");
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
        std::fs::read(
            target
                .join(stored_source.strip_prefix("/").unwrap())
                .join("kept")
        )
        .unwrap(),
        b"keep me"
    );
}

#[path = "common/prune_pause.rs"]
mod trace;
#[test]
fn replacement_supersedes_scanned_and_repack_flushed_indexes_exactly() {
    let (repo, _, _) = fixture(trace::Paused::default());
    let before = repo
        .backend
        .list(Kind::Index)
        .unwrap()
        .iter()
        .map(|e| e.name.as_str()[6..].to_owned())
        .collect::<HashSet<_>>();
    repo.backend.index_writes.lock().unwrap().clear();
    prune(&repo, 20, false).unwrap();
    let after = repo
        .backend
        .list(Kind::Index)
        .unwrap()
        .iter()
        .map(|e| e.name.as_str()[6..].to_owned())
        .collect::<HashSet<_>>();
    let written = repo.backend.index_writes.lock().unwrap();
    let mut expected = before;
    expected.extend(written.iter().filter(|id| !after.contains(*id)).cloned());
    let final_id = written.last().unwrap();
    let final_index = keelhaul::format::index::Index::parse(
        &repo
            .read_object(
                &keelhaul::backend::ObjectName::new(&format!("index/{final_id}")).unwrap(),
                Label::Index,
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        final_index.supersedes.into_iter().collect::<HashSet<_>>(),
        expected
    );
}
#[test]
fn missing_modification_times_retain_orphans_and_temps_and_report_once() {
    use std::sync::atomic::Ordering;
    let (repo, _, _) = fixture(trace::Paused::default());
    let before = repo
        .backend
        .list(Kind::Index)
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect::<HashSet<_>>();
    let mut writer = BlobStore::load(&repo).unwrap();
    writer.add(BlobKind::Data, b"orphan").unwrap();
    writer.finish().unwrap();
    for entry in repo.backend.list(Kind::Index).unwrap() {
        if !before.contains(&entry.name) {
            repo.backend.delete(&entry.name).unwrap();
        }
    }
    let packs = repo.backend.list(Kind::Data).unwrap();
    repo.backend
        .inner
        .plant_raw("config.tmp-0123456789abcdef", b"temp")
        .unwrap();
    repo.backend.unknown_times.store(true, Ordering::SeqCst);
    let summary = prune(&repo, 100, false).unwrap();
    assert_eq!(summary.warnings.len(), 1);
    assert_eq!(packs.len(), repo.backend.list(Kind::Data).unwrap().len());
    assert_eq!(repo.backend.list_foreign(Kind::Config).unwrap().len(), 1);
}
