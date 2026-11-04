use serde::de::DeserializeOwned;
use std::{fs, path::Path};

pub(crate) fn load<T: DeserializeOwned>(name: &str) -> T {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata/vectors")
        .join(name);
    let bytes = fs::read(&path)
        .unwrap_or_else(|error| panic!("cannot load vector {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("invalid vector {}: {error}", path.display()))
}

#[test]
fn hex_roundtrip() {
    #[derive(serde::Deserialize)]
    struct Vector {
        text: String,
        hex: String,
    }
    let vector: Vector = load("hex-roundtrip.json");
    assert_eq!(hex::encode(vector.text.as_bytes()), vector.hex);
    assert_eq!(hex::decode(vector.hex).unwrap(), vector.text.as_bytes());
}

#[test]
#[should_panic(expected = "cannot load vector")]
fn missing_vector_fails_clearly() {
    let _: serde_json::Value = load("missing.json");
}
