use keelhaul::backend::{Backend, BackendError, Kind, ObjectName};
use std::sync::{
    Barrier,
    atomic::{AtomicBool, Ordering},
};

pub fn name(s: &str) -> ObjectName {
    ObjectName::new(s).unwrap()
}

pub fn assert_entries(b: &dyn Backend, kind: Kind, expected: &[(ObjectName, u64)]) {
    let actual: Vec<_> = b
        .list(kind)
        .unwrap()
        .into_iter()
        .map(|entry| (entry.name, entry.size))
        .collect();
    assert_eq!(actual, expected);
}

pub fn run_suite(b: &dyn Backend, case_sensitive: bool, plant_raw: impl Fn(&str, &[u8])) {
    for kind in Kind::ALL {
        assert!(b.list(kind).unwrap().is_empty());
    }
    let missing = name(&format!("data/ff/{}", "ff".repeat(32)));
    assert!(matches!(b.read(&missing), Err(BackendError::NotFound(_))));
    assert!(matches!(
        b.read_range(&missing, 0, 1),
        Err(BackendError::NotFound(_))
    ));
    assert!(matches!(b.stat(&missing), Err(BackendError::NotFound(_))));
    b.delete(&missing).unwrap();
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
        assert_entries(b, kind, &[(object.clone(), 6)]);
    }
    for object in &names {
        b.delete(object).unwrap();
        b.delete(object).unwrap();
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
    assert_entries(b, Kind::Config, &[(object.clone(), 65536)]);
    b.delete(&object).unwrap();
    for kind in Kind::ALL {
        assert!(b.list(kind).unwrap().is_empty());
    }
    super::backend_edges::large_listing(b);
    super::backend_edges::large_ranges(b);
    super::backend_edges::foreign_entries(b, case_sensitive, &plant_raw);
    super::backend_edges::foreign_depth(b, plant_raw);
}
