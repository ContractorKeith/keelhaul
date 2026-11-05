use super::*;
use crate::{
    backend::Kind,
    crypto::envelope::Label,
    format::{config::Config, key_slot::KeySlot},
};

pub(super) fn scan<B: Backend>(state: &mut State<'_, B>) -> Result<(), Error> {
    let config = ObjectName::new("config")?;
    let config_exists = state
        .backend
        .list(Kind::Config)?
        .iter()
        .any(|e| e.name == config);
    let result = (if config_exists {
        audit::read(state.backend, &config)
    } else {
        Err(Error::Damage {
            object: "config".into(),
            problem: "missing",
            detail: "config is missing".into(),
        })
    })
    .and_then(|bytes| audit::open(&state.keys, "config", Label::Config, &bytes))
    .and_then(|bytes| Config::parse(&bytes).map_err(|e| bad("config", &e)));
    state.report.collect(result)?;
    for kind in Kind::ALL {
        for foreign in state.backend.list_foreign(kind)? {
            if !temporary(&foreign.raw) {
                state.report.note(&foreign.raw, "unknown", "unknown-object");
            }
        }
        if kind == Kind::Config {
            continue;
        }
        for entry in state.backend.list(kind)? {
            let name = &entry.name;
            if kind == Kind::Data {
                let header =
                    state
                        .report
                        .collect(audit::header(state.backend, &state.keys, name))?;
                state.packs.insert(name_id(name), header);
                continue;
            }
            if kind == Kind::Keys {
                let result = audit::read(state.backend, name)
                    .and_then(|b| KeySlot::parse(&b).map_err(|e| bad(name.as_str(), &e)))
                    .and_then(|slot| {
                        crate::crypto::slot::verify_check(&slot, &state.keys.key_slot_check)
                            .map_err(|e| Error::Damage {
                                object: name.to_string(),
                                problem: "hash-mismatch",
                                detail: e.to_string(),
                            })
                    });
                state.report.collect(result)?;
                continue;
            }
            let label = match kind {
                Kind::Index => Label::Index,
                Kind::Snapshots => Label::Snapshot,
                Kind::Locks => Label::Lock,
                _ => unreachable!("handled above"),
            };
            let object = audit::object(state.backend, &state.keys, name, label);
            if kind == Kind::Locks
                && matches!(
                    object,
                    Err(Error::Damage {
                        problem: "missing",
                        ..
                    })
                )
            {
                continue;
            }
            let Some(bytes) = state.report.collect(object)? else {
                continue;
            };
            match kind {
                Kind::Index => {
                    if let Some(index) = state.report.collect(
                        Index::parse(&bytes).map_err(|e| bad(name.as_str(), &e.to_string())),
                    )? {
                        state.indexes.insert(name.to_string(), index);
                    }
                }
                Kind::Snapshots => {
                    if let Some(snapshot) = state.report.collect(
                        Snapshot::parse(&bytes).map_err(|e| bad(name.as_str(), &e.to_string())),
                    )? {
                        state.snapshots.insert(name.to_string(), snapshot);
                    }
                }
                Kind::Locks => {
                    state
                        .report
                        .collect(super::lock::parse(&bytes).map_err(|e| bad(name.as_str(), &e)))?;
                }
                _ => unreachable!("handled above"),
            }
        }
    }
    Ok(())
}
pub(super) fn index<B: Backend>(state: &mut State<'_, B>) -> Result<(), Error> {
    let superseded: HashSet<_> = state
        .indexes
        .values()
        .flat_map(|i| i.supersedes.iter())
        .collect();
    let mut named = HashSet::new();
    for (name, index) in &state.indexes {
        if superseded.contains(&name[6..].to_string()) {
            state.report.note(name, "index", "unused-index");
            continue;
        }
        for pack in &index.packs {
            named.insert(pack.id.clone());
            match state.packs.get(&pack.id) {
                None => {
                    state.report.add(
                        crate::repo::blobs::pack_name(&pack.id)?.as_str(),
                        "pack",
                        "missing",
                        "indexed pack is missing",
                    );
                }
                Some(Some(header)) if header.blobs != pack.blobs => {
                    state.report.add(
                        name,
                        "index",
                        "index-mismatch",
                        "index differs from complete pack header",
                    );
                    // A swapped pack has an authentic header but the wrong name.
                    // Only mismatches need a full read during a structural check.
                    let object = crate::repo::blobs::pack_name(&pack.id)?;
                    if let Some(bytes) =
                        state.report.collect(audit::read(state.backend, &object))?
                        && blake3::hash(&bytes).to_hex().as_str() != pack.id
                    {
                        state.report.add(
                            object.as_str(),
                            "pack",
                            "hash-mismatch",
                            "stored pack ID differs",
                        );
                    }
                }
                _ => {}
            }
            // Keep indexed descriptors even when headers are damaged: other trees
            // and files in that pack remain independently authenticated and readable.
            for blob in &pack.blobs {
                state
                    .live
                    .entry((blob.kind, blob.id.clone()))
                    .or_default()
                    .push((pack.id.clone(), blob.clone()));
            }
            if let Some(None) = state.packs.get(&pack.id) {
                let object = crate::repo::blobs::pack_name(&pack.id)?;
                let Some(len) = state.report.collect(audit::size(state.backend, &object))? else {
                    continue;
                };
                let end = pack.blobs.last().expect("parsed packs have blobs");
                let body = end.offset + end.length;
                if len < body.saturating_add(45) {
                    state.report.add(
                        object.as_str(),
                        "pack",
                        "truncated",
                        "pack is shorter than indexed blob body and minimum header",
                    );
                }
            }
        }
    }
    for ((_, id), locations) in &state.live {
        if locations
            .iter()
            .all(|(pack, _)| !state.packs.contains_key(pack))
        {
            state.report.add(
                id,
                "blob",
                "dangling-reference",
                "blob has no available pack",
            );
        }
    }
    for id in state.packs.keys() {
        if !named.contains(id) {
            if let Some(Some(header)) = state.packs.get(id) {
                for blob in &header.blobs {
                    state
                        .live
                        .entry((blob.kind, blob.id.clone()))
                        .or_default()
                        .push((id.clone(), blob.clone()));
                }
            }
            state.report.note(
                crate::repo::blobs::pack_name(id)?.as_str(),
                "pack",
                "orphan",
            );
        }
    }
    Ok(())
}
pub(super) fn bad(object: &str, detail: &str) -> Error {
    Error::Damage {
        object: object.into(),
        problem: "bad-format",
        detail: detail.into(),
    }
}

fn temporary(raw: &str) -> bool {
    raw.rsplit_once(".tmp-").is_some_and(|(_, suffix)| {
        suffix.len() == 16
            && suffix
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}
