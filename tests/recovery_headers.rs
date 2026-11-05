use keelhaul::{
    backend::{Backend, Kind, memory::MemoryBackend},
    ops::{backup::backup, listing::ls_report_for_host, restore::restore},
    repo::Repository,
    secret::Secret,
};

#[test]
fn indexed_blobs_survive_header_and_trailer_corruption() {
    for trailer in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let source = std::fs::canonicalize(dir.path()).unwrap().join("file");
        std::fs::write(&source, b"recover every byte").unwrap();
        let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([7; 32])).unwrap();
        let snap = backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
        let name = repo.backend.list(Kind::Data).unwrap()[0].name.clone();
        let mut bytes = repo.backend.read(&name).unwrap();
        let len = bytes.len();
        let header = u32::from_le_bytes(bytes[len - 4..].try_into().unwrap()) as usize;
        let offset = if trailer {
            len - 1
        } else {
            len - 4 - header + 24
        };
        bytes[offset] ^= 128;
        repo.backend.plant_raw(name.as_str(), &bytes).unwrap();
        let target = dir.path().join("out");
        let report = restore(&repo, &snap.snapshot, &target, &[]).unwrap();
        assert!(report.failed.is_empty(), "{report:?}");
        assert_eq!(report.damaged.len(), 1, "{report:?}");
        assert_eq!(report.damaged[0].problem, "bad-header");
        assert_eq!(
            std::fs::read(target.join(source.strip_prefix("/").unwrap())).unwrap(),
            b"recover every byte"
        );
        let listed = ls_report_for_host(&repo, "latest", None, None).unwrap();
        assert_eq!(listed.damage[0].problem, "bad-header");
        let healed = keelhaul::ops::backup::backup_with_options(
            &repo,
            std::slice::from_ref(&source),
            vec![],
            &keelhaul::ops::backup::Options {
                heal: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            healed.summary.data_blobs_added, 0,
            "intact indexed blobs should remain reusable"
        );
    }
}

#[test]
fn rebuild_indexes_readable_packs_and_preserves_cached_damaged_headers() {
    for foreign in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let source = std::fs::canonicalize(dir.path()).unwrap().join("file");
        std::fs::write(&source, b"recover every byte").unwrap();
        let secret = Secret::KeyFile([7; 32]);
        let repo = Repository::init(MemoryBackend::new(), &secret).unwrap();
        let snap = backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
        let name = if foreign {
            let other = Repository::init(MemoryBackend::new(), &secret).unwrap();
            backup(&other, std::slice::from_ref(&source), vec![]).unwrap();
            let name = other.backend.list(Kind::Data).unwrap()[0].name.clone();
            repo.backend
                .plant_raw(name.as_str(), &other.backend.read(&name).unwrap())
                .unwrap();
            name
        } else {
            let name = repo.backend.list(Kind::Data).unwrap()[0].name.clone();
            let mut bytes = repo.backend.read(&name).unwrap();
            *bytes.last_mut().unwrap() ^= 128;
            repo.backend.plant_raw(name.as_str(), &bytes).unwrap();
            name
        };
        for dry in [true, false] {
            let report = keelhaul::ops::rebuild_index::rebuild(&repo, dry).unwrap();
            assert_eq!(report.packs, 1);
            assert_eq!(report.damage.len(), 1);
            assert_eq!(report.damage[0].object, name.as_str());
        }
        let target = dir.path().join("out");
        let report = restore(&repo, &snap.snapshot, &target, &[]).unwrap();
        assert!(report.failed.is_empty());
        assert_eq!(
            std::fs::read(target.join(source.strip_prefix("/").unwrap())).unwrap(),
            b"recover every byte"
        );
        if foreign {
            assert!(
                keelhaul::ops::check::check(&repo)
                    .unwrap()
                    .notes
                    .iter()
                    .any(|n| n.problem.contains("remove it by hand"))
            );
        }
    }
}
