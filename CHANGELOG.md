# Changelog

## Unreleased fixes

- Collapse nested backup roots and share root validation across check, verify and restore;
  restore unaffected subtrees when a snapshot has overlapping roots.
- Read hardlink groups once; restore disagreeing legacy groups independently.
- Exit 5 for incomplete snapshots and add `incomplete` to backup JSON.
- Bound S3 attempts to 20 seconds and operations across retries to 120 seconds by default.
- Warn when a backup root is a symlink; follow it with an explicit trailing slash.
- Exercise runtime descriptor exhaustion with a feature-gated file-open fault hook.
- Recover damaged or missing index coverage from pack headers; add `rebuild-index`
  with dry-run support and retain damage reports while restoring intact files.
- Refuse removing the last slot whose check authenticates under this repository's key.
- Report damaged `latest` candidates and identify swapped packs during structural checks.
- Honor secret flag precedence, clarify restore errors and combined prune summaries,
  and use a final macOS drive-cache flush instead of per-file full flushes.
- Correct Docker e2e targets and service startup instructions.

- Stop queued backup uploads when the repository lock disappears or refresh fails.

- Add `backup --heal` to verify indexed chunks and replace damaged copies before prune.

- Abort backups on descriptor or memory exhaustion, with a jobs/ulimit hint.

- Preserve files that change during reads and report their count in backup JSON.

- Bound local directory lock waits and write/fsync temporary objects before locking.

## 0.1.0 — unreleased

- Skeptic round 1 B: continue restores after item errors, refuse filesystem name
  collisions, and apply file metadata before publication. Normalize include/list paths.
- Skip foreign same-secret key slots; handle broken stdout pipes quietly; improve
  errors, success summaries and help. Refuse empty retention policies, capture the
  local offset before threads start, and treat empty KEELHAUL environment values as unset.
- Clarify tree/root writer rules, gear vector hashing, hostname grouping and password files.
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
