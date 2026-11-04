//! Physical duplicate damage is recoverable; an unreadable logical graph is not.
use super::super::check::State;
use crate::{
    backend::Backend,
    format::{
        pack::{Blob, BlobKind},
        tree::Tree,
    },
    repo::{Error, Repository, audit, blobs::pack_name},
};
use std::collections::{HashMap, HashSet};
pub(super) struct Reachable<'a, B> {
    pub state: State<'a, B>,
    pub selected: HashMap<String, Vec<Blob>>,
    pub force_repack: HashSet<String>,
}
fn failure(damage: &crate::damage::Damage) -> Error {
    Error::Damage {
        object: damage.object.clone(),
        problem: "bad-format",
        detail: damage.to_string(),
    }
}
pub(super) fn scan<B: Backend>(repo: &Repository<B>) -> Result<Reachable<'_, B>, Error> {
    let mut state = State::new(&repo.backend, repo.subkeys());
    state.metadata()?;
    if let Some(damage) = state
        .report
        .damage
        .iter()
        .find(|d| d.object.starts_with("snapshots/"))
    {
        return Err(failure(damage));
    }
    let metadata_end = state.report.damage.len();
    state.reachable()?;
    if let Some(damage) = state.report.damage[metadata_end..]
        .iter()
        .find(|d| !state.tree_copy_damage.contains(*d))
    {
        return Err(failure(damage));
    }
    let mut selected: HashMap<String, Vec<Blob>> = HashMap::new();
    let mut force_repack = HashSet::new();
    let mut used = state.used.iter().collect::<Vec<_>>();
    used.sort_by(|a, b| a.1.cmp(&b.1));
    for key in used {
        let locations = state.live.get(key).ok_or_else(|| Error::Damage {
            object: key.1.clone(),
            problem: "dangling-reference",
            detail: "no indexed copy".into(),
        })?;
        let mut verified = None;
        let mut failed = None;
        for (pack, indexed) in locations {
            let blob = match state.packs.get(pack).and_then(|h| h.as_ref()) {
                Some(header) => match header
                    .blobs
                    .iter()
                    .find(|b| b.kind == key.0 && b.id == key.1)
                {
                    Some(actual) => actual,
                    None => {
                        force_repack.insert(pack.clone());
                        failed = Some(Error::Damage {
                            object: pack_name(pack)?.to_string(),
                            problem: "index-mismatch",
                            detail: "used blob is absent from authenticated pack header".into(),
                        });
                        continue;
                    }
                },
                None => indexed,
            };
            let name = pack_name(pack)?;
            let result = audit::range(&repo.backend, &name, blob.offset, blob.length)
                .and_then(|bytes| audit::blob(&state.keys, &blob.id, name.as_str(), blob, &bytes))
                .and_then(|bytes| {
                    if key.0 == BlobKind::Tree {
                        Tree::parse(&bytes).map_err(|e| Error::Damage {
                            object: key.1.clone(),
                            problem: "bad-format",
                            detail: e.to_string(),
                        })?;
                    }
                    Ok(())
                });
            match result {
                Ok(()) => {
                    verified = Some((pack.clone(), blob.clone()));
                    break;
                }
                Err(e @ Error::Damage { .. }) => {
                    force_repack.insert(pack.clone());
                    failed = Some(e);
                }
                Err(e) => return Err(e),
            }
        }
        let location = verified.ok_or_else(|| {
            failed.unwrap_or_else(|| Error::Damage {
                object: key.1.clone(),
                problem: "missing",
                detail: "no verified copy".into(),
            })
        })?;
        if locations.len() > 1 {
            force_repack.extend(locations.iter().map(|(pack, _)| pack.clone()));
        }
        selected.entry(location.0).or_default().push(location.1);
    }
    Ok(Reachable {
        state,
        selected,
        force_repack,
    })
}
