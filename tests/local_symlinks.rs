#![allow(clippy::unwrap_used, reason = "test assertions and fixtures may panic")]
use keelhaul::backend::{Backend, BackendError, Kind, ObjectName, local::LocalBackend};
use std::{fs, os::unix::fs::symlink};

fn pack() -> ObjectName {
    ObjectName::new(&format!("data/ab/{}", "ab".repeat(32))).unwrap()
}

#[test]
fn linked_data_and_shards_support_listing_reads_writes_and_foreign_deletion() {
    for whole_data in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let disk = tempfile::tempdir().unwrap();
        let b = LocalBackend::new(dir.path()).unwrap();
        if whole_data {
            symlink(disk.path(), dir.path().join("data")).unwrap();
        } else {
            fs::create_dir(dir.path().join("data")).unwrap();
            symlink(disk.path(), dir.path().join("data/ab")).unwrap();
        }
        let object = pack();
        b.write(&object, b"pack bytes").unwrap();
        b.write(&object, b"pack bytes").unwrap();
        assert_eq!(b.read(&object).unwrap(), b"pack bytes");
        assert_eq!(b.read_range(&object, 2, 4).unwrap(), b"ck b");
        let entries = b.list(Kind::Data).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, object);
        assert_eq!(entries[0].size, 10);
        assert!(b.list_foreign(Kind::Data).unwrap().is_empty());
        let raw = "data/ab/unknown.tmp-0123456789abcdef";
        fs::write(dir.path().join(raw), b"partial").unwrap();
        let foreign = b.list_foreign(Kind::Data).unwrap();
        assert_eq!(foreign.len(), 1);
        assert!(foreign[0].deletable);
        b.delete_foreign(raw).unwrap();
        assert!(b.list_foreign(Kind::Data).unwrap().is_empty());
        b.delete(&object).unwrap();
        assert!(b.list(Kind::Data).unwrap().is_empty());
    }
}

#[test]
fn linked_config_lists_and_reads_the_target() {
    let dir = tempfile::tempdir().unwrap();
    let disk = tempfile::tempdir().unwrap();
    let target = disk.path().join("config");
    fs::write(&target, b"config bytes").unwrap();
    symlink(&target, dir.path().join("config")).unwrap();
    let b = LocalBackend::new(dir.path()).unwrap();
    let object = ObjectName::new("config").unwrap();
    assert_eq!(b.read(&object).unwrap(), b"config bytes");
    b.write(&object, b"config bytes").unwrap();
    let entries = b.list(Kind::Config).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, object);
    assert_eq!(entries[0].size, 12);
    assert!(b.list_foreign(Kind::Config).unwrap().is_empty());
    b.delete(&object).unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"config bytes");
}

#[test]
fn dangling_and_looping_object_links_are_foreign_without_aborting_lists() {
    let dir = tempfile::tempdir().unwrap();
    let b = LocalBackend::new(dir.path()).unwrap();
    symlink("absent", dir.path().join("config")).unwrap();
    symlink("unknown", dir.path().join("unknown")).unwrap();
    assert!(b.list(Kind::Config).unwrap().is_empty());
    let entries = b.list_foreign(Kind::Config).unwrap();
    assert_eq!(
        entries
            .iter()
            .map(|entry| (entry.raw.as_str(), entry.deletable))
            .collect::<Vec<_>>(),
        vec![("config", false), ("unknown", true)]
    );
    assert!(matches!(
        b.read(&ObjectName::new("config").unwrap()),
        Err(BackendError::NotFound(_))
    ));
    b.delete_foreign("unknown").unwrap();
    assert!(matches!(
        b.delete_foreign("config"),
        Err(BackendError::InvalidName(_))
    ));
    fs::create_dir_all(dir.path().join("data/ab")).unwrap();
    let object = pack();
    symlink("ab".repeat(32), dir.path().join(object.as_str())).unwrap();
    assert!(b.list(Kind::Data).unwrap().is_empty());
    let entries = b.list_foreign(Kind::Data).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].raw, object.as_str());
    assert!(!entries[0].deletable);
}

#[cfg(target_os = "linux")]
#[test]
fn non_utf8_names_are_displayed_lossily_and_cannot_be_deleted() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let dir = tempfile::tempdir().unwrap();
    let b = LocalBackend::new(dir.path()).unwrap();
    fs::write(dir.path().join(OsString::from_vec(vec![0xff])), b"unknown").unwrap();
    assert!(b.list(Kind::Config).unwrap().is_empty());
    let entries = b.list_foreign(Kind::Config).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].raw, "\u{fffd}");
    assert_eq!(entries[0].size, 7);
    assert!(!entries[0].deletable);
    assert!(matches!(
        b.delete_foreign(&entries[0].raw),
        Err(BackendError::InvalidName(_))
    ));
}

#[test]
fn broken_shard_links_are_non_deletable_diagnostics() {
    let dir = tempfile::tempdir().unwrap();
    let b = LocalBackend::new(dir.path()).unwrap();
    fs::create_dir(dir.path().join("data")).unwrap();
    symlink("absent", dir.path().join("data/ab")).unwrap();
    symlink("cd", dir.path().join("data/cd")).unwrap();
    assert!(b.list(Kind::Data).unwrap().is_empty());
    let entries = b.list_foreign(Kind::Data).unwrap();
    assert_eq!(
        entries
            .iter()
            .map(|entry| (entry.raw.as_str(), entry.deletable))
            .collect::<Vec<_>>(),
        vec![("data/ab", false), ("data/cd", false)]
    );
    for entry in entries {
        assert!(matches!(
            b.delete_foreign(&entry.raw),
            Err(BackendError::InvalidName(_))
        ));
    }
}
