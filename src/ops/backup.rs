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
use std::path::PathBuf;
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
pub use super::paths::normalize_root as absolute;
pub fn backup<B: Backend>(
    repo: &Repository<B>,
    paths: &[PathBuf],
    tags: Vec<String>,
) -> Result<Report, Error> {
    let mut paths = paths
        .iter()
        .map(|input| {
            absolute(input).map(|normalized| {
                let exists = std::fs::symlink_metadata(input).is_ok();
                (normalized, input, exists)
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort_by(|a, b| a.0.cmp(&b.0).then(b.2.cmp(&a.2)));
    paths.dedup_by(|a, b| a.0 == b.0);
    let strings = paths
        .iter()
        .map(|(p, _, _)| {
            p.to_str().map(String::from).ok_or_else(|| {
                Error::Refused(
                    "backup root paths must be UTF-8; names within them may be arbitrary bytes"
                        .into(),
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if strings.is_empty() {
        return Err(Error::Refused("backup needs at least one path".into()));
    }
    let mut walker = Walker {
        store: BlobStore::load(repo)?,
        summary: Summary::default(),
        skipped: Vec::new(),
        gear: repo.chunker_key(),
    };
    let mut entries = Vec::new();
    for ((_, input, _), text) in paths.iter().zip(&strings) {
        // A historical lexical fallback is a name, not permission to read a
        // different live file when the supplied source path cannot be resolved.
        if let Some(entry) = walker.walk(input, root_name(text).as_bytes())? {
            entries.push(entry);
        }
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
        paths: strings,
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
