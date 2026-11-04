use keelhaul::{
    backend::{Backend, memory::MemoryBackend},
    format::pack::BlobKind,
    ops::{backup::backup, restore::restore, verify::verify},
    repo::{
        Repository,
        blobs::{BlobStore, pack_name},
    },
    secret::Secret,
};
use std::{collections::HashSet, ffi::OsString, fs, os::unix::ffi::OsStringExt};
#[test]
fn shared_damaged_chunk_names_every_raw_path_and_snapshot_and_good_file_restores() {
    let scratch = tempfile::tempdir().unwrap();
    let source = fs::canonicalize(scratch.path()).unwrap().join("source");
    fs::create_dir(&source).unwrap();
    let names = [
        OsString::from_vec(vec![b'a', 0xff]),
        OsString::from_vec(vec![b'a', 0xfe]),
    ];
    for name in &names {
        fs::write(source.join(name), b"shared bad content").unwrap();
    }
    fs::write(source.join("good"), b"correct").unwrap();
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([7; 32])).unwrap();
    for _ in 0..2 {
        backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
    }
    let store = BlobStore::load(&repo).unwrap();
    let loc = store
        .index
        .values()
        .find(|l| l.blob.kind == BlobKind::Data && l.blob.raw_length == 18)
        .unwrap();
    let name = pack_name(&loc.pack).unwrap();
    let mut bytes = repo.backend.read(&name).unwrap();
    bytes[loc.blob.offset as usize + 24] ^= 1;
    repo.backend.plant_raw(name.as_str(), &bytes).unwrap();
    let report = verify(&repo, None).unwrap();
    let files = report
        .damage
        .iter()
        .filter(|d| d.kind == "file")
        .collect::<Vec<_>>();
    assert_eq!(files.len(), 4);
    assert_eq!(
        files
            .iter()
            .map(|d| &d.object)
            .collect::<HashSet<_>>()
            .len(),
        4
    );
    assert!(
        files
            .iter()
            .all(|d| d.detail.contains("snapshot snapshots/") && d.detail.contains("path b64:"))
    );
    assert_eq!(
        report
            .damage
            .iter()
            .filter(|d| d.object == loc.blob.id)
            .count(),
        1
    );
    let target = scratch.path().join("restore");
    let report = restore(&repo, "latest", &target, &[]).unwrap();
    assert_eq!(
        report.damaged.iter().filter(|d| d.kind == "file").count(),
        2
    );
    assert_eq!(
        report
            .damaged
            .iter()
            .filter(|d| d.object == loc.blob.id)
            .count(),
        1
    );
    let restored = target.join(source.strip_prefix("/").unwrap());
    assert_eq!(fs::read(restored.join("good")).unwrap(), b"correct");
    for name in names {
        assert!(!restored.join(name).exists());
    }
    assert_eq!(fs::read_dir(restored).unwrap().count(), 1);
}
