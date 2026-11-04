//! Audit the DAG once; expand only branches that need per-file damage attribution.
use super::*;
use crate::format::{
    snapshot::decode_root,
    tree::{Entry, Node},
};
use std::{
    collections::VecDeque,
    ffi::OsString,
    os::unix::ffi::OsStringExt,
    path::{Path, PathBuf},
};
type Issues = HashMap<String, Vec<(usize, &'static str)>>;
enum Work {
    Enter(String),
    Leave(String),
}
pub(super) fn walk<B: Backend>(state: &mut State<'_, B>, verify: bool) -> Result<(), Error> {
    let mut done = HashSet::new();
    let mut parents: HashMap<String, HashSet<String>> = HashMap::new();
    let mut issues = Issues::new();
    for (_, s) in state.snapshots.clone() {
        if let Some(tree) = state.tree(&s.tree)? {
            let paths = tree
                .entries
                .iter()
                .filter_map(|e| e.name.as_deref().and_then(|n| decode_root(n).ok()))
                .collect::<HashSet<_>>();
            if paths != s.paths.iter().cloned().collect() {
                state.report.add(
                    &s.tree,
                    "tree",
                    "bad-format",
                    "root entries differ from snapshot paths",
                );
            }
        }
        let mut active = HashSet::new();
        let mut work = vec![Work::Enter(s.tree)];
        while let Some(task) = work.pop() {
            let id = match task {
                Work::Leave(id) => {
                    active.remove(&id);
                    done.insert(id);
                    continue;
                }
                Work::Enter(id) => id,
            };
            state.used.insert((BlobKind::Tree, id.clone()));
            if active.contains(&id) {
                state
                    .report
                    .add(&id, "tree", "bad-format", "tree names an ancestor");
                continue;
            }
            if done.contains(&id) {
                continue;
            }
            let Some(tree) = state.tree(&id)? else {
                done.insert(id);
                continue;
            };
            active.insert(id.clone());
            work.push(Work::Leave(id.clone()));
            for (i, entry) in tree.entries.iter().enumerate() {
                match &entry.node {
                    Node::Dir { subtree } => {
                        parents
                            .entry(subtree.clone())
                            .or_default()
                            .insert(id.clone());
                        work.push(Work::Enter(subtree.clone()));
                    }
                    Node::File { size, content } => {
                        let mut total = Some(0u64);
                        let mut failed = false;
                        for chunk in content {
                            let key = (BlobKind::Data, chunk.clone());
                            state.used.insert(key.clone());
                            match state
                                .live
                                .get(&key)
                                .and_then(|l| l.iter().find(|(p, _)| state.packs.contains_key(p)))
                            {
                                Some((_, blob)) => {
                                    total = total.and_then(|n| n.checked_add(blob.raw_length))
                                }
                                None => {
                                    state.report.add(
                                        chunk,
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
                        if total.is_some_and(|n| n != *size) || (total.is_none() && !failed) {
                            issues
                                .entry(id.clone())
                                .or_default()
                                .push((i, "size-mismatch"));
                        }
                        if verify && failed {
                            issues
                                .entry(id.clone())
                                .or_default()
                                .push((i, "dangling-reference"));
                        }
                    }
                    Node::Symlink { .. } => {}
                }
            }
        }
    }
    // Mark ancestors of affected files, so a healthy deduplicated DAG costs O(V+E).
    let mut affected = issues.keys().cloned().collect::<HashSet<_>>();
    let mut queue = affected.iter().cloned().collect::<VecDeque<_>>();
    while let Some(id) = queue.pop_front() {
        for parent in parents.get(&id).into_iter().flatten() {
            if affected.insert(parent.clone()) {
                queue.push_back(parent.clone());
            }
        }
    }
    for (snapshot, s) in state.snapshots.clone() {
        if !affected.contains(&s.tree) {
            continue;
        }
        enum PathWork {
            Enter(String, PathBuf, bool),
            Leave(String),
        }
        let mut active = HashSet::new();
        let mut work = vec![PathWork::Enter(s.tree, PathBuf::new(), true)];
        while let Some(task) = work.pop() {
            let (id, base, root) = match task {
                PathWork::Leave(id) => {
                    active.remove(&id);
                    continue;
                }
                PathWork::Enter(id, p, r) => (id, p, r),
            };
            if !active.insert(id.clone()) {
                continue;
            }
            work.push(PathWork::Leave(id.clone()));
            let Some(tree) = state.tree(&id)? else {
                continue;
            };
            for (i, entry) in tree.entries.iter().enumerate() {
                let Some(path) = entry_path(state, &id, entry, &base, root, &s.paths)? else {
                    continue;
                };
                if let Some(problems) = issues.get(&id) {
                    for (_, problem) in problems.iter().filter(|(j, _)| *j == i) {
                        state
                            .report
                            .damage
                            .push(crate::damage::Damage::file(&snapshot, &path, problem));
                    }
                }
                if let Node::Dir { subtree } = &entry.node
                    && affected.contains(subtree)
                {
                    work.push(PathWork::Enter(subtree.clone(), path, false));
                }
            }
        }
    }
    Ok(())
}
fn entry_path<B: Backend>(
    state: &mut State<'_, B>,
    id: &str,
    entry: &Entry,
    base: &Path,
    root: bool,
    paths: &[String],
) -> Result<Option<PathBuf>, Error> {
    if root {
        let decoded = entry
            .name
            .as_deref()
            .ok_or_else(|| inventory::bad(id, "root name is not UTF-8"))
            .and_then(|n| decode_root(n).map_err(|e| inventory::bad(id, &e.to_string())));
        let Some(path) = state.report.collect(decoded)? else {
            return Ok(None);
        };
        if !paths.contains(&path) {
            state.report.add(
                id,
                "tree",
                "bad-format",
                "root entry absent from snapshot paths",
            );
            return Ok(None);
        }
        Ok(Some(PathBuf::from(path)))
    } else {
        Ok(state
            .report
            .collect(
                entry
                    .name_bytes()
                    .map_err(|e| inventory::bad(id, &e.to_string())),
            )?
            .map(|raw| base.join(OsString::from_vec(raw))))
    }
}
