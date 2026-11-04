# Repository format vectors

The test-only loader in `src/vectors.rs` reads JSON by filename and reports missing
or malformed files with their path. `hex-roundtrip.json` exercises the loader in M0;
it is a harness fixture, not a repository-format vector.

SPEC §12 files (M1 present; M2 files planned):

- `subkeys.json`: fixed master key and all derived subkeys.
- `gear.json`: first eight gear entries and hash of all 256 entries.
- `cut-points.json`: seed, length (32 MiB), and chunk lengths.
- `chunk-ids.json`: plaintext and keyed chunk IDs.
- `envelope.json`: fixed nonce and input for each envelope label.
- `envelope-compressed.json`: a compressible data payload with flag 0x01.
- `slot-password.json`: fixed salt and m_kib 8192, t 1, p 1.
- `slot-keyfile.json`: fixed key file and wrapped master key.
- `pack.json`: two-blob pack.
- `index.json`, `tree.json`, `snapshot.json`: payloads.

M1 vectors are committed and exercised by tests. M2 supplies the remaining vectors
for independent reader implementations.

## Provenance

For PR #11, the cold reviewer independently recomputed subkeys.json, all eight
labels in envelope.json, and slot-keyfile.json using a pure-Python
BLAKE3/XChaCha20-Poly1305 implementation. All outputs matched. The Argon2id KEK
is not yet independently checked by that pure-Python reviewer. The author
separately cross-checked the password slot with libargon2 through argon2-cffi
and libsodium through PyNaCl; this is not a pure-Python Argon2 implementation.

envelope-compressed.json adds a fixed data envelope with a compressible payload
and compression flag 0x01. It was generated separately; existing vectors were
not regenerated. The compressed vector has not yet received the reviewer's
independent recomputation.
