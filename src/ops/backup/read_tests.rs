use super::{files, pipeline::Event, walk::FileTask};
use crate::{format::tree::Node, ops::metadata};
use std::{
    collections::HashSet,
    fs,
    io::Write,
    sync::{Mutex, mpsc},
};

#[test]
fn append_during_read_keeps_bytes_and_open_metadata() {
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("log");
    fs::write(&path, vec![42; 1024 * 1024]).unwrap();
    let meta = fs::metadata(&path).unwrap();
    let entry = metadata::entry(
        b"log",
        &meta,
        Node::File {
            size: meta.len(),
            content: vec![],
        },
    )
    .unwrap();
    let opened_ctime = entry.ctime.clone();
    let task = FileTask {
        path: path.clone(),
        key: (0, path.clone()),
        entry,
        changed: false,
        reuse: false,
        previous_content: None,
    };
    let (tx, rx) = mpsc::sync_channel(0);
    std::thread::scope(|scope| {
        let worker = scope.spawn(|| {
            files::read(
                task,
                &[7; 32],
                &([8; 32], [9; 32]),
                &Mutex::new(HashSet::new()),
                &files::Hardlinks::default(),
                &tx,
            )
        });
        assert!(matches!(rx.recv().unwrap().unwrap(), Event::Read));
        // Append after the descriptor metadata was captured and before draining chunks.
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(&vec![43; 1024 * 1024]).unwrap();
        let mut changed = false;
        let mut saved = None;
        while saved.is_none() {
            match rx.recv().unwrap().unwrap() {
                Event::ChangedDuringRead(_) => changed = true,
                Event::Entry(_, entry, _) => saved = Some(entry),
                Event::Skip(_, _, reason) => panic!("file skipped: {reason}"),
                _ => {}
            }
        }
        worker.join().unwrap().unwrap();
        assert!(changed);
        let saved = saved.unwrap();
        assert_eq!(saved.ctime, opened_ctime);
        assert!(matches!(saved.node, Node::File { size, .. } if size >= 1024*1024));
    });
}
