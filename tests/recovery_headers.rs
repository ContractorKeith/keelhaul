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
    }
}
