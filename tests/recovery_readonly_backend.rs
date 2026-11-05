use keelhaul::{
    backend::{
        Backend, BackendError, Entry, ForeignEntry, Kind, ObjectName, memory::MemoryBackend,
    },
    ops::{backup::backup, check, listing, restore, verify},
    repo::Repository,
    secret::Secret,
};
struct ReadOnly(MemoryBackend);
fn refused() -> BackendError {
    std::io::Error::from(std::io::ErrorKind::PermissionDenied).into()
}
impl Backend for ReadOnly {
    fn write(&self, _: &ObjectName, _: &[u8]) -> Result<(), BackendError> {
        Err(refused())
    }
    fn delete(&self, _: &ObjectName) -> Result<(), BackendError> {
        Err(refused())
    }
    fn delete_foreign(&self, _: &str) -> Result<(), BackendError> {
        Err(refused())
    }
    fn read(&self, name: &ObjectName) -> Result<Vec<u8>, BackendError> {
        self.0.read(name)
    }
    fn read_range(
        &self,
        name: &ObjectName,
        offset: u64,
        len: u64,
    ) -> Result<Vec<u8>, BackendError> {
        self.0.read_range(name, offset, len)
    }
    fn stat(&self, name: &ObjectName) -> Result<u64, BackendError> {
        self.0.stat(name)
    }
    fn list(&self, kind: Kind) -> Result<Vec<Entry>, BackendError> {
        self.0.list(kind)
    }
    fn list_foreign(&self, kind: Kind) -> Result<Vec<ForeignEntry>, BackendError> {
        self.0.list_foreign(kind)
    }
}
#[test]
fn unlocked_readers_never_write_and_audits_still_examine_stored_locks() {
    let scratch = tempfile::tempdir().unwrap();
    let source = std::fs::canonicalize(scratch.path()).unwrap().join("file");
    std::fs::write(&source, b"saved bytes").unwrap();
    let secret = Secret::KeyFile([7; 32]);
    let repo = Repository::init(MemoryBackend::new(), &secret).unwrap();
    backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
    let repo = Repository::open(ReadOnly(repo.backend), &secret).unwrap();
    assert!(restore::restore(&repo, "latest", &scratch.path().join("locked"), &[]).is_err());
    let out = scratch.path().join("unlocked");
    let report = restore::restore_for_host_without_lock(&repo, "latest", &out, &[], None).unwrap();
    assert!(report.damaged.is_empty() && report.failed.is_empty());
    assert_eq!(
        std::fs::read(out.join(source.strip_prefix("/").unwrap())).unwrap(),
        b"saved bytes"
    );
    assert_eq!(repo.scan_snapshots().unwrap().entries.len(), 1);
    assert!(
        !listing::ls_report_for_host_without_lock(&repo, "latest", None, None)
            .unwrap()
            .entries
            .is_empty()
    );
    assert!(
        check::check_backend_without_lock(&repo.backend, &secret)
            .unwrap()
            .damage
            .is_empty()
    );
    assert!(
        verify::verify_backend_without_lock(&repo.backend, &secret, None)
            .unwrap()
            .damage
            .is_empty()
    );
    let name = format!("locks/{}", "ab".repeat(32));
    repo.backend.0.plant_raw(&name, b"damaged lock").unwrap();
    for report in [
        check::check_backend_without_lock(&repo.backend, &secret).unwrap(),
        verify::verify_backend_without_lock(&repo.backend, &secret, None).unwrap(),
    ] {
        assert!(report.damage.iter().any(|d| d.object == name), "{report:?}");
    }
}
