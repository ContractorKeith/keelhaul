#[path = "common/m5_prune_fixture.rs"]
mod setup;
use keelhaul::{
    backend::{Backend, Kind, memory::MemoryBackend},
    crypto::envelope::Label,
    format::{index::Index, pack::BlobKind},
    ops::{prune::prune, verify::verify},
    repo::blobs::{BlobStore, pack_name},
};
#[test]
fn damaged_first_tree_copy_is_dropped_when_an_indexed_duplicate_verifies() {
    for bad_header in [false, true] {
        let (repo, snapshot, _) = setup::fixture(MemoryBackend::new());
        let (_, saved) = repo.select_snapshot(&snapshot).unwrap();
        let bytes = BlobStore::load(&repo)
            .unwrap()
            .read(BlobKind::Tree, &saved.tree)
            .unwrap();
        let mut writer = BlobStore::repack_writer(&repo).unwrap();
        writer.add(BlobKind::Tree, &bytes).unwrap();
        writer.finish().unwrap();
        let first = repo
            .backend
            .list(Kind::Index)
            .unwrap()
            .into_iter()
            .flat_map(|entry| {
                Index::parse(&repo.read_object(&entry.name, Label::Index).unwrap())
                    .unwrap()
                    .packs
            })
            .flat_map(|pack| {
                let id = pack.id;
                pack.blobs
                    .into_iter()
                    .filter(|blob| blob.kind == BlobKind::Tree)
                    .map(move |blob| (id.clone(), blob))
            })
            .find(|(_, blob)| blob.id == saved.tree)
            .unwrap();
        let bad = pack_name(&first.0).unwrap();
        let mut bytes = repo.backend.read(&bad).unwrap();
        let offset = if bad_header {
            let len = u32::from_le_bytes(bytes[bytes.len() - 4..].try_into().unwrap()) as usize;
            bytes.len() - 4 - len + 24
        } else {
            first.1.offset as usize + 24
        };
        bytes[offset] ^= 1;
        repo.backend.plant_raw(bad.as_str(), &bytes).unwrap();
        prune(&repo, 20, false).unwrap();
        assert!(repo.backend.stat(&bad).is_err());
        assert!(verify(&repo, None).unwrap().damage.is_empty());
    }
}
