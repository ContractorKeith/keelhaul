use super::{keys::MasterKey, slot};
use crate::format::key_slot::KeySlot;
use serde_json::{Value, json};
fn fixed_master() -> MasterKey {
    MasterKey::from_bytes(std::array::from_fn(|i| i as u8))
}

#[test]
#[ignore = "M3 adds checks to existing vectors without regenerating their envelopes"]
fn add_slot_checks() {
    for name in ["slot-password", "slot-keyfile"] {
        let path = format!("testdata/vectors/{name}.json");
        let mut value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        value["slot"]["check"] = json!("00".repeat(32));
        let slot = KeySlot::parse(&serde_json::to_vec(&value["slot"]).unwrap()).unwrap();
        value["slot"]["check"] =
            json!(slot::check(&slot, &fixed_master().subkeys().key_slot_check).unwrap());
        std::fs::write(
            path,
            format!("{}\n", serde_json::to_string_pretty(&value).unwrap()),
        )
        .unwrap();
    }
    let path = "testdata/vectors/subkeys.json";
    let mut value: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    value["key_slot_check"] = json!(hex::encode(fixed_master().subkeys().key_slot_check));
    std::fs::write(
        path,
        format!("{}\n", serde_json::to_string_pretty(&value).unwrap()),
    )
    .unwrap();
}
