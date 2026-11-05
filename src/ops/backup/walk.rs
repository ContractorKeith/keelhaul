//! One metadata walker; file bytes are handled by the bounded worker pool.
use super::{
    Options, Patterns,
    parent::Previous,
    pipeline::{Event, send},
};
use crate::{
    format::{
        snapshot::root_name,
        tree::{Node, encode_bytes},
    },
    ops::metadata,
    repo::Error,
};
use std::{
    fs,
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::PathBuf,
    sync::mpsc::SyncSender,
};

pub(super) struct FileTask {
    pub path: PathBuf,
    pub key: (usize, PathBuf),
    pub entry: crate::format::tree::Entry,
    pub changed: bool,
    pub reuse: bool,
    pub previous_content: Option<Vec<String>>,
}
pub(super) fn run(
    roots: &[(PathBuf, &PathBuf)],
    strings: &[String],
    previous: &Previous,
    patterns: &Patterns,
    options: &Options,
    tasks: SyncSender<FileTask>,
    events: SyncSender<Result<Event, Error>>,
) -> Result<(), Error> {
    for (root_index, ((normalized, input), text)) in roots.iter().zip(strings).enumerate() {
        let device = match fs::symlink_metadata(input) {
            Ok(m) => m.dev(),
            Err(e) => {
                super::resources::check(input, &e)?;
                send(
                    &events,
                    Event::Skip(
                        (root_index, normalized.clone()),
                        input.to_path_buf(),
                        e.to_string(),
                    ),
                )?;
                continue;
            }
        };
        let mut work = vec![(
            input.to_path_buf(),
            normalized.clone(),
            root_name(text).into_bytes(),
        )];
        while let Some((path, key, name)) = work.pop() {
            let meta = match fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(e) => {
                    super::resources::check(&path, &e)?;
                    send(&events, Event::Skip((root_index, key), path, e.to_string()))?;
                    continue;
                }
            };
            if path != **input
                && (patterns.excluded(path.strip_prefix(input).unwrap_or(&path), meta.is_dir())
                    || (options.one_file_system && meta.dev() != device))
            {
                send(&events, Event::Ignored((root_index, key)))?;
                continue;
            }
            let node = if meta.is_dir() {
                let children = match fs::read_dir(&path) {
                    Ok(children) => children,
                    Err(e) => {
                        super::resources::check(&path, &e)?;
                        send(&events, Event::Skip((root_index, key), path, e.to_string()))?;
                        continue;
                    }
                };
                let mut children_paths = Vec::new();
                for child in children {
                    match child {
                        Ok(c) => children_paths.push(c.path()),
                        Err(e) => {
                            super::resources::check(&path, &e)?;
                            send(&events, Event::Warning(path.clone(), e.to_string()))?;
                        }
                    }
                }
                let count = children_paths.len();
                children_paths.sort();
                for child in children_paths.into_iter().rev() {
                    let raw = child
                        .file_name()
                        .expect("read_dir children have names")
                        .as_bytes()
                        .to_vec();
                    work.push((child, key.join(std::ffi::OsStr::from_bytes(&raw)), raw));
                }
                let entry = metadata::entry(
                    &name,
                    &meta,
                    Node::Dir {
                        subtree: String::new(),
                    },
                )?;
                send(
                    &events,
                    Event::Directory((root_index, key), Box::new(entry), count),
                )?;
                continue;
            } else if meta.is_symlink() {
                match fs::read_link(&path) {
                    Ok(t) => {
                        let (target, target_b64) = encode_bytes(t.as_os_str().as_bytes());
                        Node::Symlink { target, target_b64 }
                    }
                    Err(e) => {
                        super::resources::check(&path, &e)?;
                        send(&events, Event::Skip((root_index, key), path, e.to_string()))?;
                        continue;
                    }
                }
            } else if meta.is_file() {
                Node::File {
                    size: meta.len(),
                    content: Vec::new(),
                }
            } else {
                send(
                    &events,
                    Event::Skip((root_index, key), path, "unsupported file type".into()),
                )?;
                continue;
            };
            let mut entry = metadata::entry(&name, &meta, node)?;
            if meta.is_file() {
                let old = previous.get(&key);
                let metadata_unchanged = old.is_some_and(|old| {
                    matches!(old.node, Node::File { size, .. } if size == meta.len())
                        && old.mtime == entry.mtime
                        && old.ctime == entry.ctime
                        && old.inode == entry.inode
                        && old.device == entry.device
                });
                if metadata_unchanged && entry.links <= 1 && !options.force_rehash && !options.heal
                {
                    entry.node = old.expect("reuse requires a parent entry").node.clone();
                    send(
                        &events,
                        Event::Entry(
                            (root_index, key),
                            Box::new(entry),
                            super::pipeline::Status::Unchanged,
                        ),
                    )?;
                } else {
                    let reuse = metadata_unchanged && !options.force_rehash && !options.heal;
                    if reuse {
                        entry.node = old.expect("reuse requires a parent entry").node.clone();
                    }
                    tasks
                        .send(FileTask {
                            path,
                            key: (root_index, key),
                            entry,
                            changed: old.is_some(),
                            reuse,
                            previous_content: if metadata_unchanged {
                                old.and_then(|entry| match &entry.node {
                                    Node::File { content, .. } => Some(content.clone()),
                                    _ => None,
                                })
                            } else {
                                None
                            },
                        })
                        .map_err(|_| super::pipeline::closed())?;
                }
            } else {
                send(
                    &events,
                    Event::Entry(
                        (root_index, key),
                        Box::new(entry),
                        super::pipeline::Status::Other,
                    ),
                )?;
            }
        }
    }
    Ok(())
}
