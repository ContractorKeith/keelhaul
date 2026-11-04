//! These packs have valid AEAD: the oracle distinguishes framing/cache failures
//! from keyed-ID failures. No audit output is used to construct expectations.
use super::*;
use keelhaul::{
    backend::{Backend, Kind, ObjectName},
    crypto::{
        envelope::{self, Label},
        slot,
    },
    format::{
        index::Index,
        pack::{Blob, BlobKind, Header},
    },
    repo::blobs::pack_name,
};
fn keys(f: &Fixture) -> keelhaul::crypto::keys::Subkeys {
    let slot = f
        .repo
        .list_keys()
        .unwrap()
        .into_iter()
        .find(|(_, s)| s.kind == keelhaul::format::key_slot::SlotKind::Keyfile)
        .unwrap()
        .1;
    slot::unwrap(&slot, &f.key).unwrap().subkeys()
}
fn pair(f: &Fixture, seed: u64) -> (ObjectName, Vec<Blob>, String) {
    let before = f
        .repo
        .backend
        .list(Kind::Index)
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect::<std::collections::HashSet<_>>();
    let mut store = BlobStore::load(&f.repo).unwrap();
    let mut ids = vec![];
    for n in [0, 1] {
        let mut bytes = vec![0; 1024];
        let mut h = blake3::Hasher::new();
        h.update(&seed.to_le_bytes());
        h.update(&[n]);
        h.finalize_xof().fill(&mut bytes);
        ids.push(store.add(BlobKind::Data, &bytes).unwrap());
    }
    store.finish().unwrap();
    let location = &store.index[&(BlobKind::Data, ids[0].clone())];
    let pack = location.pack.clone();
    let mut blobs = ids
        .iter()
        .map(|id| store.index[&(BlobKind::Data, id.clone())].blob.clone())
        .collect::<Vec<_>>();
    blobs.sort_by_key(|b| b.offset);
    assert_eq!(blobs[0].length, blobs[1].length);
    let index = f
        .repo
        .backend
        .list(Kind::Index)
        .unwrap()
        .into_iter()
        .find(|e| !before.contains(&e.name))
        .unwrap()
        .name
        .to_string();
    (pack_name(&pack).unwrap(), blobs, index)
}
fn swap_ranges(bytes: &mut [u8], a: &Blob, b: &Blob) {
    let n = a.length as usize;
    let x = a.offset as usize;
    let y = b.offset as usize;
    for i in 0..n {
        bytes.swap(x + i, y + i);
    }
}
fn replace_pack(f: &Fixture, old: &ObjectName, bytes: &[u8], index: &str) -> (String, String) {
    let id = blake3::hash(bytes).to_hex().to_string();
    let name = pack_name(&id).unwrap();
    f.repo.backend.write(&name, bytes).unwrap();
    f.repo.backend.delete(old).unwrap();
    let old_id = old.as_str().rsplit('/').next().unwrap();
    let payload = f
        .repo
        .read_object(&ObjectName::new(index).unwrap(), Label::Index)
        .unwrap();
    let mut value = Index::parse(&payload).unwrap();
    for pack in &mut value.packs {
        if pack.id == old_id {
            pack.id = id.clone();
        }
    }
    f.repo
        .backend
        .delete(&ObjectName::new(index).unwrap())
        .unwrap();
    let new_index = f
        .repo
        .write_object("index", Label::Index, &serde_json::to_vec(&value).unwrap())
        .unwrap();
    (name.to_string(), format!("index/{new_index}"))
}
pub fn run_cases(f: &Fixture, seed: u64) {
    duplicate_case(f, seed);
    // Change the complete valid descriptor list, re-seal it, and rename the pack.
    // Matching body permutation keeps the stored blobs valid; only the index lies.
    f.reset();
    let (name, mut blobs, index) = pair(f, seed);
    let mut bytes = f.bytes(name.as_str());
    swap_ranges(&mut bytes, &blobs[0], &blobs[1]);
    let body = (blobs[1].offset + blobs[1].length) as usize;
    let id = blobs[0].id.clone();
    blobs[0].id = blobs[1].id.clone();
    blobs[1].id = id;
    let header = envelope::seal(
        &keys(f).encryption,
        Label::PackHeader,
        &serde_json::to_vec(&Header {
            version: 1,
            blobs: blobs.clone(),
        })
        .unwrap(),
    )
    .unwrap();
    bytes.truncate(body);
    bytes.extend(&header);
    bytes.extend((header.len() as u32).to_le_bytes());
    let (_, index) = replace_pack(f, &name, &bytes, &index);
    let e = expected([signature(&index, "index", "index-mismatch")]);
    run(
        f,
        seed,
        "re-sealed descriptor permutation with valid body; stale index",
        e.clone(),
        e,
    );
    // Swap only two equal-length authenticated data envelopes, leaving descriptors.
    // Updating the stored pack ID removes whole-pack hashing as a competing detector.
    f.reset();
    let (name, blobs, index) = pair(f, seed);
    let mut bytes = f.bytes(name.as_str());
    swap_ranges(&mut bytes, &blobs[0], &blobs[1]);
    replace_pack(f, &name, &bytes, &index);
    let v = blobs
        .iter()
        .map(|b| signature(&b.id, "blob", "hash-mismatch"))
        .collect();
    run(
        f,
        seed,
        "equal-length data envelope swap, pack re-hashed/renamed",
        Expected::new(),
        v,
    );
    // Whole-pack contents swapped without renaming: both name hashes fail, and
    // each cached header disagrees. The data envelopes themselves remain valid.
    f.reset();
    let (a, _, ia) = pair(f, seed);
    let (b, _, ib) = pair(f, seed.wrapping_add(10000));
    let aa = f.bytes(a.as_str());
    let bb = f.bytes(b.as_str());
    f.plant(a.as_str(), &bb);
    f.plant(b.as_str(), &aa);
    let c = expected([
        signature(&ia, "index", "index-mismatch"),
        signature(&ib, "index", "index-mismatch"),
    ]);
    let mut v = c.clone();
    v.insert(signature(a.as_str(), "pack", "hash-mismatch"));
    v.insert(signature(b.as_str(), "pack", "hash-mismatch"));
    run(f, seed, "swap two whole packs' contents", c, v);
}

/// A stored location may be damaged while its logical chunk remains restorable.
/// The oracle retains physical pack/blob damage and adds referring files only
/// after both independently sealed, indexed copies fail authentication.
fn duplicate_case(f: &Fixture, seed: u64) {
    f.reset();
    let mut store = BlobStore::load(&f.repo).unwrap();
    let loc = store.index[&(BlobKind::Data, f.data.2.clone())].clone();
    let payload = store.read(BlobKind::Data, &loc.blob.id).unwrap();
    let mut blob = loc.blob.clone();
    blob.offset = 0;
    let mut stored = envelope::seal(&keys(f).encryption, Label::Data, &payload).unwrap();
    assert_eq!(stored.len() as u64, blob.length);
    let header = envelope::seal(
        &keys(f).encryption,
        Label::PackHeader,
        &serde_json::to_vec(&Header {
            version: 1,
            blobs: vec![blob.clone()],
        })
        .unwrap(),
    )
    .unwrap();
    stored.extend(&header);
    stored.extend((header.len() as u32).to_le_bytes());
    let id = blake3::hash(&stored).to_hex().to_string();
    let copy = pack_name(&id).unwrap();
    f.repo.backend.write(&copy, &stored).unwrap();
    f.repo
        .write_object(
            "index",
            Label::Index,
            &serde_json::to_vec(&Index {
                version: 1,
                supersedes: vec![],
                packs: vec![keelhaul::format::index::Pack {
                    id,
                    blobs: vec![blob],
                }],
            })
            .unwrap(),
        )
        .unwrap();
    let mut original = f.bytes(&f.data.0);
    original[f.data.1 as usize + 24] ^= 1;
    f.plant(&f.data.0, &original);
    let mut v = expected([
        signature(&f.data.0, "pack", "hash-mismatch"),
        signature(&f.data.2, "blob", "undecryptable"),
    ]);
    run(
        f,
        seed,
        "bad data location plus healthy indexed duplicate; no damaged files",
        Expected::new(),
        v.clone(),
    );
    stored[24] ^= 1;
    f.plant(copy.as_str(), &stored);
    v.insert(signature(copy.as_str(), "pack", "hash-mismatch"));
    v.extend(file_errors(f, &f.data.2));
    run(
        f,
        seed,
        "both indexed copies fail; every referring file is damaged",
        Expected::new(),
        v,
    );
}
