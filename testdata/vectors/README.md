# Repository format vectors

The test-only loader in `src/vectors.rs` reads JSON by filename and reports missing
or malformed files with their path. `hex-roundtrip.json` exercises the loader in M0;
it is a harness fixture, not a repository-format vector.

SPEC §12 files:

- `subkeys.json`: fixed master key and all derived subkeys.
- `gear.json`: first eight gear entries and hash of all 256 entries.
- `chunks.json`: seed, length (32 MiB), and chunk lengths.
- `chunk-id.json`: plaintext and keyed chunk IDs.
- `envelope.json`: fixed nonce and input for each envelope label.
- `envelope-compressed.json`: a compressible data payload with flag 0x01.
- `slot-password.json`: fixed salt and m_kib 8192, t 1, p 1.
- `slot-keyfile.json`: fixed key file and wrapped master key.
- `pack.json`: two-blob pack.
- `index.json`, `tree.json`, `snapshot.json`: payloads.

M1 and M2 vectors are committed and exercised by tests.

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

## M2 production and independent check

`tests/m2_vectors.rs` produces the M2 files with
`KEELHAUL_GENERATE_VECTORS=1 cargo test --locked --test m2_vectors m2_vectors`.
The master key is the 32 bytes 00..1f from `subkeys.json`. `gear.json` hashes all
2048 little-endian gear bytes. `chunks.json` expands BLAKE3 XOF of the UTF-8 seed
`keelhaul-vector-stream` to 32 MiB. `chunk-id.json` hashes its literal payload.
The two-blob `pack.json` uses raw compression flags and nonce bytes 00, 01, and
02 repeated 24 times for data, tree, and header respectively. Its JSON header
uses the Rust struct field order; `stored_hex` pins those exact bytes. The index,
tree, and snapshot files are their literal payloads; tree IDs hash the compact
JSON payload embedded in the pack, without reserializing the pretty vector file.
The test opens each envelope, checks IDs and framing, and parses every payload.

`reference/chunker.py` independently transcribes SPEC §6 and compares both gear
and cut points, using Python's optional `blake3` package for hashing only. It was
run in an isolated venv with that package and matched both committed vectors.
The Rust test invokes it when python3 and the package are available; otherwise
it prints a skip note. This is an independent chunker implementation, not an
independent implementation of BLAKE3 itself. M2 pack encryption vectors have not
yet received an independent cryptographic recomputation.

`root-name.json` transcribes SPEC §10's percent-first encoding, including literal
`%`, `%2F`, and `%25` inside a path component. The vector test checks both
encoding and decoding; the backup/restore regression uses those names on disk.
M2 review round 1 corrected the tree file size to 15 bytes and regenerated its
pack, index and snapshot references. Vector tests now check file sizes against
pack chunk raw lengths and compare the decrypted tree to `tree.json`.
