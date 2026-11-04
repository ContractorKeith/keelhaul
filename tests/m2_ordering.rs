use keelhaul::{
    backend::{
        Backend, BackendError, Entry, ForeignEntry, Kind, ObjectName, memory::MemoryBackend,
    },
    crypto::envelope::Label,
    format::index::Index,
    ops::backup::backup,
    repo::Repository,
    secret::Secret,
};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
struct Trace {
    backend: MemoryBackend,
    writes: Mutex<Vec<Kind>>,
    fail_snapshot: AtomicBool,
}
impl Backend for Trace {
    fn write(&self, n: &ObjectName, b: &[u8]) -> Result<(), BackendError> {
        if n.kind() == Kind::Snapshots && self.fail_snapshot.load(Ordering::Relaxed) {
            return Err(std::io::Error::other("injected interruption before snapshot").into());
        }
        self.backend.write(n, b)?;
        self.writes.lock().unwrap().push(n.kind());
        Ok(())
    }
    fn read(&self, n: &ObjectName) -> Result<Vec<u8>, BackendError> {
        self.backend.read(n)
    }
    fn read_range(&self, n: &ObjectName, o: u64, l: u64) -> Result<Vec<u8>, BackendError> {
        self.backend.read_range(n, o, l)
    }
    fn stat(&self, n: &ObjectName) -> Result<u64, BackendError> {
        self.backend.stat(n)
    }
    fn list(&self, k: Kind) -> Result<Vec<Entry>, BackendError> {
        self.backend.list(k)
    }
    fn list_foreign(&self, k: Kind) -> Result<Vec<ForeignEntry>, BackendError> {
        self.backend.list_foreign(k)
    }
    fn delete_foreign(&self, n: &str) -> Result<(), BackendError> {
        self.backend.delete_foreign(n)
    }
    fn delete(&self, n: &ObjectName) -> Result<(), BackendError> {
        self.backend.delete(n)
    }
}
#[test]
fn indexes_flush_after_eight_packs_and_interrupted_snapshot_reuses_all_flushed_blobs() {
    let d = tempfile::tempdir().unwrap();
    let source = d.path().join("stream");
    let mut bytes = vec![0; 32 * 1024 * 1024];
    blake3::Hasher::new().finalize_xof().fill(&mut bytes);
    std::fs::write(&source, &bytes).unwrap();
    drop(bytes);
    let backend = Trace {
        backend: MemoryBackend::new(),
        writes: Mutex::new(Vec::new()),
        fail_snapshot: AtomicBool::new(true),
    };
    let mut repo = Repository::init(backend, &Secret::KeyFile([9; 32])).unwrap();
    repo.config.pack_target = 1024 * 1024;
    repo.backend.writes.lock().unwrap().clear();
    assert!(backup(&repo, std::slice::from_ref(&source), vec![]).is_err());
    assert!(repo.snapshots().unwrap().is_empty());
    let writes = repo
        .backend
        .writes
        .lock()
        .unwrap()
        .iter()
        .copied()
        .filter(|kind| *kind != Kind::Locks)
        .collect::<Vec<_>>();
    let first = writes.iter().position(|k| *k == Kind::Index).unwrap();
    assert_eq!(first, 8);
    assert!(writes[..first].iter().all(|k| *k == Kind::Data));
    for entry in repo.backend.list(Kind::Index).unwrap() {
        let index = Index::parse(&repo.read_object(&entry.name, Label::Index).unwrap()).unwrap();
        for pack in index.packs {
            assert!(
                repo.backend
                    .stat(&keelhaul::repo::blobs::pack_name(&pack.id).unwrap())
                    .is_ok()
            );
        }
    }
    repo.backend.fail_snapshot.store(false, Ordering::Relaxed);
    let report = backup(&repo, &[source], vec![]).unwrap();
    assert_eq!(report.summary.data_blobs_added, 0);
    assert_eq!(
        repo.backend.writes.lock().unwrap().last(),
        Some(&Kind::Snapshots)
    );
}
