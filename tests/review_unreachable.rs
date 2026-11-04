use keelhaul::{
    backend::{Backend, Kind, memory::MemoryBackend},
    format::pack::BlobKind,
    ops::{backup::backup, check::check, verify::verify},
    repo::{
        Repository,
        blobs::{BlobStore, pack_name},
    },
    secret::Secret,
};
#[test]
fn trees_of_deleted_snapshots_are_metadata_even_without_an_index() {
    for orphan in [false, true] {
        let source = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("file"), b"data").unwrap();
        let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([7; 32])).unwrap();
        backup(&repo, &[source.path().to_path_buf()], vec![]).unwrap();
        let (_, snapshot) = repo.select_snapshot("latest").unwrap();
        let store = BlobStore::load(&repo).unwrap();
        let loc = &store.index[&(BlobKind::Tree, snapshot.tree.clone())];
        let name = pack_name(&loc.pack).unwrap();
        let mut bytes = repo.backend.read(&name).unwrap();
        bytes[loc.blob.offset as usize + 24] ^= 1;
        repo.backend.plant_raw(name.as_str(), &bytes).unwrap();
        for e in repo.backend.list(Kind::Snapshots).unwrap() {
            repo.backend.delete(&e.name).unwrap();
        }
        if orphan {
            for e in repo.backend.list(Kind::Index).unwrap() {
                repo.backend.delete(&e.name).unwrap();
            }
        }
        for report in [check(&repo).unwrap(), verify(&repo, None).unwrap()] {
            assert!(
                report.damage.iter().any(|d| d.object == snapshot.tree
                    && d.kind == "blob"
                    && d.problem == "undecryptable"),
                "{report:?}"
            );
        }
    }
}
