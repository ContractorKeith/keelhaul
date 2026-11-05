mod excludes;
mod files;
mod parent;
mod pipeline;
mod publication;
#[cfg(test)]
#[path = "backup/read_tests.rs"]
mod read_tests;
#[path = "backup/resources.rs"]
mod resources;
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
    pub blobs_healed: u64,
    pub files_read: u64,
    pub files_changed_during_read: u64,
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
    pub heal: bool,
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
            heal: false,
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
    backup_with_clock(repo, paths, tags, options, &crate::repo::lock::SystemClock)
}
pub fn backup_with_clock<B: Backend>(
    repo: &Repository<B>,
    paths: &[PathBuf],
    tags: Vec<String>,
    options: &Options,
    clock: &dyn crate::repo::lock::Clock,
) -> Result<Report, Error> {
    let lock = crate::repo::lock::Guard::acquire(&repo.backend, repo.subkeys(), false, clock)?;
    lock.run(|lock| backup_locked(repo, paths, tags, options, lock))
}
fn backup_locked<B: Backend>(
    repo: &Repository<B>,
    paths: &[PathBuf],
    tags: Vec<String>,
    options: &Options,
    lock: &crate::repo::lock::Guard<'_, B>,
) -> Result<Report, Error> {
    if !(1..=256).contains(&options.jobs) {
        return Err(Error::Refused("jobs must be between 1 and 256".into()));
    }
    if paths.is_empty() {
        return Err(Error::Refused("backup needs at least one path".into()));
    }
    resources::jobs(options.jobs)?;
    let patterns = Patterns::new(&options.excludes)?;
    let mut failures = Vec::new();
    let mut roots = Vec::new();
    for input in paths {
        let normalized = match absolute(input) {
            Ok(path) => path,
            Err(error) => {
                resources::check(input, &error)?;
                failures.push(skipped(input, error));
                continue;
            }
        };
        let meta = match std::fs::symlink_metadata(input) {
            Ok(meta) => meta,
            Err(error) => {
                resources::check(input, &error)?;
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
    let result = pipeline::run(
        repo, store, &roots, &strings, previous, &patterns, options, lock,
    )?;
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
    lock.self_check()?;
    let id = repo.write_object(
        "snapshots",
        Label::Snapshot,
        &serde_json::to_vec(&snapshot)?,
    )?;
    publication::check(repo, &id, &result.blobs)?;
    Ok(Report {
        snapshot: id,
        summary: ReportSummary {
            totals: summary,
            blobs_healed: result.blobs_healed,
            files_read: result.files_read,
            files_changed_during_read: result.files_changed_during_read,
        },
        skipped: failures,
    })
}
