#![allow(clippy::unwrap_used, reason = "test assertions and fixtures may panic")]
use keelhaul::backend::{
    Backend, BackendError, Kind, ObjectName, local::LocalBackend, memory::MemoryBackend,
};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
};

struct RestoreMode(PathBuf, fs::Permissions);
impl Drop for RestoreMode {
    fn drop(&mut self) {
        fs::set_permissions(&self.0, self.1.clone()).expect("restore scratch permissions");
    }
}

#[test]
fn io_errors_include_the_object_and_failing_path() {
    let dir = tempfile::tempdir().unwrap();
    let b = LocalBackend::new(dir.path()).unwrap();
    let _restore = RestoreMode(
        dir.path().to_owned(),
        fs::metadata(dir.path()).unwrap().permissions(),
    );
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o500)).unwrap();
    let name = ObjectName::new("config").unwrap();
    let error = b.write(&name, b"content").unwrap_err();
    let message = error.to_string();
    let BackendError::Io {
        name: Some(object),
        path: Some(path),
        source,
    } = error
    else {
        panic!("expected contextual I/O error");
    };
    assert_eq!(object, name);
    assert!(path.starts_with(dir.path()));
    assert!(message.contains(&path.display().to_string()));
    assert_eq!(source.kind(), std::io::ErrorKind::PermissionDenied);
}

#[test]
fn timestamps_match_storage_and_survive_identical_retries() {
    let dir = tempfile::tempdir().unwrap();
    let b = LocalBackend::new(dir.path()).unwrap();
    let name = ObjectName::new("config").unwrap();
    b.write(&name, b"content").unwrap();
    let entries = b.list(Kind::Config).unwrap();
    assert_eq!(
        entries[0].modified,
        Some(
            fs::metadata(dir.path().join("config"))
                .unwrap()
                .modified()
                .unwrap()
        )
    );
    b.write(&name, b"content").unwrap();
    assert_eq!(b.list(Kind::Config).unwrap(), entries);
    let raw = "config.tmp-0123456789abcdef";
    fs::write(dir.path().join(raw), b"partial").unwrap();
    assert_eq!(
        b.list_foreign(Kind::Config).unwrap()[0].modified,
        Some(
            fs::metadata(dir.path().join(raw))
                .unwrap()
                .modified()
                .unwrap()
        )
    );

    let memory = MemoryBackend::new();
    memory.write(&name, b"content").unwrap();
    let entries = memory.list(Kind::Config).unwrap();
    assert!(entries[0].modified.is_some());
    memory.write(&name, b"content").unwrap();
    assert_eq!(memory.list(Kind::Config).unwrap(), entries);
    memory.plant_raw(raw, b"partial").unwrap();
    assert!(
        memory.list_foreign(Kind::Config).unwrap()[0]
            .modified
            .is_some()
    );
}

#[test]
fn foreign_deletion_refuses_nested_symlink_paths() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let b = LocalBackend::new(dir.path()).unwrap();
    fs::create_dir(dir.path().join("keys")).unwrap();
    fs::write(outside.path().join("unknown"), b"preserve").unwrap();
    symlink(outside.path(), dir.path().join("keys/link")).unwrap();
    assert!(matches!(
        b.delete_foreign("keys/link/unknown"),
        Err(BackendError::InvalidName(_))
    ));
    assert_eq!(
        fs::read(outside.path().join("unknown")).unwrap(),
        b"preserve"
    );
    let foreign = b.list_foreign(Kind::Keys).unwrap();
    assert_eq!(foreign.len(), 1);
    assert_eq!(foreign[0].raw, "keys/link");
    assert!(!foreign[0].deletable);
}

#[test]
fn unsafe_foreign_names_remain_visible_for_check() {
    let dir = tempfile::tempdir().unwrap();
    let b = LocalBackend::new(dir.path()).unwrap();
    fs::create_dir(dir.path().join("keys")).unwrap();
    fs::write(dir.path().join("keys/a..b"), b"unknown").unwrap();
    assert_eq!(b.list_foreign(Kind::Keys).unwrap()[0].raw, "keys/a..b");
    assert!(matches!(
        b.delete_foreign("keys/a..b"),
        Err(BackendError::InvalidName(_))
    ));
}

#[test]
fn conflicting_length_does_not_read_existing_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let b = LocalBackend::new(dir.path()).unwrap();
    let name = ObjectName::new("config").unwrap();
    b.write(&name, b"content").unwrap();
    let path = dir.path().join("config");
    let _restore = RestoreMode(path.clone(), fs::metadata(&path).unwrap().permissions());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
    assert!(matches!(
        b.write(&name, b"x"),
        Err(BackendError::AlreadyExists(_))
    ));
    let error = b.write(&name, b"content").unwrap_err();
    assert!(error.to_string().contains(&path.display().to_string()));
    assert!(
        matches!(error, BackendError::Io { source, .. } if source.kind() == std::io::ErrorKind::PermissionDenied)
    );
}
