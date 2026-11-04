use keelhaul::{
    backend::{
        Backend, BackendError, Entry, ForeignEntry, Kind, ObjectName, memory::MemoryBackend,
    },
    crypto::envelope::Label,
    format::{
        pack::BlobKind,
        snapshot::{Snapshot, Summary},
    },
    ops::{check::check, listing::ls, restore::restore, verify::verify},
    repo::{Repository, blobs::BlobStore},
    secret::Secret,
};
use std::sync::atomic::{AtomicUsize, Ordering};
struct Reads {
    inner: MemoryBackend,
    ranges: AtomicUsize,
}
impl Backend for Reads {
    fn read(&self, n: &ObjectName) -> Result<Vec<u8>, BackendError> {
        self.inner.read(n)
    }
    fn read_range(&self, n: &ObjectName, o: u64, l: u64) -> Result<Vec<u8>, BackendError> {
        self.ranges.fetch_add(1, Ordering::Relaxed);
        self.inner.read_range(n, o, l)
    }
    fn write(&self, n: &ObjectName, b: &[u8]) -> Result<(), BackendError> {
        self.inner.write(n, b)
    }
    fn stat(&self, n: &ObjectName) -> Result<u64, BackendError> {
        self.inner.stat(n)
    }
    fn list(&self, k: Kind) -> Result<Vec<Entry>, BackendError> {
        self.inner.list(k)
    }
    fn list_foreign(&self, k: Kind) -> Result<Vec<ForeignEntry>, BackendError> {
        self.inner.list_foreign(k)
    }
    fn delete(&self, n: &ObjectName) -> Result<(), BackendError> {
        self.inner.delete(n)
    }
    fn delete_foreign(&self, n: &str) -> Result<(), BackendError> {
        self.inner.delete_foreign(n)
    }
}
fn dir(name: &str, id: &str) -> serde_json::Value {
    serde_json::json!({"name":name,"type":"dir","subtree":id,"mode":493,"mtime":"2026-11-04T14:03:22.123456789Z","ctime":"2026-11-04T14:03:22.123456789Z","uid":0,"gid":0,"inode":1,"device":1,"links":1})
}
fn fixture(depth: usize) -> Repository<Reads> {
    let repo = Repository::init(
        Reads {
            inner: MemoryBackend::new(),
            ranges: AtomicUsize::new(0),
        },
        &Secret::KeyFile([7; 32]),
    )
    .unwrap();
    let mut store = BlobStore::load(&repo).unwrap();
    let mut id = store
        .add(BlobKind::Tree, br#"{"version":1,"entries":[]}"#)
        .unwrap();
    for _ in 0..depth {
        id = store
            .add(
                BlobKind::Tree,
                &serde_json::to_vec(
                    &serde_json::json!({"version":1,"entries":[dir("a",&id),dir("b",&id)]}),
                )
                .unwrap(),
            )
            .unwrap();
    }
    id = store
        .add(
            BlobKind::Tree,
            &serde_json::to_vec(&serde_json::json!({"version":1,"entries":[dir("%2Fsource",&id)]}))
                .unwrap(),
        )
        .unwrap();
    store.finish().unwrap();
    let s = Snapshot {
        version: 1,
        time: "2026-11-04T14:03:22.123456789Z".into(),
        host: keelhaul::host::hostname(),
        username: "test".into(),
        paths: vec!["/source".into()],
        tags: vec![],
        excludes: vec![],
        tree: id,
        parent: None,
        program: "test".into(),
        summary: Summary::default(),
    };
    repo.write_object(
        "snapshots",
        Label::Snapshot,
        &serde_json::to_vec(&s).unwrap(),
    )
    .unwrap();
    repo
}
#[test]
fn depth_24_shared_tree_dag_is_linear_for_audits_and_listing() {
    let repo = fixture(24);
    for operation in ["check", "verify", "ls"] {
        repo.backend.ranges.store(0, Ordering::Relaxed);
        let start = std::time::Instant::now();
        match operation {
            "check" => assert!(check(&repo).unwrap().damage.is_empty()),
            "verify" => assert!(verify(&repo, None).unwrap().damage.is_empty()),
            _ => {
                let output = ls(&repo, "latest", None).unwrap();
                assert_eq!(output.len(), 49);
                assert_eq!(output.iter().filter(|e| e.shared).count(), 24);
            }
        }
        let elapsed = start.elapsed();
        println!("depth 24 {operation}: {elapsed:?}");
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "{operation}: {elapsed:?}"
        );
        assert!(
            repo.backend.ranges.load(Ordering::Relaxed) <= 28,
            "{operation} re-read a shared tree"
        );
    }
    let filter = std::path::PathBuf::from(format!("/source/{}", vec!["b"; 24].join("/")));
    let entries = ls(&repo, "latest", Some(&filter)).unwrap();
    assert_eq!(entries.len(), 1);
    assert!(!entries[0].shared);
}
#[test]
fn restore_decrypts_shared_trees_once_and_still_creates_every_path() {
    let repo = fixture(6);
    let target = tempfile::tempdir().unwrap();
    repo.backend.ranges.store(0, Ordering::Relaxed);
    let report = restore(&repo, "latest", target.path(), &[]).unwrap();
    assert!(report.damaged.is_empty());
    assert_eq!(report.restored, 127);
    assert!(repo.backend.ranges.load(Ordering::Relaxed) <= 10);
    fn count(p: &std::path::Path) -> usize {
        std::fs::read_dir(p)
            .unwrap()
            .map(|e| {
                let p = e.unwrap().path();
                1 + if p.is_dir() { count(&p) } else { 0 }
            })
            .sum()
    }
    assert_eq!(count(target.path()), 127);
}
