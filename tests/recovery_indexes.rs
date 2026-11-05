use keelhaul::{
    backend::{Backend, Kind, memory::MemoryBackend},
    ops::{
        backup::backup, check::check, listing::ls, prune::prune, rebuild_index::rebuild,
        restore::restore,
    },
    repo::Repository,
    secret::Secret,
};
#[test]
fn damaged_deleted_and_foreign_indexes_recover_and_rebuild() {
    for mutation in ["flip", "delete", "foreign"] {
        let dir = tempfile::tempdir().unwrap();
        let source = std::fs::canonicalize(dir.path()).unwrap().join("source");
        std::fs::create_dir(&source).unwrap();
        for n in 0..3 {
            std::fs::write(source.join(format!("file{n}")), format!("contents {n}")).unwrap();
        }
        let secret = Secret::KeyFile([7; 32]);
        let repo = Repository::init(MemoryBackend::new(), &secret).unwrap();
        let snap = backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
        let name = repo.backend.list(Kind::Index).unwrap()[0].name.clone();
        match mutation {
            "flip" => {
                let mut bytes = repo.backend.read(&name).unwrap();
                *bytes.last_mut().unwrap() ^= 1;
                repo.backend.plant_raw(name.as_str(), &bytes).unwrap();
            }
            "delete" => repo.backend.delete(&name).unwrap(),
            _ => {
                let foreign = Repository::init(MemoryBackend::new(), &secret).unwrap();
                backup(&foreign, std::slice::from_ref(&source), vec![]).unwrap();
                let other = &foreign.backend.list(Kind::Index).unwrap()[0].name;
                repo.backend
                    .plant_raw(other.as_str(), &foreign.backend.read(other).unwrap())
                    .unwrap();
            }
        }
        let target = dir.path().join("out");
        let report = restore(&repo, &snap.snapshot, &target, &[]).unwrap();
        assert!(report.failed.is_empty(), "{report:?}");
        assert_eq!(report.damaged.is_empty(), mutation == "delete");
        for n in 0..3 {
            let path = source.join(format!("file{n}"));
            assert_eq!(
                std::fs::read(&path).unwrap(),
                std::fs::read(target.join(path.strip_prefix("/").unwrap())).unwrap()
            );
        }
        assert!(!ls(&repo, "latest", None).unwrap().is_empty());
        let before = repo.backend.list(Kind::Index).unwrap();
        rebuild(&repo, true).unwrap();
        assert_eq!(before, repo.backend.list(Kind::Index).unwrap());
        rebuild(&repo, false).unwrap();
        assert!(check(&repo).unwrap().damage.is_empty());
        prune(&repo, 20, false).unwrap();
        assert!(check(&repo).unwrap().damage.is_empty());
    }
}

#[test]
fn swapped_pack_bytes_are_named_by_structural_check() {
    let dir = tempfile::tempdir().unwrap();
    let source = std::fs::canonicalize(dir.path()).unwrap().join("file");
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([7; 32])).unwrap();
    for bytes in [b"first", b"other"] {
        std::fs::write(&source, bytes).unwrap();
        backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
    }
    let packs = repo.backend.list(Kind::Data).unwrap();
    assert_eq!(packs.len(), 2);
    let a = repo.backend.read(&packs[0].name).unwrap();
    let b = repo.backend.read(&packs[1].name).unwrap();
    repo.backend.plant_raw(packs[0].name.as_str(), &b).unwrap();
    repo.backend.plant_raw(packs[1].name.as_str(), &a).unwrap();
    let report = check(&repo).unwrap();
    for entry in packs {
        assert!(
            report
                .damage
                .iter()
                .any(|d| d.object == entry.name.as_str() && d.problem == "hash-mismatch"),
            "{report:?}"
        );
    }
}

#[test]
fn each_index_corruption_preserves_every_snapshot() {
    let scratch = tempfile::tempdir().unwrap();
    let source = std::fs::canonicalize(scratch.path()).unwrap().join("file");
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([7; 32])).unwrap();
    let mut snapshots = Vec::new();
    for text in ["first snapshot bytes", "second snapshot bytes"] {
        std::fs::write(&source, text).unwrap();
        let snapshot = backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
        snapshots.push((snapshot.snapshot, text));
    }
    let indexes = repo.backend.list(Kind::Index).unwrap();
    assert_eq!(indexes.len(), 2);
    for entry in indexes {
        let original = repo.backend.read(&entry.name).unwrap();
        let mut damaged = original.clone();
        *damaged.last_mut().unwrap() ^= 1;
        repo.backend
            .plant_raw(entry.name.as_str(), &damaged)
            .unwrap();
        for (snapshot, text) in &snapshots {
            let target = tempfile::tempdir().unwrap();
            let report = restore(&repo, snapshot, target.path(), &[]).unwrap();
            assert!(report.failed.is_empty(), "{report:?}");
            assert!(
                report
                    .damaged
                    .iter()
                    .any(|d| d.object == entry.name.as_str())
            );
            assert_eq!(
                std::fs::read(target.path().join(source.strip_prefix("/").unwrap())).unwrap(),
                text.as_bytes()
            );
        }
        repo.backend
            .plant_raw(entry.name.as_str(), &original)
            .unwrap();
    }
}
