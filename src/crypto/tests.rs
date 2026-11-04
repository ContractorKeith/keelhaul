use super::{
    envelope::{self, Label},
    keys::MasterKey,
    slot,
};
use crate::{
    format::{
        config::Config,
        key_slot::{Kdf, KeySlot},
    },
    secret::Secret,
};
use base64::{Engine, prelude::BASE64_STANDARD};
use serde_json::{Value, json};

fn fixed_master() -> MasterKey {
    MasterKey::from_bytes(std::array::from_fn(|i| i as u8))
}
fn fixed_nonce() -> [u8; 24] {
    std::array::from_fn(|i| i as u8)
}

#[test]
#[ignore = "explicit vector regeneration only"]
fn generate_vectors() {
    fn write(name: &str, value: Value) {
        std::fs::write(
            format!("testdata/vectors/{name}.json"),
            format!("{}\n", serde_json::to_string_pretty(&value).unwrap()),
        )
        .unwrap();
    }
    let master = fixed_master();
    let keys = master.subkeys();
    write(
        "subkeys",
        json!({
            "master": hex::encode(master.0), "encryption": hex::encode(keys.encryption),
            "chunk_id": hex::encode(keys.chunk_id), "gear": hex::encode(keys.gear),
        }),
    );
    let vectors: Vec<_> = Label::ALL.into_iter().map(|label| {
        let payload = if label == Label::KeySlot { master.0.to_vec() } else { b"keelhaul vector payload".to_vec() };
        json!({"label": label.as_str(), "key": hex::encode(keys.encryption), "nonce": hex::encode(fixed_nonce()),
            "payload": hex::encode(&payload),
            "envelope": hex::encode(envelope::seal_with_nonce(&keys.encryption, label, &payload, &fixed_nonce()).unwrap())})
    }).collect();
    write("envelope", json!(vectors));
    for (name, secret, kdf) in [
        (
            "slot-password",
            Secret::Password("vector password".into()),
            Some(Kdf::new(8192, 1, 1, &[0x42; 16]).unwrap()),
        ),
        ("slot-keyfile", Secret::KeyFile([0x24; 32]), None),
    ] {
        let kek = slot::kek(&secret, kdf.as_ref()).unwrap();
        let mut slot = slot::wrap(&master, &secret, kdf).unwrap();
        slot.created = "2026-11-04T14:03:22.123456789Z".into();
        slot.host = "vector-host".into();
        slot.username = "vector-user".into();
        slot.wrapped = BASE64_STANDARD.encode(
            envelope::seal_with_nonce(&kek, Label::KeySlot, &master.0, &fixed_nonce()).unwrap(),
        );
        let secret_value = match secret {
            Secret::Password(pw) => pw,
            Secret::KeyFile(bytes) => hex::encode(bytes),
        };
        write(
            name,
            json!({"master": hex::encode(master.0), "secret": secret_value, "nonce": hex::encode(fixed_nonce()), "slot": slot}),
        );
    }
    let mut config = serde_json::to_value(Config::new().unwrap()).unwrap();
    config["id"] = json!("aa".repeat(32));
    config["created"] = json!("2026-11-04T14:03:22.123456789Z");
    write("config", config);
}

#[test]
fn subkey_vector() {
    let v: Value = crate::vectors::load("subkeys.json");
    let master = MasterKey::from_bytes(
        hex::decode(v["master"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap(),
    );
    let keys = master.subkeys();
    for (name, key) in [
        ("encryption", keys.encryption),
        ("chunk_id", keys.chunk_id),
        ("gear", keys.gear),
    ] {
        assert_eq!(hex::encode(key), v[name].as_str().unwrap());
    }
}

#[test]
fn envelope_vectors() {
    let vectors: Vec<Value> = crate::vectors::load("envelope.json");
    assert_eq!(vectors.len(), Label::ALL.len());
    for (label, v) in Label::ALL.into_iter().zip(vectors) {
        assert_eq!(label.as_str(), v["label"]);
        let key: [u8; 32] = hex::decode(v["key"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        let nonce: [u8; 24] = hex::decode(v["nonce"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        let payload = hex::decode(v["payload"].as_str().unwrap()).unwrap();
        let stored = hex::decode(v["envelope"].as_str().unwrap()).unwrap();
        assert_eq!(
            envelope::seal_with_nonce(&key, label, &payload, &nonce).unwrap(),
            stored
        );
        assert_eq!(envelope::open(&key, label, &stored).unwrap(), payload);
    }
}

#[test]
fn slot_vectors_and_tampering() {
    for (name, password) in [("slot-password.json", true), ("slot-keyfile.json", false)] {
        let v: Value = crate::vectors::load(name);
        let mut slot = KeySlot::parse(&serde_json::to_vec(&v["slot"]).unwrap()).unwrap();
        let secret = if password {
            Secret::Password(v["secret"].as_str().unwrap().into())
        } else {
            Secret::KeyFile(
                hex::decode(v["secret"].as_str().unwrap())
                    .unwrap()
                    .try_into()
                    .unwrap(),
            )
        };
        assert_eq!(
            hex::encode(slot::unwrap(&slot, &secret).unwrap().0),
            v["master"]
        );
        let wrapped = BASE64_STANDARD.decode(&slot.wrapped).unwrap();
        let positions = if password {
            vec![30]
        } else {
            (0..wrapped.len()).collect()
        };
        for position in positions {
            let mut corrupt = wrapped.clone();
            corrupt[position] ^= 1;
            slot.wrapped = BASE64_STANDARD.encode(corrupt);
            assert!(slot::unwrap(&slot, &secret).is_err());
        }
    }
}

#[test]
fn kdf_bounds_rejected_before_hashing() {
    let v: Value = crate::vectors::load("slot-password.json");
    for (field, bad) in [
        ("m_kib", 8191),
        ("m_kib", 4194305),
        ("m_kib", 1 << 30),
        ("t", 0),
        ("t", 11),
        ("p", 0),
        ("p", 17),
    ] {
        let mut value = v["slot"].clone();
        value["kdf"][field] = json!(bad);
        // Deserialize internally to exercise the KDF boundary as well as the format parser.
        let slot: KeySlot = serde_json::from_value(value.clone()).unwrap();
        let start = std::time::Instant::now();
        assert!(slot::unwrap(&slot, &Secret::Password("pw".into())).is_err());
        assert!(start.elapsed() < std::time::Duration::from_millis(50));
        assert!(KeySlot::parse(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    assert!(Kdf::new(8192, 1, 1, &[0; 15]).is_err());
}

#[test]
fn config_vector_and_refusals() {
    let v: Value = crate::vectors::load("config.json");
    let config = Config::parse(&serde_json::to_vec(&v).unwrap()).unwrap();
    assert_eq!(serde_json::to_value(config).unwrap(), v);
    for (field, bad, message) in [
        ("version", json!(2), "2"),
        (
            "chunker",
            json!({"alg":"unknown", "min":1,"avg":2,"max":3}),
            "unknown",
        ),
    ] {
        let mut bad_config = v.clone();
        bad_config[field] = bad;
        assert!(
            Config::parse(&serde_json::to_vec(&bad_config).unwrap())
                .unwrap_err()
                .contains(message)
        );
    }
    let mut extra = v;
    extra["future"] = json!(true);
    assert!(Config::parse(&serde_json::to_vec(&extra).unwrap()).is_ok());
}

#[test]
fn config_pack_targets_and_strict_timestamps() {
    let mut v: Value = crate::vectors::load("config.json");
    for target in [1 << 20, 32 << 20, 256 << 20] {
        v["pack_target"] = json!(target);
        assert!(Config::parse(&serde_json::to_vec(&v).unwrap()).is_ok());
    }
    for target in [0, (1 << 20) - 1, (256 << 20) + 1] {
        v["pack_target"] = json!(target);
        assert!(Config::parse(&serde_json::to_vec(&v).unwrap()).is_err());
    }
    v["pack_target"] = json!(32 << 20);
    for time in [
        "2026-11-04T14:03:22+02:00",
        "2026-11-04T14:03:22Z",
        "2026-11-04T14:03:22.12345678Z",
        "2026-11-04t14:03:22.123456789Z",
    ] {
        v["created"] = json!(time);
        assert!(
            Config::parse(&serde_json::to_vec(&v).unwrap()).is_err(),
            "{time}"
        );
        let mut slot: Value = crate::vectors::load("slot-keyfile.json");
        slot["slot"]["created"] = json!(time);
        assert!(KeySlot::parse(&serde_json::to_vec(&slot["slot"]).unwrap()).is_err());
    }
}
