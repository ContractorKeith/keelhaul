//! Partial replacements are additive. Only the final object supersedes old indexes.
use crate::{
    backend::{Backend, Kind},
    crypto::envelope::Label,
    format::index::{Index, Pack},
    repo::{Error, Repository, lock::Guard},
};
use std::collections::HashSet;
pub(super) const LIMIT: usize = 16 * 1024 * 1024;
pub(super) fn unchanged<B: Backend>(
    repo: &Repository<B>,
    expected: &HashSet<String>,
) -> Result<(), Error> {
    if repo
        .backend
        .list(Kind::Index)?
        .iter()
        .any(|e| !expected.contains(e.name.as_str()))
    {
        return Err(Error::Refused(
            "an index appeared during prune; rerun prune".into(),
        ));
    }
    Ok(())
}
pub(super) fn write<B: Backend>(
    repo: &Repository<B>,
    guard: &Guard<'_, B>,
    packs: Vec<Pack>,
    supersedes: Vec<String>,
    limit: usize,
) -> Result<Vec<String>, Error> {
    if limit == 0 || limit > LIMIT {
        return Err(Error::Refused(
            "index payload limit must be 1–16 MiB".into(),
        ));
    }
    let mut index = Index {
        version: 1,
        packs: Vec::new(),
        supersedes: Vec::new(),
    };
    let empty = serde_json::to_vec(&index)?.len();
    let final_empty = serde_json::to_vec(&Index {
        version: 1,
        packs: Vec::new(),
        supersedes: supersedes.clone(),
    })?
    .len();
    if final_empty > limit {
        return Err(Error::Refused(
            "supersedes list exceeds index payload limit".into(),
        ));
    }
    let mut size = empty;
    let mut written = Vec::new();
    for pack in packs {
        let bytes = serde_json::to_vec(&pack)?.len();
        if bytes + empty > limit {
            return Err(Error::Refused(
                "one pack's descriptors exceed index payload limit".into(),
            ));
        }
        let comma = usize::from(!index.packs.is_empty());
        if size + bytes + comma > limit {
            publish(repo, guard, &index, limit, &mut written)?;
            index.packs.clear();
            size = empty;
        }
        size += bytes + usize::from(!index.packs.is_empty());
        index.packs.push(pack);
    }
    if size - empty + final_empty > limit {
        publish(repo, guard, &index, limit, &mut written)?;
        index.packs.clear();
    }
    index.supersedes = supersedes;
    publish(repo, guard, &index, limit, &mut written)?;
    Ok(written)
}
fn publish<B: Backend>(
    repo: &Repository<B>,
    guard: &Guard<'_, B>,
    index: &Index,
    limit: usize,
    written: &mut Vec<String>,
) -> Result<(), Error> {
    let bytes = serde_json::to_vec(index)?;
    if bytes.len() > limit {
        return Err(Error::Refused("index payload exceeds split limit".into()));
    }
    guard.self_check()?;
    written.push(repo.write_object("index", Label::Index, &bytes)?);
    Ok(())
}
