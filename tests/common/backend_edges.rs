use super::backend_suite::{assert_entries, name};
use keelhaul::backend::{Backend, BackendError, Kind, ObjectName};

pub fn large_listing(b: &dyn Backend) {
    // S3's default ListObjectsV2 page limit is 1000. Write in reverse order so
    // insertion ordering cannot accidentally satisfy the sorted-list contract.
    let expected: Vec<_> = (0..1003)
        .map(|id| (name(&format!("index/{id:064x}")), 1))
        .collect();
    for (object, _) in expected.iter().rev() {
        b.write(object, b"x").unwrap();
    }
    assert_eq!(b.list(Kind::Index).unwrap().len(), 1003);
    assert_entries(b, Kind::Index, &expected);
    for (object, _) in expected {
        b.delete(&object).unwrap();
    }
    assert!(b.list(Kind::Index).unwrap().is_empty());
    // Exercise both shard traversal and lexicographic ordering.
    let a = name(&format!("data/aa/{}", "aa".repeat(32)));
    let z = name(&format!("data/ff/{}", "ff".repeat(32)));
    b.write(&z, b"last").unwrap();
    b.write(&a, b"first").unwrap();
    assert_entries(b, Kind::Data, &[(a.clone(), 5), (z.clone(), 4)]);
    b.delete(&a).unwrap();
    b.delete(&z).unwrap();
}

pub fn large_ranges(b: &dyn Backend) {
    let object = name(&format!("data/ab/{}", "ab".repeat(32)));
    let bytes: Vec<u8> = (0..256 * 1024_u32)
        .map(|i| ((i * 31 + i / 251) % 256) as u8)
        .collect();
    b.write(&object, &bytes).unwrap();
    for (offset, len) in [
        (32760, 40),
        (65530, 80),
        (1024, 140000),
        (131072, 131072),
        (262100, 1000),
        (262144, 20),
        (262145, 10),
    ] {
        let start = offset.min(bytes.len());
        let end = (offset + len).min(bytes.len());
        assert_eq!(
            b.read_range(&object, offset as u64, len as u64).unwrap(),
            bytes[start..end],
            "offset={offset}, len={len}"
        );
    }
    // Same-size conflicting writes must still compare bytes.
    let other = vec![0; bytes.len()];
    assert!(matches!(
        b.write(&object, &other),
        Err(BackendError::AlreadyExists(_))
    ));
    b.write(&object, &bytes).unwrap();
    assert_eq!(b.read(&object).unwrap(), bytes);
    b.delete(&object).unwrap();
}

pub fn foreign_entries(b: &dyn Backend, case_sensitive: bool, plant_raw: impl Fn(&str, &[u8])) {
    let id = "ab".repeat(32);
    for (kind, base, directory) in [
        (Kind::Config, "config".to_owned(), ""),
        (Kind::Keys, format!("keys/{id}"), "keys/"),
        (Kind::Data, format!("data/ab/{id}"), "data/ab/"),
        (Kind::Index, format!("index/{id}"), "index/"),
        (Kind::Snapshots, format!("snapshots/{id}"), "snapshots/"),
        (Kind::Locks, format!("locks/{id}"), "locks/"),
    ] {
        assert!(b.list_foreign(kind).unwrap().is_empty());
        let mut raw_names = vec![
            format!("{base}.tmp-0123456789abcdef"),
            format!("{directory}unknown"),
        ];
        if kind == Kind::Data {
            raw_names.push(format!("data/ff/{id}")); // wrong shard
            // On case-folding filesystems this aliases the valid lowercase ID.
            // Only this fixture is omitted; memory and case-sensitive storage keep it.
            if case_sensitive {
                raw_names.push(format!("data/ab/{}", id.to_uppercase()));
            }
        }
        for raw in &raw_names {
            plant_raw(raw, b"partial");
            // The typed read API refuses foreign names before any backend I/O.
            assert!(matches!(
                ObjectName::new(raw),
                Err(BackendError::InvalidName(_))
            ));
        }
        assert!(b.list(kind).unwrap().is_empty());
        assert!(matches!(
            b.read(&name(&base)),
            Err(BackendError::NotFound(_))
        ));
        raw_names.sort();
        let foreign = b.list_foreign(kind).unwrap();
        assert_eq!(foreign.len(), raw_names.len());
        assert_eq!(
            foreign
                .iter()
                .map(|entry| entry.raw.as_str())
                .collect::<Vec<_>>(),
            raw_names
        );
        assert!(
            foreign
                .iter()
                .all(|entry| entry.size == 7 && entry.deletable)
        );
        // Valid and foreign entries remain disjoint even when both exist.
        b.write(&name(&base), b"complete").unwrap();
        assert_entries(b, kind, &[(name(&base), 8)]);
        assert_eq!(b.list_foreign(kind).unwrap(), foreign);
        for raw in raw_names {
            b.delete_foreign(&raw).unwrap();
            b.delete_foreign(&raw).unwrap();
        }
        assert!(b.list_foreign(kind).unwrap().is_empty());
        b.delete(&name(&base)).unwrap();
    }
    // Refuse traversal, absolute paths, unknown directories, and valid objects.
    for raw in [
        "",
        "/config.tmp-0123456789abcdef",
        "../outside",
        "keys/../../outside",
        "data/ab/../outside",
        "keys/a..b",
        "keys//junk",
        "keys/./junk",
        "keys/sub/config",
        "index/sub/junk",
        "data/ab/sub/junk",
        "data/ZZ/junk",
        "data/ab",
        "keys/\u{fffd}",
        "other/junk",
        "keys/\\junk",
        "keys/\0junk",
        "config",
    ] {
        assert!(
            matches!(b.delete_foreign(raw), Err(BackendError::InvalidName(_))),
            "{raw:?}"
        );
    }
    assert!(matches!(
        b.delete_foreign(&format!("keys/{id}")),
        Err(BackendError::InvalidName(_))
    ));
    for kind in Kind::ALL {
        assert!(b.list(kind).unwrap().is_empty());
        assert!(b.list_foreign(kind).unwrap().is_empty());
    }
}

// Nested files must neither be traversed nor accepted for raw deletion.
pub fn foreign_depth(b: &dyn Backend, plant_raw: impl Fn(&str, &[u8])) {
    let config = name("config");
    b.write(&config, b"preserve").unwrap();
    for raw in [
        "keys/sub/config",
        "index/sub/config",
        "snapshots/sub/config",
        "locks/sub/config",
        "data/ab/sub/config",
    ] {
        plant_raw(raw, b"nested");
        assert!(matches!(
            b.delete_foreign(raw),
            Err(BackendError::InvalidName(_))
        ));
    }
    for kind in Kind::ALL {
        assert!(b.list_foreign(kind).unwrap().is_empty());
    }
    assert_eq!(b.read(&config).unwrap(), b"preserve");
    b.delete(&config).unwrap();
}
