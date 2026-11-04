//! Regression for the SPEC §11 suspended-backup publication window.
use keelhaul::{
    backend::{
        Backend, BackendError, Entry, ForeignEntry, Kind, ObjectName, memory::MemoryBackend,
    },
    ops::{
        backup::{Options, backup_with_clock},
        check::check,
        prune::prune,
    },
    repo::Repository,
    repo::lock::Clock,
    secret::Secret,
};
use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};
struct Paused {
    inner: MemoryBackend,
    pause: AtomicBool,
    reached: (Mutex<bool>, Condvar),
    resume: (Mutex<bool>, Condvar),
}
impl Backend for Paused {
    fn write(&self, n: &ObjectName, b: &[u8]) -> Result<(), BackendError> {
        if n.kind() == Kind::Snapshots && self.pause.load(Ordering::SeqCst) {
            *self.reached.0.lock().unwrap() = true;
            self.reached.1.notify_all();
            let mut resume = self.resume.0.lock().unwrap();
            while !*resume {
                resume = self.resume.1.wait(resume).unwrap();
            }
        }
        self.inner.write(n, b)
    }
    fn read(&self, n: &ObjectName) -> Result<Vec<u8>, BackendError> {
        self.inner.read(n)
    }
    fn read_range(&self, n: &ObjectName, o: u64, l: u64) -> Result<Vec<u8>, BackendError> {
        self.inner.read_range(n, o, l)
    }
    fn list(&self, k: Kind) -> Result<Vec<Entry>, BackendError> {
        self.inner.list(k)
    }
    fn list_foreign(&self, k: Kind) -> Result<Vec<ForeignEntry>, BackendError> {
        self.inner.list_foreign(k)
    }
    fn delete_foreign(&self, n: &str) -> Result<(), BackendError> {
        self.inner.delete_foreign(n)
    }
    fn stat(&self, n: &ObjectName) -> Result<u64, BackendError> {
        self.inner.stat(n)
    }
    fn delete(&self, n: &ObjectName) -> Result<(), BackendError> {
        self.inner.delete(n)
    }
}
struct Fake(AtomicU64);
impl Clock for Fake {
    fn now(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(self.0.load(Ordering::SeqCst))
    }
}
#[test]
fn stale_backup_blocks_prune_and_unlock_prune_is_caught_at_publication() {
    let scratch = tempfile::tempdir().unwrap();
    let real = std::fs::canonicalize(scratch.path()).unwrap();
    let alias = real.join("alias");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    let file = alias.join("source");
    std::fs::write(&file, b"irreplaceable snapshot bytes").unwrap();
    let repo = Arc::new(
        Repository::init(
            Paused {
                inner: MemoryBackend::new(),
                pause: AtomicBool::new(true),
                reached: (Mutex::new(false), Condvar::new()),
                resume: (Mutex::new(false), Condvar::new()),
            },
            &Secret::KeyFile([91; 32]),
        )
        .unwrap(),
    );
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let clock = Arc::new(Fake(AtomicU64::new(now - 1801)));
    let r = repo.clone();
    let c = clock.clone();
    let backup = std::thread::spawn(move || {
        backup_with_clock(&r, &[file], vec![], &Options::default(), c.as_ref())
    });
    let mut reached = repo.backend.reached.0.lock().unwrap();
    while !*reached {
        reached = repo.backend.reached.1.wait(reached).unwrap();
    }
    drop(reached);
    // Snapshot publication has begun, after SPEC §11's required self-check.
    // Advance the clock by 30m+1s while the process is suspended at that point.
    clock.0.store(now, Ordering::SeqCst);
    assert_eq!(repo.backend.list(Kind::Snapshots).unwrap().len(), 0);
    assert!(!repo.backend.list(Kind::Index).unwrap().is_empty());
    let error = prune(&repo, 20, false).unwrap_err();
    assert_eq!(
        keelhaul::exit::failure(&error),
        keelhaul::exit::Failure::Locked
    );
    assert!(
        error
            .to_string()
            .contains("if that process is gone, run `keelhaul unlock`")
    );
    assert!(!repo.backend.list(Kind::Data).unwrap().is_empty());
    let (chunk_id, encryption) = keelhaul::repo::blobs::BlobStore::load(&repo)
        .unwrap()
        .worker_keys();
    let keys = keelhaul::crypto::keys::Subkeys {
        encryption,
        chunk_id,
        gear: [0; 32],
        key_slot_check: [0; 32],
    };
    let removed = keelhaul::repo::lock::unlock(&repo.backend, &keys, clock.as_ref()).unwrap();
    assert_eq!(removed.len(), 1);
    prune(&repo, 20, false).unwrap();
    assert!(repo.backend.list(Kind::Data).unwrap().is_empty());
    repo.backend.pause.store(false, Ordering::SeqCst);
    *repo.backend.resume.0.lock().unwrap() = true;
    repo.backend.resume.1.notify_all();
    let error = backup.join().unwrap().unwrap_err();
    assert_eq!(
        keelhaul::exit::failure(&error),
        keelhaul::exit::Failure::Error
    );
    assert_eq!(
        error.to_string(),
        "repository was pruned during this backup; rerun"
    );
    assert!(repo.backend.list(Kind::Snapshots).unwrap().is_empty());
    assert!(check(&repo).unwrap().damage.is_empty());
    let source = alias.join("source");
    let stored_source = std::fs::canonicalize(&source).unwrap();
    assert_ne!(source, stored_source, "fixture must exercise a path alias");
    let report =
        keelhaul::ops::backup::backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
    let target = scratch.path().join("restored");
    keelhaul::ops::restore::restore(&repo, &report.snapshot, &target, &[]).unwrap();
    assert_eq!(
        std::fs::read(target.join(stored_source.strip_prefix("/").unwrap())).unwrap(),
        b"irreplaceable snapshot bytes"
    );
}
