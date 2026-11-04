use crate::{
    backend::Backend,
    format::{
        pack::BlobKind,
        snapshot::decode_root,
        tree::{Entry, Node, Tree},
    },
    repo::{Error, Repository, blobs::BlobStore},
};
use base64::{Engine, prelude::BASE64_STANDARD};
use serde::Serialize;
use std::{
    collections::HashSet,
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};
#[derive(Debug, Serialize)]
pub struct Listed {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_b64: Option<String>,
    #[serde(skip)]
    raw_path: PathBuf,
    #[serde(flatten)]
    pub entry: Entry,
}
pub fn ls<B: Backend>(
    repo: &Repository<B>,
    selector: &str,
    filter: Option<&Path>,
) -> Result<Vec<Listed>, Error> {
    ls_for_host(repo, selector, filter, None)
}
pub fn ls_for_host<B: Backend>(
    repo: &Repository<B>,
    selector: &str,
    filter: Option<&Path>,
    host: Option<&str>,
) -> Result<Vec<Listed>, Error> {
    let (_, snapshot) = repo.select_snapshot_for_host(selector, host)?;
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
        let path = if filter.is_absolute() {
            filter.to_path_buf()
        } else {
            Path::new("/").join(filter)
        };
        output.retain(|e| e.raw_path.starts_with(&path));
    }
    Ok(output)
}
enum Work {
    Tree(String, PathBuf, bool),
    Emit(Box<Listed>),
    Leave(String),
}
fn walk<B: Backend>(
    store: &mut BlobStore<'_, B>,
    id: &str,
    base: PathBuf,
    root: bool,
    active: &mut HashSet<String>,
    out: &mut Vec<Listed>,
) -> Result<(), Error> {
    let mut work = vec![Work::Tree(id.into(), base, root)];
    while let Some(task) = work.pop() {
        match task {
            Work::Leave(id) => {
                active.remove(&id);
            }
            Work::Emit(entry) => out.push(*entry),
            Work::Tree(id, base, root) => {
                if !active.insert(id.clone()) {
                    return Err(damage(&id, "tree names an ancestor"));
                }
                let tree = Tree::parse(&store.read(BlobKind::Tree, &id)?)
                    .map_err(|e| damage(&id, &e.to_string()))?;
                tree.validate_content_lengths(|id| {
                    store
                        .raw_length(BlobKind::Data, id)
                        .map_err(|e| crate::format::ParseError(e.to_string()))
                })
                .map_err(|e| damage(&id, &e.to_string()))?;
                work.push(Work::Leave(id.clone()));
                for entry in tree.entries.into_iter().rev() {
                    let path = if root {
                        PathBuf::from(
                            decode_root(
                                entry
                                    .name
                                    .as_deref()
                                    .ok_or_else(|| damage(&id, "invalid root name"))?,
                            )
                            .map_err(|e| damage(&id, &e.to_string()))?
                            .trim_start_matches('/'),
                        )
                    } else {
                        base.join(std::ffi::OsString::from_vec(
                            entry
                                .name_bytes()
                                .map_err(|e| damage(&id, &e.to_string()))?,
                        ))
                    };
                    let subtree = if let Node::Dir { subtree } = &entry.node {
                        Some(subtree.clone())
                    } else {
                        None
                    };
                    let raw_path = Path::new("/").join(&path);
                    let path_b64 = raw_path
                        .to_str()
                        .is_none()
                        .then(|| BASE64_STANDARD.encode(raw_path.as_os_str().as_bytes()));
                    let listed = Listed {
                        path: raw_path.to_string_lossy().into_owned(),
                        path_b64,
                        raw_path,
                        entry,
                    };
                    if let Some(subtree) = subtree {
                        work.push(Work::Tree(subtree, path, false));
                    }
                    work.push(Work::Emit(Box::new(listed)));
                }
            }
        }
    }
    Ok(())
}
pub(super) fn damage(object: &str, detail: &str) -> Error {
    Error::Damage {
        object: object.into(),
        problem: "bad-format",
        detail: detail.into(),
    }
}
