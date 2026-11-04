#![forbid(unsafe_code)]

/// SPEC §12's algorithms and payloads must ship as usable JSON fixtures.
#[test]
fn spec_vectors_exist_and_parse() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/vectors");
    for name in [
        "subkeys.json",
        "gear.json",
        "chunks.json",
        "chunk-id.json",
        "envelope.json",
        "envelope-compressed.json",
        "slot-password.json",
        "slot-keyfile.json",
        "pack.json",
        "index.json",
        "tree.json",
        "snapshot.json",
        "config.json",
    ] {
        let bytes = std::fs::read(root.join(name))
            .unwrap_or_else(|error| panic!("SPEC §12 vector {name} is missing: {error}"));
        serde_json::from_slice::<serde_json::Value>(&bytes)
            .unwrap_or_else(|error| panic!("SPEC §12 vector {name} is invalid JSON: {error}"));
    }
}
