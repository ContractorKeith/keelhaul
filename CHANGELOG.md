# Changelog

## 0.1.0 — 2026-11-05

First release. Repository format version 1 (`docs/SPEC.md`) is frozen from this release on:
later versions keep reading v1 repositories.

### Features
- Encrypted repositories: XChaCha20-Poly1305 for every object, a random master key wrapped by
  Argon2id password slots or 32-byte key files, key add/list/remove without re-encrypting data.
- Deduplication: keyed FastCDC chunking (512 KiB / 1 MiB / 8 MiB), keyed BLAKE3 chunk IDs, zstd
  per blob, immutable packs plus an index; dedup across every snapshot.
- Backends: local disk, SFTP through the system `ssh` (honours `~/.ssh/config`), and
  S3-compatible storage.
- `backup` with parent-snapshot change detection, gitignore-style excludes,
  `--one-file-system`, a bounded parallel pipeline (`--jobs`), and resume-by-rerun after a crash.
- `restore` of a whole snapshot or `--include` paths with POSIX metadata, symlinks and
  hardlinks; never writes outside `--target` or over existing files; `--skip-identical` to
  resume an interrupted restore.
- `check` (structure) and `verify` (every byte, `--subset N/M`) with an exact damage list and
  exit 3; `backup --heal` re-stores damaged data; `rebuild-index` rebuilds the index from pack
  headers.
- `forget` with keep-last/hourly/daily/weekly/monthly/yearly/tag policies and `prune` with
  repacking; shared/exclusive repository locks, `unlock`, and `--no-lock` for read-only
  repositories.
- `--json` on every command; exit codes 0 ok, 1 error, 2 usage, 3 damage, 4 locked,
  5 snapshot written but incomplete.
- Release archives for Linux (x86_64, arm64, static musl) and macOS (x86_64, arm64) with
  SHA256SUMS; Homebrew tap and crates.io.

### Verification before release
- Seeded damage harness (200 seeds) and SIGKILL crash loops for backup and prune (500
  iterations each), with a mutation gate proving the harness fails when a check is removed.
- Three skeptic-review rounds on Linux and macOS (issues #23–#31, #35–#43, #46–#49, all fixed),
  including independent readers written from `docs/SPEC.md` alone that restored files byte
  for byte.
