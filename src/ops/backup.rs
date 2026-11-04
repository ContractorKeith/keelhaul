mod excludes;
mod files;
mod parent;
mod pipeline;
#[cfg(test)]
mod tests;
mod trees;
mod walk;
mod workers;
use crate::{
    backend::Backend,
    crypto::envelope::Label,
    format::snapshot::{Snapshot, Summary},
    repo::{Error, Repository, blobs::BlobStore},
};
pub use excludes::Patterns;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub struct Skipped {
    pub path: String,
    pub reason: String,
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub snapshot: String,
    pub summary: ReportSummary,
    pub skipped: Vec<Skipped>,
}
#[derive(Debug, Serialize)]
pub struct ReportSummary {
    #[serde(flatten)]
    pub totals: Summary,
    pub files_read: u64,
}
impl std::ops::Deref for ReportSummary {
    type Target = Summary;
    fn deref(&self) -> &Summary {
        &self.totals
    }
}
pub struct Options {
    pub parent: Option<String>,
    pub force_rehash: bool,
    pub excludes: Vec<String>,
    pub one_file_system: bool,
    pub host: Option<String>,
    pub jobs: usize,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            parent: None,
            force_rehash: false,
            excludes: Vec::new(),
            one_file_system: false,
            host: None,
            jobs: 4,
        }
    }
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
    backup_with_options(
        repo,
        paths,
        tags,
        &Options {
            jobs: 4,
            ..Options::default()
        },
    )
}
pub fn backup_with_options<B: Backend>(
    repo: &Repository<B>,
    paths: &[PathBuf],
    tags: Vec<String>,
    options: &Options,
) -> Result<Report, Error> {
    // M5: shared lock
    if !(1..=256).contains(&options.jobs) {
        return Err(Error::Refused("jobs must be between 1 and 256".into()));
    }
    if paths.is_empty() {
        return Err(Error::Refused("backup needs at least one path".into()));
    }
    let patterns = Patterns::new(&options.excludes)?;
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
        let meta = match std::fs::symlink_metadata(input) {
            Ok(meta) => meta,
            Err(error) => {
                failures.push(skipped(input, error));
                continue;
            }
        };
        let candidate = normalized.file_name().map(Path::new).unwrap_or(&normalized);
        if patterns.excluded(candidate, meta.is_dir()) {
            failures.push(skipped(input, "excluded by pattern"));
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
    let host = options.host.clone().unwrap_or_else(crate::host::hostname);
    let mut store = BlobStore::load(repo)?;
    let (parent, previous) = parent::load(repo, &mut store, &host, &strings, options)?;
    let result = pipeline::run(repo, store, &roots, &strings, previous, &patterns, options)?;
    let mut summary = result.summary;
    summary.files_skipped += failures.len() as u64;
    failures.extend(result.skipped);
    let snapshot = Snapshot {
        version: 1,
        time: crate::format::timestamp(),
        host,
        username: crate::host::username(),
        paths: result.paths,
        tags,
        excludes: options.excludes.clone(),
        tree: result.tree,
        parent,
        program: format!("keelhaul {}", env!("CARGO_PKG_VERSION")),
        summary: summary.clone(),
    };
    let id = repo.write_object(
        "snapshots",
        Label::Snapshot,
        &serde_json::to_vec(&snapshot)?,
    )?;
    Ok(Report {
        snapshot: id,
        summary: ReportSummary {
            totals: summary,
            files_read: result.files_read,
        },
        skipped: failures,
    })
}
