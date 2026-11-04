use super::*;
use crate::format::{
    snapshot::decode_root,
    tree::{Node, Tree},
};
use std::{ffi::OsString, os::unix::ffi::OsStringExt, path::PathBuf};
enum Work {
    Tree(String, PathBuf, bool),
    Leave(String),
}
pub(super) fn walk<B: Backend>(state: &mut State<'_, B>, verify: bool) -> Result<(), Error> {
    for (snapshot, s) in state.snapshots.clone() {
        let mut active = HashSet::new();
        let mut work = vec![Work::Tree(s.tree, PathBuf::new(), true)];
        while let Some(task) = work.pop() {
            let Work::Tree(id, base, root) = task else {
                if let Work::Leave(id) = task {
                    active.remove(&id);
                }
                continue;
            };
            state.used.insert((BlobKind::Tree, id.clone()));
            if !active.insert(id.clone()) {
                state
                    .report
                    .add(&id, "tree", "bad-format", "tree names an ancestor");
                continue;
            }
            work.push(Work::Leave(id.clone()));
            let Some(bytes) = state.report.collect(state.tree(&id))? else {
                continue;
            };
            let Some(tree) = state
                .report
                .collect(Tree::parse(&bytes).map_err(|e| inventory::bad(&id, &e.to_string())))?
            else {
                continue;
            };
            if root {
                let paths = tree
                    .entries
                    .iter()
                    .filter_map(|e| e.name.as_deref().and_then(|n| decode_root(n).ok()))
                    .collect::<HashSet<_>>();
                if paths != s.paths.iter().cloned().collect() {
                    state.report.add(
                        &id,
                        "tree",
                        "bad-format",
                        "root entries differ from snapshot paths",
                    );
                }
            }
            for entry in tree.entries {
                let path = if root {
                    let decoded = entry
                        .name
                        .as_deref()
                        .ok_or_else(|| inventory::bad(&id, "root name is not UTF-8"))
                        .and_then(|n| {
                            decode_root(n).map_err(|e| inventory::bad(&id, &e.to_string()))
                        });
                    let Some(decoded) = state.report.collect(decoded)? else {
                        continue;
                    };
                    if !s.paths.contains(&decoded) {
                        state.report.add(
                            &id,
                            "tree",
                            "bad-format",
                            "root entry absent from snapshot paths",
                        );
                        continue;
                    }
                    PathBuf::from(decoded)
                } else {
                    base.join(OsString::from_vec(
                        entry
                            .name_bytes()
                            .map_err(|e| inventory::bad(&id, &e.to_string()))?,
                    ))
                };
                match entry.node {
                    Node::Dir { subtree } => work.push(Work::Tree(subtree, path, false)),
                    Node::File { size, content } => {
                        let mut total = Some(0u64);
                        let mut failed = false;
                        for chunk in content {
                            let key = (BlobKind::Data, chunk.clone());
                            state.used.insert(key.clone());
                            match state.live.get(&key).and_then(|locs| {
                                locs.iter().find(|(pack, _)| state.packs.contains_key(pack))
                            }) {
                                Some((_, blob)) => {
                                    total = total.and_then(|v| v.checked_add(blob.raw_length));
                                }
                                None => {
                                    state.report.add(
                                        &chunk,
                                        "blob",
                                        "dangling-reference",
                                        "data blob has no available live index location",
                                    );
                                    failed = true;
                                    total = None;
                                }
                            }
                            if verify && state.bad.contains(&key) {
                                failed = true;
                            }
                        }
                        if total.is_some_and(|n| n != size) || (total.is_none() && !failed) {
                            state.report.damage.push(crate::damage::Damage::file(
                                &snapshot,
                                &path,
                                "size-mismatch",
                            ));
                        }
                        if verify && failed {
                            state.report.damage.push(crate::damage::Damage::file(
                                &snapshot,
                                &path,
                                "dangling-reference",
                            ));
                        }
                    }
                    Node::Symlink { .. } => {}
                }
            }
        }
    }
    Ok(())
}
