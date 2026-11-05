use keelhaul::{
    backend::{Backend, Kind, ObjectName, memory::MemoryBackend},
    ops::{check::check_backend, key, verify::verify_backend},
    repo::{Error, Repository},
    secret::Secret,
};
#[test]
fn damaged_check_still_opens_and_all_slots_can_be_managed() {
    for secret in [
        Secret::KeyFile([7; 32]),
        Secret::Password("recovery password".into()),
    ] {
        let repo = Repository::init(MemoryBackend::new(), &secret).unwrap();
        let old = repo.backend.list(Kind::Keys).unwrap()[0].name.clone();
        let mut value: serde_json::Value =
            serde_json::from_slice(&repo.backend.read(&old).unwrap()).unwrap();
        value["check"] = "00".repeat(32).into();
        repo.backend
            .plant_raw(old.as_str(), &serde_json::to_vec(&value).unwrap())
            .unwrap();
        let repo = Repository::open(repo.backend, &secret).unwrap();
        for report in [
            check_backend(&repo.backend, &secret).unwrap(),
            verify_backend(&repo.backend, &secret, None).unwrap(),
        ] {
            assert!(
                report
                    .damage
                    .iter()
                    .any(|d| d.object == old.as_str() && d.problem == "hash-mismatch")
            );
        }
        let new = repo.add_key(&Secret::KeyFile([8; 32])).unwrap();
        let junk = ObjectName::new(&format!("keys/{}", "f".repeat(64))).unwrap();
        repo.backend.plant_raw(junk.as_str(), b"junk").unwrap();
        let slots = key::list(&repo).unwrap();
        assert_eq!(
            slots
                .iter()
                .find(|s| s.id == old.as_str()[5..])
                .unwrap()
                .status,
            "damaged"
        );
        assert_eq!(slots.iter().find(|s| s.id == new).unwrap().status, "ok");
        assert_eq!(
            slots
                .iter()
                .find(|s| s.id == junk.as_str()[5..])
                .unwrap()
                .status,
            "unreadable"
        );
        repo.remove_key(&old.as_str()[5..]).unwrap();
        repo.remove_key(&junk.as_str()[5..]).unwrap();
        assert!(
            check_backend(&repo.backend, &Secret::KeyFile([8; 32]))
                .unwrap()
                .damage
                .is_empty()
        );
    }
}
#[test]
fn unreadable_slots_without_an_opening_slot_are_damage() {
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([7; 32])).unwrap();
    let slot = repo.backend.list(Kind::Keys).unwrap()[0].name.clone();
    repo.backend.plant_raw(slot.as_str(), b"junk").unwrap();
    let error = Repository::open(repo.backend, &Secret::KeyFile([7; 32]))
        .err()
        .unwrap();
    assert!(matches!(&error, Error::Damage { object, .. } if object == slot.as_str()));
}
