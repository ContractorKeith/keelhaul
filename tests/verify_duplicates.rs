//! Resolve files through live index locations, including duplicates and subsets.
use keelhaul::{
    backend::{Backend, Kind, memory::MemoryBackend},
    crypto::{
        envelope::{self, Label},
        slot,
    },
    format::{
        index::{Index, Pack},
        pack::{BlobKind, Header},
    },
    ops::{
        backup::backup,
        verify::{Subset, verify},
    },
    repo::{
        Repository,
        blobs::{BlobStore, pack_name},
    },
    secret::Secret,
};
#[test]
fn subset_does_not_condemn_a_file_with_an_untested_copy_and_orphans_do_not_hide_damage() {
    let secret = Secret::KeyFile([7; 32]);
    let repo = Repository::init(MemoryBackend::new(), &secret).unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("file"), b"hello").unwrap();
    backup(&repo, &[source.path().to_path_buf()], vec![]).unwrap();
    let store = BlobStore::load(&repo).unwrap();
    let loc = store
        .index
        .values()
        .find(|l| l.blob.kind == BlobKind::Data)
        .unwrap();
    let master = slot::unwrap(&repo.list_keys().unwrap()[0].1, &secret).unwrap();
    let header = envelope::seal(
        &master.subkeys().encryption,
        Label::PackHeader,
        &serde_json::to_vec(&Header {
            version: 1,
            blobs: vec![loc.blob.clone()],
        })
        .unwrap(),
    )
    .unwrap();
    let mut bytes = envelope::seal(&master.subkeys().encryption, Label::Data, b"hello").unwrap();
    bytes.extend(&header);
    bytes.extend((header.len() as u32).to_le_bytes());
    let copy = blake3::hash(&bytes).to_hex().to_string();
    repo.backend
        .write(&pack_name(&copy).unwrap(), &bytes)
        .unwrap();
    let corrupt = pack_name(&loc.pack).unwrap();
    let mut bytes = repo.backend.read(&corrupt).unwrap();
    bytes[loc.blob.offset as usize + 24] ^= 1;
    repo.backend.plant_raw(corrupt.as_str(), &bytes).unwrap();
    // An orphan's good blob is not a usable live-index location.
    assert!(
        verify(&repo, None)
            .unwrap()
            .damage
            .iter()
            .any(|d| d.kind == "file")
    );
    let index = repo
        .write_object(
            "index",
            Label::Index,
            &serde_json::to_vec(&Index {
                version: 1,
                supersedes: vec![],
                packs: vec![Pack {
                    id: copy.clone(),
                    blobs: vec![loc.blob.clone()],
                }],
            })
            .unwrap(),
        )
        .unwrap();
    assert!(
        !verify(&repo, None)
            .unwrap()
            .damage
            .iter()
            .any(|d| d.kind == "file")
    );
    let first = u128::from_str_radix(&loc.pack[..32], 16).unwrap();
    let subset = format!("{}/{m}", first + 1, m = u128::MAX)
        .parse::<Subset>()
        .unwrap();
    assert!(subset.contains(&loc.pack));
    assert!(!subset.contains(&copy));
    let report = verify(&repo, Some(subset)).unwrap();
    assert!(report.damage.iter().any(|d| d.object == loc.blob.id));
    assert!(!report.damage.iter().any(|d| d.kind == "file"));
    // Once both indexed copies fail, every referring file is damaged.
    let copy_name = pack_name(&copy).unwrap();
    let mut bytes = repo.backend.read(&copy_name).unwrap();
    bytes[24] ^= 1;
    repo.backend.plant_raw(copy_name.as_str(), &bytes).unwrap();
    assert_eq!(
        verify(&repo, None)
            .unwrap()
            .damage
            .iter()
            .filter(|d| d.kind == "file")
            .count(),
        1
    );
    assert!(
        repo.backend
            .list(Kind::Index)
            .unwrap()
            .iter()
            .any(|e| e.name.as_str() == format!("index/{index}"))
    );
}
