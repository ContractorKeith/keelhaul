//! Root validation shared by audit and restore; damage is local to affected roots.
use super::{
    snapshot::{Snapshot, decode_root},
    tree::{Entry, Tree},
};
use std::{collections::HashSet, path::PathBuf};

pub struct Shape {
    pub roots: Vec<Option<String>>,
    pub blocked: Vec<PathBuf>,
    pub problems: Vec<String>,
}

pub fn validate(snapshot: &Snapshot, tree: &Tree) -> Shape {
    let mut shape = Shape {
        roots: Vec::new(),
        blocked: Vec::new(),
        problems: Vec::new(),
    };
    for path in &snapshot.paths {
        if snapshot
            .paths
            .iter()
            .any(|parent| parent != path && std::path::Path::new(path).starts_with(parent))
        {
            shape
                .blocked
                .push(PathBuf::from(path.trim_start_matches('/')));
            shape
                .problems
                .push(format!("overlapping snapshot roots: {path}"));
        }
    }
    let mut found = HashSet::new();
    for entry in &tree.entries {
        let decoded = root_path(&snapshot.paths, entry);
        match decoded {
            Ok(path) => {
                found.insert(path.clone());
                shape.roots.push(Some(path));
            }
            Err(problem) => {
                shape.problems.push(problem);
                shape.roots.push(None);
            }
        }
    }
    for path in &snapshot.paths {
        if !found.contains(path) {
            shape
                .problems
                .push(format!("root entry missing for snapshot path: {path}"));
        }
    }
    shape
}

/// Decode only roots named by this snapshot; ordinary tree names never use this encoding.
pub fn root_path(paths: &[String], entry: &Entry) -> Result<String, String> {
    let name = entry
        .name
        .as_deref()
        .ok_or_else(|| "root name is not UTF-8".to_string())?;
    let path = decode_root(name).map_err(|e| e.to_string())?;
    if !paths.contains(&path) {
        return Err("root entry absent from snapshot paths".into());
    }
    Ok(path)
}
