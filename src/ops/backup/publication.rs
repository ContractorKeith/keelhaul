//! Retain the snapshot's referenced blob set across the publication window.
use crate::{
    backend::{Backend, Kind, ObjectName},
    format::{
        pack::BlobKind,
        tree::{Node, Tree},
    },
    repo::{Error, Repository, blobs::BlobStore},
};
use std::collections::HashSet;

pub(super) fn referenced_blobs<B: Backend>(
    store: &mut BlobStore<'_, B>,
    root: &str,
) -> Result<HashSet<(BlobKind, String)>, Error> {
    let mut blobs = HashSet::new();
    let mut visited = HashSet::new();
    let mut work = vec![root.to_owned()];
    while let Some(id) = work.pop() {
        if !visited.insert(id.clone()) {
            continue;
        }
        record(store, BlobKind::Tree, &id, &mut blobs)?;
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
                        record(store, BlobKind::Data, &id, &mut blobs)?;
                    }
                }
                Node::Symlink { .. } => {}
            }
        }
    }
    Ok(blobs)
}
fn record<B>(
    store: &BlobStore<'_, B>,
    kind: BlobKind,
    id: &str,
    blobs: &mut HashSet<(BlobKind, String)>,
) -> Result<(), Error> {
    if !store.index.contains_key(&(kind, id.to_owned())) {
        return Err(Error::Damage {
            object: id.into(),
            problem: "dangling-reference",
            detail: "snapshot blob has no live index location".into(),
        });
    }
    blobs.insert((kind, id.to_owned()));
    Ok(())
}

pub(super) fn check<B: Backend>(
    repo: &Repository<B>,
    snapshot: &str,
    expected: &HashSet<(BlobKind, String)>,
) -> Result<(), Error> {
    let result = (|| {
        let live = BlobStore::load(repo)?;
        let present = repo
            .backend
            .list(Kind::Data)?
            .into_iter()
            .map(|e| e.name.as_str()[8..].to_owned())
            .collect::<HashSet<_>>();
        if !live.indexed_blobs_exist(expected, &present) {
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
