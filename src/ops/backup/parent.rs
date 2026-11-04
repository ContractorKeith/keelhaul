use super::Options;
use crate::{
    backend::Backend,
    format::{
        pack::BlobKind,
        tree::{Entry, Node, Tree},
    },
    repo::{Error, Repository, blobs::BlobStore},
};
use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    os::unix::ffi::OsStringExt,
    path::PathBuf,
};
pub(super) type Previous = HashMap<PathBuf, Entry>;
pub(super) fn load<B: Backend>(
    repo: &Repository<B>,
    store: &mut BlobStore<'_, B>,
    host: &str,
    paths: &[String],
    options: &Options,
) -> Result<(Option<String>, Previous), Error> {
    let selected = match &options.parent {
        Some(selector) if selector != "latest" => Some(repo.select_snapshot(selector)?),
        _ => repo
            .snapshots()?
            .into_iter()
            .rev()
            .find(|(_, s)| s.host == host && s.paths == paths),
    };
    let Some((id, snapshot)) = selected else {
        return Ok((None, HashMap::new()));
    };
    match read(store, &snapshot.tree) {
        Ok(previous) => Ok((Some(id), previous)),
        Err(error) => {
            eprintln!("warning: parent {id} unavailable; reading all files: {error}");
            Ok((None, HashMap::new()))
        }
    }
}
fn read<B: Backend>(store: &mut BlobStore<'_, B>, root: &str) -> Result<Previous, Error> {
    let mut result = HashMap::new();
    let mut work = vec![(PathBuf::new(), root.to_string(), HashSet::new())];
    while let Some((path, id, mut ancestors)) = work.pop() {
        if !ancestors.insert(id.clone()) {
            return Err(Error::Refused("parent tree cycle".into()));
        }
        let bytes = store.read_parent_tree(&id)?;
        let tree = Tree::parse(&bytes).map_err(|e| {
            store.suspect_blob(BlobKind::Tree, &id);
            Error::Refused(e.to_string())
        })?;
        for entry in tree.entries {
            let name = entry
                .name_bytes()
                .map_err(|e| Error::Refused(e.to_string()))?;
            let child = if path.as_os_str().is_empty() {
                let text = std::str::from_utf8(&name).map_err(|e| Error::Refused(e.to_string()))?;
                PathBuf::from(
                    crate::format::snapshot::decode_root(text)
                        .map_err(|e| Error::Refused(e.to_string()))?,
                )
            } else {
                path.join(OsString::from_vec(name))
            };
            entry
                .validate_content_lengths(|chunk| {
                    store
                        .raw_length(BlobKind::Data, chunk)
                        .map_err(|e| crate::format::ParseError(e.to_string()))
                })
                .map_err(|e| Error::Refused(e.to_string()))?;
            if let Node::Dir { subtree } = &entry.node {
                work.push((child.clone(), subtree.clone(), ancestors.clone()));
            }
            result.insert(child, entry);
        }
    }
    Ok(result)
}
