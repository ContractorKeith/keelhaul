//! Seeded damage proof. Expected rules:
//! - Every mutated config/key/index/snapshot is named; metadata hash failures stop
//!   parsing that object. Deleted packs are named plus every indexed blob they held.
//! - Missing/corrupt indexes make reachable root trees dangling; their descendants
//!   cannot be enumerated and are not guessed. A broken root likewise hides its files.
//! - A body flip names its blob; verify additionally names the pack and each reachable
//!   file using a failed data blob. Check never reads data ciphertext.
//! - Header/trailer damage names the pack; intact indexed blobs remain readable.
//! - Removing an unreferenced snapshot/key/index is indistinguishable from legitimate
//!   removal under SPEC, so deletion injections target config and indexed packs.
//!
//! Each seed exercises every category plus simultaneous independent config/slot faults.
#[path = "common/backup_fixture.rs"]
#[allow(dead_code, reason = "shared round-trip fixture helpers")]
mod backup_fixture;
#[path = "damage_support/mod.rs"]
mod support;
use keelhaul::{
    backend::{Backend, ObjectName},
    ops::{check::check_backend, verify::verify_backend},
};
use support::{Fixture, Objects, objects};
fn set(items: impl IntoIterator<Item = String>) -> Objects {
    items.into_iter().collect()
}
fn run(f: &Fixture, seed: u64, label: &str, check: Objects, verify: Objects) {
    let c = check_backend(&f.repo.backend, &f.key)
        .unwrap_or_else(|e| panic!("seed {seed}, corruption {label}: {e}"));
    let v = verify_backend(&f.repo.backend, &f.key, None)
        .unwrap_or_else(|e| panic!("seed {seed}, corruption {label}: {e}"));
    assert_eq!(
        objects(&c),
        check,
        "seed {seed}, corruption {label}; check: {c:?}"
    );
    assert_eq!(
        objects(&v),
        verify,
        "seed {seed}, corruption {label}; verify: {v:?}"
    );
}
#[test]
fn seeded_damage_exact_sets() {
    let scratch = tempfile::tempdir().unwrap();
    let source = std::fs::canonicalize(scratch.path())
        .unwrap()
        .join("source");
    backup_fixture::damage_fixture(&source);
    let f = Fixture::new(&source);
    run(&f, 0, "clean", Objects::new(), Objects::new());
    let count = if std::env::var_os("KEELHAUL_SOAK").is_some() {
        200
    } else {
        20
    };
    let seeds = match std::env::var("KEELHAUL_SEED") {
        Ok(s) => vec![s.parse::<u64>().unwrap()],
        Err(_) => (0..count).collect(),
    };
    let seeds_run = seeds.len();
    for seed in seeds {
        let mut rng = blake3::Hasher::new();
        rng.update(&seed.to_le_bytes());
        let mut random = [0; 32];
        rng.finalize_xof().fill(&mut random);
        let flips = 1 + random[0] as usize % 8;
        for (name, offset, id, is_tree) in [
            (&f.data.0, f.data.1, &f.data.2, false),
            (&f.tree.0, f.tree.1, &f.tree.2, true),
        ] {
            f.reset();
            let mut bytes = f.bytes(name);
            for i in 0..flips {
                bytes[offset as usize + 24 + i] ^= random[i + 1] | 1;
            }
            f.plant(name, &bytes);
            let c = set([id.clone()])
                .into_iter()
                .filter(|_| is_tree)
                .collect::<Objects>();
            let mut v = set([name.clone(), id.clone()]);
            if !is_tree {
                v.extend(f.files_using(id));
            }
            run(
                &f,
                seed,
                &format!("flip {flips} body bytes of {name}, blob {id}"),
                c,
                v,
            );
        }
        // First encrypted header byte; or remove part of its envelope, leaving all blobs.
        f.reset();
        let (name, start) = &f.header;
        let mut bytes = f.bytes(name);
        if random[2] % 2 == 0 {
            bytes[*start as usize + 24] ^= 1;
        } else {
            bytes.truncate(*start as usize + 10);
        }
        f.plant(name, &bytes);
        run(
            &f,
            seed,
            &format!("header flip/truncation {name}"),
            set([name.clone()]),
            set([name.clone()]),
        );
        // Trailer destruction and append-junk exercise invalid lengths and shifted tails.
        f.reset();
        let name = &f.trailer;
        let mut bytes = f.bytes(name);
        if random[3] % 2 == 0 {
            let end = bytes.len();
            bytes[end - 1] ^= 128;
        } else {
            bytes.extend(b"junk");
        }
        f.plant(name, &bytes);
        run(
            &f,
            seed,
            &format!("trailer flip/junk {name}"),
            set([name.clone()]),
            set([name.clone()]),
        );
        // Hash-addressed metadata damage; an invalid index makes the roots dangling.
        for name in [&f.index, &f.snapshot] {
            f.reset();
            let mut bytes = f.bytes(name);
            match random[4] % 3 {
                0 => bytes[24] ^= 1,
                1 => bytes.truncate(bytes.len() / 2),
                _ => bytes.extend(b"junk"),
            }
            f.plant(name, &bytes);
            let mut expected = set([name.clone()]);
            if name == &f.index {
                expected.extend(f.roots.iter().cloned());
            }
            run(
                &f,
                seed,
                &format!("metadata flip/truncate/junk {name}"),
                expected.clone(),
                expected,
            );
        }
        // N=2 independent faults, with every mutation family including a two-object swap.
        f.reset();
        let mut config = f.bytes("config");
        let mut slot = f.bytes(&f.slot);
        match random[5] % 4 {
            0 => {
                config[24] ^= 1;
                slot[0] ^= 1;
            }
            1 => {
                config.truncate(12);
                slot.truncate(12);
            }
            2 => {
                config.extend(b"junk");
                slot.extend(b"junk");
            }
            _ => std::mem::swap(&mut config, &mut slot),
        }
        f.plant("config", &config);
        f.plant(&f.slot, &slot);
        run(
            &f,
            seed,
            "config + non-opening slot flip/truncate/junk/swap",
            set(["config".into(), f.slot.clone()]),
            set(["config".into(), f.slot.clone()]),
        );
        f.reset();
        f.repo
            .backend
            .delete(&ObjectName::new("config").unwrap())
            .unwrap();
        run(
            &f,
            seed,
            "delete config",
            set(["config".into()]),
            set(["config".into()]),
        );
        // Delete a data-only pack: all indexed blobs are dangling, all their files affected.
        f.reset();
        let name = &f.header.0;
        let store = keelhaul::repo::blobs::BlobStore::load(&f.repo).unwrap();
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
        let mut c = set([name.clone()]);
        c.extend(ids.clone());
        let mut v = c.clone();
        for id in ids {
            v.extend(f.files_using(&id));
        }
        run(&f, seed, &format!("delete indexed pack {name}"), c, v);
        // A seeded N-object batch checks that collecting one failure does not hide others.
        f.reset();
        let mut c = Objects::new();
        let mut v = Objects::new();
        let mut corruptions = Vec::new();
        for name in ["config", f.slot.as_str(), f.snapshot.as_str()] {
            if name == "config" || random[6] & if name == f.slot { 1 } else { 2 } != 0 {
                let mut bytes = f.bytes(name);
                bytes[0] ^= 1;
                f.plant(name, &bytes);
                c.insert(name.into());
                v.insert(name.into());
                corruptions.push(name);
            }
        }
        let mut bytes = f.bytes(&f.data.0);
        bytes[f.data.1 as usize + 24] ^= 1;
        f.plant(&f.data.0, &bytes);
        v.insert(f.data.0.clone());
        v.insert(f.data.2.clone());
        v.extend(
            f.files_using(&f.data.2)
                .into_iter()
                .filter(|path| !c.contains(&f.snapshot) || !path.starts_with(&f.snapshot)),
        );
        run(
            &f,
            seed,
            &format!("batch {corruptions:?} + data blob {}", f.data.2),
            c,
            v,
        );
    }
    f.reset();
    run(&f, 0, "clean after reset", Objects::new(), Objects::new());
    println!("damage: {seeds_run} seeds green (KEELHAUL_SEED selects one)");
}
