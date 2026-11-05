//! Preflight every destination, then publish only completely verified files.
mod destination;
mod files;
mod plan;
use crate::{
    backend::Backend,
    format::tree::Node,
    repo::{Error, Repository, blobs::BlobStore},
};
use serde::Serialize;
use std::path::{Path, PathBuf};
#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub restored: u64,
    #[serde(rename = "damage")]
    pub damaged: Vec<crate::damage::Damage>,
    pub failed: Vec<Failure>,
}
#[derive(Debug, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Failure {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_b64: Option<String>,
    pub error: String,
}
impl Report {
    fn failure(&mut self, path: &Path, error: Error) {
        use base64::Engine;
        use std::os::unix::ffi::OsStrExt;
        let path = Path::new("/").join(path);
        self.failed.push(Failure {
            path: path.to_string_lossy().into_owned(),
            path_b64: path
                .to_str()
                .is_none()
                .then(|| base64::prelude::BASE64_STANDARD.encode(path.as_os_str().as_bytes())),
            error: error.to_string(),
        });
    }
    pub(super) fn damage(&mut self, error: Error) -> Result<(), Error> {
        self.damaged.push(crate::damage::Damage::from_error(error)?);
        Ok(())
    }
}
pub fn restore<B: Backend>(
    repo: &Repository<B>,
    selector: &str,
    target: &Path,
    includes: &[PathBuf],
) -> Result<Report, Error> {
    restore_for_host(repo, selector, target, includes, None)
}
pub fn restore_for_host<B: Backend>(
    repo: &Repository<B>,
    selector: &str,
    target: &Path,
    includes: &[PathBuf],
    host: Option<&str>,
) -> Result<Report, Error> {
    crate::repo::lock::with(&repo.backend, repo.subkeys(), false, |_| {
        restore_for_host_without_lock(repo, selector, target, includes, host)
    })
}
/// Read without a repository lock; callers must exclude concurrent writers.
pub fn restore_for_host_without_lock<B: Backend>(
    repo: &Repository<B>,
    selector: &str,
    target: &Path,
    includes: &[PathBuf],
    host: Option<&str>,
) -> Result<Report, Error> {
    restore_with_hook(repo, selector, target, includes, host, |_| {})
}
fn restore_with_hook<B: Backend>(
    repo: &Repository<B>,
    selector: &str,
    target: &Path,
    includes: &[PathBuf],
    host: Option<&str>,
    mut before_entry: impl FnMut(&Path),
) -> Result<Report, Error> {
    let (snapshot_id, snapshot) = super::listing::select_snapshot_for_host(repo, selector, host)?;
    let mut store = BlobStore::load_quiet(repo)?;
    let mut report = Report {
        damaged: store.damage.clone(),
        ..Report::default()
    };
    let items = plan::build(&mut store, &snapshot, includes, &mut report)?;
    let destination = destination::Destination::new(target)
        .map_err(|error| Error::Refused(format!("{}: {error}", target.display())))?;
    let mut preflight = destination.preflight(&items)?;
    let mut hardlinks = files::Hardlinks::new();
    let mut blocked = std::collections::HashSet::new();
    for (path, error) in preflight.failed.drain(..) {
        report.failure(&path, error);
        blocked.insert(path);
    }
    let mut completed = std::collections::HashSet::new();
    for item in &items {
        if item.path.ancestors().any(|path| blocked.contains(path)) {
            continue;
        }
        let path = &item.path;
        before_entry(path);
        let outcome = (|| {
            destination.check_collision(path)?;
            match &item.entry.node {
                Node::Dir { .. } => {
                    if !preflight.reused.contains(path) {
                        destination.mkdir(path)?;
                    }
                }
                Node::Symlink { .. } => {
                    let raw = item
                        .entry
                        .target_bytes()
                        .map_err(|e| super::listing::damage(&item.object, &e.to_string()))?;
                    use std::os::unix::ffi::OsStringExt;
                    destination.symlink(path, &std::ffi::OsString::from_vec(raw), &item.entry)?;
                }
                Node::File { .. } => {
                    files::restore_file(&destination, &mut store, item, &mut hardlinks)?
                }
            }
            destination.remember(path)?;
            Ok(())
        })();
        match outcome {
            Ok(()) => {
                report.restored += 1;
                completed.insert(path.clone());
            }
            Err(e) => {
                let damaged = match crate::damage::Damage::from_error(e) {
                    Ok(damage) => {
                        report.damaged.push(damage);
                        true
                    }
                    Err(error) => {
                        report.failure(path, error);
                        false
                    }
                };
                if matches!(item.entry.node, Node::Dir { .. }) {
                    blocked.insert(path.clone());
                }
                if damaged && matches!(item.entry.node, Node::File { .. }) {
                    report.damaged.push(crate::damage::Damage::file(
                        &format!("snapshots/{snapshot_id}"),
                        &Path::new("/").join(&item.path),
                        "dangling-reference",
                    ));
                }
            }
        }
    }
    for item in items.iter().rev() {
        if matches!(item.entry.node, Node::Dir { .. })
            && !preflight.reused.contains(&item.path)
            && completed.contains(&item.path)
            && let Err(error) = destination.directory_metadata(&item.path, &item.entry)
        {
            report.failure(&item.path, error);
        }
    }
    for (path, error) in preflight.restore_mtimes() {
        report.failure(&path, error);
    }
    destination.finish()?;
    report.damaged.sort_by(|a, b| {
        (&a.object, &a.problem, &a.detail).cmp(&(&b.object, &b.problem, &b.detail))
    });
    report.damaged.dedup();
    report.failed.sort();
    report.failed.dedup();
    Ok(report)
}

#[cfg(test)]
mod tests;
