use super::metadata;
use crate::{
    backend::Backend,
    chunker::Chunker,
    crypto::envelope::Label,
    format::{
        snapshot::{Snapshot, Summary, root_name},
        tree::{Entry, Node, Tree, encode_bytes},
    },
    repo::{Error, Repository, blobs::BlobStore},
};
use serde::Serialize;
use std::{
    fs,
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Component, Path, PathBuf},
};
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
struct Walker<'a, B> {
    store: BlobStore<'a, B>,
    summary: Summary,
    skipped: Vec<Skipped>,
    gear: [u8; 32],
}
impl<B: Backend> Walker<'_, B> {
    fn skip(&mut self, path: &Path, reason: impl ToString) {
        let reason = reason.to_string();
        eprintln!("warning: {}: {reason}", path.display());
        self.summary.files_skipped += 1;
        self.skipped.push(Skipped {
            path: path.to_string_lossy().into_owned(),
            reason,
        });
    }
    fn walk(&mut self, path: &Path, name: &[u8]) -> Result<Option<Entry>, Error> {
        let meta = match fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(e) => {
                self.skip(path, e);
                return Ok(None);
            }
        };
        let node = if meta.is_symlink() {
            match fs::read_link(path) {
                Ok(target) => {
                    let (target, target_b64) = encode_bytes(target.as_os_str().as_bytes());
                    Node::Symlink { target, target_b64 }
                }
                Err(e) => {
                    self.skip(path, e);
                    return Ok(None);
                }
            }
        } else if meta.is_dir() {
            let children = match fs::read_dir(path) {
                Ok(d) => d,
                Err(e) => {
                    self.skip(path, e);
                    return Ok(None);
                }
            };
            let mut paths = Vec::new();
            for child in children {
                match child {
                    Ok(c) => paths.push(c.path()),
                    Err(e) => self.skip(path, e),
                }
            }
            paths.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
            let mut entries = Vec::new();
            for child in paths {
                let name = child
                    .file_name()
                    .expect("read_dir child has a name")
                    .as_bytes();
                if let Some(e) = self.walk(&child, name)? {
                    entries.push(e);
                }
            }
            let tree = Tree {
                version: 1,
                entries,
            };
            let subtree = self.store.add(
                crate::format::pack::BlobKind::Tree,
                &serde_json::to_vec(&tree)?,
            )?;
            self.summary.dirs += 1;
            Node::Dir { subtree }
        } else if meta.is_file() {
            let file = match fs::OpenOptions::new()
                .read(true)
                .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
                .open(path)
            {
                Ok(f) => f,
                Err(e) => {
                    self.skip(path, e);
                    return Ok(None);
                }
            };
            let opened = match file.metadata() {
                Ok(m) => m,
                Err(e) => {
                    self.skip(path, e);
                    return Ok(None);
                }
            };
            if opened.ino() != meta.ino() || opened.dev() != meta.dev() || !opened.is_file() {
                self.skip(path, "changed while opening");
                return Ok(None);
            }
            let mut reader = Chunker::new(&file, &self.gear);
            let mut content = Vec::new();
            let mut size = 0;
            loop {
                match reader.next_chunk() {
                    Ok(Some(bytes)) => {
                        size += bytes.len() as u64;
                        content.push(
                            self.store
                                .add(crate::format::pack::BlobKind::Data, &bytes)?,
                        );
                    }
                    Ok(None) => break,
                    Err(e) => {
                        self.skip(path, e);
                        return Ok(None);
                    }
                }
            }
            let after = match file.metadata() {
                Ok(m) => m,
                Err(e) => {
                    self.skip(path, e);
                    return Ok(None);
                }
            };
            if size != meta.len()
                || meta.mtime() != after.mtime()
                || meta.mtime_nsec() != after.mtime_nsec()
                || meta.ctime() != after.ctime()
                || meta.ctime_nsec() != after.ctime_nsec()
            {
                self.skip(path, "changed while reading");
                return Ok(None);
            }
            self.summary.files_new += 1;
            self.summary.bytes_processed += size;
            Node::File { size, content }
        } else {
            self.skip(path, "unsupported file type");
            return Ok(None);
        };
        Ok(Some(metadata::entry(name, &meta, node)?))
    }
}
/// Lexical cleaning preserves the final symlink so the walk never follows it.
pub fn absolute(path: &Path) -> Result<PathBuf, Error> {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut clean = PathBuf::new();
    for c in joined.components() {
        match c {
            Component::ParentDir => {
                clean.pop();
            }
            Component::CurDir => {}
            other => clean.push(other.as_os_str()),
        }
    }
    Ok(clean)
}
pub fn backup<B: Backend>(
    repo: &Repository<B>,
    paths: &[PathBuf],
    tags: Vec<String>,
) -> Result<Report, Error> {
    let mut paths = paths
        .iter()
        .map(|p| absolute(p))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    paths.dedup();
    let strings = paths
        .iter()
        .map(|p| {
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
    for (path, text) in paths.iter().zip(&strings) {
        if let Some(entry) = walker.walk(path, root_name(text).as_bytes())? {
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
