use keelhaul::{
    backend::memory::MemoryBackend,
    ops::{
        backup::{Options, backup, backup_with_options},
        restore::restore,
        verify::verify,
    },
    repo::Repository,
    secret::Secret,
};
use std::{
    fs,
    io::Write,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[test]
fn growing_file_restores_to_recorded_prefix_and_is_read_again() {
    let scratch = tempfile::tempdir().unwrap();
    let source = fs::canonicalize(scratch.path()).unwrap().join("log");
    let prefix = vec![42; 16 * 1024 * 1024];
    fs::write(&source, &prefix).unwrap();
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([11; 32])).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let path = source.clone();
    let writer = std::thread::spawn(move || {
        let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
        for _ in 0..2048 {
            if stopped.load(Ordering::Relaxed) {
                break;
            }
            file.write_all(&[43; 1024]).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    });
    let report = backup_with_options(
        &repo,
        std::slice::from_ref(&source),
        vec![],
        &Options {
            jobs: 1,
            ..Options::default()
        },
    );
    stop.store(true, Ordering::Relaxed);
    writer.join().unwrap();
    let report = report.unwrap();
    assert_eq!(report.summary.files_changed_during_read, 1);
    assert_eq!(report.summary.files_skipped, 0);
    let output = scratch.path().join("out");
    assert!(
        restore(&repo, &report.snapshot, &output, &[])
            .unwrap()
            .damaged
            .is_empty()
    );
    let bytes = fs::read(output.join(source.strip_prefix("/").unwrap())).unwrap();
    assert_eq!(bytes.len() as u64, report.summary.bytes_processed);
    assert!(fs::read(&source).unwrap().starts_with(&bytes));
    assert!(verify(&repo, None).unwrap().damage.is_empty());
    assert_eq!(
        backup(&repo, &[source], vec![]).unwrap().summary.files_read,
        1
    );
}
