# keelhaul — Build Plan

Status: planned, no code yet (2026-11-04). Decisions come from the 2026-11-04 grill. Studied
restic (scope and spec model), borg, kopia, duplicacy (license excludes it as a model) and
rclone (backend shape). The repository format is `docs/SPEC.md`.

## 1. Problem and goal

Backups fail silently. A tool that "mostly works" loses data the one time it matters, and
most people never find out until a restore. keelhaul is a single Rust binary for Linux and
macOS that backs directories up to local disk, SFTP or S3-compatible storage. Content-defined
chunking means unchanged data is never stored twice. Everything is encrypted. A `verify`
command proves every snapshot is restorable by reading every byte back. The format is
documented well enough that someone else could write a compatible reader.

v1 is done when every **v1** row in §3 works on Linux and macOS, `scripts/accept.sh` passes
from a clean clone, and the two proofs in §8 hold. The damage harness must have every
injected corruption reported, with none missed and no false positives. The crash loop must
show that a SIGKILL at any point leaves a repository that `check` passes and a rerun
completes.

## 2. User stories

- As someone with a laptop and a NAS, I back up `~/` nightly from cron, and each run uploads
  only what changed.
- As someone with an S3 bucket I don't fully trust, I know the provider sees only encrypted
  packs and cannot fingerprint my files.
- As someone restoring after a disk failure, I restore one file or a whole snapshot and get
  exactly the bytes I backed up, or a loud error naming what is damaged.
- As someone keeping a year of history, `forget --keep-daily 7 --keep-weekly 4
  --keep-monthly 12 --prune` keeps the repo bounded.
- As someone who gets interrupted (laptop lid, Wi-Fi), I rerun the backup and it continues
  from what was already uploaded.
- As a developer, I can write my own reader from `docs/SPEC.md` and the test vectors.

## 3. Feature matrix

**v1** = in scope now. **Later** = after v1. **Drop** = deliberately never.

| # | Feature | Decision | Milestone | Notes |
|---|---|---|---|---|
| 1 | Backend trait, local backend, in-memory test backend | v1 | M0 | conformance suite every backend passes |
| 2 | Spec test-vector harness | v1 | M0 | vectors filled in as each format piece lands |
| 3 | Master key, password and key-file key slots, `init`, `key add/list/remove` | v1 | M1 | Argon2id; no data rewrite on key change |
| 4 | Keyed FastCDC chunker, blobs, packs, index, trees, snapshots | v1 | M2 | SPEC §6–§10 |
| 5 | `backup` (full), `snapshots`, `ls`, `restore` full and `--include` single file | v1 | M2 | |
| 6 | zstd per blob and per object | v1 | M2 | raw when it doesn't shrink |
| 7 | POSIX metadata: mode, mtime, uid/gid and names, symlinks, hardlinks, empty dirs | v1 | M2 | owner restored only as root |
| 8 | `check` (structure) and `verify` (every byte, `--subset n/m`) | v1 | M3 | exit 3 + exact damage list |
| 9 | Damage harness (seeded byte flips, truncation, deletion, junk) | v1 | M3 | the core proof |
| 10 | Incremental via parent snapshot (size, mtime, ctime, inode), `--force-rehash` | v1 | M4 | |
| 11 | Include/exclude (gitignore-style globs), `--exclude-file`, `--one-file-system` | v1 | M4 | |
| 12 | Parallel chunk/compress/encrypt/upload pipeline, `--jobs` | v1 | M4 | bounded channels |
| 13 | Index flush during backup; resume = rerun | v1 | M4 | SPEC §8, §13 |
| 14 | Fault-injecting backend and SIGKILL loop for backup | v1 | M4 | |
| 15 | Locks (shared/exclusive, refresh, stale, self-check), `unlock` | v1 | M5 | SPEC §11 |
| 16 | `forget` with retention policy, `--dry-run`, `--prune` | v1 | M5 | grouped by host + paths |
| 17 | `prune` with repack, SIGKILL loop for prune | v1 | M5 | SPEC §14 |
| 18 | S3-compatible backend (rusty-s3 + ureq), versitygw e2e | v1 | M6 | AWS_* env credentials; create-once is exact only on servers with conditional PUT; otherwise concurrent `init` of one repo is unsafe |
| 19 | SFTP backend over system `ssh -s sftp`, own SFTP v3 client, sshd e2e | v1 | M7 | honours `~/.ssh/config` |
| 20 | `--json` on every command, exit codes 0/1/2/3/4 | v1 | M0–M5 | each milestone for its commands |
| 21 | Release binaries (linux x64/arm64, macOS arm64), Homebrew formula, crates.io | v1 | M8 | |
| 22 | TUI (ratatui) | Later | | explicitly cut from v1 |
| 23 | xattrs, ACLs, macOS resource forks | Later | | tree reserves `xattrs` |
| 24 | `dump`, `diff`, `stats`, `copy`, `repair`, `mount` | Later | | `repair` after real damage reports |
| 25 | Config file / named profiles | Later | | flags + env only in v1 |
| 26 | More backends (B2, Azure, GCS, rclone) | Later | | S3-compatible covers most |
| 31 | Sparse-file holes (SEEK_HOLE on backup, sparse restore) | Later | | |
| 27 | Windows | Drop | | VSS, ACLs, path rules |
| 28 | Unencrypted repositories | Drop | | ADR 0001 |
| 29 | Cross-repository dedup | Drop | | keyed IDs, ADR 0001 |
| 30 | Lock-free concurrent prune | Drop | | exclusive lock is the model |

## 4. Architecture

```
src/
  main.rs            clap CLI: parse, open repo, call lib, render text or --json, map exit codes
  lib.rs             re-exports; no logic
  backend/           Backend trait, local, memory, fault (test-only wrapper), s3, sftp
  crypto/            subkeys, envelope, key slots (Argon2id, key file)
  format/            config, pack, index, tree, snapshot JSON types; parse = validate
  chunker.rs         keyed FastCDC (SPEC §6), streaming over Read
  repo/              Repository: open, blob store (pack writer, index cache), locks
  ops/               backup, restore, ls, check, verify, forget, prune, key
  output.rs          text and JSON renderers, Damage list type
tests/               integration tests against the in-memory and local backends
testdata/vectors/    SPEC §12 vectors (JSON)
scripts/accept.sh    the acceptance (§8)
compose.yaml         versitygw and openssh-server for e2e
```

Rules that hold the shape:
- Only `backend/` does I/O against the repository, and it knows names and bytes, not the
  format. Everything above it is tested on the in-memory backend.
- `format/` parse functions validate everything SPEC requires, so a value of a format type
  is always well formed. Callers never re-check.
- Every failure to read, decrypt, hash-match or parse a repository object becomes a
  `Damage` value naming the object. Damage never panics and is never swallowed as "empty".
  Wrong password and I/O errors are errors, not damage.
- Restore never writes unverified bytes to their final path. Each file is assembled in a
  temp name beside its target, every chunk ID is verified, then the temp file is renamed.
- No async runtime. Concurrency is `std::thread::scope`, bounded `std::sync::mpsc` channels
  and a fixed pool size (ADR 0002).
- `#![forbid(unsafe_code)]` crate-wide. Platform calls go through safe crates.
- Process: `rust-best-practices` while building, `rust-review` before landing (dotfiles
  skills).

### Dependencies and why

| Crate | Why |
|---|---|
| clap (derive) | the CLI contract, help, completions |
| serde, serde_json | SPEC is JSON |
| blake3 | chunk IDs, object IDs, subkeys, gear |
| chacha20poly1305 | XChaCha20-Poly1305 (RustCrypto) |
| argon2 | password key slots |
| zstd | compression; builds bundled libzstd with `cc`, statically linked |
| getrandom | CSPRNG for keys, salts, nonces |
| base64, hex | SPEC encodings |
| time | RFC 3339 with nanoseconds |
| thiserror (lib), anyhow (main only) | typed errors where callers branch, context at the edge |
| globset | gitignore-style excludes |
| rpassword | hidden TTY password prompt |
| uzers | uid/gid to names without unsafe |
| rusty-s3, ureq (M6) | sans-IO S3 signing over a sync HTTP client |
| dev: proptest, tempfile | round trips and scratch dirs |

Anything beyond this list needs a line in the PR saying why.

## 5. Data model

`docs/SPEC.md` is the data model: repository layout (§1), key slots (§3), envelope (§4),
config (§5), chunking and IDs (§6), pack (§7), index (§8), tree (§9), snapshot (§10),
lock (§11), test vectors (§12), write ordering (§13), prune (§14).

## 6. CLI contract

```
keelhaul init
keelhaul key list
keelhaul key add [--new-key-file PATH]          new password slot (prompt) or new key file
keelhaul key remove <slot-id>
keelhaul backup <path>... [--exclude GLOB]... [--exclude-file F] [--one-file-system]
                [--tag T]... [--host H] [--parent SNAP] [--force-rehash]
keelhaul snapshots [--host H] [--tag T] [--path P]
keelhaul ls <snap> [<path>]
keelhaul restore <snap> --target DIR [--include PATH]...
keelhaul check
keelhaul verify [--subset N/M]
keelhaul forget [<snap>...] [--keep-last N] [--keep-hourly N] [--keep-daily N]
                [--keep-weekly N] [--keep-monthly N] [--keep-yearly N] [--keep-tag T]...
                [--group-by host,paths] [--dry-run] [--prune]
keelhaul prune [--max-unused PCT] [--dry-run]
keelhaul unlock
keelhaul version
```

`<snap>` is a unique ID prefix or `latest` (newest for this host and paths).

Global flags: `-r/--repo` (or `KEELHAUL_REPO`): a path, `sftp:[user@]host:/path`, or
`s3:https://endpoint/bucket[/prefix]`. For S3, the endpoint is scheme plus authority:
the first path segment is always the bucket (3–63 lowercase letters, digits, dots or
hyphens), followed by the optional repository prefix. Endpoint path prefixes are not
supported; `s3:https://host/s3/bucket/p` selects bucket `s3` and is rejected because its
name is too short, rather than routing through an endpoint path `/s3`. Secrets come from one of `--password-file` /
`KEELHAUL_PASSWORD_FILE`, `--password-command` / `KEELHAUL_PASSWORD_COMMAND`, `--key-file` /
`KEELHAUL_KEY_FILE`, or a TTY prompt. Never from argv. S3 credentials come from
`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN`, `AWS_REGION`. Other flags:
`--json`, `-q/--quiet`, `-v/--verbose`, `--jobs N` (default 4). Precedence: flag > env >
prompt.

Output: human text on stdout, progress on stderr only when stderr is a TTY. `--json` prints
one JSON document on stdout and nothing else there; its shape per command is pinned by
golden tests. Damage list (check, verify, restore):

```json
{ "damage": [ { "object": "data/ab/ab12…", "kind": "pack", "problem": "hash-mismatch", "detail": "…" } ],
  "notes":  [ { "object": "data/cd/cd34…", "kind": "pack", "problem": "orphan" } ] }
```

`problem` is one of `missing`, `truncated`, `hash-mismatch`, `undecryptable`, `bad-format`,
`bad-header`, `index-mismatch`, `dangling-reference`, `size-mismatch`, plus notes `orphan`,
`unknown-object`, `unused-index`.

Exit codes: 0 ok · 1 error (I/O, wrong password, refused operation) · 2 usage · 3 damage
found (check, verify, restore, prune, snapshots) · 4 repository locked · 5 snapshot
written; entries skipped (incomplete backup).

## 7. Milestones

One issue, one branch, one draft PR, one worker thread in its own worktree each. Workers
run in parallel as soon as their dependencies merge.

| M | Deliverable | Depends on | Done when (what the e2e asserts) |
|---|---|---|---|
| M0 | Cargo package, lint policy, CI (Linux + macOS), `scripts/accept.sh`, Backend trait + local + memory backends, backend conformance suite, test-vector loader, `compose.yaml` (versitygw, openssh-server), `version` | – | accept passes; conformance suite green on local and memory; atomic write leaves no partial object visible |
| M1 | Subkeys, envelope, password and key-file slots, config, `init`, `key add/list/remove`, vectors §3–§5 | M0 | init → key add → remove original → open with new secret; wrong secret exits 1; refusing the last slot; a tampered slot or config is refused |
| M2 | Gear and chunker, blobs, pack writer and reader, index, tree, snapshot, `backup`, `snapshots`, `ls`, `restore`, vectors §6–§10 | M1 | back up a seeded fixture tree (binary, empty, sparse-ish, unicode and non-UTF-8 names, symlinks, hardlinks, empty dirs) → restore → byte-for-byte and metadata equal; second identical backup adds zero data blobs; single-file `--include` |
| M3 | `check`, `verify --subset`, damage harness, restore refuses damaged files | M2 | harness: seeded N corruptions of every object kind; verify lists exactly the damaged objects (exit 3), check lists exactly the damaged metadata objects; restore writes no wrong bytes and restores every undamaged file; clean repo exits 0 |
| M4 | Parent-snapshot change detection, excludes, `--one-file-system`, parallel pipeline, index flush, fault backend, backup SIGKILL loop | M2 | incremental reads only changed files (counter); excludes honoured; kill loop: after each SIGKILL `check` passes (orphans allowed), rerun completes and reuses flushed blobs, restore matches |
| M5 | Locks, `unlock`, `forget` + retention policy, `prune` + repack, prune SIGKILL loop | M3, M4 | policy golden over a synthetic year of snapshots; prune frees the forgotten data and `verify` stays clean; shared vs exclusive conflict exits 4; stale-lock self-check aborts a snapshot; prune kill loop leaves `check` clean |
| M6 | S3 backend, versitygw e2e in CI and locally | M0 | conformance suite and the M2 round trip pass against versitygw |
| M7 | SFTP backend: spawn `ssh -s sftp`, SFTP v3 client, sshd e2e | M0 | conformance suite and the M2 round trip pass against the openssh-server container through an `ssh_config` Host alias |
| M8 | Release workflow, Homebrew formula, crates.io metadata, README usage, SPEC frozen as v1 | all | release dry-run builds three targets with checksums; `brew install` from the tap on both boxes; `cargo publish --dry-run` clean |

Merge order: M0 alone, then M1, M6 and M7 in parallel (each merges when ready), M2, then
M3 and M4 in parallel (merge M3 first), then M5, then M8. M6/M7 PRs that land before M2
skip the round-trip assertion and add it in a follow-up comment on M2's issue.

## 8. Testing strategy and acceptance

- **Unit and property tests** run on the in-memory backend. proptest round-trips the
  chunker (cut points are a pure function of bytes and gear key), envelope, pack framing,
  and every JSON type. Each also checks against the SPEC §12 vectors.
- **Conformance suite** (`tests/backend_conformance.rs`): one generic test set every
  backend runs. It covers write, read, ranged read, list, delete, atomic visibility,
  missing-object errors and name validation.
- **Damage harness** (`tests/damage.rs`, M3): build a repo, then for each seed pick N
  objects across every kind and apply one corruption each (flip 1..8 random bytes,
  truncate, delete, append junk, swap two objects' contents). Assert that `verify`'s damage
  list equals the expected set exactly, and that `check`'s list equals the metadata subset.
  Seeds are logged, so a failure reproduces with `KEELHAUL_SEED=<n>`.
- **Crash loops** (M4, M5): run the real binary on a local repo and SIGKILL it after a
  random delay drawn from a seeded RNG, 50 iterations in CI and 500 with
  `KEELHAUL_SOAK=1`. After each kill, `check` must exit 0 (notes allowed), and the next
  run must complete and restore byte for byte.
- **Docker e2e** (M6, M7): `compose.yaml` starts versitygw and openssh-server. These tests
  run when `KEELHAUL_E2E=1` and fail, never skip, if Docker is unreachable. Linux CI runs
  them. macOS CI runs everything else.
- `scripts/accept.sh` runs fmt, clippy with `-D warnings`, tests, a release build, then a
  live CLI script on a scratch local repo (init → backup → snapshots → restore → diff →
  corrupt one pack → verify exits 3 → check). Worker, reviewer and CI run the same script.

## 9. Process

- One milestone per worker thread (Codex gpt-6.1-sol, medium), briefed with
  `worker-brief`. Draft PR; the orchestrator reads the full diff and reruns
  `scripts/accept.sh`. Then a cold `reviewer` subagent, then `pr-stress` by a fresh thread,
  with fix rounds in the author's thread. One PR comment per round, then `gh pr ready` and
  merge.
- `skeptic-review` on main before the v0.1.0 tag, on Omarchy (Linux) and the Mac Studio
  (macOS); reports go in `worklog/reports/`.
- Tracking: GitHub issues labelled `milestone` are the truth; the Basecamp `keelhaul` card
  on the `github` board has one step per milestone.

## 10. Acceptance checklist (v0.1.0)

- [ ] Every v1 row in §3 merged with its tests.
- [ ] `scripts/accept.sh` green on Linux and macOS from a clean clone; CI green.
- [ ] Damage harness: 200 seeds, zero misses, zero false positives.
- [ ] Crash loops: 500 iterations each for backup and prune, zero failures.
- [ ] Round trip through all three backends, byte for byte.
- [ ] A 10 GiB mixed fixture: second backup with no changes finishes in under 10% of the
      first run's time and adds zero data blobs.
- [ ] SPEC test vectors complete; SPEC matches the code (ship gate).
- [ ] skeptic-review on Linux and macOS with every confirmed bug fixed.

## 11. Open questions

- Large-file resume: a 50 GiB file killed at 90% re-reads the file but re-uploads only
  unflushed packs. If that proves too slow in M4's measurements, flush the index more
  often rather than adding checkpoint state.
- `--jobs` default per backend (local may want fewer, S3 more). Measure in M4 and M6.
