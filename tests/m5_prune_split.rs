#[path = "common/m5_prune_fixture.rs"]
mod setup;
#[path = "common/prune_pause.rs"]
mod trace;
use keelhaul::{
    backend::{Backend, Kind, ObjectName},
    crypto::envelope::Label,
    format::index::Index,
    ops::{check::check, prune::prune_with_index_limit, restore::restore, verify::verify},
};
use std::{collections::HashSet, sync::atomic::Ordering};
#[test]
fn split_indexes_are_additive_until_last_commit_and_safe_between_writes() {
    let (mut repo, snapshot, contents) = setup::fixture_with_packs(trace::Paused::default(), true);
    // Keep rescued blobs in small packs so the seam tests index splitting.
    repo.config.pack_target = 1000;
    repo.backend.index_calls.store(0, Ordering::SeqCst);
    repo.backend.index_writes.lock().unwrap().clear();
    repo.backend.panic_index_at.store(2, Ordering::SeqCst);
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| prune_with_index_limit(
            &repo, 100, false, 1100
        )))
        .is_err()
    );
    let partial = repo.backend.index_writes.lock().unwrap().clone();
    assert_eq!(partial.len(), 2);
    for id in partial {
        let index = Index::parse(
            &repo
                .read_object(
                    &ObjectName::new(&format!("index/{id}")).unwrap(),
                    Label::Index,
                )
                .unwrap(),
        )
        .unwrap();
        assert!(index.supersedes.is_empty());
    }
    assert!(check(&repo).unwrap().damage.is_empty());
    assert!(verify(&repo, None).unwrap().damage.is_empty());
    let scratch = tempfile::tempdir().unwrap();
    restore(&repo, &snapshot, &scratch.path().join("restored"), &[]).unwrap();
    assert_eq!(
        std::fs::read(scratch.path().join("restored/file")).unwrap(),
        contents[..7].concat()
    );
    repo.backend.panic_index_at.store(0, Ordering::SeqCst);
    repo.backend.index_writes.lock().unwrap().clear();
    let before = repo
        .backend
        .list(Kind::Index)
        .unwrap()
        .iter()
        .map(|e| e.name.as_str()[6..].to_owned())
        .collect::<HashSet<_>>();
    prune_with_index_limit(&repo, 100, false, 1100).unwrap();
    let after = repo.backend.list(Kind::Index).unwrap();
    assert!(after.len() >= 3);
    let written = repo.backend.index_writes.lock().unwrap();
    let last = written.last().unwrap();
    let ids = after
        .iter()
        .map(|e| e.name.as_str()[6..].to_owned())
        .collect::<HashSet<_>>();
    let mut expected = before;
    expected.extend(written.iter().filter(|id| !ids.contains(*id)).cloned());
    for entry in after {
        let bytes = repo.read_object(&entry.name, Label::Index).unwrap();
        assert!(bytes.len() <= 1100);
        let index = Index::parse(&bytes).unwrap();
        if &entry.name.as_str()[6..] == last {
            assert_eq!(
                index.supersedes.into_iter().collect::<HashSet<_>>(),
                expected
            );
        } else {
            assert!(index.supersedes.is_empty());
        }
    }
    assert!(verify(&repo, None).unwrap().damage.is_empty());
}
