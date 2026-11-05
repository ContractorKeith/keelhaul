//! Oracle rules: AEAD body flips => blob undecryptable; only verify hashes packs.
//! Invalid metadata ciphertext => hash-mismatch, except config (no hash name).
//! A truncated envelope => bad-format; absent config/pack => missing. Indexed
//! blobs without any available pack => dangling-reference, plus referring files
//! in verify. Unreadable roots/indexes hide descendants; no descendant is guessed.
use super::*;
use base64::{Engine, prelude::BASE64_STANDARD};
use keelhaul::backend::{Backend, Kind, ObjectName};
pub fn run_cases(f: &Fixture, seed: u64, r: [u8; 32]) {
    let flips = 1 + r[0] as usize % 8;
    for (name, offset, id, tree) in [
        (&f.data.0, f.data.1, &f.data.2, false),
        (&f.tree.0, f.tree.1, &f.tree.2, true),
    ] {
        f.reset();
        let mut bytes = f.bytes(name);
        for i in 0..flips {
            bytes[offset as usize + 24 + i] ^= r[i + 1] | 1;
        }
        f.plant(name, &bytes);
        let c = if tree {
            expected([signature(id, "blob", "undecryptable")])
        } else {
            Expected::new()
        };
        let mut v = expected([
            signature(name, "pack", "hash-mismatch"),
            signature(id, "blob", "undecryptable"),
        ]);
        if !tree {
            v.extend(file_errors(f, id));
        }
        run(
            f,
            seed,
            &format!("flip {flips} bytes in {name}, blob {id}"),
            c,
            v,
        );
    }
    // A deleted snapshot's root remains metadata, even when nothing references it.
    f.reset();
    for e in f.repo.backend.list(Kind::Snapshots).unwrap() {
        f.repo.backend.delete(&e.name).unwrap();
    }
    let mut bytes = f.bytes(&f.tree.0);
    bytes[f.tree.1 as usize + 24] ^= 1;
    f.plant(&f.tree.0, &bytes);
    run(
        f,
        seed,
        "unreachable tree of deleted snapshots",
        expected([signature(&f.tree.2, "blob", "undecryptable")]),
        expected([
            signature(&f.tree.2, "blob", "undecryptable"),
            signature(&f.tree.0, "pack", "hash-mismatch"),
        ]),
    );
    f.reset();
    let (name, start) = &f.header;
    let mut bytes = f.bytes(name);
    let mut c = expected([signature(name, "pack", "bad-header")]);
    if r[2].is_multiple_of(2) {
        bytes[*start as usize + 24] ^= 1;
    } else {
        bytes.truncate(*start as usize + 10);
        let end = bytes.len();
        bytes[end - 4..].fill(255);
        c.insert(signature(name, "pack", "truncated"));
    }
    f.plant(name, &bytes);
    let mut v = c.clone();
    v.insert(signature(name, "pack", "hash-mismatch"));
    run(f, seed, "header flip/truncate", c, v);
    assert_all_restored(f, seed);
    f.reset();
    let name = &f.trailer;
    let mut bytes = f.bytes(name);
    if r[3].is_multiple_of(2) {
        let end = bytes.len();
        bytes[end - 1] ^= 128;
    } else {
        bytes.extend(b"junk");
    }
    f.plant(name, &bytes);
    run(
        f,
        seed,
        "trailer flip/junk",
        expected([signature(name, "pack", "bad-header")]),
        expected([
            signature(name, "pack", "bad-header"),
            signature(name, "pack", "hash-mismatch"),
        ]),
    );
    assert_all_restored(f, seed);
    for name in [&f.index, &f.snapshot] {
        f.reset();
        let mut bytes = f.bytes(name);
        match r[4] % 3 {
            0 => bytes[24] ^= 1,
            1 => bytes.truncate(bytes.len() / 2),
            _ => bytes.extend(b"junk"),
        };
        f.plant(name, &bytes);
        let e = expected([signature(
            name,
            if name == &f.index {
                "index"
            } else {
                "snapshot"
            },
            "hash-mismatch",
        )]);
        run(
            f,
            seed,
            &format!("metadata flip/truncate/junk {name}"),
            e.clone(),
            e,
        );
        if name == &f.index {
            for (id, _) in f.repo.snapshots().unwrap() {
                let scratch = tempfile::tempdir().unwrap();
                let restored =
                    keelhaul::ops::restore::restore(&f.repo, &id, scratch.path(), &[]).unwrap();
                assert!(restored.failed.is_empty(), "seed {seed}: {restored:?}");
                for file in f
                    .files
                    .iter()
                    .filter(|file| file.path.starts_with(&format!("snapshots/{id}:")))
                {
                    let path = file.path.split_once(':').unwrap().1;
                    assert_eq!(
                        std::fs::read(path).unwrap(),
                        std::fs::read(scratch.path().join(path.trim_start_matches('/'))).unwrap(),
                        "seed {seed}: {path}"
                    );
                }
            }
        }
    }
    f.reset();
    let mut config = f.bytes("config");
    let mut slot = f.bytes(&f.slot);
    let config_problem = match r[5] % 4 {
        0 => {
            config[24] ^= 1;
            slot[0] ^= 1;
            "undecryptable"
        }
        1 => {
            config.truncate(12);
            slot.truncate(12);
            "bad-format"
        }
        2 => {
            config.extend(b"junk");
            slot.extend(b"junk");
            "undecryptable"
        }
        _ => {
            std::mem::swap(&mut config, &mut slot);
            "undecryptable"
        }
    };
    f.plant("config", &config);
    f.plant(&f.slot, &slot);
    let e = expected([
        signature("config", "config", config_problem),
        signature(&f.slot, "key", "bad-format"),
    ]);
    run(
        f,
        seed,
        "config + slot flip/truncate/junk/swap",
        e.clone(),
        e,
    );
    // Slot check authenticates the wrapped tag and KDF costs without that secret.
    for cost in [false, true] {
        f.reset();
        let mut value: serde_json::Value = serde_json::from_slice(&f.bytes(&f.slot)).unwrap();
        if cost {
            value["kdf"]["m_kib"] = 65537.into();
        } else {
            let mut bytes = BASE64_STANDARD
                .decode(value["wrapped"].as_str().unwrap())
                .unwrap();
            *bytes.last_mut().unwrap() ^= 1;
            value["wrapped"] = BASE64_STANDARD.encode(bytes).into();
        }
        f.plant(&f.slot, &serde_json::to_vec(&value).unwrap());
        let e = expected([signature(&f.slot, "key", "hash-mismatch")]);
        run(
            f,
            seed,
            if cost {
                "edit non-opening slot m_kib"
            } else {
                "flip non-opening slot wrapped tag"
            },
            e.clone(),
            e,
        );
    }
    f.reset();
    f.repo
        .backend
        .delete(&ObjectName::new("config").unwrap())
        .unwrap();
    let e = expected([signature("config", "config", "missing")]);
    run(f, seed, "delete config", e.clone(), e);
    delete_indexed_pack(f, seed, &f.header.0);
    f.reset();
    let mut c = Expected::new();
    let mut names = Vec::new();
    for (name, kind, problem) in [
        ("config", "config", "undecryptable"),
        (f.slot.as_str(), "key", "bad-format"),
        (f.snapshot.as_str(), "snapshot", "hash-mismatch"),
    ] {
        if name == "config" || r[6] & if name == f.slot { 1 } else { 2 } != 0 {
            let mut b = f.bytes(name);
            b[0] ^= 1;
            f.plant(name, &b);
            c.insert(signature(name, kind, problem));
            names.push(name);
        }
    }
    let mut b = f.bytes(&f.data.0);
    b[f.data.1 as usize + 24] ^= 1;
    f.plant(&f.data.0, &b);
    let mut v = c.clone();
    v.insert(signature(&f.data.0, "pack", "hash-mismatch"));
    v.insert(signature(&f.data.2, "blob", "undecryptable"));
    v.extend(
        file_errors(f, &f.data.2).into_iter().filter(|(p, _, _)| {
            !names.contains(&f.snapshot.as_str()) || !p.starts_with(&f.snapshot)
        }),
    );
    run(
        f,
        seed,
        &format!("batch {names:?} + data blob {}", f.data.2),
        c,
        v,
    );
}

/// Deleted metadata hides descendants even when their chunks also occur elsewhere.
pub fn delete_indexed_pack(f: &Fixture, seed: u64, name: &str) {
    f.reset();
    let store = BlobStore::load(&f.repo).unwrap();
    let ids = store
        .index
        .values()
        .filter(|l| keelhaul::repo::blobs::pack_name(&l.pack).unwrap().as_str() == name)
        .map(|l| l.blob.id.clone())
        .collect::<Vec<_>>();
    f.repo
        .backend
        .delete(&ObjectName::new(name).unwrap())
        .unwrap();
    let mut c = expected([signature(name, "pack", "missing")]);
    c.extend(
        ids.iter()
            .map(|id| signature(id, "blob", "dangling-reference")),
    );
    let mut v = c.clone();
    v.extend(
        f.files
            .iter()
            .filter(|file| {
                !file.tree_packs.contains(name) && file.chunks.iter().any(|id| ids.contains(id))
            })
            .map(|file| signature(&file.path, "file", "dangling-reference")),
    );
    run(f, seed, "delete indexed pack", c, v);
}

fn assert_all_restored(f: &Fixture, seed: u64) {
    for (id, _) in f.repo.snapshots().unwrap() {
        let target = tempfile::tempdir().unwrap();
        let report = keelhaul::ops::restore::restore(&f.repo, &id, target.path(), &[]).unwrap();
        assert!(report.failed.is_empty(), "seed {seed}: {report:?}");
        for file in f
            .files
            .iter()
            .filter(|f| f.path.starts_with(&format!("snapshots/{id}:")))
        {
            let path = file.path.split_once(':').unwrap().1;
            assert_eq!(
                std::fs::read(path).unwrap(),
                std::fs::read(target.path().join(path.trim_start_matches('/'))).unwrap(),
                "seed {seed}: {path}"
            );
        }
    }
}
