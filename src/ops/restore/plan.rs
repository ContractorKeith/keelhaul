use super::Report;
use crate::ops::listing::damage;
use crate::{
    backend::Backend,
    format::{
        snapshot::Snapshot,
        tree::{Entry, Node},
    },
    repo::{Error, blobs::BlobStore, trees::TreeCache},
};
use std::{
    collections::HashSet,
    ffi::OsString,
    os::unix::ffi::OsStringExt,
    path::{Component, PathBuf},
};
pub(super) struct Item {
    pub path: PathBuf,
    pub entry: Entry,
    pub object: String,
}
struct Planner<'a> {
    snapshot: &'a Snapshot,
    blocked: Vec<PathBuf>,
    includes: Vec<PathBuf>,
    matched: HashSet<PathBuf>,
    active: HashSet<String>,
    cache: TreeCache,
    items: Vec<Item>,
    report: &'a mut Report,
}
enum Work {
    Tree {
        id: String,
        base: PathBuf,
        root: bool,
        owner: Option<Item>,
    },
    Entry {
        entry: Entry,
        base: PathBuf,
        root: bool,
        object: String,
    },
    Leave(String),
}
impl Planner<'_> {
    fn walk<B: Backend>(&mut self, store: &mut BlobStore<'_, B>, id: &str) -> Result<(), Error> {
        let mut work = vec![Work::Tree {
            id: id.into(),
            base: PathBuf::new(),
            root: true,
            owner: None,
        }];
        while let Some(task) = work.pop() {
            match task {
                Work::Leave(id) => {
                    self.active.remove(&id);
                }
                Work::Tree {
                    id,
                    base,
                    root,
                    owner,
                } => {
                    if self.active.contains(&id) {
                        self.report.damage(damage(&id, "tree names an ancestor"))?;
                        continue;
                    }
                    let parsed = self.cache.salvage(store, &id);
                    let (tree, problems) = match parsed {
                        Ok(t) => t,
                        Err(error) => {
                            match crate::damage::Damage::from_error(error) {
                                Ok(damage) => self.report.damaged.push(damage),
                                Err(error) => self.report.failure(&base, error),
                            }
                            continue;
                        }
                    };
                    for problem in problems {
                        self.report.damage(damage(&id, &problem))?;
                    }
                    let mut tree = (*tree).clone();
                    if root {
                        let shape = crate::format::snapshot_shape::validate(self.snapshot, &tree);
                        if !shape.problems.is_empty() {
                            self.report.damaged.push(crate::damage::Damage::new(
                                &id,
                                "tree",
                                "bad-format",
                                &shape.problems.join("; "),
                            ));
                        }
                        self.blocked = shape.blocked;
                        tree.entries = tree
                            .entries
                            .into_iter()
                            .zip(shape.roots)
                            .filter_map(|(mut entry, path)| {
                                path.map(|path| {
                                    entry.name = Some(path);
                                    entry
                                })
                            })
                            .collect();
                    }
                    if let Some(item) = owner {
                        self.items.push(item);
                    }
                    self.active.insert(id.clone());
                    work.push(Work::Leave(id.clone()));
                    for entry in tree.entries.iter().rev().cloned() {
                        work.push(Work::Entry {
                            entry,
                            base: base.clone(),
                            root,
                            object: id.clone(),
                        });
                    }
                }
                Work::Entry {
                    entry,
                    base,
                    root,
                    object,
                } => {
                    let path = if root {
                        let decoded = entry
                            .name
                            .as_deref()
                            .expect("shared validator supplies decoded root names");
                        PathBuf::from(decoded.trim_start_matches('/'))
                    } else {
                        base.join(OsString::from_vec(
                            entry
                                .name_bytes()
                                .map_err(|e| damage(&object, &e.to_string()))?,
                        ))
                    };
                    if self.blocked.iter().any(|blocked| path.starts_with(blocked)) {
                        continue;
                    }
                    if self.includes.contains(&path) {
                        self.matched.insert(path.clone());
                    }
                    let selected = self.includes.is_empty()
                        || self.includes.iter().any(|i| path.starts_with(i));
                    let ancestor = self.includes.iter().any(|i| i.starts_with(&path));
                    if !selected && !ancestor {
                        continue;
                    }
                    let subtree = if let Node::Dir { subtree } = &entry.node {
                        Some(subtree.clone())
                    } else {
                        None
                    };
                    let item = Item {
                        path: path.clone(),
                        entry,
                        object,
                    };
                    if let Some(id) = subtree {
                        work.push(Work::Tree {
                            id,
                            base: path,
                            root: false,
                            owner: Some(item),
                        });
                    } else if selected {
                        self.items.push(item);
                    }
                }
            }
        }
        Ok(())
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
            crate::ops::paths::normalize_root(path)
                .map_err(|error| Error::Refused(format!("{}: {error}", path.display())))?
                .components()
                .filter(|c| matches!(c, Component::Normal(_)))
                .collect(),
        );
    }
    let mut planner = Planner {
        snapshot,
        blocked: Vec::new(),
        includes: normalized,
        matched: HashSet::new(),
        active: HashSet::new(),
        cache: TreeCache::default(),
        items: Vec::new(),
        report,
    };
    planner.walk(store, &snapshot.tree)?;
    for (requested, normalized) in includes.iter().zip(&planner.includes) {
        if !planner.matched.contains(normalized) {
            return Err(Error::Refused(format!(
                "include path not found: {}",
                requested.display()
            )));
        }
    }
    let mut items = planner.items;
    items.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(items)
}
