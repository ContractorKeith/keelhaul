//! Plan fully before mutating; publish the replacement index before deleting anything.
mod delete;
mod indexes;
mod used;
use crate::{
    backend::{Backend, Kind},
    format::index::Pack,
    repo::{
        Error, Repository, audit,
        blobs::{BlobStore, pack_name},
        lock::{self, Clock, STALE, SystemClock},
    },
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, HashSet},
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
    pub indexes_written: usize,
    pub warnings: Vec<String>,
}
pub fn prune<B: Backend>(
    repo: &Repository<B>,
    max_unused: u8,
    dry_run: bool,
) -> Result<Summary, Error> {
    prune_with_clock(repo, max_unused, dry_run, &SystemClock)
}
pub fn prune_with_clock<B: Backend>(
    repo: &Repository<B>,
    max_unused: u8,
    dry_run: bool,
    clock: &dyn Clock,
) -> Result<Summary, Error> {
    let guard = lock::Guard::acquire(&repo.backend, repo.subkeys(), true, clock)?;
    guard.run(|guard| run(repo, max_unused, dry_run, clock, guard))
}

fn old(modified: Option<SystemTime>, now: SystemTime) -> bool {
    // Missing modification times disable both age-based cleanups: retain orphan
    // packs and temporary names because their safe cleanup age is unknown.
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
    run_limit(repo, max_unused, dry_run, clock, guard, indexes::LIMIT)
}
pub fn prune_with_index_limit<B: Backend>(
    repo: &Repository<B>,
    max_unused: u8,
    dry_run: bool,
    limit: usize,
) -> Result<Summary, Error> {
    let guard = lock::Guard::acquire(&repo.backend, repo.subkeys(), true, &SystemClock)?;
    guard.run(|guard| run_limit(repo, max_unused, dry_run, &SystemClock, guard, limit))
}
fn run_limit<B: Backend>(
    repo: &Repository<B>,
    max_unused: u8,
    dry_run: bool,
    clock: &dyn Clock,
    guard: &lock::Guard<'_, B>,
    limit: usize,
) -> Result<Summary, Error> {
    if max_unused > 100 {
        return Err(Error::Usage("max-unused must be 0–100".into()));
    }
    let scanned_indexes = repo
        .backend
        .list(Kind::Index)?
        .into_iter()
        .map(|e| e.name.to_string())
        .collect::<HashSet<_>>();
    let used::Reachable {
        state,
        mut selected,
        mut force_repack,
    } = used::scan(repo)?;
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
        let header = state.packs.get(&id).ok_or_else(|| {
            Error::Refused(format!(
                "new pack appeared during prune: {}; rerun prune",
                entry.name
            ))
        })?;
        // An unreadable header is distinct from a new pack. Verified indexed
        // ranges can be rescued, but this damaged physical pack is never kept.
        if header.is_none() {
            force_repack.insert(id.clone());
        }
        let mut chosen = selected.remove(&id).unwrap_or_default();
        chosen.sort_by_key(|blob| blob.offset);
        if chosen.is_empty() {
            if !named.contains(&id) && !old(entry.modified, clock.now()) {
                if entry.modified.is_none() {
                    unknown_modified(&mut summary);
                }
                summary.packs_kept += 1;
                continue;
            }
            summary.packs_removed += 1;
            summary.bytes_freed += entry.size;
            remove.push(entry.name);
            continue;
        }
        let total: u128 = header
            .as_ref()
            .map(|h| h.blobs.iter().map(|b| u128::from(b.length)).sum())
            .unwrap_or(u128::from(entry.size));
        let retained: u128 = chosen.iter().map(|b| u128::from(b.length)).sum();
        let total = total.max(retained);
        if (total - retained) * 100 > total * u128::from(max_unused) || force_repack.contains(&id) {
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
                    blobs: header
                        .as_ref()
                        .expect("unreadable headers are always repacked")
                        .blobs
                        .clone(),
                },
            );
        }
    }
    let mut temporary = Vec::new();
    for kind in Kind::ALL {
        for entry in repo.backend.list_foreign(kind)? {
            if entry.deletable && is_temporary(&entry.raw) && entry.modified.is_none() {
                unknown_modified(&mut summary);
            }
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
    let mut expected_indexes = scanned_indexes;
    expected_indexes.extend(
        writer
            .written_indexes()
            .iter()
            .map(|id| format!("index/{id}")),
    );
    indexes::unchanged(repo, &expected_indexes)?;
    guard.self_check()?;
    // Publish replacements before deleting any old object.
    let replacement_ids = indexes::write(
        repo,
        guard,
        keep.into_values().collect(),
        old_indexes
            .iter()
            .map(|e| e.name.as_str()[6..].to_owned())
            .collect(),
        limit,
    )?;
    summary.indexes_written = replacement_ids.len();
    expected_indexes.extend(replacement_ids.iter().map(|id| format!("index/{id}")));
    let snapshots = state.snapshots.keys().cloned().collect::<HashSet<_>>();
    if let Err(error) = delete::fence(repo, guard, &snapshots)
        .and_then(|()| indexes::unchanged(repo, &expected_indexes))
    {
        // Before the first deletion, withdrawing only our replacement objects
        // restores old indexes needed by a backup that ran while we slept.
        for id in replacement_ids.iter().rev() {
            repo.backend
                .delete(&crate::backend::ObjectName::new(&format!("index/{id}"))?)?;
        }
        return Err(error);
    }
    let targets = old_indexes
        .into_iter()
        .map(|entry| delete::Target::Object(entry.name))
        .chain(
            repack
                .into_iter()
                .map(|(name, _)| delete::Target::Object(name)),
        )
        .chain(remove.into_iter().map(delete::Target::Object))
        .chain(temporary.into_iter().map(delete::Target::Foreign));
    delete::run(repo, guard, &snapshots, clock, targets)?;
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

fn unknown_modified(summary: &mut Summary) {
    if summary.warnings.is_empty() {
        summary.warnings.push("modification times unavailable; retaining orphan packs and temporary names of unknown age".into());
    }
}
