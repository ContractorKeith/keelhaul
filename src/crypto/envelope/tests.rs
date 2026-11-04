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
