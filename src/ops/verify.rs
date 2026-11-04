//! Whole-pack hashing and independent verification of every stored blob.
use super::check::State;
use crate::{
    backend::Backend,
    damage::Report,
    format::pack::{Blob, BlobKind},
    repo::{Error, Repository, audit, blobs::pack_name},
    secret::Secret,
};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Subset {
    n: u128,
    m: u128,
}
impl std::str::FromStr for Subset {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        let (n, m) = s.split_once('/').ok_or("subset must be N/M")?;
        let n = n.parse::<u128>().map_err(|_| "invalid subset numerator")?;
        let m = m
            .parse::<u128>()
            .map_err(|_| "invalid subset denominator")?;
        if n == 0 || n > m {
            return Err("subset requires 1 <= N <= M".into());
        }
        Ok(Self { n, m })
    }
}
impl Subset {
    pub fn contains(&self, id: &str) -> bool {
        id.get(..32)
            .and_then(|p| u128::from_str_radix(p, 16).ok())
            .is_some_and(|v| v % self.m == self.n - 1)
    }
}
pub fn verify<B: Backend>(repo: &Repository<B>, subset: Option<Subset>) -> Result<Report, Error> {
    run(State::new(&repo.backend, repo.subkeys()), subset)
}
pub fn verify_backend<B: Backend>(
    backend: &B,
    secret: &Secret,
    subset: Option<Subset>,
) -> Result<Report, Error> {
    run(
        State::new(backend, Repository::unlock(backend, secret)?.subkeys()),
        subset,
    )
}
fn run<B: Backend>(mut state: State<'_, B>, subset: Option<Subset>) -> Result<Report, Error> {
    state.metadata()?;
    let mut good = HashSet::new();
    let mut indexed_packs: HashMap<&str, HashMap<_, _>> = HashMap::new();
    for (pack, blob) in state.live.values().flatten() {
        indexed_packs
            .entry(pack)
            .or_default()
            .insert(descriptor(blob), blob);
    }
    for (id, header) in &state.packs {
        if subset.is_some_and(|s| !s.contains(id)) {
            continue;
        }
        let name = pack_name(id)?;
        let Some(bytes) = state.report.collect(audit::read(state.backend, &name))? else {
            continue;
        };
        if blake3::hash(&bytes).to_hex().as_str() != id {
            state.report.add(
                name.as_str(),
                "pack",
                "hash-mismatch",
                "stored pack ID differs",
            );
        }
        // A corrupt header must not hide the independently indexed blobs.
        let indexed = indexed_packs.get(id.as_str());
        let mut blobs = HashMap::new();
        if let Some(header) = header {
            for blob in &header.blobs {
                blobs.insert(descriptor(blob), blob);
            }
        }
        if let Some(indexed) = indexed {
            blobs.extend(indexed.iter().map(|(key, blob)| (*key, *blob)));
        }
        for blob in blobs.into_values() {
            let stored = blob
                .offset
                .checked_add(blob.length)
                .and_then(|end| {
                    let start = usize::try_from(blob.offset).ok()?;
                    let end = usize::try_from(end).ok()?;
                    bytes.get(start..end)
                })
                .unwrap_or(&[]);
            let result = audit::blob(&state.keys, &blob.id, blob, stored).and_then(|bytes| {
                if blob.kind == crate::format::pack::BlobKind::Tree {
                    crate::format::tree::Tree::parse(&bytes).map_err(|e| Error::Damage {
                        object: blob.id.clone(),
                        problem: "bad-format",
                        detail: e.to_string(),
                    })?;
                }
                Ok(bytes)
            });
            let key = (blob.kind, blob.id.clone());
            if state.report.collect(result)?.is_some() {
                if indexed.is_some_and(|v| v.contains_key(&descriptor(blob))) {
                    good.insert(key);
                }
            } else {
                state.bad.insert(key);
            }
        }
    }
    // One healthy duplicate is sufficient to resolve a file.
    state.bad.retain(|k| {
        !good.contains(k)
            && state.live.get(k).is_none_or(|locations| {
                locations.iter().all(|(pack, _)| {
                    !state.packs.contains_key(pack) || subset.is_none_or(|s| s.contains(pack))
                })
            })
    });
    state.finish(true)
}

// Include lengths and offsets: conflicting index descriptors need independent reads.
fn descriptor(blob: &Blob) -> (BlobKind, &str, u64, u64, u64) {
    (
        blob.kind,
        &blob.id,
        blob.offset,
        blob.length,
        blob.raw_length,
    )
}
