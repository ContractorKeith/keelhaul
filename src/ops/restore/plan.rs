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
    path::{Component, PathBuf},
};
pub(super) struct Item {
    pub path: PathBuf,
    pub entry: Entry,
    pub object: String,
}
struct Planner<'a> {
    snapshot: &'a Snapshot,
    includes: Vec<PathBuf>,
    matched: HashSet<PathBuf>,
    active: HashSet<String>,
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
                    let parsed = store.read(BlobKind::Tree, &id).and_then(|bytes| {
                        Tree::salvage(&bytes).map_err(|e| damage(&id, &e.to_string()))
                    });
                    let (tree, problems) = match parsed {
                        Ok(t) => t,
                        Err(e) => {
                            self.report.damage(e)?;
                            continue;
                        }
                    };
                    for problem in problems {
                        self.report.damage(damage(&id, &problem))?;
                    }
                    if let Some(item) = owner {
                        self.items.push(item);
                    }
                    self.active.insert(id.clone());
                    work.push(Work::Leave(id.clone()));
                    for entry in tree.entries.into_iter().rev() {
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
                            .ok_or_else(|| damage(&object, "root name is not UTF-8"))
                            .and_then(|name| {
                                decode_root(name).map_err(|e| damage(&object, &e.to_string()))
                            });
                        let decoded = match decoded {
                            Ok(p) => p,
                            Err(e) => {
                                self.report.damage(e)?;
                                continue;
                            }
                        };
                        if !self.snapshot.paths.contains(&decoded) {
                            self.report
                                .damage(damage(&object, "root entry absent from snapshot paths"))?;
                            continue;
                        }
                        PathBuf::from(decoded.trim_start_matches('/'))
                    } else {
                        base.join(OsString::from_vec(
                            entry
                                .name_bytes()
                                .map_err(|e| damage(&object, &e.to_string()))?,
                        ))
                    };
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
            path.components()
                .filter(|c| matches!(c, Component::Normal(_)))
                .collect(),
        );
    }
    let mut planner = Planner {
        snapshot,
        includes: normalized,
        matched: HashSet::new(),
        active: HashSet::new(),
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
