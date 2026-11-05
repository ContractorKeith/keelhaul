use super::*;
use crate::{
    backend::{fault::FaultBackend, memory::MemoryBackend},
    repo::Repository,
    secret::Secret,
};
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{Instant, SystemTime},
};
struct Fake(AtomicU64);
impl Clock for Fake {
    fn now(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(self.0.load(Ordering::SeqCst))
    }
}
#[test]
fn background_refresher_uses_injected_clock_and_stops_on_error() {
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([43; 32])).unwrap();
    let clock = Fake(AtomicU64::new(2_000_000_000));
    let guard = Guard::acquire(&repo.backend, repo.subkeys(), false, &clock).unwrap();
    let before = repo.backend.list(Kind::Locks).unwrap()[0].name.clone();
    let result: Result<(), Error> = guard.run_interval(Duration::from_millis(5), |_| {
        clock.0.fetch_add(300, Ordering::SeqCst);
        let start = Instant::now();
        while repo
            .backend
            .list(Kind::Locks)
            .unwrap()
            .iter()
            .any(|e| e.name == before)
        {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "refresher did not replace lock"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        let locks = list(&repo.backend, &repo.subkeys()).unwrap();
        assert_eq!(locks.len(), 1);
        assert_eq!(locks[0].1.age(clock.now()).unwrap(), Duration::ZERO);
        Err(Error::Refused("operation failed".into()))
    });
    assert!(result.is_err());
    drop(guard);
    assert!(repo.backend.list(Kind::Locks).unwrap().is_empty());
}
#[test]
fn acquisition_list_error_removes_own_lock() {
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([43; 32])).unwrap();
    let keys = repo.subkeys();
    let backend = FaultBackend::new(repo.backend, "list", 1, false);
    assert!(Guard::acquire(&backend, keys, false, &SystemClock).is_err());
    assert!(backend.list(Kind::Locks).unwrap().is_empty());
}

#[test]
fn failed_refresh_remains_fatal_even_if_lock_reappears() {
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([43; 32])).unwrap();
    let guard = Guard::acquire(&repo.backend, repo.subkeys(), false, &SystemClock).unwrap();
    let entry = repo.backend.list(Kind::Locks).unwrap().pop().unwrap();
    let bytes = repo.backend.read(&entry.name).unwrap();
    repo.backend.delete(&entry.name).unwrap();
    assert!(guard.refresh().is_err());
    repo.backend.write(&entry.name, &bytes).unwrap();
    assert!(matches!(guard.self_check(), Err(Error::Locked(_))));
}
