//! Bounded walker → file workers → assembler → upload workers.
use super::{Options, Patterns, Skipped, files, parent::Previous, trees::Trees, walk, workers};
use crate::{
    backend::Backend,
    format::{
        snapshot::Summary,
        tree::{Entry, Node},
    },
    repo::{
        Error, Repository,
        blobs::{BlobStore, SealedBlob, Upload},
    },
};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        mpsc::{self, SyncSender},
    },
    thread,
};

pub(super) enum Status {
    New,
    Changed,
    Unchanged,
    Other,
}
pub(super) enum Event {
    Blob(SealedBlob),
    Entry((usize, PathBuf), Box<Entry>, Status),
    Directory((usize, PathBuf), Box<Entry>, usize),
    Skip((usize, PathBuf), PathBuf, String),
    Ignored((usize, PathBuf)),
    Warning(PathBuf, String),
    Read,
    ChangedDuringRead(PathBuf),
    Finished(bool),
}
pub(super) fn closed() -> Error {
    Error::Refused("backup pipeline stopped".into())
}
pub(super) fn send(sender: &SyncSender<Result<Event, Error>>, event: Event) -> Result<(), Error> {
    sender.send(Ok(event)).map_err(|_| closed())
}
pub(super) struct Output {
    pub tree: String,
    pub blobs: std::collections::HashSet<(crate::format::pack::BlobKind, String)>,
    pub paths: Vec<String>,
    pub summary: Summary,
    pub skipped: Vec<Skipped>,
    pub blobs_healed: u64,
    pub files_read: u64,
    pub files_changed_during_read: u64,
}
pub(super) fn run<B: Backend>(
    repo: &Repository<B>,
    mut store: BlobStore<'_, B>,
    roots: &[(PathBuf, &PathBuf)],
    strings: &[String],
    previous: Previous,
    patterns: &Patterns,
    options: &Options,
) -> Result<Output, Error> {
    let jobs = options.jobs;
    let seen = Mutex::new(
        store
            .index
            .keys()
            .filter(|id| !options.heal && !store.suspect.contains(*id))
            .cloned()
            .collect(),
    );
    let keys = store.worker_keys();
    let gear = repo.chunker_key();
    thread::scope(|scope| {
        let (tasks_tx, tasks_rx) = mpsc::sync_channel(jobs);
        let tasks_rx = Arc::new(Mutex::new(tasks_rx));
        let (events_tx, events_rx) = mpsc::sync_channel(jobs);
        let (uploads_tx, uploads_rx) = mpsc::sync_channel::<Upload>(jobs);
        let uploads_rx = Arc::new(Mutex::new(uploads_rx));
        // At most eight packs await a flush. This capacity also allows each
        // uploader to report failure while the assembler dispatches the batch.
        let (done_tx, done_rx) = mpsc::sync_channel(jobs + 8);
        store.upload_pool(uploads_tx, done_rx);
        for _ in 0..jobs {
            let receiver = uploads_rx.clone();
            let done = done_tx.clone();
            let errors = events_tx.clone();
            thread::Builder::new().spawn_scoped(scope, move || {
                workers::run("upload", errors, move || {
                    while let Some(task) = workers::receive(&receiver, "upload")? {
                        if done.send(workers::upload(repo, &task)).is_err() {
                            break;
                        }
                    }
                    Ok(())
                });
            })?;
            let receiver = tasks_rx.clone();
            let events = events_tx.clone();
            let seen = &seen;
            thread::Builder::new().spawn_scoped(scope, move || {
                workers::run("file", events.clone(), move || {
                    while let Some(task) = workers::receive(&receiver, "file")? {
                        files::read(task, &gear, &keys, seen, &events)?;
                    }
                    Ok(())
                });
            })?;
        }
        drop(done_tx);
        drop(tasks_rx);
        drop(uploads_rx);
        let walker_events = events_tx.clone();
        thread::Builder::new().spawn_scoped(scope, move || {
            workers::run("walker", walker_events.clone(), move || {
                walk::run(
                    roots,
                    strings,
                    &previous,
                    patterns,
                    options,
                    tasks_tx,
                    walker_events,
                )
            });
        })?;
        drop(events_tx);
        // An early return drops the receivers and store's upload sender, which
        // releases every blocked send before the scope joins its threads.
        let mut summary = Summary::default();
        let mut healing = Default::default();
        let mut blobs_healed = 0;
        let mut skipped = Vec::new();
        let mut files_read = 0;
        let mut files_changed_during_read = 0;
        let mut trees = Trees::new(roots);
        let mut unfinished = jobs + 1;
        while unfinished > 0 {
            match events_rx.recv().map_err(|_| closed())?? {
                Event::Finished(false) => unfinished -= 1,
                Event::Finished(true) => {
                    return Err(Error::Refused("upload pool stopped early".into()));
                }
                Event::Blob(blob) => {
                    if options.heal {
                        blobs_healed += u64::from(store.add_healing(blob, &mut healing)?);
                    } else {
                        store.add_sealed(blob)?;
                    }
                }
                Event::Read => files_read += 1,
                Event::ChangedDuringRead(path) => {
                    files_changed_during_read += 1;
                    eprintln!(
                        "warning: {}: changed while reading; stored as read",
                        path.display()
                    );
                }
                Event::Directory(key, entry, count) => {
                    summary.dirs += 1;
                    trees.directory(key, *entry, count, &mut store)?;
                }
                Event::Ignored(key) => trees.complete(key, None, &mut store)?,
                Event::Warning(path, reason) => {
                    summary.files_skipped += 1;
                    skipped.push(super::skipped(&path, reason));
                }
                Event::Skip(key, path, reason) => {
                    summary.files_skipped += 1;
                    skipped.push(super::skipped(&path, reason));
                    trees.complete(key, None, &mut store)?;
                }
                Event::Entry(path, entry, status) => {
                    match status {
                        Status::New => summary.files_new += 1,
                        Status::Changed => summary.files_changed += 1,
                        Status::Unchanged => summary.files_unchanged += 1,
                        Status::Other => {}
                    }
                    if let Node::File { size, .. } = entry.node {
                        summary.bytes_processed += size;
                    }
                    trees.complete(path, Some(*entry), &mut store)?;
                }
            }
        }
        let (tree, paths) = trees.finish(strings, &mut store)?;
        store.finish()?;
        let blobs = super::publication::referenced_blobs(&mut store, &tree)?;
        summary.data_blobs_added = store.data_blobs_added;
        summary.bytes_added = store.bytes_added;
        // The sender must be dropped inside the scope, before it joins uploads.
        drop(store);
        for _ in 0..jobs {
            match events_rx.recv().map_err(|_| closed())?? {
                Event::Finished(true) => {}
                _ => return Err(closed()),
            }
        }
        Ok(Output {
            tree,
            blobs,
            paths,
            summary,
            skipped,
            blobs_healed,
            files_read,
            files_changed_during_read,
        })
    })
}
