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
