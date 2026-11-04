use keelhaul::{
    chunker::{self, Chunker},
    crypto::{
        envelope::{self, Label},
        keys::MasterKey,
    },
    format::{
        index::{Index, Pack},
        pack::{Blob, BlobKind, Header},
        snapshot::Snapshot,
        tree::Tree,
    },
};
use serde_json::{Value, json};
use std::path::Path;
fn load(name: &str) -> Value {
    serde_json::from_slice(
        &std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("testdata/vectors")
                .join(name),
        )
        .unwrap(),
    )
    .unwrap()
}
fn write(name: &str, value: Value) {
    std::fs::write(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata/vectors")
            .join(name),
        format!("{}\n", serde_json::to_string_pretty(&value).unwrap()),
    )
    .unwrap();
}
fn stream() -> Vec<u8> {
    let mut bytes = vec![0; 32 * 1024 * 1024];
    let mut h = blake3::Hasher::new();
    h.update(b"keelhaul-vector-stream");
    h.finalize_xof().fill(&mut bytes);
    bytes
}
fn lengths(bytes: &[u8], key: &[u8; 32]) -> Vec<usize> {
    let mut chunker = Chunker::new(bytes, key);
    let mut lengths = Vec::new();
    while let Some(c) = chunker.next_chunk().unwrap() {
        lengths.push(c.len());
    }
    lengths
}
fn fixed_envelope(key: &[u8; 32], label: Label, payload: &[u8], nonce: [u8; 24]) -> Vec<u8> {
    use chacha20poly1305::{
        XChaCha20Poly1305, XNonce,
        aead::{Aead, KeyInit, Payload},
    };
    let mut plaintext = vec![0];
    plaintext.extend(payload);
    let mut out = nonce.to_vec();
    out.extend(
        XChaCha20Poly1305::new(key.into())
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &plaintext,
                    aad: label.as_str().as_bytes(),
                },
            )
            .unwrap(),
    );
    out
}
#[test]
fn m2_vectors() {
    let master = MasterKey::from_bytes(std::array::from_fn(|i| i as u8));
    let keys = master.subkeys();
    let gear = chunker::gear(&keys.gear);
    let gear_bytes = gear
        .iter()
        .flat_map(|g| g.to_le_bytes())
        .collect::<Vec<_>>();
    let bytes = stream();
    let chunk_lengths = lengths(&bytes, &keys.gear);
    if std::env::var_os("KEELHAUL_GENERATE_VECTORS").is_some() {
        write(
            "gear.json",
            json!({"first8":gear[..8],"blake3":blake3::hash(&gear_bytes).to_hex().to_string()}),
        );
        write(
            "chunks.json",
            json!({"seed":"keelhaul-vector-stream","len":bytes.len(),"lengths":chunk_lengths}),
        );
        write(
            "chunk-id.json",
            json!({"payload_hex":hex::encode(b"hello keelhaul\n"),"id":blake3::keyed_hash(&keys.chunk_id,b"hello keelhaul\n").to_hex().to_string()}),
        );
        let tree = json!({"version":1,"entries":[{"name":"%2Ffixture","type":"file","mode":420,"mtime":"2026-11-04T14:03:22.123456789Z","ctime":"2026-11-04T14:03:22.123456789Z","uid":1000,"gid":1000,"user":"vector-user","group":"vector-group","inode":7,"device":1,"links":1,"size":14,"content":[blake3::keyed_hash(&keys.chunk_id,b"hello keelhaul\n").to_hex().to_string()]}]});
        let data = b"hello keelhaul\n".to_vec();
        let tree_bytes = serde_json::to_vec(&tree).unwrap();
        let mut stored = Vec::new();
        let mut blobs = Vec::new();
        for (i, (kind, payload)) in [(BlobKind::Data, data), (BlobKind::Tree, tree_bytes)]
            .into_iter()
            .enumerate()
        {
            let sealed = fixed_envelope(&keys.encryption, kind.label(), &payload, [i as u8; 24]);
            blobs.push(Blob {
                id: blake3::keyed_hash(&keys.chunk_id, &payload)
                    .to_hex()
                    .to_string(),
                kind,
                offset: stored.len() as u64,
                length: sealed.len() as u64,
                raw_length: payload.len() as u64,
            });
            stored.extend(sealed);
        }
        let header = Header { version: 1, blobs };
        let sealed = fixed_envelope(
            &keys.encryption,
            Label::PackHeader,
            &serde_json::to_vec(&header).unwrap(),
            [2; 24],
        );
        stored.extend(&sealed);
        stored.extend((sealed.len() as u32).to_le_bytes());
        let pack_id = blake3::hash(&stored).to_hex().to_string();
        write(
            "pack.json",
            json!({"stored_hex":hex::encode(stored),"id":pack_id,"header":header}),
        );
        write(
            "index.json",
            serde_json::to_value(Index {
                version: 1,
                supersedes: vec![],
                packs: vec![Pack {
                    id: pack_id,
                    blobs: header.blobs.clone(),
                }],
            })
            .unwrap(),
        );
        write("tree.json", tree);
        write(
            "snapshot.json",
            json!({"version":1,"time":"2026-11-04T14:03:22.123456789Z","host":"vector-host","username":"vector-user","paths":["/fixture"],"tags":["test"],"excludes":[],"tree":header.blobs[1].id,"parent":null,"program":"keelhaul 0.1.0","summary":{"files_new":1,"files_changed":0,"files_unchanged":0,"dirs":0,"files_skipped":0,"data_blobs_added":1,"bytes_added":55,"bytes_processed":14}}),
        );
    }
    let v = load("gear.json");
    assert_eq!(v["first8"], json!(gear[..8]));
    assert_eq!(v["blake3"], blake3::hash(&gear_bytes).to_hex().as_str());
    let v = load("chunks.json");
    assert_eq!(v["lengths"], json!(chunk_lengths));
    assert_eq!(v["len"], bytes.len());
    assert_eq!(v["seed"], "keelhaul-vector-stream");
    let v = load("chunk-id.json");
    assert_eq!(
        v["id"],
        blake3::keyed_hash(
            &keys.chunk_id,
            &hex::decode(v["payload_hex"].as_str().unwrap()).unwrap()
        )
        .to_hex()
        .as_str()
    );
    let v = load("pack.json");
    let stored = hex::decode(v["stored_hex"].as_str().unwrap()).unwrap();
    assert_eq!(v["id"], blake3::hash(&stored).to_hex().as_str());
    let len =
        keelhaul::format::pack::header_length(&stored[stored.len() - 4..], stored.len() as u64)
            .unwrap() as usize;
    let payload = envelope::open(
        &keys.encryption,
        Label::PackHeader,
        &stored[stored.len() - 4 - len..stored.len() - 4],
    )
    .unwrap();
    let header = Header::parse(&payload, (stored.len() - 4 - len) as u64).unwrap();
    assert_eq!(serde_json::to_value(&header).unwrap(), v["header"]);
    for b in &header.blobs {
        let payload = envelope::open(
            &keys.encryption,
            b.kind.label(),
            &stored[b.offset as usize..(b.offset + b.length) as usize],
        )
        .unwrap();
        assert_eq!(
            b.id,
            blake3::keyed_hash(&keys.chunk_id, &payload)
                .to_hex()
                .as_str()
        );
        assert_eq!(b.raw_length, payload.len() as u64);
    }
    let index = Index::parse(&serde_json::to_vec(&load("index.json")).unwrap()).unwrap();
    assert_eq!(index.packs[0].blobs, header.blobs);
    Tree::parse(&serde_json::to_vec(&load("tree.json")).unwrap()).unwrap();
    Snapshot::parse(&serde_json::to_vec(&load("snapshot.json")).unwrap()).unwrap();
}

#[test]
fn independent_python_chunker_if_available() {
    match std::process::Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/testdata/vectors/reference/chunker.py"
        ))
        .output()
    {
        Ok(output) if output.status.code() == Some(77) => {
            eprintln!("{}", String::from_utf8_lossy(&output.stdout))
        }
        Ok(output) => assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("reference chunker: skipped (python3 unavailable)")
        }
        Err(e) => panic!("cannot run independent reference: {e}"),
    }
}
