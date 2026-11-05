# Changelog

## Unreleased fixes

- Bound local directory lock waits and write/fsync temporary objects before locking.

## 0.1.0 — unreleased

- M0: local and in-memory storage backends, conformance harness, JSON output and
  stable exit codes; Linux and macOS acceptance gates.
- M1: encrypted repositories, Argon2id password slots, key files and key rotation.
- M2: keyed FastCDC deduplication, zstd compression, encrypted immutable packs,
  snapshots, full and selected-file restores, POSIX metadata and format vectors.
- M3: structural checks, full and subset verification, damage reporting and
  corruption tests that exercise failed restores.
- M4: incremental backups, include/exclude filters, bounded parallel processing,
  index flushing and backup crash recovery tests.
- M5: shared/exclusive locks with refresh and self-checks, exit 4 on lock conflicts,
  and stale/dead-process cleanup through `unlock`.
- M5: grouped retention policies, explicit snapshot removal, `forget --dry-run`
  and combined `forget --prune`; configurable repacking with `prune --max-unused`.
- M5: replacement index payloads split at 16 MiB, with only the final index
  superseding old indexes; lock and snapshot checks before and during deletion,
  replacement-index withdrawal on pre-delete abort, and prune crash recovery tests.
- M6: S3-compatible storage with environment credentials and Docker end-to-end tests.
- M7: SFTP over system SSH, ssh_config aliases and Docker end-to-end tests.
- M8: four-platform release archives, SHA256SUMS, Homebrew formula preparation,
  crates.io package metadata and installation/scheduling documentation.

The repository format remains draft until the 0.1.0 release tag. Release readiness
is tracked in `docs/PLAN.md` §10.
