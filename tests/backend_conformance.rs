#![allow(clippy::unwrap_used, reason = "test assertions and fixtures may panic")]
mod common;
use common::backend_suite::{name, run_suite};
use keelhaul::backend::{Backend, BackendError, local::LocalBackend, memory::MemoryBackend};
use std::{path::Path, sync::Barrier};

fn local_suite(root: &Path) {
    run_suite(&LocalBackend::new(root).unwrap(), |raw, bytes| {
        let path = root.join(raw);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    });
}

#[test]
fn memory_conformance() {
    let b = MemoryBackend::new();
    run_suite(&b, |raw, bytes| b.plant_raw(raw, bytes).unwrap());
}

#[test]
fn local_conformance() {
    let dir = tempfile::tempdir().unwrap();
    local_suite(dir.path());
}

#[test]
fn independent_local_instances_do_not_replace_each_other() {
    let dir = tempfile::tempdir().unwrap();
    let barrier = Barrier::new(8);
    std::thread::scope(|scope| {
        let writers: Vec<_> = (0..8_u8)
            .map(|id| {
                let barrier = &barrier;
                let root = dir.path();
                scope.spawn(move || {
                    let backend = LocalBackend::new(root).unwrap();
                    barrier.wait();
                    backend.write(&name("config"), &[id])
                })
            })
            .collect();
        let results: Vec<_> = writers.into_iter().map(|w| w.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert!(
            results
                .iter()
                .all(|r| r.is_ok() || matches!(r, Err(BackendError::AlreadyExists(_))))
        );
    });
}

#[cfg(target_os = "linux")]
#[test]
fn local_handles_non_utf8_repository_paths() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let dir = tempfile::tempdir().unwrap();
    let root = dir
        .path()
        .join(OsString::from_vec(b"repository-\xff".to_vec()));
    local_suite(&root);
}
