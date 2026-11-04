//! Retain the snapshot's indexed pack set across the publication window.
use crate::{
    backend::{Backend, Kind, ObjectName},
    format::{
        pack::BlobKind,
        tree::{Node, Tree},
    },
    repo::{Error, Repository, blobs::BlobStore},
};
use std::collections::HashSet;

pub(super) fn referenced_packs<B: Backend>(
    store: &mut BlobStore<'_, B>,
    root: &str,
) -> Result<HashSet<String>, Error> {
    let mut packs = HashSet::new();
    let mut visited = HashSet::new();
    let mut work = vec![root.to_owned()];
    while let Some(id) = work.pop() {
        if !visited.insert(id.clone()) {
            continue;
        }
        record(store, BlobKind::Tree, &id, &mut packs)?;
        let bytes = store.read(BlobKind::Tree, &id)?;
        let tree = Tree::parse(&bytes).map_err(|e| Error::Damage {
            object: id,
            problem: "bad-format",
            detail: e.to_string(),
        })?;
        for entry in tree.entries {
            match entry.node {
                Node::Dir { subtree } => work.push(subtree),
                Node::File { content, .. } => {
                    for id in content {
                        record(store, BlobKind::Data, &id, &mut packs)?;
                    }
                }
                Node::Symlink { .. } => {}
            }
        }
    }
    Ok(packs)
}
fn record<B>(
    store: &BlobStore<'_, B>,
    kind: BlobKind,
    id: &str,
    packs: &mut HashSet<String>,
) -> Result<(), Error> {
    let location = store
        .index
        .get(&(kind, id.to_owned()))
        .ok_or_else(|| Error::Damage {
            object: id.into(),
            problem: "dangling-reference",
            detail: "snapshot blob has no live index location".into(),
        })?;
    packs.insert(location.pack.clone());
    Ok(())
}
pub(super) fn check<B: Backend>(
    repo: &Repository<B>,
    snapshot: &str,
    expected: &HashSet<String>,
) -> Result<(), Error> {
    let result = (|| {
        let present = repo
            .backend
            .list(Kind::Data)?
            .into_iter()
            .map(|e| e.name.as_str()[8..].to_owned())
            .collect::<HashSet<_>>();
        if !expected.is_subset(&present) {
            return Err(Error::Refused(
                "repository was pruned during this backup; rerun".into(),
            ));
        }
        Ok(())
    })();
    if result.is_err() {
        repo.backend
            .delete(&ObjectName::new(&format!("snapshots/{snapshot}"))?)?;
    }
    result
}
