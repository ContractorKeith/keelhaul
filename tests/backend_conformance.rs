#![allow(clippy::unwrap_used, reason = "test assertions and fixtures may panic")]
use keelhaul::backend::{
    Backend, BackendError, Entry, Kind, ObjectName, local::LocalBackend, memory::MemoryBackend,
};
use std::sync::{
    Barrier,
    atomic::{AtomicBool, Ordering},
};

fn name(s: &str) -> ObjectName {
    ObjectName::new(s).unwrap()
}

// New backends opt into this complete contract with run_suite(&backend).
fn run_suite(b: &dyn Backend) {
    for kind in Kind::ALL {
        assert!(b.list(kind).unwrap().is_empty());
    }
    let missing = name(&format!("data/ff/{}", "ff".repeat(32)));
    assert!(matches!(b.read(&missing), Err(BackendError::NotFound(_))));
    assert!(matches!(
        b.read_range(&missing, 0, 0),
        Err(BackendError::NotFound(_))
    ));
    assert!(matches!(b.stat(&missing), Err(BackendError::NotFound(_))));
    assert!(matches!(b.delete(&missing), Err(BackendError::NotFound(_))));
    for invalid in [
        "../config",
        "config/extra",
        "data/aa/no",
        "config.tmp-0123456789abcdef",
    ] {
        assert!(matches!(
            ObjectName::new(invalid),
            Err(BackendError::InvalidName(_))
        ));
    }
    let names = [
        name("config"),
        name(&format!("keys/{}", "11".repeat(32))),
        name(&format!("data/ab/{}", "ab".repeat(32))),
        name(&format!("index/{}", "22".repeat(32))),
        name(&format!("snapshots/{}", "33".repeat(32))),
        name(&format!("locks/{}", "44".repeat(32))),
    ];
    for object in &names {
        b.write(object, b"abcdef").unwrap();
        assert_eq!(b.read(object).unwrap(), b"abcdef");
        assert_eq!(b.stat(object).unwrap(), 6);
        for (offset, len, expected) in [
            (0, 3, &b"abc"[..]),
            (2, 2, &b"cd"[..]),
            (4, 20, &b"ef"[..]),
            (6, 1, &b""[..]),
            (99, 3, &b""[..]),
            (0, 0, &b""[..]),
            (u64::MAX, u64::MAX, &b""[..]),
            (2, u64::MAX, &b"cdef"[..]),
        ] {
            assert_eq!(b.read_range(object, offset, len).unwrap(), expected);
        }
        b.write(object, b"abcdef").unwrap();
        assert!(matches!(
            b.write(object, b"other"),
            Err(BackendError::AlreadyExists(_))
        ));
        assert_eq!(b.read(object).unwrap(), b"abcdef");
    }
    for kind in Kind::ALL {
        let object = names.iter().find(|n| n.kind() == kind).unwrap();
        assert_eq!(
            b.list(kind).unwrap(),
            vec![Entry {
                name: object.clone(),
                size: 6
            }]
        );
    }
    for object in &names {
        b.delete(object).unwrap();
        assert!(matches!(b.delete(object), Err(BackendError::NotFound(_))));
        b.write(object, b"").unwrap();
        assert_eq!(b.read(object).unwrap(), b"");
        assert_eq!(b.stat(object).unwrap(), 0);
        b.delete(object).unwrap();
    }
    // Different payloads expose check-then-rename races. A concurrent reader must
    // see either NotFound or one complete, immutable payload, never a prefix.
    let object = name("config");
    let barrier = Barrier::new(8);
    let done = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let reader = scope.spawn(|| {
            while !done.load(Ordering::Acquire) {
                match b.read(&object) {
                    Ok(bytes) => {
                        assert!(bytes.len() == 65536 && bytes.iter().all(|byte| *byte == bytes[0]))
                    }
                    Err(BackendError::NotFound(_)) => {}
                    Err(error) => panic!("reader: {error}"),
                }
            }
        });
        let writers: Vec<_> = (0..8_u8)
            .map(|id| {
                let object = &object;
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    let bytes = vec![id; 65536];
                    match b.write(object, &bytes) {
                        Ok(()) => Some(bytes),
                        Err(BackendError::AlreadyExists(_)) => None,
                        Err(error) => panic!("writer: {error}"),
                    }
                })
            })
            .collect();
        let results: Vec<_> = writers.into_iter().map(|writer| writer.join()).collect();
        done.store(true, Ordering::Release);
        reader.join().unwrap();
        let winners: Vec<_> = results
            .into_iter()
            .filter_map(|result| result.unwrap())
            .collect();
        assert_eq!(winners.len(), 1);
        assert_eq!(b.read(&object).unwrap(), winners[0]);
    });
    assert_eq!(
        b.list(Kind::Config).unwrap(),
        vec![Entry {
            name: object.clone(),
            size: 65536
        }]
    );
    b.delete(&object).unwrap();
    for kind in Kind::ALL {
        assert!(b.list(kind).unwrap().is_empty());
    }
}

#[test]
fn memory_conformance() {
    run_suite(&MemoryBackend::new());
}

#[test]
fn local_conformance() {
    let dir = tempfile::tempdir().unwrap();
    run_suite(&LocalBackend::new(dir.path()).unwrap());
}

#[test]
fn incomplete_uploads_and_unknown_names_are_invisible() {
    let dir = tempfile::tempdir().unwrap();
    let b = LocalBackend::new(dir.path()).unwrap();
    std::fs::write(dir.path().join("config.tmp-0123456789abcdef"), b"partial").unwrap();
    std::fs::write(dir.path().join("unknown"), b"junk").unwrap();
    let shard = dir.path().join("data/ab");
    std::fs::create_dir_all(&shard).unwrap();
    std::fs::write(
        shard.join(format!("{}.tmp-0123456789abcdef", "ab".repeat(32))),
        b"partial",
    )
    .unwrap();
    std::fs::write(shard.join("ab".repeat(32).to_uppercase()), b"junk").unwrap();
    std::fs::write(shard.join("ff".repeat(32)), b"wrong shard").unwrap();
    for kind in Kind::ALL {
        assert!(b.list(kind).unwrap().is_empty());
    }
    assert!(matches!(
        b.read(&name("config")),
        Err(BackendError::NotFound(_))
    ));
    b.write(&name("config"), b"complete").unwrap();
    assert_eq!(b.read(&name("config")).unwrap(), b"complete");
}

#[test]
fn independent_local_instances_do_not_replace_each_other() {
    let dir = tempfile::tempdir().unwrap();
    let barrier = Barrier::new(8);
    std::thread::scope(|scope| {
        let writers: Vec<_> = (0..8_u8)
            .map(|id| {
                let barrier = &barrier;
                let root = dir.path();
                scope.spawn(move || {
                    let backend = LocalBackend::new(root).unwrap();
                    barrier.wait();
                    backend.write(&name("config"), &[id])
                })
            })
            .collect();
        let results: Vec<_> = writers.into_iter().map(|w| w.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert!(
            results
                .iter()
                .all(|r| r.is_ok() || matches!(r, Err(BackendError::AlreadyExists(_))))
        );
    });
}

#[test]
fn local_handles_non_utf8_repository_paths() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let dir = tempfile::tempdir().unwrap();
    let root = dir
        .path()
        .join(OsString::from_vec(b"repository-\xff".to_vec()));
    run_suite(&LocalBackend::new(root).unwrap());
}

#[test]
fn listing_is_sorted_and_spans_data_shards() {
    fn check(b: &dyn Backend) {
        let a = name(&format!("data/aa/{}", "aa".repeat(32)));
        let z = name(&format!("data/ff/{}", "ff".repeat(32)));
        b.write(&z, b"last").unwrap();
        b.write(&a, b"first").unwrap();
        assert_eq!(
            b.list(Kind::Data).unwrap(),
            vec![Entry { name: a, size: 5 }, Entry { name: z, size: 4 }]
        );
    }
    check(&MemoryBackend::new());
    let dir = tempfile::tempdir().unwrap();
    check(&LocalBackend::new(dir.path()).unwrap());
}
