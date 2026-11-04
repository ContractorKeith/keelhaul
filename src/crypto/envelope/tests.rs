use super::*;

#[test]
fn labels_flags_and_compression() {
    for label in Label::ALL {
        let payload = if label == Label::KeySlot {
            vec![7; 32]
        } else {
            vec![7; 8192]
        };
        let stored = seal(&[4; 32], label, &payload).unwrap();
        assert_eq!(open(&[4; 32], label, &stored).unwrap(), payload);
        let wrong = if label == Label::Config {
            Label::Data
        } else {
            Label::Config
        };
        assert!(matches!(
            open(&[4; 32], wrong, &stored),
            Err(Error::Undecryptable)
        ));
        for length in [0, 1, 23, 39] {
            assert!(matches!(
                open(&[4; 32], label, &stored[..length]),
                Err(Error::BadFormat(_))
            ));
        }
        let mut corrupt = stored;
        corrupt[24] ^= 1;
        assert!(matches!(
            open(&[4; 32], label, &corrupt),
            Err(Error::Undecryptable)
        ));
    }
    let stored = encrypt(&[4; 32], Label::Data, &[2, 0], &[0; 24]).unwrap();
    assert!(matches!(
        open(&[4; 32], Label::Data, &stored),
        Err(Error::BadFormat(_))
    ));
}

#[test]
fn refuses_zstd_bomb_with_bounded_streaming_read() {
    let compressed = zstd::stream::encode_all(std::io::repeat(0).take(MAX_PAYLOAD + 1), 3).unwrap();
    let mut plaintext = vec![1];
    plaintext.extend(compressed);
    let stored = encrypt(&[4; 32], Label::Data, &plaintext, &[0; 24]).unwrap();
    assert!(matches!(
        open(&[4; 32], Label::Data, &stored),
        Err(Error::BadFormat(_))
    ));
}

#[test]
fn rejects_invalid_zstd_and_extra_frames() {
    let frame = zstd::stream::encode_all(&b"payload"[..], 3).unwrap();
    for bytes in [
        b"not zstd".to_vec(),
        [frame.as_slice(), frame.as_slice()].concat(),
        [frame.as_slice(), b"junk"].concat(),
    ] {
        let mut plaintext = vec![1];
        plaintext.extend(bytes);
        let stored = encrypt(&[4; 32], Label::Data, &plaintext, &[0; 24]).unwrap();
        assert!(matches!(
            open(&[4; 32], Label::Data, &stored),
            Err(Error::BadFormat(_))
        ));
    }
}

#[test]
fn every_stored_byte_is_authenticated() {
    let original = seal(&[4; 32], Label::Config, b"config").unwrap();
    for i in 0..original.len() {
        let mut corrupt = original.clone();
        corrupt[i] ^= 1;
        assert!(
            matches!(
                open(&[4; 32], Label::Config, &corrupt),
                Err(Error::Undecryptable)
            ),
            "byte {i}"
        );
    }
}

#[test]
fn refuses_skippable_only_frame_and_large_window_header() {
    let skippable = [0x50, 0x2a, 0x4d, 0x18, 0, 0, 0, 0];
    let stored = encrypt(
        &[4; 32],
        Label::Data,
        &[&[1], &skippable[..]].concat(),
        &[0; 24],
    )
    .unwrap();
    assert!(matches!(
        open(&[4; 32], Label::Data, &stored),
        Err(Error::BadFormat(_))
    ));
    // A streamed frame with descriptor 0x90 advertises a 256 MiB window.
    let mut frame = zstd::stream::encode_all(&vec![42; 8192][..], 3).unwrap();
    assert_eq!(
        frame[4] & 0x20,
        0,
        "streaming encoder uses an explicit window"
    );
    frame[5] = 0x90;
    let stored = encrypt(
        &[4; 32],
        Label::Data,
        &[&[1], &frame[..]].concat(),
        &[0; 24],
    )
    .unwrap();
    assert!(matches!(
        open(&[4; 32], Label::Data, &stored),
        Err(Error::BadFormat(_))
    ));
}

#[test]
#[ignore = "explicit compressed vector creation only; existing vectors stay unchanged"]
fn generate_compressed_vector() {
    let key = [0x37; 32];
    let nonce = [0x17; 24];
    let payload = b"keelhaul compressed vector ".repeat(64);
    let stored = seal_with_nonce(&key, Label::Data, &payload, &nonce).unwrap();
    let v = serde_json::json!({
        "label": Label::Data.as_str(), "key": hex::encode(key), "nonce": hex::encode(nonce),
        "payload": hex::encode(payload), "compression": 1, "envelope": hex::encode(stored),
    });
    std::fs::write(
        "testdata/vectors/envelope-compressed.json",
        format!(
            "{}
",
            serde_json::to_string_pretty(&v).unwrap()
        ),
    )
    .unwrap();
}

#[test]
fn compressed_envelope_vector() {
    let v: serde_json::Value = crate::vectors::load("envelope-compressed.json");
    let key: [u8; 32] = hex::decode(v["key"].as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap();
    let nonce: [u8; 24] = hex::decode(v["nonce"].as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap();
    let stored = hex::decode(v["envelope"].as_str().unwrap()).unwrap();
    let payload = hex::decode(v["payload"].as_str().unwrap()).unwrap();
    assert_eq!(
        seal_with_nonce(&key, Label::Data, &payload, &nonce).unwrap(),
        stored
    );
    assert_eq!(open(&key, Label::Data, &stored).unwrap(), payload);
    let plaintext = XChaCha20Poly1305::new((&key).into())
        .decrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &stored[24..],
                aad: Label::Data.as_str().as_bytes(),
            },
        )
        .unwrap();
    assert_eq!(plaintext[0], 1);
}
