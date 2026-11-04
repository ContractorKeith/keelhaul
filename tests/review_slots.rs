use base64::{Engine, prelude::BASE64_STANDARD};
use keelhaul::{
    backend::{ObjectName, memory::MemoryBackend},
    crypto::slot,
    ops::{check::check, verify::verify},
    repo::Repository,
    secret::Secret,
};
#[test]
fn non_opening_slots_authenticate_wrapped_tag_and_cost_without_their_secret() {
    let secret = Secret::KeyFile([7; 32]);
    let repo = Repository::init(MemoryBackend::new(), &secret).unwrap();
    let master = slot::unwrap(&repo.list_keys().unwrap()[0].1, &secret).unwrap();
    let secondary = slot::create(&master, &Secret::Password("other secret".into())).unwrap();
    let name = ObjectName::new(&format!("keys/{}", "ab".repeat(32))).unwrap();
    let original = serde_json::to_value(secondary).unwrap();
    for cost in [false, true] {
        let mut value = original.clone();
        if cost {
            value["kdf"]["m_kib"] = 65537.into();
        } else {
            let mut bytes = BASE64_STANDARD
                .decode(value["wrapped"].as_str().unwrap())
                .unwrap();
            *bytes.last_mut().unwrap() ^= 1;
            value["wrapped"] = BASE64_STANDARD.encode(bytes).into();
        }
        repo.backend
            .plant_raw(name.as_str(), &serde_json::to_vec(&value).unwrap())
            .unwrap();
        for report in [check(&repo).unwrap(), verify(&repo, None).unwrap()] {
            assert_eq!(report.damage.len(), 1);
            let d = &report.damage[0];
            assert_eq!(
                (&*d.object, &*d.kind, &*d.problem),
                (name.as_str(), "key", "hash-mismatch")
            );
        }
    }
}
#[test]
fn unlock_rejects_a_bad_check_after_successful_unwrapping() {
    let secret = Secret::KeyFile([7; 32]);
    let repo = Repository::init(MemoryBackend::new(), &secret).unwrap();
    let (id, slot) = repo.list_keys().unwrap().pop().unwrap();
    let mut value = serde_json::to_value(slot).unwrap();
    value["check"] = "00".repeat(32).into();
    repo.backend
        .plant_raw(&format!("keys/{id}"), &serde_json::to_vec(&value).unwrap())
        .unwrap();
    assert!(matches!(
        Repository::open(repo.backend, &secret),
        Err(keelhaul::repo::Error::Damage {
            problem: "hash-mismatch",
            ..
        })
    ));
}
