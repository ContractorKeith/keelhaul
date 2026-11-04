# Damage harness mutation gate

Run `python3 tests/run_damage_mutations.py`. Each mutant runs
`KEELHAUL_SEED=0 cargo test --locked --release --test damage -- --nocapture`.
Failures must be harness assertion failures, not compiler errors. The runner restores
each source after its run, verifies the original source hashes, then requires a green
unmodified baseline. Full output is saved under `.tmp/damage-mutations/`.

| Mutation | Observed result | Wall time |
|---|---|---|
| drop-keyed-blob-id | KILLED (exit 101) | 9.45s |
| ignore-index-header-mismatch | KILLED (exit 101) | 8.87s |
| tree-read-failure-as-empty | KILLED (exit 101) | 8.07s |
| drop-pack-hash | KILLED (exit 101) | 8.74s |
| drop-missing-pack | KILLED (exit 101) | 8.11s |

The mutations remove the keyed blob-ID comparison, suppress index/header disagreement,
turn a failed tree read into an empty tree, remove whole-pack hash validation, and
suppress the missing-pack report. Exact `(object, kind, problem)` sets reject all five.
The re-sealed descriptor permutation makes index disagreement observable; the
equal-length authenticated envelope swap makes keyed-ID validation independently
observable. Pack hashes and missing packs have their own expected signatures.

Original sources restored: yes. Unmodified baseline: PASS.

## Duplicate-copy addendum

The seeded harness also creates a freshly sealed, indexed duplicate of a referenced
data blob. With the original location corrupted, the exact expected set retains its
pack/hash-mismatch and blob/undecryptable entries and excludes every referring file.
Once both locations are corrupted, all referring snapshot/path file entries appear.

`tests/verify_duplicate_cli.rs` forces the original location to be preferred. Verify
exits 3 with exactly the damaged pack/blob entries; restore succeeds with correct bytes
and does not rewrite the repository. Corrupting the second copy produces file damage.
The verify-only CLI regression also passed with the single-copy reader from c5d7a7c
temporarily substituted, proving verify does not depend on the general reader fallback.
The original reader was restored byte-for-byte, then both CLI regressions passed.
