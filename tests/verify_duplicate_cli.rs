//! Verify's any-good-copy decision is separate from the general reader fallback.
use keelhaul::{
    backend::{Backend, Kind, local::LocalBackend},
    crypto::{
        envelope::{self, Label},
        slot,
    },
    format::{
        index::{Index, Pack},
        pack::{BlobKind, Header},
    },
    ops::backup::backup,
    repo::{
        Repository,
        blobs::{BlobStore, Location, pack_name},
    },
    secret::Secret,
};
use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};
struct Fixture {
    _scratch: tempfile::TempDir,
    root: PathBuf,
    repository: PathBuf,
    key: PathBuf,
    source: PathBuf,
    original: Location,
    copy: String,
}
impl Fixture {
    fn new() -> Self {
        let scratch = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(scratch.path()).unwrap();
        let repository = root.join("repository");
        let key = root.join("key");
        let source = root.join("source");
        fs::write(&key, [7; 32]).unwrap();
        fs::create_dir(&source).unwrap();
        fs::write(source.join("file"), b"hello").unwrap();
        let secret = Secret::KeyFile([7; 32]);
        let repo = Repository::init(LocalBackend::new(&repository).unwrap(), &secret).unwrap();
        backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
        let store = BlobStore::load(&repo).unwrap();
        let original = store
            .index
            .values()
            .find(|l| l.blob.kind == BlobKind::Data)
            .unwrap()
            .clone();
        let master = slot::unwrap(&repo.list_keys().unwrap()[0].1, &secret).unwrap();
        let mut descriptor = original.blob.clone();
        descriptor.offset = 0;
        let mut bytes =
            envelope::seal(&master.subkeys().encryption, Label::Data, b"hello").unwrap();
        assert_eq!(bytes.len() as u64, descriptor.length);
        let header = envelope::seal(
            &master.subkeys().encryption,
            Label::PackHeader,
            &serde_json::to_vec(&Header {
                version: 1,
                blobs: vec![descriptor.clone()],
            })
            .unwrap(),
        )
        .unwrap();
        bytes.extend(&header);
        bytes.extend((header.len() as u32).to_le_bytes());
        let copy = blake3::hash(&bytes).to_hex().to_string();
        repo.backend
            .write(&pack_name(&copy).unwrap(), &bytes)
            .unwrap();
        // A combined replacement index fixes preferred-location order: the bad
        // original is first, and the freshly sealed duplicate is second.
        let indexes = repo.backend.list(Kind::Index).unwrap();
        let mut combined = Index {
            version: 1,
            supersedes: vec![],
            packs: vec![],
        };
        for old in indexes {
            combined.supersedes.push(old.name.as_str()[6..].to_owned());
            combined.packs.extend(
                Index::parse(&repo.read_object(&old.name, Label::Index).unwrap())
                    .unwrap()
                    .packs,
            );
        }
        combined.packs.push(Pack {
            id: copy.clone(),
            blobs: vec![descriptor],
        });
        repo.write_object(
            "index",
            Label::Index,
            &serde_json::to_vec(&combined).unwrap(),
        )
        .unwrap();
        assert_eq!(
            BlobStore::load(&repo).unwrap().index[&(BlobKind::Data, original.blob.id.clone())].pack,
            original.pack
        );
        let corrupt = pack_name(&original.pack).unwrap();
        let mut bytes = repo.backend.read(&corrupt).unwrap();
        bytes[original.blob.offset as usize + 24] ^= 1;
        fs::write(repository.join(corrupt.as_str()), bytes).unwrap();
        Self {
            _scratch: scratch,
            root,
            repository,
            key,
            source,
            original,
            copy,
        }
    }
    fn cli(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_keelhaul"))
            .arg("--repo")
            .arg(&self.repository)
            .arg("--key-file")
            .arg(&self.key)
            .arg("--json")
            .args(args)
            .env_remove("KEELHAUL_PASSWORD_FILE")
            .env_remove("KEELHAUL_PASSWORD_COMMAND")
            .output()
            .unwrap()
    }
}
#[test]
fn verify_names_the_bad_pack_without_condemning_a_file_that_has_a_good_copy() {
    let f = Fixture::new();
    let corrupt = pack_name(&f.original.pack).unwrap();
    let verified = f.cli(&["verify"]);
    assert_eq!(
        verified.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&verified.stderr)
    );
    let report: Value = serde_json::from_slice(&verified.stdout).unwrap();
    let damage = report["damage"].as_array().unwrap();
    assert_eq!(damage.len(), 2, "{report}");
    assert!(damage.iter().any(|d| d["object"] == corrupt.as_str()
        && d["kind"] == "pack"
        && d["problem"] == "hash-mismatch"));
    assert!(damage.iter().any(|d| d["object"] == f.original.blob.id
        && d["kind"] == "blob"
        && d["problem"] == "undecryptable"
        && d["detail"].as_str().unwrap().contains(corrupt.as_str())));
    assert!(!damage.iter().any(|d| d["kind"] == "file"));
    // Only when both indexed copies fail does the referring file become damaged.
    let second = pack_name(&f.copy).unwrap();
    let path = f.repository.join(second.as_str());
    let mut bytes = fs::read(&path).unwrap();
    bytes[24] ^= 1;
    fs::write(path, bytes).unwrap();
    let verified = f.cli(&["verify"]);
    assert_eq!(verified.status.code(), Some(3));
    let report: Value = serde_json::from_slice(&verified.stdout).unwrap();
    assert_eq!(
        report["damage"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| d["kind"] == "file")
            .count(),
        1
    );
}
#[test]
fn restore_succeeds_when_the_preferred_indexed_copy_is_damaged() {
    let f = Fixture::new();
    let corrupt = pack_name(&f.original.pack).unwrap();
    let before = fs::read(f.repository.join(corrupt.as_str())).unwrap();
    let target = f.root.join("restored");
    let restored = f.cli(&["restore", "latest", "--target", target.to_str().unwrap()]);
    assert_eq!(
        restored.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&restored.stderr)
    );
    let report: Value = serde_json::from_slice(&restored.stdout).unwrap();
    assert_eq!(report["damage"], serde_json::json!([]));
    assert_eq!(
        fs::read(
            target
                .join(f.source.strip_prefix("/").unwrap())
                .join("file")
        )
        .unwrap(),
        b"hello"
    );
    assert_eq!(
        fs::read(f.repository.join(corrupt.as_str())).unwrap(),
        before,
        "restore must not heal the stored copy"
    );
}
