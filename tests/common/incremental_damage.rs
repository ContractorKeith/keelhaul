use super::*;
use keelhaul::{
    backend::local::LocalBackend,
    format::pack::BlobKind,
    repo::blobs::{BlobStore, pack_name},
};
#[test]
fn damaged_parent_is_replaced_and_every_snapshot_restores_through_verified_copies() {
    for corrupt_tree in [false, true] {
        let scratch = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(scratch.path()).unwrap();
        let source = root.join("source");
        fs::create_dir_all(source.join("nested/empty")).unwrap();
        fs::write(source.join("alpha"), b"first file").unwrap();
        fs::write(source.join("nested/beta"), b"second\0binary file").unwrap();
        let repo_path = root.join("repo");
        let key = root.join("key");
        fs::write(&key, [8; 32]).unwrap();
        let repo = Repository::init(
            LocalBackend::new(&repo_path).unwrap(),
            &Secret::KeyFile([8; 32]),
        )
        .unwrap();
        let first = backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
        assert_eq!(repo.backend.list(Kind::Data).unwrap().len(), 1);
        let (_, original) = repo.select_snapshot(&first.snapshot).unwrap();
        let store = BlobStore::load(&repo).unwrap();
        let data = store
            .index
            .values()
            .find(|l| l.blob.kind == BlobKind::Data)
            .unwrap()
            .clone();
        let damaged = if corrupt_tree {
            store.index[&(BlobKind::Tree, original.tree.clone())].clone()
        } else {
            data.clone()
        };
        let name = pack_name(&damaged.pack).unwrap();
        let mut bytes = repo.backend.read(&name).unwrap();
        bytes[damaged.blob.offset as usize] ^= 1;
        fs::write(repo_path.join(name.as_str()), bytes).unwrap();
        let second = backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
        assert_eq!(second.summary.files_read, 2);
        assert_eq!(
            second.summary.data_blobs_added, 2,
            "a known damaged pack must not supply dedup hints"
        );
        assert!(second.summary.bytes_added > 0);
        let (_, recovered) = repo.select_snapshot(&second.snapshot).unwrap();
        assert!(recovered.parent.is_none());
        assert_eq!(
            original.tree, recovered.tree,
            "same plaintext IDs must acquire healthy stored copies"
        );
        // Pin the damaged location as primary, independent of random index IDs.
        let mut reader = BlobStore::load(&repo).unwrap();
        reader.index.insert(
            (damaged.blob.kind, damaged.blob.id.clone()),
            damaged.clone(),
        );
        assert!(reader.read(damaged.blob.kind, &damaged.blob.id).is_ok());
        for (n, snapshot) in [&first.snapshot, &second.snapshot].into_iter().enumerate() {
            let target = root.join(format!("out-{n}"));
            let report = keelhaul::ops::restore::restore(&repo, snapshot, &target, &[]).unwrap();
            assert!(report.damaged.is_empty(), "{report:?}");
            for file in ["alpha", "nested/beta"] {
                assert_eq!(
                    fs::read(source.join(file)).unwrap(),
                    fs::read(target.join(source.strip_prefix("/").unwrap()).join(file)).unwrap()
                );
            }
        }
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_keelhaul"))
            .env("HOME", &root)
            .env_remove("KEELHAUL_PASSWORD_FILE")
            .env_remove("KEELHAUL_PASSWORD_COMMAND")
            .env_remove("KEELHAUL_FAULT")
            .arg("-r")
            .arg(&repo_path)
            .arg("--key-file")
            .arg(&key)
            .args(["verify", "--json"])
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(3),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let damage = report["damage"].as_array().unwrap();
        assert!(
            damage
                .iter()
                .any(|d| d["object"] == name.as_str() && d["problem"] == "hash-mismatch")
        );
        assert!(
            !damage.iter().any(|d| d["kind"] == "file"),
            "healthy copies must resolve every file: {report}"
        );
    }
}
