use keelhaul::{
    backend::{Backend, local::LocalBackend},
    format::pack::BlobKind,
    ops::backup::backup,
    repo::{
        Repository,
        blobs::{BlobStore, pack_name},
    },
    secret::Secret,
};
use std::{fs, process::Command};
#[test]
fn partial_restore_json_lists_failures_and_damage_wins_exit_code() {
    let scratch = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(scratch.path()).unwrap();
    let source = root.join("source");
    fs::create_dir_all(source.join("directory")).unwrap();
    for (name, bytes) in [
        ("a", b"bad content".as_slice()),
        ("b", b"existing conflict"),
        ("directory/child", b"child"),
        ("z", b"last"),
    ] {
        fs::write(source.join(name), bytes).unwrap();
    }
    fs::write(root.join("key"), [1; 32]).unwrap();
    let repo = Repository::init(
        LocalBackend::new(root.join("repo")).unwrap(),
        &Secret::KeyFile([1; 32]),
    )
    .unwrap();
    backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
    for damaged in [false, true] {
        if damaged {
            let store = BlobStore::load(&repo).unwrap();
            let loc = store
                .index
                .values()
                .find(|loc| loc.blob.kind == BlobKind::Data && loc.blob.raw_length == 11)
                .unwrap();
            let name = pack_name(&loc.pack).unwrap();
            let mut bytes = repo.backend.read(&name).unwrap();
            bytes[loc.blob.offset as usize + 24] ^= 1;
            fs::write(root.join("repo").join(name.as_str()), bytes).unwrap();
        }
        let target = root.join(if damaged { "damaged-out" } else { "out" });
        let restored = target.join(source.strip_prefix("/").unwrap());
        fs::create_dir_all(&restored).unwrap();
        fs::write(restored.join("b"), b"keep").unwrap();
        fs::write(restored.join("directory"), b"not a directory").unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_keelhaul"))
            .env_clear()
            .env("HOME", &root)
            .env("PATH", std::env::var_os("PATH").unwrap())
            .arg("-r")
            .arg(root.join("repo"))
            .arg("--key-file")
            .arg(root.join("key"))
            .args(["restore", "latest", "--target"])
            .arg(&target)
            .arg("--json")
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(if damaged { 3 } else { 1 }),
            "{:?}",
            output
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["failed"].as_array().unwrap().len(), 3); // b, directory, directory/child
        assert_eq!(report["damage"].as_array().unwrap().is_empty(), !damaged);
        assert_eq!(fs::read(restored.join("z")).unwrap(), b"last");
        assert_eq!(fs::read(restored.join("b")).unwrap(), b"keep");
        assert!(!restored.join("directory/child").exists());
    }
}
