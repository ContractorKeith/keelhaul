//! Opt-in fault injection. Ordinary builds neither compile nor read this hook.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct FaultBackend<B> {
    inner: B,
    operation: String,
    at: u64,
    hang: bool,
    calls: AtomicU64,
}
impl<B> FaultBackend<B> {
    pub fn new(inner: B, operation: &str, at: u64, hang: bool) -> Self {
        Self {
            inner,
            operation: operation.into(),
            at,
            hang,
            calls: AtomicU64::new(0),
        }
    }
    pub fn from_env(inner: B) -> Result<Self, BackendError> {
        let hook = std::env::var("KEELHAUL_FAULT").unwrap_or_default();
        if hook.is_empty() {
            return Ok(Self::new(inner, "", 0, false));
        }
        let fields = hook.split(':').collect::<Vec<_>>();
        if fields.len() != 3
            || !["read", "write", "list"].contains(&fields[0])
            || !["fail", "hang"].contains(&fields[2])
        {
            return Err(BackendError::InvalidConfig(
                "fault must be read|write|list:N:fail|hang",
            ));
        }
        let at = fields[1]
            .parse::<u64>()
            .ok()
            .filter(|n| *n > 0)
            .ok_or(BackendError::InvalidConfig("fault count must be positive"))?;
        Ok(Self::new(inner, fields[0], at, fields[2] == "hang"))
    }
    fn hit(&self, operation: &str) -> Result<(), BackendError> {
        if self.operation == operation && self.calls.fetch_add(1, Ordering::SeqCst) + 1 == self.at {
            if self.hang {
                loop {
                    std::thread::park();
                }
            }
            return Err(
                io::Error::other(format!("injected {operation} failure at {}", self.at)).into(),
            );
        }
        Ok(())
    }
}
impl<B: Backend> Backend for FaultBackend<B> {
    fn write(&self, name: &ObjectName, bytes: &[u8]) -> Result<(), BackendError> {
        self.hit("write")?;
        self.inner.write(name, bytes)
    }
    fn read(&self, name: &ObjectName) -> Result<Vec<u8>, BackendError> {
        self.hit("read")?;
        self.inner.read(name)
    }
    fn read_range(
        &self,
        name: &ObjectName,
        offset: u64,
        len: u64,
    ) -> Result<Vec<u8>, BackendError> {
        self.hit("read")?;
        self.inner.read_range(name, offset, len)
    }
    fn list(&self, kind: Kind) -> Result<Vec<Entry>, BackendError> {
        self.hit("list")?;
        self.inner.list(kind)
    }
    fn list_foreign(&self, kind: Kind) -> Result<Vec<ForeignEntry>, BackendError> {
        self.hit("list")?;
        self.inner.list_foreign(kind)
    }
    fn stat(&self, name: &ObjectName) -> Result<u64, BackendError> {
        self.hit("read")?;
        self.inner.stat(name)
    }
    fn delete(&self, name: &ObjectName) -> Result<(), BackendError> {
        self.inner.delete(name)
    }
    fn delete_foreign(&self, raw: &str) -> Result<(), BackendError> {
        self.inner.delete_foreign(raw)
    }
    fn contains_destination(&self, path: &Path) -> Result<bool, BackendError> {
        self.inner.contains_destination(path)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_each_selected_operation_only() {
        for operation in ["write", "read", "list"] {
            let backend = FaultBackend::new(memory::MemoryBackend::new(), operation, 2, false);
            let name = ObjectName::new("config").unwrap();
            let call = || match operation {
                "write" => backend.write(&name, b"x").map(|_| ()),
                "read" => backend.read(&name).map(|_| ()),
                _ => backend.list(Kind::Data).map(|_| ()),
            };
            backend.inner.write(&name, b"x").unwrap();
            call().unwrap();
            assert!(call().unwrap_err().to_string().contains("injected"));
            call().unwrap();
        }
    }
}
