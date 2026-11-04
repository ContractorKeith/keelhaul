//! Plan fully before mutating; publish the replacement index before deleting anything.
use super::check::{BlobKey, State};
use crate::{
    backend::{Backend, Kind},
    crypto::envelope::Label,
    format::{
        index::{Index, Pack},
        pack::Blob,
    },
    repo::{
        Error, Repository, audit,
        blobs::{BlobStore, pack_name},
        lock::{self, Clock, STALE, SystemClock},
    },
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    time::SystemTime,
};
#[derive(Debug, Default, Serialize)]
pub struct Summary {
    pub packs_removed: usize,
    pub packs_repacked: usize,
    pub packs_kept: usize,
    pub bytes_freed: u64,
    pub blobs_copied: usize,
    pub temporary_removed: usize,
    pub dry_run: bool,
}
pub fn prune<B: Backend>(
    repo: &Repository<B>,
    max_unused: u8,
    dry_run: bool,
) -> Result<Summary, Error> {
    lock::with(&repo.backend, repo.subkeys(), true, |guard| {
        run(repo, max_unused, dry_run, &SystemClock, guard)
    })
}
fn old(modified: Option<SystemTime>, now: SystemTime) -> bool {
    modified
        .and_then(|t| now.duration_since(t).ok())
        .is_some_and(|age| age > STALE)
}
pub(super) fn run<B: Backend>(
    repo: &Repository<B>,
    max_unused: u8,
    dry_run: bool,
    clock: &dyn Clock,
    guard: &lock::Guard<'_, B>,
) -> Result<Summary, Error> {
    if max_unused > 100 {
        return Err(Error::Usage("max-unused must be 0–100".into()));
    }
    let mut state = State::new(&repo.backend, repo.subkeys());
    state.metadata()?;
    state.reachable()?;
    if let Some(damage) = state.report.damage.first() {
        return Err(Error::Damage {
            object: damage.object.clone(),
            problem: "bad-format",
            detail: damage.to_string(),
        });
    }
    // Choose one verified location per reachable blob, trying duplicates on damage.
    let mut selected: HashMap<BlobKey, (String, Blob)> = HashMap::new();
    let mut duplicates = HashSet::new();
    let mut used = state.used.iter().collect::<Vec<_>>();
    used.sort_by(|a, b| a.1.cmp(&b.1));
    for key in used {
        let locations = state.live.get(key).ok_or_else(|| Error::Damage {
            object: key.1.clone(),
            problem: "dangling-reference",
            detail: "no indexed copy".into(),
        })?;
        let mut verified = None;
        let mut failure = None;
        for (pack, blob) in locations {
            let name = pack_name(pack)?;
            let result = audit::range(&repo.backend, &name, blob.offset, blob.length)
                .and_then(|bytes| audit::blob(&state.keys, &blob.id, name.as_str(), blob, &bytes));
            match result {
                Ok(_) => {
                    verified = Some((pack.clone(), blob.clone()));
                    break;
                }
                Err(e @ Error::Damage { .. }) => failure = Some(e),
                Err(e) => return Err(e),
            }
        }
        let location = verified.ok_or_else(|| {
            failure.unwrap_or_else(|| Error::Damage {
                object: key.1.clone(),
                problem: "missing",
                detail: "no verified copy".into(),
            })
        })?;
        if locations.len() > 1 {
            for (pack, _) in locations {
                duplicates.insert(pack.clone());
            }
        }
        selected.insert(key.clone(), location);
    }
    let named = state
        .live
        .values()
        .flatten()
        .map(|(p, _)| p.clone())
        .collect::<HashSet<_>>();
    let mut keep = BTreeMap::new();
    let mut repack = Vec::new();
    let mut remove = Vec::new();
    let mut repack_unused = 0u64;
    let mut summary = Summary {
        dry_run,
        ..Summary::default()
    };
    for entry in repo.backend.list(Kind::Data)? {
        let id = entry.name.as_str()[8..].to_owned();
        let Some(Some(header)) = state.packs.get(&id) else {
            return Err(Error::Refused(
                "pack inventory changed under exclusive lock".into(),
            ));
        };
        let chosen = header
            .blobs
            .iter()
            .filter(|blob| {
                selected
                    .get(&(blob.kind, blob.id.clone()))
                    .is_some_and(|(p, b)| p == &id && b == *blob)
            })
            .cloned()
            .collect::<Vec<_>>();
        if chosen.is_empty() {
            if !named.contains(&id) && !old(entry.modified, clock.now()) {
                summary.packs_kept += 1;
                continue;
            }
            summary.packs_removed += 1;
            summary.bytes_freed += entry.size;
            remove.push(entry.name);
            continue;
        }
        let total: u128 = header.blobs.iter().map(|b| u128::from(b.length)).sum();
        let retained: u128 = chosen.iter().map(|b| u128::from(b.length)).sum();
        if (total - retained) * 100 > total * u128::from(max_unused) || duplicates.contains(&id) {
            summary.packs_repacked += 1;
            summary.blobs_copied += chosen.len();
            // Exact stored-byte saving is finalized after the replacement packs exist.
            summary.bytes_freed += u64::try_from(total - retained)
                .map_err(|_| Error::Refused("pack size overflow".into()))?;
            repack_unused += u64::try_from(total - retained)
                .map_err(|_| Error::Refused("pack size overflow".into()))?;
            repack.push((entry.name, chosen));
        } else {
            summary.packs_kept += 1;
            keep.insert(
                id.clone(),
                Pack {
                    id,
                    blobs: header.blobs.clone(),
                },
            );
        }
    }
    let mut temporary = Vec::new();
    for kind in Kind::ALL {
        for entry in repo.backend.list_foreign(kind)? {
            if entry.deletable && is_temporary(&entry.raw) && old(entry.modified, clock.now()) {
                summary.temporary_removed += 1;
                summary.bytes_freed += entry.size;
                temporary.push(entry.raw);
            }
        }
    }
    if dry_run {
        return Ok(summary);
    }
    guard.self_check()?;
    let mut writer = BlobStore::repack_writer(repo)?;
    let mut replaced_bytes = 0u64;
    for (name, blobs) in &repack {
        replaced_bytes += repo.backend.stat(name)?;
        for blob in blobs {
            let bytes = audit::range(&repo.backend, name, blob.offset, blob.length)?;
            let payload = audit::blob(&state.keys, &blob.id, name.as_str(), blob, &bytes)?;
            writer.add(blob.kind, &payload)?;
        }
    }
    writer.finish()?;
    let written = writer.written_packs();
    let mut written_bytes = 0;
    for pack in written {
        written_bytes += repo.backend.stat(&pack_name(&pack.id)?)?;
        keep.insert(pack.id.clone(), pack);
    }
    summary.bytes_freed = summary
        .bytes_freed
        .saturating_sub(repack_unused)
        .saturating_add(replaced_bytes)
        .saturating_sub(written_bytes);
    // Include any additive indexes the pack writer flushed. They stay valid on a crash.
    let old_indexes = repo.backend.list(Kind::Index)?;
    let replacement = Index {
        version: 1,
        supersedes: old_indexes
            .iter()
            .map(|e| e.name.as_str()[6..].to_owned())
            .collect(),
        packs: keep.into_values().collect(),
    };
    let bytes = serde_json::to_vec(&replacement)?;
    // An atomic replacement must fit the format's object limit; never partially supersede.
    if bytes.len() > 64 * 1024 * 1024 {
        return Err(Error::Refused(
            "replacement index exceeds 64 MiB object limit".into(),
        ));
    }
    guard.self_check()?;
    repo.write_object("index", Label::Index, &bytes)?;
    for entry in old_indexes {
        repo.backend.delete(&entry.name)?;
    }
    for (name, _) in repack {
        repo.backend.delete(&name)?;
    }
    for name in remove {
        repo.backend.delete(&name)?;
    }
    for raw in temporary {
        repo.backend.delete_foreign(&raw)?;
    }
    Ok(summary)
}
fn is_temporary(raw: &str) -> bool {
    raw.rsplit_once(".tmp-").is_some_and(|(_, suffix)| {
        suffix.len() == 16
            && suffix
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}
