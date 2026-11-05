use keelhaul::ops::forget::Options;
#[test]
fn zero_only_policies_are_refused_but_tags_and_explicit_ids_work() {
    for options in [
        Options {
            keep_last: Some(0),
            ..Default::default()
        },
        Options {
            keep_hourly: Some(0),
            keep_daily: Some(0),
            keep_yearly: Some(0),
            ..Default::default()
        },
    ] {
        assert!(matches!(
            options.validate(),
            Err(keelhaul::repo::Error::Usage(_))
        ));
    }
    assert!(
        Options {
            keep_last: Some(0),
            keep_tag: vec!["keep".into()],
            ..Default::default()
        }
        .validate()
        .is_ok()
    );
    assert!(
        Options {
            snapshots: vec!["12345678".into()],
            ..Default::default()
        }
        .validate()
        .is_ok()
    );
}

#[test]
fn supplied_local_offset_survives_a_running_backend_thread() {
    use keelhaul::backend::Backend;
    use keelhaul::{
        backend::memory::MemoryBackend,
        crypto::envelope::Label,
        ops::{backup::backup, forget::forget_with_offset},
        repo::Repository,
        secret::Secret,
    };
    let scratch = tempfile::tempdir().unwrap();
    let source = scratch.path().join("file");
    std::fs::write(&source, b"data").unwrap();
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([1; 32])).unwrap();
    let first = backup(&repo, &[source], vec![]).unwrap();
    let (_, mut snapshot) = repo.select_snapshot(&first.snapshot).unwrap();
    repo.backend
        .delete(
            &keelhaul::backend::ObjectName::new(&format!("snapshots/{}", first.snapshot)).unwrap(),
        )
        .unwrap();
    let mut ids = Vec::new();
    for stamp in [
        "2026-11-04T06:00:00.000000000Z",
        "2026-11-04T02:00:00.000000000Z",
    ] {
        snapshot.time = stamp.into();
        ids.push(
            repo.write_object(
                "snapshots",
                Label::Snapshot,
                &serde_json::to_vec(&snapshot).unwrap(),
            )
            .unwrap(),
        );
    }
    let (send, recv) = std::sync::mpsc::channel::<()>();
    let thread = std::thread::spawn(move || recv.recv().unwrap());
    let report = forget_with_offset(
        &repo,
        &Options {
            keep_daily: Some(2),
            dry_run: true,
            ..Default::default()
        },
        Some(time::UtcOffset::from_hms(-5, 0, 0).unwrap()),
    )
    .unwrap();
    send.send(()).unwrap();
    thread.join().unwrap();
    // Both timestamps are Nov 4 UTC, but Nov 3 / Nov 4 in New York.
    assert_eq!(report.groups[0].keep.len(), 2);
}
