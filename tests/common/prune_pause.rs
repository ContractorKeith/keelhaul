#![allow(
    dead_code,
    reason = "shared fault backend supports several prune integration targets"
)]
use keelhaul::backend::{
    Backend, BackendError, Entry, ForeignEntry, Kind, ObjectName, memory::MemoryBackend,
};
use std::sync::{
    Condvar, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Duration;
type DeleteHook = Box<dyn Fn(&ObjectName, &MemoryBackend) + Send + Sync>;
#[derive(Default)]
pub struct Paused {
    pub inner: MemoryBackend,
    pub armed: AtomicBool,
    pub pause_snapshot: AtomicBool,
    pub index_writes: Mutex<Vec<String>>,
    pub index_calls: AtomicUsize,
    pub panic_index_at: AtomicUsize,
    pub unknown_times: AtomicBool,
    pub on_delete: Mutex<Option<DeleteHook>>,
    reached: (Mutex<bool>, Condvar),
    resume: (Mutex<bool>, Condvar),
}
impl Paused {
    pub fn wait(&self) {
        let ready = self.reached.0.lock().unwrap();
        let (ready, _) = self
            .reached
            .1
            .wait_timeout_while(ready, Duration::from_secs(20), |ready| !*ready)
            .unwrap();
        assert!(*ready, "operation did not reach its publication checkpoint");
    }
    pub fn release(&self) {
        *self.resume.0.lock().unwrap() = true;
        self.resume.1.notify_all();
    }
}
impl Backend for Paused {
    fn write(&self, n: &ObjectName, b: &[u8]) -> Result<(), BackendError> {
        if (n.kind() == Kind::Index && self.armed.swap(false, Ordering::SeqCst))
            || (n.kind() == Kind::Snapshots && self.pause_snapshot.swap(false, Ordering::SeqCst))
        {
            *self.reached.0.lock().unwrap() = true;
            self.reached.1.notify_all();
            let mut resume = self.resume.0.lock().unwrap();
            while !*resume {
                resume = self.resume.1.wait(resume).unwrap();
            }
        }
        self.inner.write(n, b)?;
        if n.kind() == Kind::Index {
            self.index_writes
                .lock()
                .unwrap()
                .push(n.as_str()[6..].to_owned());
            let number = self.index_calls.fetch_add(1, Ordering::SeqCst) + 1;
            assert_ne!(
                number,
                self.panic_index_at.load(Ordering::SeqCst),
                "injected index checkpoint panic"
            );
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
        let mut entries = self.inner.list(k)?;
        if self.unknown_times.load(Ordering::SeqCst) {
            for e in &mut entries {
                e.modified = None;
            }
        }
        Ok(entries)
    }
    fn list_foreign(&self, k: Kind) -> Result<Vec<ForeignEntry>, BackendError> {
        let mut entries = self.inner.list_foreign(k)?;
        if self.unknown_times.load(Ordering::SeqCst) {
            for e in &mut entries {
                e.modified = None;
            }
        }
        Ok(entries)
    }
    fn delete_foreign(&self, n: &str) -> Result<(), BackendError> {
        self.inner.delete_foreign(n)
    }
    fn stat(&self, n: &ObjectName) -> Result<u64, BackendError> {
        self.inner.stat(n)
    }
    fn delete(&self, n: &ObjectName) -> Result<(), BackendError> {
        self.inner.delete(n)?;
        if let Some(hook) = self.on_delete.lock().unwrap().as_ref() {
            hook(n, &self.inner);
        }
        Ok(())
    }
}
