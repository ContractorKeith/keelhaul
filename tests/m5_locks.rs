use keelhaul::{
    backend::{Backend, Kind, memory::MemoryBackend},
    crypto::keys::Subkeys,
    ops::{
        backup::{Options, backup_with_clock},
        check::check,
    },
    repo::{
        Error, Repository,
        lock::{self, Clock, Guard, Lock, STALE, SystemClock},
    },
    secret::Secret,
};
use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime},
};
fn keys<B: Backend>(repo: &Repository<B>) -> Subkeys {
    let (chunk_id, encryption) = keelhaul::repo::blobs::BlobStore::load(repo)
        .unwrap()
        .worker_keys();
    Subkeys {
        encryption,
        chunk_id,
        gear: [0; 32],
        key_slot_check: [0; 32],
    }
}
struct Fake(AtomicU64);
impl Clock for Fake {
    fn now(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(self.0.load(Ordering::SeqCst))
    }
}
#[test]
fn shared_conflicts_refresh_release_and_unlock() {
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([9; 32])).unwrap();
    let clock = Fake(AtomicU64::new(2_000_000_000));
    let a = Guard::acquire(&repo.backend, keys(&repo), false, &clock).unwrap();
    let b = Guard::acquire(&repo.backend, keys(&repo), false, &clock).unwrap();
    assert!(matches!(
        Guard::acquire(&repo.backend, keys(&repo), true, &clock),
        Err(Error::Locked(_))
    ));
    assert_eq!(repo.backend.list(Kind::Locks).unwrap().len(), 2);
    assert!(
        lock::unlock(&repo.backend, &keys(&repo), &clock)
            .unwrap()
            .is_empty()
    );
    let before = repo.backend.list(Kind::Locks).unwrap();
    clock.0.fetch_add(300, Ordering::SeqCst);
    a.refresh().unwrap();
    assert_ne!(before, repo.backend.list(Kind::Locks).unwrap());
    a.self_check().unwrap();
    drop(b);
    drop(a);
    let exclusive = Guard::acquire(&repo.backend, keys(&repo), true, &clock).unwrap();
    assert!(matches!(
        Guard::acquire(&repo.backend, keys(&repo), false, &clock),
        Err(Error::Locked(_))
    ));
    clock.0.fetch_add(STALE.as_secs() + 1, Ordering::SeqCst);
    assert!(exclusive.self_check().is_err());
    assert!(exclusive.refresh().is_err());
    assert_eq!(
        lock::unlock(&repo.backend, &keys(&repo), &clock)
            .unwrap()
            .len(),
        1
    );
    let new = Guard::acquire(&repo.backend, keys(&repo), true, &clock).unwrap();
    drop(exclusive);
    drop(new);
    assert!(repo.backend.list(Kind::Locks).unwrap().is_empty());
}
#[test]
fn dead_pid_is_ignored_and_collected_but_fresh_live_locks_survive_unlock() {
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([9; 32])).unwrap();
    let mut dead = Lock::new(true, SystemTime::now()).unwrap();
    dead.pid = 2_000_000_000;
    repo.write_object(
        "locks",
        keelhaul::crypto::envelope::Label::Lock,
        &serde_json::to_vec(&dead).unwrap(),
    )
    .unwrap();
    let guard = Guard::acquire(&repo.backend, keys(&repo), true, &SystemClock).unwrap();
    assert_eq!(
        lock::unlock(&repo.backend, &keys(&repo), &SystemClock)
            .unwrap()
            .len(),
        1
    );
    guard.self_check().unwrap();
    drop(guard);
}
struct Jump(AtomicU64);
impl Clock for Jump {
    fn now(&self) -> SystemTime {
        SystemTime::now()
            + Duration::from_secs(if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
                0
            } else {
                1801
            })
    }
}
#[test]
fn expired_backup_never_publishes_snapshot_and_leaves_check_clean() {
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([9; 32])).unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let source = scratch.path().join("file");
    fs::write(&source, b"data").unwrap();
    let error = backup_with_clock(
        &repo,
        &[source],
        vec![],
        &Options::default(),
        &Jump(AtomicU64::new(0)),
    )
    .unwrap_err();
    assert!(matches!(error, Error::Locked(_)));
    assert!(repo.backend.list(Kind::Snapshots).unwrap().is_empty());
    assert!(repo.backend.list(Kind::Locks).unwrap().is_empty());
    assert!(check(&repo).unwrap().damage.is_empty());
}
#[path = "common/m5_races.rs"]
mod races;
