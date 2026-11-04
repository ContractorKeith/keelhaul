use keelhaul::format::{
    index::Index,
    pack::{Header, header_length},
    snapshot::{Snapshot, decode_root, root_name},
    tree::Tree,
};
use serde_json::{Value, json};
fn raw(v: &Value) -> Vec<u8> {
    serde_json::to_vec(v).unwrap()
}
fn entry() -> Value {
    json!({"name":"file","type":"file","mode":420,"mtime":"2026-11-04T14:03:22.123456789Z","ctime":"2026-11-04T14:03:22.123456789Z","uid":1,"gid":1,"inode":1,"device":1,"links":1,"size":1,"content":["ab".repeat(32)]})
}
#[test]
fn tree_roundtrip_and_every_invariant() {
    let valid = json!({"version":1,"entries":[entry()]});
    let t = Tree::parse(&raw(&valid)).unwrap();
    assert_eq!(serde_json::to_value(&t).unwrap(), valid);
    for (key, value) in [
        ("name", json!("")),
        ("name", json!(".")),
        ("name", json!("..")),
        ("name", json!("a/b")),
        ("name", json!("a\u{0000}b")),
        ("mode", json!(0o100644)),
        ("mtime", json!("2026-11-04T14:03:22Z")),
        ("ctime", json!("2026-11-04T14:03:22.123456789+00:00")),
        ("content", json!(["bad"])),
        ("size", json!(0)),
        ("type", json!("socket")),
        ("uid", json!(-1)),
    ] {
        let mut v = valid.clone();
        v["entries"][0][key] = value;
        assert!(Tree::parse(&raw(&v)).is_err(), "{key}: {v}");
    }
    let mut v = valid.clone();
    v["version"] = 2.into();
    assert!(Tree::parse(&raw(&v)).is_err());
    let mut v = valid.clone();
    v["entries"].as_array_mut().unwrap().push(entry());
    assert!(Tree::parse(&raw(&v)).is_err());
    let mut v = valid.clone();
    v["entries"][0]["name"] = "z".into();
    v["entries"].as_array_mut().unwrap().push(entry());
    assert!(Tree::parse(&raw(&v)).is_err());
    let mut v = valid.clone();
    v["entries"][0]["name_b64"] = "/w==".into();
    assert!(Tree::parse(&raw(&v)).is_err());
    v["entries"][0].as_object_mut().unwrap().remove("name");
    assert!(Tree::parse(&raw(&v)).is_ok());
    v["entries"][0]["name_b64"] = "Li4=".into();
    assert!(Tree::parse(&raw(&v)).is_err());
    v["entries"][0]["name_b64"] = "???".into();
    assert!(Tree::parse(&raw(&v)).is_err());
    for node in [
        json!({"type":"dir","subtree":"bad"}),
        json!({"type":"symlink","target":"a\u{0000}b"}),
        json!({"type":"symlink"}),
        json!({"type":"symlink","target":"x","target_b64":"/w=="}),
    ] {
        let mut e = entry();
        e.as_object_mut()
            .unwrap()
            .extend(node.as_object().unwrap().clone());
        let v = json!({"version":1,"entries":[e]});
        assert!(Tree::parse(&raw(&v)).is_err());
    }
    let mut extra = valid;
    extra["future"] = "ignored".into();
    assert!(Tree::parse(&raw(&extra)).is_ok());
}
#[test]
fn pack_refuses_gaps_overlap_duplicate_overflow_bad_ids_lengths_versions_and_coverage() {
    let b = json!({"id":"ab".repeat(32),"type":"data","offset":0,"length":42,"raw_length":1});
    let good = json!({"version":1,"blobs":[b]});
    let h = Header::parse(&raw(&good), 42).unwrap();
    assert_eq!(serde_json::to_value(h).unwrap(), good);
    for (key, value) in [
        ("id", json!("bad")),
        ("type", json!("other")),
        ("offset", json!(1)),
        ("length", json!(40)),
        ("length", json!(43)),
        ("raw_length", json!(0)),
        ("raw_length", json!(8388609)),
    ] {
        let mut v = good.clone();
        v["blobs"][0][key] = value;
        assert!(Header::parse(&raw(&v), 42).is_err(), "{key}");
    }
    let mut v = good.clone();
    v["blobs"] = json!([]);
    assert!(Header::parse(&raw(&v), 0).is_err());
    let mut v = good.clone();
    v["version"] = 2.into();
    assert!(Header::parse(&raw(&v), 42).is_err());
    let mut v = good.clone();
    let mut second = v["blobs"][0].clone();
    second["offset"] = 42.into();
    v["blobs"].as_array_mut().unwrap().push(second);
    assert!(Header::parse(&raw(&v), 84).is_err());
    v["blobs"][1]["id"] = "cd".repeat(32).into();
    assert!(Header::parse(&raw(&v), 84).is_ok());
    v["blobs"][1]["offset"] = 41.into();
    assert!(Header::parse(&raw(&v), 84).is_err());
    v["blobs"][1]["offset"] = 43.into();
    assert!(Header::parse(&raw(&v), 84).is_err());
    assert!(Header::parse(&raw(&good), 43).is_err());
    for len in [0u32, 40, 16 * 1024 * 1024 + 1, u32::MAX] {
        assert!(header_length(&len.to_le_bytes(), u64::MAX).is_err());
    }
    assert!(header_length(&41u32.to_le_bytes(), 44).is_err());
    assert!(header_length(&41u32.to_le_bytes(), 45).is_ok());
    assert!(header_length(&[0; 3], 45).is_err());
}
#[test]
fn snapshot_and_index_validation_and_root_encoding() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/vectors");
    let snapshot: Value =
        serde_json::from_slice(&std::fs::read(path.join("snapshot.json")).unwrap()).unwrap();
    Snapshot::parse(&raw(&snapshot)).unwrap();
    let mut absent = snapshot.clone();
    absent.as_object_mut().unwrap().remove("parent");
    assert!(Snapshot::parse(&raw(&absent)).is_err());
    for (k, v) in [
        ("version", json!(2)),
        ("time", json!("2026-11-04T00:00:00.1Z")),
        ("tree", json!("bad")),
        ("parent", json!("bad")),
        ("paths", json!([])),
        ("paths", json!(["relative"])),
        ("paths", json!(["/a/../b"])),
        ("paths", json!(["/a//b"])),
        ("paths", json!(["/b", "/a"])),
        ("paths", json!(["/a", "/a"])),
    ] {
        let mut s = snapshot.clone();
        s[k] = v;
        assert!(Snapshot::parse(&raw(&s)).is_err(), "{k}");
    }
    let index: Value =
        serde_json::from_slice(&std::fs::read(path.join("index.json")).unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(Index::parse(&raw(&index)).unwrap()).unwrap(),
        index
    );
    for (k, v) in [("version", json!(2)), ("supersedes", json!(["bad"]))] {
        let mut i = index.clone();
        i[k] = v;
        assert!(Index::parse(&raw(&i)).is_err());
    }
    let mut i = index;
    i["packs"][0]["id"] = "bad".into();
    assert!(Index::parse(&raw(&i)).is_err());
    for p in ["/", "/a/b", "/a%2F/b%25"] {
        assert_eq!(decode_root(&root_name(p)).unwrap(), p);
    }
    for p in ["%2F..%2Fescape", "%2Fa%00", "relative", "%2Fa%2F%2Fb"] {
        assert!(decode_root(p).is_err());
    }
}
