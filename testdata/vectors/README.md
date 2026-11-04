# Repository format vectors

The test-only loader in `src/vectors.rs` reads JSON by filename and reports missing
or malformed files with their path. `hex-roundtrip.json` exercises the loader in M0;
it is a harness fixture, not a repository-format vector.

Planned SPEC §12 files (M1/M2):

- `subkeys.json`: fixed master key and all derived subkeys.
- `gear.json`: first eight gear entries and hash of all 256 entries.
- `cut-points.json`: seed, length (32 MiB), and chunk lengths.
- `chunk-ids.json`: plaintext and keyed chunk IDs.
- `envelopes.json`: fixed nonce and input for each envelope label.
- `password-slot.json`: fixed salt and m_kib 8192, t 1, p 1.
- `keyfile-slot.json`: fixed key file and wrapped master key.
- `pack.json`: two-blob pack.
- `index.json`, `tree.json`, `snapshot.json`: payloads.

These filenames reserve the intended fixture inventory; future milestones supply
the actual vectors for independent reader implementations.
