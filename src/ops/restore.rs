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
}
impl Report {
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
    let (snapshot_id, snapshot) = repo.select_snapshot_for_host(selector, host)?;
    let mut store = BlobStore::load(repo)?;
    let mut report = Report::default();
    let items = plan::build(&mut store, &snapshot, includes, &mut report)?;
    let destination = destination::Destination::new(target)?;
    let preflight = destination.preflight(&items)?;
    let mut hardlinks = files::Hardlinks::new();
    for item in &items {
        let path = &item.path;
        let outcome = (|| {
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
            Ok(())
        })();
        match outcome {
            Ok(()) => report.restored += 1,
            Err(e) => {
                report.damage(e)?;
                if matches!(item.entry.node, Node::File { .. }) {
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
        if matches!(item.entry.node, Node::Dir { .. }) && !preflight.reused.contains(&item.path) {
            destination.directory_metadata(&item.path, &item.entry)?;
        }
    }
    preflight.restore_mtimes()?;
    report.damaged.sort_by(|a, b| {
        (&a.object, &a.problem, &a.detail).cmp(&(&b.object, &b.problem, &b.detail))
    });
    report.damaged.dedup();
    Ok(report)
}
