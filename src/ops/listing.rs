use crate::{
    backend::Backend,
    format::{
        pack::BlobKind,
        snapshot::decode_root,
        tree::{Entry, Node, Tree},
    },
    repo::{Error, Repository, blobs::BlobStore},
};
use serde::Serialize;
use std::{collections::HashSet, os::unix::ffi::OsStringExt, path::PathBuf};
#[derive(Debug, Serialize)]
pub struct Listed {
    pub path: String,
    #[serde(flatten)]
    pub entry: Entry,
}
pub fn ls<B: Backend>(
    repo: &Repository<B>,
    selector: &str,
    filter: Option<&str>,
) -> Result<Vec<Listed>, Error> {
    let (_, snapshot) = repo.select_snapshot(selector)?;
    let mut store = BlobStore::load(repo)?;
    let mut output = Vec::new();
    walk(
        &mut store,
        &snapshot.tree,
        PathBuf::new(),
        true,
        &mut HashSet::new(),
        &mut output,
    )?;
    if let Some(filter) = filter {
        let path = PathBuf::from(filter.trim_start_matches('/'));
        output.retain(|e| PathBuf::from(e.path.trim_start_matches('/')).starts_with(&path));
    }
    Ok(output)
}
fn walk<B: Backend>(
    store: &mut BlobStore<'_, B>,
    id: &str,
    base: PathBuf,
    root: bool,
    active: &mut HashSet<String>,
    out: &mut Vec<Listed>,
) -> Result<(), Error> {
    if active.len() >= 256 || !active.insert(id.into()) {
        return Err(damage(id, "cyclic or excessively deep tree"));
    }
    let tree =
        Tree::parse(&store.read(BlobKind::Tree, id)?).map_err(|e| damage(id, &e.to_string()))?;
    for entry in tree.entries {
        let path = if root {
            PathBuf::from(
                decode_root(
                    entry
                        .name
                        .as_deref()
                        .ok_or_else(|| damage(id, "invalid root name"))?,
                )
                .map_err(|e| damage(id, &e.to_string()))?
                .trim_start_matches('/'),
            )
        } else {
            base.join(std::ffi::OsString::from_vec(
                entry.name_bytes().map_err(|e| damage(id, &e.to_string()))?,
            ))
        };
        if let Node::Dir { subtree } = &entry.node {
            walk(store, subtree, path.clone(), false, active, out)?;
        }
        out.push(Listed {
            path: format!("/{}", path.display()),
            entry,
        });
    }
    active.remove(id);
    Ok(())
}
pub(super) fn damage(object: &str, detail: &str) -> Error {
    Error::Damage {
        object: object.into(),
        problem: "bad-format",
        detail: detail.into(),
    }
}
