use keelhaul::{
    backend::{Backend, Kind},
    ops::{backup::backup, restore::restore},
    repo::Repository,
    secret::Secret,
};
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStringExt;
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    path::Path,
};
fn fixture(root: &Path) {
    fs::create_dir_all(root.join("nested/empty")).unwrap();
    for (name, size) in [
        ("empty", 0),
        ("small", 15),
        ("min-1", 524287),
        ("min", 524288),
        ("max", 8388608),
        ("max+1", 8388609),
        ("nested/binary", 3 * 1024 * 1024),
    ] {
        let mut bytes = vec![0; size];
        let mut h = blake3::Hasher::new();
        h.update(name.as_bytes());
        h.finalize_xof().fill(&mut bytes);
        fs::write(root.join(name), bytes).unwrap();
    }
    fs::write(root.join("héllo"), b"unicode").unwrap();
    #[cfg(target_os = "linux")]
    fs::write(
        root.join(std::ffi::OsString::from_vec(vec![b'n', 0xff])),
        b"raw",
    )
    .unwrap();
    symlink("nested/binary", root.join("link")).unwrap();
    symlink("missing", root.join("dangling")).unwrap();
    fs::hard_link(root.join("small"), root.join("hardlink")).unwrap();
    fs::set_permissions(root.join("small"), fs::Permissions::from_mode(0o640)).unwrap();
    fn times(path: &Path) {
        let meta = fs::symlink_metadata(path).unwrap();
        if meta.is_dir() {
            for e in fs::read_dir(path).unwrap() {
                times(&e.unwrap().path());
            }
        }
        let t = filetime::FileTime::from_unix_time(1_700_000_000, 123456789);
        filetime::set_symlink_file_times(path, t, t).unwrap();
    }
    times(root);
}
fn compare(a: &Path, b: &Path) {
    let am = fs::symlink_metadata(a).unwrap();
    let bm = fs::symlink_metadata(b).unwrap();
    assert_eq!(
        am.mode() & 0o7777,
        bm.mode() & 0o7777,
        "mode {}",
        a.display()
    );
    assert_eq!(
        (am.mtime(), am.mtime_nsec()),
        (bm.mtime(), bm.mtime_nsec()),
        "mtime {}",
        a.display()
    );
    if am.is_symlink() {
        assert_eq!(fs::read_link(a).unwrap(), fs::read_link(b).unwrap());
    } else if am.is_dir() {
        let mut aa = fs::read_dir(a)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect::<Vec<_>>();
        aa.sort();
        let mut bb = fs::read_dir(b)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect::<Vec<_>>();
        bb.sort();
        assert_eq!(aa, bb);
        for n in aa {
            compare(&a.join(&n), &b.join(&n));
        }
    } else {
        assert_eq!(fs::read(a).unwrap(), fs::read(b).unwrap());
    }
}
pub fn run<B: Backend>(backend: B) {
    let scratch = tempfile::tempdir().unwrap();
    let source = scratch.path().join("source%25");
    fixture(&source);
    let repo = Repository::init(backend, &Secret::KeyFile([7; 32])).unwrap();
    let first = backup(&repo, std::slice::from_ref(&source), vec!["test".into()]).unwrap();
    assert_eq!(first.summary.files_skipped, 0);
    let packs = repo.backend.list(Kind::Data).unwrap().len();
    let second = backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
    assert_eq!(second.summary.data_blobs_added, 0);
    assert_eq!(repo.backend.list(Kind::Data).unwrap().len(), packs);
    let target = scratch.path().join("restore");
    let report = restore(&repo, "latest", &target, &[]).unwrap();
    assert!(report.damaged.is_empty());
    let restored = target.join(source.strip_prefix("/").unwrap());
    compare(&source, &restored);
    assert_eq!(
        fs::metadata(restored.join("small")).unwrap().ino(),
        fs::metadata(restored.join("hardlink")).unwrap().ino()
    );
    let before = fs::read(restored.join("small")).unwrap();
    assert!(
        restore(&repo, "latest", &target, &[])
            .unwrap_err()
            .to_string()
            .contains("conflict")
    );
    assert_eq!(fs::read(restored.join("small")).unwrap(), before);
    let single = scratch.path().join("single");
    let file = source.join("nested/binary");
    restore(
        &repo,
        &first.snapshot[..8],
        &single,
        std::slice::from_ref(&file),
    )
    .unwrap();
    assert_eq!(
        fs::read(&file).unwrap(),
        fs::read(single.join(file.strip_prefix("/").unwrap())).unwrap()
    );
    assert!(
        !single
            .join(source.strip_prefix("/").unwrap())
            .join("small")
            .exists()
    );
    let entries = keelhaul::ops::listing::ls(&repo, "latest", None).unwrap();
    assert!(!entries.is_empty());
}
