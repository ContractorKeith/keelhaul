use keelhaul::{
    backend::{Backend, Kind, memory::MemoryBackend},
    repo::{Error, Repository},
    secret::Secret,
};
#[test]
fn a_foreign_slot_does_not_allow_removing_the_last_genuine_slot() {
    let secret = Secret::KeyFile([7; 32]);
    let repo = Repository::init(MemoryBackend::new(), &secret).unwrap();
    let genuine = repo.backend.list(Kind::Keys).unwrap()[0].name.clone();
    let foreign = Repository::init(MemoryBackend::new(), &secret).unwrap();
    let other = foreign.backend.list(Kind::Keys).unwrap()[0].name.clone();
    repo.backend
        .plant_raw(other.as_str(), &foreign.backend.read(&other).unwrap())
        .unwrap();
    let error = repo.remove_key(&genuine.as_str()[5..]).unwrap_err();
    assert!(matches!(error, Error::Refused(_)));
    assert!(error.to_string().contains("valid check tag"));
    assert!(repo.backend.read(&genuine).is_ok());
    repo.remove_key(&other.as_str()[5..]).unwrap();
}
