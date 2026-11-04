mod walk;
use crate::{
    backend::Backend,
    crypto::envelope::Label,
    format::{
        snapshot::{Snapshot, Summary, root_name},
        tree::Tree,
    },
    repo::{Error, Repository, blobs::BlobStore},
};
use serde::Serialize;
use std::path::{Path, PathBuf};
use walk::Walker;
#[derive(Debug, Serialize)]
pub struct Skipped {
    pub path: String,
    pub reason: String,
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub snapshot: String,
    pub summary: Summary,
    pub skipped: Vec<Skipped>,
}
fn skipped(path: &Path, reason: impl ToString) -> Skipped {
    let reason = reason.to_string();
    eprintln!("warning: {}: {reason}", path.display());
    Skipped {
        path: path.to_string_lossy().into_owned(),
        reason,
    }
}
pub use super::paths::normalize_root as absolute;
pub fn backup<B: Backend>(
    repo: &Repository<B>,
    paths: &[PathBuf],
    tags: Vec<String>,
) -> Result<Report, Error> {
    if paths.is_empty() {
        return Err(Error::Refused("backup needs at least one path".into()));
    }
    let mut failures = Vec::new();
    let mut roots = Vec::new();
    for input in paths {
        let normalized = match absolute(input) {
            Ok(path) => path,
            Err(error) => {
                failures.push(skipped(input, error));
                continue;
            }
        };
        if let Err(error) = std::fs::symlink_metadata(input) {
            failures.push(skipped(input, error));
            continue;
        }
        roots.push((normalized, input));
    }
    roots.sort_by(|a, b| a.0.cmp(&b.0));
    roots.dedup_by(|a, b| a.0 == b.0);
    if roots.is_empty() {
        return Err(Error::Refused(
            "no backup roots could be read; no snapshot written".into(),
        ));
    }
    let strings = roots
        .iter()
        .map(|(p, _)| {
            p.to_str().map(String::from).ok_or_else(|| {
                Error::Refused(
                    "backup root paths must be UTF-8; names within them may be arbitrary bytes"
                        .into(),
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut walker = Walker {
        store: BlobStore::load(repo)?,
        summary: Summary {
            files_skipped: failures.len() as u64,
            ..Summary::default()
        },
        skipped: failures,
        gear: repo.chunker_key(),
    };
    let mut entries = Vec::new();
    let mut saved_paths = Vec::new();
    for ((_, input), text) in roots.iter().zip(&strings) {
        // A historical lexical fallback is a name, not permission to read a
        // different live file when the supplied source path cannot be resolved.
        if let Some(entry) = walker.walk(input, root_name(text).as_bytes())? {
            entries.push(entry);
            saved_paths.push(text.clone());
        }
    }
    if entries.is_empty() {
        return Err(Error::Refused(
            "no backup roots could be read; no snapshot written".into(),
        ));
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    let tree = walker.store.add(
        crate::format::pack::BlobKind::Tree,
        &serde_json::to_vec(&Tree {
            version: 1,
            entries,
        })?,
    )?;
    walker.store.finish()?;
    walker.summary.data_blobs_added = walker.store.data_blobs_added;
    walker.summary.bytes_added = walker.store.bytes_added;
    let snapshot = Snapshot {
        version: 1,
        time: crate::format::timestamp(),
        host: crate::host::hostname(),
        username: crate::host::username(),
        paths: saved_paths,
        tags,
        excludes: Vec::new(),
        tree,
        parent: None,
        program: format!("keelhaul {}", env!("CARGO_PKG_VERSION")),
        summary: walker.summary.clone(),
    };
    let id = repo.write_object(
        "snapshots",
        Label::Snapshot,
        &serde_json::to_vec(&snapshot)?,
    )?;
    Ok(Report {
        snapshot: id,
        summary: walker.summary,
        skipped: walker.skipped,
    })
}
