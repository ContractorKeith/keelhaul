//! Temporary deterministic reproduction of the SPEC §11 publication window.
use keelhaul::{backend::{Backend, BackendError, Entry, ForeignEntry, Kind, ObjectName, memory::MemoryBackend}, repo::Repository, secret::Secret, ops::{backup::{backup_with_clock, Options}, prune::prune, check::check}, repo::lock::Clock};
use std::{sync::{Arc, Mutex, Condvar, atomic::{AtomicBool, AtomicU64, Ordering}}, time::{SystemTime, Duration}};
struct Paused {
    inner: MemoryBackend,
    pause: AtomicBool,
    reached: (Mutex<bool>, Condvar),
    resume: (Mutex<bool>, Condvar),
}
impl Backend for Paused {
    fn write(&self,n:&ObjectName,b:&[u8])->Result<(),BackendError>{
        if n.kind()==Kind::Snapshots && self.pause.load(Ordering::SeqCst) {
            *self.reached.0.lock().unwrap()=true; self.reached.1.notify_all();
            let mut resume=self.resume.0.lock().unwrap(); while !*resume { resume=self.resume.1.wait(resume).unwrap(); }
        }
        self.inner.write(n,b)
    }
    fn read(&self,n:&ObjectName)->Result<Vec<u8>,BackendError>{self.inner.read(n)}
    fn read_range(&self,n:&ObjectName,o:u64,l:u64)->Result<Vec<u8>,BackendError>{self.inner.read_range(n,o,l)}
    fn list(&self,k:Kind)->Result<Vec<Entry>,BackendError>{self.inner.list(k)}
    fn list_foreign(&self,k:Kind)->Result<Vec<ForeignEntry>,BackendError>{self.inner.list_foreign(k)}
    fn delete_foreign(&self,n:&str)->Result<(),BackendError>{self.inner.delete_foreign(n)}
    fn stat(&self,n:&ObjectName)->Result<u64,BackendError>{self.inner.stat(n)}
    fn delete(&self,n:&ObjectName)->Result<(),BackendError>{self.inner.delete(n)}
}
struct Fake(AtomicU64);
impl Clock for Fake { fn now(&self)->SystemTime{SystemTime::UNIX_EPOCH+Duration::from_secs(self.0.load(Ordering::SeqCst))} }
#[test]
fn stale_backup_publishes_after_prune_despite_required_self_check() {
    let scratch=tempfile::tempdir().unwrap(); let file=scratch.path().join("source"); std::fs::write(&file,b"irreplaceable snapshot bytes").unwrap();
    let repo=Arc::new(Repository::init(Paused{inner:MemoryBackend::new(),pause:AtomicBool::new(true),reached:(Mutex::new(false),Condvar::new()),resume:(Mutex::new(false),Condvar::new())},&Secret::KeyFile([91;32])).unwrap());
    let now=SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
    let clock=Arc::new(Fake(AtomicU64::new(now-1801)));
    let r=repo.clone(); let c=clock.clone();
    let backup=std::thread::spawn(move||backup_with_clock(&r,&[file],vec![],&Options::default(),c.as_ref()));
    let mut reached=repo.backend.reached.0.lock().unwrap();
    while !*reached { reached=repo.backend.reached.1.wait(reached).unwrap(); }
    drop(reached);
    // Snapshot publication has begun, after SPEC §11's required self-check.
    // Advance the clock by 30m+1s while the process is suspended at that point.
    clock.0.store(now,Ordering::SeqCst);
    assert_eq!(repo.backend.list(Kind::Snapshots).unwrap().len(),0);
    assert!(!repo.backend.list(Kind::Index).unwrap().is_empty());
    let summary=prune(&repo,20,false).unwrap();
    eprintln!("prune succeeded while backup paused: {summary:?}");
    assert!(repo.backend.list(Kind::Data).unwrap().is_empty());
    *repo.backend.resume.0.lock().unwrap()=true; repo.backend.resume.1.notify_all();
    let report=backup.join().unwrap().unwrap();
    eprintln!("backup succeeded, publishing snapshot {}",report.snapshot);
    let report=check(&repo).unwrap();
    eprintln!("check after both successful commands: {report:?}");
    assert!(report.damage.is_empty(),"SPEC §11's self-check does not protect snapshot publication from stale-lock prune");
}
