#[path = "common/prune_pause.rs"]
mod pause;
use keelhaul::{
    backend::Backend,
    format::pack::BlobKind,
    ops::{
        backup::{Options, backup, backup_with_clock},
        prune::prune,
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
fn publication_accepts_verified_blobs_moved_to_new_packs_by_prune() {
    let scratch = tempfile::tempdir().unwrap();
    let source = fs::canonicalize(scratch.path()).unwrap().join("source");
    let repo =
        Arc::new(Repository::init(pause::Paused::default(), &Secret::KeyFile([84; 32])).unwrap());
    let mut bytes = vec![0; 1000];
    blake3::Hasher::new().finalize_xof().fill(&mut bytes);
    fs::write(&source, &bytes).unwrap();
    let mut store = BlobStore::load(&repo).unwrap();
    let data_id = store.add(BlobKind::Data, &bytes).unwrap();
    store.add(BlobKind::Data, &vec![17; 2000]).unwrap();
    store.finish().unwrap();
    let old = store.index[&(BlobKind::Data, data_id)].pack.clone();
    drop(store);
    backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let clock = Arc::new(Fake(AtomicU64::new(now - 1801)));
    repo.backend.pause_snapshot.store(true, Ordering::SeqCst);
    let r = repo.clone();
    let c = clock.clone();
    let file = source.clone();
    let child = std::thread::spawn(move || {
        backup_with_clock(&r, &[file], vec![], &Options::default(), c.as_ref())
    });
    repo.backend.wait();
    clock.0.store(now, Ordering::SeqCst);
    let (chunk_id, encryption) = BlobStore::load(&repo).unwrap().worker_keys();
    assert_eq!(
        unlock(
            &repo.backend,
            &keelhaul::crypto::keys::Subkeys {
                chunk_id,
                encryption,
                gear: [0; 32],
                key_slot_check: [0; 32]
            },
            clock.as_ref()
        )
        .unwrap()
        .len(),
        1
    );
    let summary = prune(&repo, 0, false).unwrap();
    assert!(summary.packs_repacked > 0);
    assert!(
        repo.backend
            .stat(&keelhaul::repo::blobs::pack_name(&old).unwrap())
            .is_err()
    );
    repo.backend.release();
    let snapshot = child.join().unwrap().unwrap();
    assert!(verify(&repo, None).unwrap().damage.is_empty());
    let target = scratch.path().join("restore");
    restore(&repo, &snapshot.snapshot, &target, &[]).unwrap();
    assert_eq!(
        fs::read(target.join(source.strip_prefix("/").unwrap())).unwrap(),
        bytes
    );
}
