use keelhaul::{
    backend::{
        Backend, BackendError, Entry, ForeignEntry, Kind, ObjectName, memory::MemoryBackend,
    },
    ops::backup::{Options, backup_with_options},
    repo::{Error, Repository},
    secret::Secret,
};
use std::{
    fs,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

#[derive(Default)]
struct LoseLock {
    inner: MemoryBackend,
    armed: AtomicBool,
    writes: AtomicUsize,
}
impl Backend for LoseLock {
    fn write(&self, name: &ObjectName, bytes: &[u8]) -> Result<(), BackendError> {
        self.inner.write(name, bytes)?;
        if name.kind() == Kind::Data {
            self.writes.fetch_add(1, Ordering::SeqCst);
            if self.armed.swap(false, Ordering::SeqCst) {
                for entry in self.inner.list(Kind::Locks)? {
                    self.inner.delete(&entry.name)?;
                }
            }
        }
        Ok(())
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
    fn delete_foreign(&self, s: &str) -> Result<(), BackendError> {
        self.inner.delete_foreign(s)
    }
    fn stat(&self, n: &ObjectName) -> Result<u64, BackendError> {
        self.inner.stat(n)
    }
    fn delete(&self, n: &ObjectName) -> Result<(), BackendError> {
        self.inner.delete(n)
    }
}

#[test]
fn lost_lock_stops_next_upload_and_preserves_locked_error() {
    let scratch = tempfile::tempdir().unwrap();
    let mut bytes = vec![0; 512 * 1024];
    for n in 0..24u64 {
        blake3::Hasher::new()
            .update(&n.to_le_bytes())
            .finalize_xof()
            .fill(&mut bytes);
        fs::write(scratch.path().join(format!("{n}")), &bytes).unwrap();
    }
    for jobs in [1, 4] {
        let mut repo = Repository::init(LoseLock::default(), &Secret::KeyFile([21; 32])).unwrap();
        repo.config.pack_target = 256 * 1024;
        repo.backend.armed.store(true, Ordering::SeqCst);
        let result = backup_with_options(
            &repo,
            &[scratch.path().into()],
            vec![],
            &Options {
                jobs,
                ..Options::default()
            },
        );
        assert!(matches!(result, Err(Error::Locked(_))), "{result:?}");
        let writes = repo.backend.writes.load(Ordering::SeqCst);
        assert!(
            writes <= jobs,
            "{writes} packs written with {jobs} upload workers after first pack removed lock"
        );
        assert!(repo.backend.list(Kind::Snapshots).unwrap().is_empty());
        println!("jobs {jobs}: {writes} pack writes total, no snapshot after lock loss");
    }
}
