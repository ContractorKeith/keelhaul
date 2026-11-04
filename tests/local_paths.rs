#![allow(clippy::unwrap_used, reason = "test assertions and fixtures may panic")]
use keelhaul::backend::local::LocalBackend;

#[cfg(target_os = "linux")]
#[test]
fn repository_parent_need_not_be_readable() {
    use keelhaul::backend::{Backend, ObjectName};
    use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};
    struct RestoreMode(PathBuf, fs::Permissions);
    impl Drop for RestoreMode {
        fn drop(&mut self) {
            fs::set_permissions(&self.0, self.1.clone()).expect("restore scratch permissions");
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let parent = dir.path().join("restricted");
    fs::create_dir(&parent).unwrap();
    let existing = parent.join("existing");
    fs::create_dir(&existing).unwrap();
    let _restore = RestoreMode(parent.clone(), fs::metadata(&parent).unwrap().permissions());
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o311)).unwrap();
    for root in [existing, parent.join("new")] {
        let backend = LocalBackend::new(&root).unwrap();
        for object in ["config".to_owned(), format!("data/ab/{}", "ab".repeat(32))] {
            let name = ObjectName::new(&object).unwrap();
            backend.write(&name, b"complete").unwrap();
            assert_eq!(backend.read(&name).unwrap(), b"complete");
        }
    }
}

#[cfg(target_os = "macos")]
#[test]
fn non_utf8_repository_path_returns_error() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let dir = tempfile::tempdir().unwrap();
    let root = dir
        .path()
        .join(OsString::from_vec(b"repository-\xff".to_vec()));
    assert!(LocalBackend::new(root).is_err());
}
