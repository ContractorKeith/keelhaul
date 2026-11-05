# keelhaul

A deduplicating, encrypted backup CLI in Rust for Linux and macOS. It backs up directories
to local disk, SFTP (through the system `ssh`) or S3-compatible storage. Data is cut by
keyed FastCDC into chunks, compressed with zstd, and sealed with XChaCha20-Poly1305 into
immutable packs. Snapshots, forget/prune with retention policies, and full or single-file
restore sit on top. `verify` proves every snapshot is restorable by reading every byte
back. The repository format is a written spec (`docs/SPEC.md`) a third party can implement
a reader from. MIT. Modelled on restic's scope and design doc. "Done" for v1 is every row
marked v1 in `docs/PLAN.md` §3, plus the damage harness and crash loops of §8 and the
checklist in §10.

## Status

0.1.0 released 2026-11-05: milestones M0–M8 (#1–#9) merged; three skeptic-review rounds on Linux and macOS fixed (#23–#49). Format v1 frozen. Next: post-v1 Later rows in `docs/PLAN.md` §3.

## Commands

```bash
scripts/accept.sh                       # the acceptance: worker, reviewer and CI run exactly this
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked                     # unit, property, conformance, damage harness (in-memory + local)
docker compose up -d --wait s3 sshd
KEELHAUL_E2E=1 cargo test --locked --test e2e_s3 --test e2e_sftp   # fails if Docker is down
KEELHAUL_SOAK=1 cargo test --locked --release --test crash   # 500-iteration crash loops
cargo build --release                   # target/release/keelhaul
```

## Pointers

- `docs/SPEC.md` — the repository format, byte level. The contract; change it first, in its own PR.
- `docs/RELEASING.md` — tag, release workflow, Homebrew and crates.io steps.
- `docs/PLAN.md` — feature matrix, architecture rules, dependencies, CLI contract, milestones, testing, acceptance.
- `CONTEXT.md` — glossary. Use its words in code, issues and PRs.
- `docs/adr/` — keyed chunk IDs and gear; sync core and system ssh.
- `testdata/vectors/` — SPEC §12 test vectors.
- Session logs: `~/Projects/worklog/projects/keelhaul.md` (not in this repo).

## Conventions & Gotchas

- Rust work follows the `rust-best-practices` skill; review with `rust-review`.
- `#![forbid(unsafe_code)]`. No async runtime (ADR 0002). Dependencies beyond PLAN §4 need a reason in the PR.
- Only `src/backend/` touches storage. Everything above it is tested on the in-memory backend.
- A repository read that fails to decrypt, hash-match or parse is `Damage` naming the object. It never panics, is never read as empty, and maps to exit 3. A wrong password or an I/O error is an error (exit 1).
- Restore assembles each file under a temp name, checks every chunk ID, then renames. Wrong bytes never reach a target path.
- Object IDs (packs, index, snapshots, locks) are unkeyed BLAKE3 of the stored ciphertext. Chunk and tree IDs are keyed BLAKE3 of the plaintext. Don't mix them up.
- Writes are create-once, atomic (temp name + rename, fsync on local). Nothing is modified in place.
- Tests use `tempfile` scratch dirs and never the real `$HOME`. Seeds come from `KEELHAUL_SEED` and are printed on failure.
- Docker e2e must fail, not skip, when `KEELHAUL_E2E=1` and Docker is unreachable. On Omarchy, run socket commands under `newgrp docker` on stdin.
- `/tmp` on Omarchy is a small tmpfs. Point `TMPDIR` at `$PWD/.tmp` for big fixtures and crash loops.
- One milestone = one issue = one branch = one draft PR = one T3 worker thread in its own worktree. Merge order is in PLAN §7.
- Commits: `<type>: <description>`, author ContractorKeith only, no AI attribution lines.

## Tracking

- GitHub issues are the tickets: one per milestone, label `milestone`; `skeptic` for review findings.
- Basecamp project "github" has ONE card titled `keelhaul` with a step per milestone (`project-board` skill).
- Per PR: worker's done-when → orchestrator full-diff read and `scripts/accept.sh` rerun → cold `reviewer` → `pr-stress` by a fresh thread → fixes in the author's thread → one PR comment per round → `gh pr ready` → merge. `skeptic-review` is the release gate before a tag.

## Out of Scope

- Windows, unencrypted repositories, cross-repository dedup, lock-free concurrent prune. Never.
- TUI, xattrs/ACLs, mount, dump/diff/stats/copy/repair, config files, more backends. After v1.
