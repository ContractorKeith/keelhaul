use keelhaul::{
    backend::memory::MemoryBackend,
    ops::{backup::backup, restore::restore},
    repo::Repository,
    secret::Secret,
};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::PathBuf,
};
#[test]
fn existing_directory_and_disjoint_sibling_are_reused_without_metadata_changes() {
    let d = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(d.path()).unwrap();
    let source = root.as_path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("foo"), b"content").unwrap();
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([1; 32])).unwrap();
    backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
    let target = root.as_path().join("out");
    let existing = target.join(source.strip_prefix("/").unwrap());
    fs::create_dir_all(&existing).unwrap();
    fs::write(existing.join("unrelated"), b"keep").unwrap();
    fs::set_permissions(&existing, fs::Permissions::from_mode(0o751)).unwrap();
    let time = filetime::FileTime::from_unix_time(123456, 789);
    filetime::set_file_mtime(&existing, time).unwrap();
    let before = fs::metadata(&existing).unwrap();
    restore(&repo, "latest", &target, &[]).unwrap();
    let after = fs::metadata(&existing).unwrap();
    assert_eq!(before.mode(), after.mode());
    assert_eq!(
        (before.mtime(), before.mtime_nsec()),
        (after.mtime(), after.mtime_nsec())
    );
    assert_eq!((before.uid(), before.gid()), (after.uid(), after.gid()));
    assert_eq!(fs::read(existing.join("unrelated")).unwrap(), b"keep");
    assert_eq!(fs::read(existing.join("foo")).unwrap(), b"content");
    let other = root.as_path().join("conflict");
    let path = other.join(source.strip_prefix("/").unwrap());
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"user file").unwrap();
    assert!(
        restore(&repo, "latest", &other, &[])
            .unwrap_err()
            .to_string()
            .contains("conflict")
    );
    assert_eq!(fs::read(path).unwrap(), b"user file");
}
#[test]
fn every_include_must_match_exactly_before_any_destination_is_written() {
    let d = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(d.path()).unwrap();
    let source = root.as_path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("foo"), b"content").unwrap();
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([1; 32])).unwrap();
    backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
    for includes in [
        vec![source.join("fo")],
        vec![source.join("foo"), source.join("missing")],
        vec![PathBuf::from("fo")],
    ] {
        let target = root.as_path().join("out");
        let error = restore(&repo, "latest", &target, &includes)
            .unwrap_err()
            .to_string();
        assert!(error.contains("include path not found"));
        assert!(error.contains(&includes.last().unwrap().display().to_string()));
        assert!(!target.exists());
    }
    let target = root.as_path().join("single");
    restore(&repo, "latest", &target, &[source.join("foo")]).unwrap();
    assert_eq!(
        fs::read(target.join(source.strip_prefix("/").unwrap()).join("foo")).unwrap(),
        b"content"
    );
}
