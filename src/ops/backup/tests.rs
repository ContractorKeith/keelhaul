use super::*;
use crate::{
    backend::{fault::FaultBackend, memory::MemoryBackend},
    ops::restore::restore,
    secret::Secret,
};
use std::{fs, sync::mpsc, time::Duration};

#[test]
fn upload_and_index_failures_release_backpressure_and_preserve_snapshots() {
    let scratch = tempfile::tempdir().unwrap();
    let source = fs::canonicalize(scratch.path()).unwrap().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("old"), b"earlier snapshot").unwrap();
    let secret = Secret::KeyFile([31; 32]);
    let repo = Repository::init(MemoryBackend::new(), &secret).unwrap();
    let first = backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
    let mut bytes = vec![0; 512 * 1024];
    for n in 0..48u64 {
        blake3::Hasher::new()
            .update(&n.to_le_bytes())
            .finalize_xof()
            .fill(&mut bytes);
        fs::write(source.join(format!("new-{n}")), &bytes).unwrap();
    }
    // Both failure points occur while other readers and uploads have work:
    // first pack, first index, then a pack after a flushed index.
    for at in [2, 10, 11] {
        let copied = MemoryBackend::new();
        for kind in crate::backend::Kind::ALL {
            for entry in repo.backend.list(kind).unwrap() {
                copied
                    .write(&entry.name, &repo.backend.read(&entry.name).unwrap())
                    .unwrap();
            }
        }
        let mut failing =
            Repository::open(FaultBackend::new(copied, "write", at, false), &secret).unwrap();
        failing.config.pack_target = 1024 * 1024;
        let root = source.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        let handle = std::thread::spawn(move || {
            let result = backup_with_options(
                &failing,
                &[root],
                vec![],
                &Options {
                    jobs: 8,
                    ..Options::default()
                },
            );
            sender.send((failing, result)).unwrap();
        });
        let (failing, result) = receiver
            .recv_timeout(Duration::from_secs(30))
            .expect("pipeline must stop and join after a storage failure");
        handle.join().unwrap();
        assert!(result.unwrap_err().to_string().contains("injected"));
        let target = scratch.path().join(format!("restore-{at}"));
        assert!(
            restore(&failing, &first.snapshot, &target, &[])
                .unwrap()
                .damaged
                .is_empty()
        );
        let rerun = backup_with_options(
            &failing,
            std::slice::from_ref(&source),
            vec![],
            &Options {
                jobs: 8,
                ..Options::default()
            },
        )
        .unwrap();
        if at == 11 {
            assert!(rerun.summary.bytes_added < 24 * 1024 * 1024);
        }
        let target = scratch.path().join(format!("rerun-{at}"));
        assert!(
            restore(&failing, &rerun.snapshot, &target, &[])
                .unwrap()
                .damaged
                .is_empty()
        );
        assert_eq!(
            fs::read(
                target
                    .join(source.strip_prefix("/").unwrap())
                    .join("new-47")
            )
            .unwrap(),
            bytes
        );
    }
}
