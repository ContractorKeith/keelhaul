use super::{
    pipeline::{Event, Status, send},
    walk::FileTask,
};
use crate::{
    chunker::Chunker,
    crypto::envelope,
    format::{pack::BlobKind, tree::Node},
    ops::metadata,
    repo::{Error, blobs::SealedBlob},
};
use std::{
    collections::HashSet,
    fs,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    sync::{Mutex, mpsc::SyncSender},
};

pub(super) type Seen = Mutex<HashSet<(BlobKind, String)>>;
pub(super) fn read(
    task: FileTask,
    gear: &[u8; 32],
    keys: &([u8; 32], [u8; 32]),
    seen: &Seen,
    events: &SyncSender<Result<Event, Error>>,
) -> Result<(), Error> {
    let FileTask {
        path,
        key,
        mut entry,
        changed,
        previous_content,
    } = task;
    // User file failures are skips; storage/pipeline failures abort the backup.
    let file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags((rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32)
        .open(&path)
    {
        Ok(f) => f,
        Err(e) => {
            return send(events, Event::Skip(key, path, e.to_string()));
        }
    };
    let opened = match file.metadata() {
        Ok(m) => m,
        Err(e) => {
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
