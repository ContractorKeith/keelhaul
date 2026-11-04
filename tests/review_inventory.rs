use keelhaul::{
    backend::{Backend, Kind, local::LocalBackend},
    ops::{
        check::check,
        verify::{Subset, verify},
    },
    repo::Repository,
    secret::Secret,
};
#[test]
fn foreign_data_files_object_named_directories_and_partial_uploads() {
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("repo");
    let repo =
        Repository::init(LocalBackend::new(&path).unwrap(), &Secret::KeyFile([7; 32])).unwrap();
    for directory in [
        format!("index/{}", "ab".repeat(32)),
        format!("snapshots/{}", "cd".repeat(32)),
        format!("data/ef/{}", "ef".repeat(32)),
    ] {
        std::fs::create_dir_all(path.join(directory)).unwrap();
    }
    std::fs::write(path.join("data/unknown"), b"foreign").unwrap();
    std::fs::write(path.join("data/ab"), b"not a shard directory").unwrap();
    for temp in [
        "config.tmp-0123456789abcdef",
        "data/random.tmp-0123456789abcdef",
    ] {
        std::fs::write(path.join(temp), b"partial").unwrap();
    }
    std::fs::create_dir_all(path.join("index")).unwrap();
    let zero = format!("index/{}", "00".repeat(32));
    std::fs::write(path.join(&zero), []).unwrap();
    for report in [check(&repo).unwrap(), verify(&repo, None).unwrap()] {
        assert_eq!(report.damage.len(), 1, "{report:?}");
        assert_eq!(
            (&*report.damage[0].object, &*report.damage[0].problem),
            (&*zero, "hash-mismatch")
        );
        let names = report
            .notes
            .iter()
            .map(|n| n.object.clone())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            names,
            [
                "data/unknown".into(),
                "data/ab".into(),
                format!("index/{}", "ab".repeat(32)),
                format!("snapshots/{}", "cd".repeat(32)),
                format!("data/ef/{}", "ef".repeat(32))
            ]
            .into_iter()
            .collect()
        );
        assert!(report.notes.iter().all(|n| n.problem == "unknown-object"));
    }
    assert!(repo.backend.list(Kind::Data).unwrap().is_empty());
}
#[test]
fn malformed_ids_belong_to_subset_one_exactly_once() {
    for id in [
        "".to_owned(),
        "abc".into(),
        "g".repeat(64),
        format!("{}g", "0".repeat(63)),
        "0".repeat(65),
    ] {
        let selected = (1..=3)
            .filter(|n| format!("{n}/3").parse::<Subset>().unwrap().contains(&id))
            .collect::<Vec<_>>();
        assert_eq!(selected, vec![1], "{id}");
    }
}
