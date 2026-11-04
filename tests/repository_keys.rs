use keelhaul::{
    backend::{Backend, ObjectName, memory::MemoryBackend},
    crypto::envelope::{self, Label},
    repo::{Error, Repository},
    secret::Secret,
};

#[test]
fn rotation_wrong_secret_last_slot_and_config_damage() {
    let backend = MemoryBackend::new();
    let original = Secret::KeyFile([1; 32]);
    let repo = Repository::init(backend, &original).unwrap();
    assert!(repo.remove_key(&repo.list_keys().unwrap()[0].0).is_err());
    let added = repo.add_key(&Secret::KeyFile([2; 32])).unwrap();
    let original_id = repo
        .list_keys()
        .unwrap()
        .into_iter()
        .find(|(id, _)| id != &added)
        .unwrap()
        .0;
    repo.remove_key(&original_id).unwrap();
    let repo = Repository::open(repo.backend, &Secret::KeyFile([2; 32])).unwrap();
    assert!(matches!(
        Repository::open(repo.backend, &original),
        Err(Error::WrongSecret)
    ));
}

#[test]
fn config_corruption_and_missing_are_damage() {
    for bytes in [Some(vec![0; 90]), None] {
        let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([3; 32])).unwrap();
        repo.backend
            .delete(&ObjectName::new("config").unwrap())
            .unwrap();
        if let Some(bytes) = bytes {
            repo.backend.plant_raw("config", &bytes).unwrap();
        }
        assert!(
            matches!(Repository::open(repo.backend, &Secret::KeyFile([3; 32])), Err(Error::Damage { object, .. }) if object == "config")
        );
    }
}

#[test]
fn slots_tried_in_order_until_one_decrypts() {
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([3; 32])).unwrap();
    let other = Repository::init(MemoryBackend::new(), &Secret::KeyFile([4; 32])).unwrap();
    let wrong_slot = serde_json::to_vec(&other.list_keys().unwrap()[0].1).unwrap();
    repo.backend
        .plant_raw(&format!("keys/{}", "01".repeat(32)), &wrong_slot)
        .unwrap();
    let slot = serde_json::to_value(
        &repo
            .list_keys()
            .unwrap()
            .into_iter()
            .find(|(id, _)| id != &"01".repeat(32))
            .unwrap()
            .1,
    )
    .unwrap();
    let bytes = serde_json::to_vec(&slot).unwrap();
    repo.backend
        .plant_raw(&format!("keys/{}", "00".repeat(32)), b"broken")
        .unwrap();
    repo.backend
        .plant_raw(&format!("keys/{}", "ff".repeat(32)), &bytes)
        .unwrap();
    assert!(Repository::open(repo.backend, &Secret::KeyFile([3; 32])).is_ok());
}

#[test]
fn init_refuses_existing_repository_without_rewriting() {
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([1; 32])).unwrap();
    assert!(matches!(
        Repository::init(repo.backend, &Secret::KeyFile([2; 32])),
        Err(Error::Refused(_))
    ));
}

#[test]
fn envelope_random_roundtrips_and_nonces() {
    for len in [0, 1, 31, 1024, 524288] {
        let mut payload = vec![0; len];
        getrandom::fill(&mut payload).unwrap();
        let first = envelope::seal(&[7; 32], Label::Data, &payload).unwrap();
        let second = envelope::seal(&[7; 32], Label::Data, &payload).unwrap();
        assert_ne!(&first[..24], &second[..24]);
        assert_eq!(
            envelope::open(&[7; 32], Label::Data, &first).unwrap(),
            payload
        );
    }
}
