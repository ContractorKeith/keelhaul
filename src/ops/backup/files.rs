use super::{
    pipeline::{Event, Status, send},
    walk::FileTask,
};
use crate::{
    chunker::Chunker,
    crypto::envelope,
    format::{
        pack::BlobKind,
        tree::{Entry, Node},
    },
    ops::metadata,
    repo::{Error, blobs::SealedBlob},
};
use std::{
    collections::{HashMap, HashSet},
    os::unix::fs::MetadataExt,
    sync::{Mutex, mpsc::SyncSender},
};

pub(super) type Seen = Mutex<HashSet<(BlobKind, String)>>;
pub(super) type Hardlinks = Mutex<HashMap<(u64, u64), Entry>>;
pub(super) fn read(
    task: FileTask,
    gear: &[u8; 32],
    keys: &([u8; 32], [u8; 32]),
    seen: &Seen,
    hardlinks: &Hardlinks,
    events: &SyncSender<Result<Event, Error>>,
) -> Result<(), Error> {
    if task.entry.links > 1 {
        // Serialize linked inode reads so every alias gets one descriptor's metadata and bytes.
        let mut groups = hardlinks
            .lock()
            .map_err(|_| Error::Refused("hardlink cache mutex poisoned".into()))?;
        let inode = (task.entry.device, task.entry.inode);
        if let Some(first) = groups.get(&inode) {
            let mut entry = first.clone();
            entry.name = task.entry.name;
            entry.name_b64 = task.entry.name_b64;
            return send(
                events,
                Event::Entry(
                    task.key,
                    Box::new(entry),
                    if task.previous_content.as_ref().is_some_and(
                        |old| matches!(&first.node, Node::File { content, .. } if content == old),
                    ) {
                        Status::Unchanged
                    } else if task.changed {
                        Status::Changed
                    } else {
                        Status::New
                    },
                ),
            );
        }
        let mut saved = None;
        read_one(task, gear, keys, seen, events, &mut saved)?;
        if let Some(entry) = saved {
            groups.insert(inode, entry);
        }
        Ok(())
    } else {
        read_one(task, gear, keys, seen, events, &mut None)
    }
}
fn read_one(
    task: FileTask,
    gear: &[u8; 32],
    keys: &([u8; 32], [u8; 32]),
    seen: &Seen,
    events: &SyncSender<Result<Event, Error>>,
    saved: &mut Option<Entry>,
) -> Result<(), Error> {
    if task.reuse {
        *saved = Some(task.entry.clone());
        return send(
            events,
            Event::Entry(task.key, Box::new(task.entry), Status::Unchanged),
        );
    }
    let FileTask {
        path,
        key,
        mut entry,
        changed,
        previous_content,
        reuse: _,
    } = task;
    // User file failures are skips; storage/pipeline failures abort the backup.
    let file = match super::resources::open_file(&path) {
        Ok(f) => f,
        Err(e) => {
            super::resources::check(&path, &e)?;
            return send(events, Event::Skip(key, path, e.to_string()));
        }
    };
    let opened = match file.metadata() {
        Ok(m) => m,
        Err(e) => {
            super::resources::check(&path, &e)?;
            return send(events, Event::Skip(key, path, e.to_string()));
        }
    };
    if opened.ino() != entry.inode || opened.dev() != entry.device || !opened.is_file() {
        return send(
            events,
            Event::Skip(key, path, "changed while opening".into()),
        );
    }
    send(events, Event::Read)?;
    // Pin metadata to the descriptor, rather than the earlier walk observation.
    let name = entry
        .name_bytes()
        .map_err(|e| Error::Refused(e.to_string()))?;
    entry = metadata::entry(&name, &opened, entry.node)?;
    let mut reader = Chunker::new(&file, gear);
    let mut content = Vec::new();
    let mut size = 0;
    loop {
        let bytes = match reader.next_chunk() {
            Ok(Some(bytes)) => bytes,
            Ok(None) => break,
            Err(e) => {
                super::resources::check(&path, &e)?;
                return send(events, Event::Skip(key, path, e.to_string()));
            }
        };
        size += bytes.len() as u64;
        let id = blake3::keyed_hash(&keys.0, bytes).to_hex().to_string();
        let fresh = seen
            .lock()
            .map_err(|_| Error::Refused("chunk reservation mutex poisoned".into()))?
            .insert((BlobKind::Data, id.clone()));
        if fresh {
            let stored = envelope::seal(&keys.1, BlobKind::Data.label(), bytes)?;
            send(
                events,
                Event::Blob(SealedBlob {
                    id: id.clone(),
                    kind: BlobKind::Data,
                    stored,
                    raw_length: bytes.len() as u64,
                }),
            )?;
        }
        content.push(id);
    }
    let after = match file.metadata() {
        Ok(m) => m,
        Err(e) => {
            super::resources::check(&path, &e)?;
            return send(events, Event::Skip(key, path, e.to_string()));
        }
    };
    if opened.len() != size
        || after.len() != size
        || metadata::timestamp(after.mtime(), after.mtime_nsec())? != entry.mtime
        || metadata::timestamp(after.ctime(), after.ctime_nsec())? != entry.ctime
    {
        send(events, Event::ChangedDuringRead(path))?;
    }
    let unchanged = previous_content.as_ref() == Some(&content);
    entry.node = Node::File { size, content };
    *saved = Some(entry.clone());
    send(
        events,
        Event::Entry(
            key,
            Box::new(entry),
            if unchanged {
                Status::Unchanged
            } else if changed {
                Status::Changed
            } else {
                Status::New
            },
        ),
    )
}
