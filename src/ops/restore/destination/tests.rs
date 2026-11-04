use super::*;
#[test]
fn replaced_directory_symlink_is_never_traversed() {
    let scratch = tempfile::tempdir().unwrap();
    let target = scratch.path().join("target");
    let outside = scratch.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    let destination = Destination::new(&target).unwrap();
    destination.mkdir(Path::new("child")).unwrap();
    std::fs::remove_dir(target.join("child")).unwrap();
    std::os::unix::fs::symlink(&outside, target.join("child")).unwrap();
    assert!(destination.temporary(Path::new("child/escape")).is_err());
    assert!(std::fs::read_dir(outside).unwrap().next().is_none());
}
#[test]
fn publication_refuses_a_conflict_created_after_preflight() {
    let scratch = tempfile::tempdir().unwrap();
    let destination = Destination::new(scratch.path()).unwrap();
    let temporary = destination.temporary(Path::new("file")).unwrap();
    std::fs::write(scratch.path().join("file"), b"user bytes").unwrap();
    let entry: Entry = serde_json::from_value(serde_json::json!({
        "name":"file","type":"file","mode":420,
        "mtime":"2026-11-04T14:03:22.123456789Z", "ctime":"2026-11-04T14:03:22.123456789Z",
        "uid":0,"gid":0,"inode":1,"device":1,"links":1,"size":0,"content":[]
    }))
    .unwrap();
    assert!(temporary.publish(&entry).is_err());
    drop(temporary);
    assert_eq!(
        std::fs::read(scratch.path().join("file")).unwrap(),
        b"user bytes"
    );
    assert_eq!(std::fs::read_dir(scratch.path()).unwrap().count(), 1);
}

#[test]
fn target_swap_after_open_never_redirects_writes() {
    let scratch = tempfile::tempdir().unwrap();
    let target = scratch.path().join("target");
    let pinned = scratch.path().join("pinned");
    let outside = scratch.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    let destination = Destination::new(&target).unwrap();
    std::fs::rename(&target, &pinned).unwrap();
    std::os::unix::fs::symlink(&outside, &target).unwrap();
    destination.preflight(&[]).unwrap();
    let mut temp = destination.temporary(Path::new("file")).unwrap();
    use std::io::Write;
    temp.file.write_all(b"pinned").unwrap();
    assert!(std::fs::read_dir(&outside).unwrap().next().is_none());
    assert_eq!(std::fs::read_dir(&pinned).unwrap().count(), 1);
    assert!(Destination::new(&target).is_err());
}
