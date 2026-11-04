use super::Report;
use crate::ops::listing::damage;
use crate::{
    backend::Backend,
    format::{
        pack::BlobKind,
        snapshot::{Snapshot, decode_root},
        tree::{Entry, Node, Tree},
    },
    repo::{Error, blobs::BlobStore},
};
use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    os::unix::ffi::OsStringExt,
    path::{Component, Path, PathBuf},
};
pub(super) struct Item {
    pub path: PathBuf,
    pub entry: Entry,
    pub object: String,
}
struct Planner<'a> {
    snapshot: &'a Snapshot,
    includes: Vec<PathBuf>,
    active: HashSet<String>,
    items: Vec<Item>,
    report: &'a mut Report,
}
impl Planner<'_> {
    fn walk<B: Backend>(
        &mut self,
        store: &mut BlobStore<'_, B>,
        id: &str,
        base: &Path,
        root: bool,
    ) -> Result<bool, Error> {
        if self.active.len() >= 256 || !self.active.insert(id.into()) {
            self.report
                .damage(damage(id, "cyclic or excessively deep tree"))?;
            return Ok(false);
        }
        let result = (|| {
            let bytes = store.read(BlobKind::Tree, id)?;
            let (tree, problems) = Tree::salvage(&bytes).map_err(|e| damage(id, &e.to_string()))?;
            for problem in problems {
                self.report.damage(damage(id, &problem))?;
            }
            for entry in tree.entries {
                let path = if root {
                    let result = entry
                        .name
                        .as_deref()
                        .ok_or_else(|| damage(id, "root name is not UTF-8"))
                        .and_then(|name| decode_root(name).map_err(|e| damage(id, &e.to_string())));
                    let decoded = match result {
                        Ok(p) => p,
                        Err(e) => {
                            self.report.damage(e)?;
                            continue;
                        }
                    };
                    if !self.snapshot.paths.contains(&decoded) {
                        self.report
                            .damage(damage(id, "root entry absent from snapshot paths"))?;
                        continue;
                    }
                    PathBuf::from(decoded.trim_start_matches('/'))
                } else {
                    base.join(OsString::from_vec(
                        entry.name_bytes().map_err(|e| damage(id, &e.to_string()))?,
                    ))
                };
                let selected =
                    self.includes.is_empty() || self.includes.iter().any(|i| path.starts_with(i));
                let ancestor = self.includes.iter().any(|i| i.starts_with(&path));
                if !selected && !ancestor {
                    continue;
                }
                let subtree = if let Node::Dir { subtree } = &entry.node {
                    Some(subtree.clone())
                } else {
                    None
                };
                let readable = match subtree {
                    Some(subtree) => self.walk(store, &subtree, &path, false)?,
                    None => true,
                };
                if readable && (selected || matches!(entry.node, Node::Dir { .. })) {
                    self.items.push(Item {
                        path,
                        entry,
                        object: id.into(),
                    });
                }
            }
            Ok(())
        })();
        self.active.remove(id);
        match result {
            Ok(()) => Ok(true),
            Err(e) => {
                self.report.damage(e)?;
                Ok(false)
            }
        }
    }
}
pub(super) fn build<B: Backend>(
    store: &mut BlobStore<'_, B>,
    snapshot: &Snapshot,
    includes: &[PathBuf],
    report: &mut Report,
) -> Result<Vec<Item>, Error> {
    let mut normalized = Vec::new();
    for path in includes {
        if path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::Prefix(_)))
        {
            return Err(Error::Refused("include path must not contain '..'".into()));
        }
        normalized.push(
            path.components()
                .filter(|c| matches!(c, Component::Normal(_)))
                .collect(),
        );
    }
    let mut planner = Planner {
        snapshot,
        includes: normalized,
        active: HashSet::new(),
        items: Vec::new(),
        report,
    };
    planner.walk(store, &snapshot.tree, Path::new(""), true)?;
    // Overlapping backup roots can describe the same destination; require agreement.
    let mut unique: HashMap<PathBuf, usize> = HashMap::new();
    let mut items: Vec<Item> = Vec::new();
    for item in planner.items {
        if let Some(&i) = unique.get(&item.path) {
            let mut a = serde_json::to_value(&items[i].entry)?;
            let mut b = serde_json::to_value(&item.entry)?;
            for value in [&mut a, &mut b] {
                if let Some(m) = value.as_object_mut() {
                    m.remove("name");
                    m.remove("name_b64");
                }
            }
            if a != b {
                return Err(damage(&item.object, "overlapping roots disagree"));
            }
        } else {
            unique.insert(item.path.clone(), items.len());
            items.push(item);
        }
    }
    items.sort_by(|a, b| a.path.cmp(&b.path));
    let directories = items
        .iter()
        .map(|i| (&i.path, matches!(i.entry.node, Node::Dir { .. })))
        .collect::<HashMap<_, _>>();
    for item in &items {
        for ancestor in item.path.ancestors().skip(1) {
            if directories.get(&ancestor.to_path_buf()) == Some(&false) {
                return Err(damage(
                    &item.object,
                    "non-directory ancestor in restore plan",
                ));
            }
        }
    }
    Ok(items)
}
pub(super) fn preflight(target: &Path, items: &[Item]) -> Result<(), Error> {
    match std::fs::symlink_metadata(target) {
        Ok(m) if !m.is_dir() || m.is_symlink() => {
            return Err(Error::Refused(format!(
                "restore target is not a directory: {}",
                target.display()
            )));
        }
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
        _ => {}
    }
    for item in items {
        let mut path = target.to_path_buf();
        for part in item.path.components() {
            path.push(part.as_os_str());
            match std::fs::symlink_metadata(&path) {
                Ok(_) => {
                    return Err(Error::Refused(format!(
                        "restore conflict: {}",
                        path.display()
                    )));
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
    }
    Ok(())
}
