#[path = "common/prune_pause.rs"]
mod pause;
use keelhaul::{
    backend::{Backend, Kind},
    ops::{
        backup::backup,
        check::check,
        forget::{Options, forget},
        prune::prune_with_clock,
        restore::restore,
        verify::verify,
    },
    repo::{
        Repository,
        blobs::BlobStore,
        lock::{Clock, unlock},
    },
    secret::Secret,
};
use std::{
    fs,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};
struct Fake(AtomicU64);
impl Clock for Fake {
    fn now(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(self.0.load(Ordering::SeqCst))
    }
}
#[test]
fn suspended_prune_unlock_backup_cannot_delete_new_snapshot_data() {
    let scratch = tempfile::tempdir().unwrap();
    let source = fs::canonicalize(scratch.path()).unwrap().join("source");
    let repo =
        Arc::new(Repository::init(pause::Paused::default(), &Secret::KeyFile([83; 32])).unwrap());
    fs::write(&source, b"forgotten bytes that the new backup reuses").unwrap();
    backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
    fs::write(&source, b"retained bytes").unwrap();
    backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
    forget(
        &repo,
        &Options {
            keep_last: Some(1),
            ..Options::default()
        },
    )
    .unwrap();
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let clock = Arc::new(Fake(AtomicU64::new(now - 1801)));
    repo.backend.armed.store(true, Ordering::SeqCst);
    let r = repo.clone();
    let c = clock.clone();
    let child = std::thread::spawn(move || prune_with_clock(&r, 100, false, c.as_ref()));
    repo.backend.wait();
    clock.0.store(now, Ordering::SeqCst);
    let (chunk_id, encryption) = BlobStore::load(&repo).unwrap().worker_keys();
    let keys = keelhaul::crypto::keys::Subkeys {
        encryption,
        chunk_id,
        gear: [0; 32],
        key_slot_check: [0; 32],
    };
    assert_eq!(
        unlock(&repo.backend, &keys, clock.as_ref()).unwrap().len(),
        1
    );
    fs::write(&source, b"forgotten bytes that the new backup reuses").unwrap();
    let snapshot = backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
    assert_eq!(
        snapshot.summary.data_blobs_added, 0,
        "must exercise dedup against the original pack"
    );
    repo.backend.release();
    assert!(
        child.join().unwrap().is_err(),
        "a prune whose lock was explicitly removed must abort"
    );
    assert!(check(&repo).unwrap().damage.is_empty());
    assert!(verify(&repo, None).unwrap().damage.is_empty());
    let target = scratch.path().join("restore");
    restore(&repo, &snapshot.snapshot, &target, &[]).unwrap();
    assert_eq!(
        fs::read(target.join(source.strip_prefix("/").unwrap())).unwrap(),
        b"forgotten bytes that the new backup reuses"
    );
    assert!(repo.backend.list(Kind::Locks).unwrap().is_empty());
}

#[path = "common/m5_prune_fixture.rs"]
mod setup;
#[test]
fn new_snapshot_or_index_before_delete_aborts_and_withdraws_replacement() {
    use keelhaul::{crypto::envelope::Label, format::index::Index};
    for snapshot in [true, false] {
        let (repo, id, _) = setup::fixture(pause::Paused::default());
        let repo = Arc::new(repo);
        let before = repo.backend.list(Kind::Data).unwrap();
        repo.backend.armed.store(true, Ordering::SeqCst);
        let r = repo.clone();
        let child = std::thread::spawn(move || keelhaul::ops::prune::prune(&r, 100, false));
        repo.backend.wait();
        if snapshot {
            let saved = repo.select_snapshot(&id).unwrap().1;
            repo.write_object(
                "snapshots",
                Label::Snapshot,
                &serde_json::to_vec(&saved).unwrap(),
            )
            .unwrap();
        } else {
            repo.write_object(
                "index",
                Label::Index,
                &serde_json::to_vec(&Index {
                    version: 1,
                    supersedes: vec![],
                    packs: vec![],
                })
                .unwrap(),
            )
            .unwrap();
        }
        repo.backend.release();
        let error = child.join().unwrap().unwrap_err();
        assert_eq!(
            keelhaul::exit::failure(&error),
            keelhaul::exit::Failure::Error
        );
        assert!(error.to_string().contains(if snapshot {
            "a backup ran during prune; rerun prune"
        } else {
            "an index appeared during prune; rerun prune"
        }));
        assert_eq!(before, repo.backend.list(Kind::Data).unwrap());
        assert!(check(&repo).unwrap().damage.is_empty());
        assert!(verify(&repo, None).unwrap().damage.is_empty());
    }
}
#[test]
fn deletion_rechecks_expiry_after_ten_seconds_and_ownership_after_256_deletes() {
    use keelhaul::{crypto::envelope::Label, format::index::Index};
    use std::sync::atomic::AtomicUsize;
    for limit in [1, 256] {
        let (repo, _, _) = setup::fixture(pause::Paused::default());
        for _ in 0..260 {
            repo.write_object(
                "index",
                Label::Index,
                &serde_json::to_vec(&Index {
                    version: 1,
                    supersedes: vec![],
                    packs: vec![],
                })
                .unwrap(),
            )
            .unwrap();
        }
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let clock = Arc::new(Fake(AtomicU64::new(now)));
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        let time = clock.clone();
        *repo.backend.on_delete.lock().unwrap() = Some(Box::new(move |name, backend| {
            if name.kind() == Kind::Index && seen.fetch_add(1, Ordering::SeqCst) + 1 == limit {
                if limit == 1 {
                    time.0.fetch_add(1801, Ordering::SeqCst);
                } else {
                    for lock in backend.list(Kind::Locks).unwrap() {
                        backend.delete(&lock.name).unwrap();
                    }
                }
            }
        }));
        assert!(prune_with_clock(&repo, 100, false, clock.as_ref()).is_err());
        assert_eq!(
            count.load(Ordering::SeqCst),
            limit,
            "must stop before any more deletes"
        );
        assert!(check(&repo).unwrap().damage.is_empty());
        assert!(verify(&repo, None).unwrap().damage.is_empty());
    }
}
