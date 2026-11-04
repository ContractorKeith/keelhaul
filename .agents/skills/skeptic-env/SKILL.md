---
name: skeptic-env
description: "How a skeptical reviewer builds, runs, seeds and attacks keelhaul. Read by the skeptic-review and pr-stress skills; not a workflow on its own."
---

# skeptic-env — keelhaul

A deduplicating, encrypted backup CLI: keyed FastCDC chunks, zstd, XChaCha20-Poly1305,
packs plus an index, snapshots of trees, and local, SFTP and S3 backends. The one rule that
matters: data loss and silent wrong bytes are never acceptable. "Done" for v1 is
`docs/PLAN.md` §10.

## Stack and commands

```bash
export TMPDIR=$PWD/.tmp; mkdir -p .tmp          # Omarchy /tmp is a small tmpfs
scripts/accept.sh                               # the full gate, exactly as CI
cargo test --locked --test damage               # damage harness; KEELHAUL_SEED=<n> reproduces
KEELHAUL_SOAK=1 cargo test --locked --release --test crash
echo "docker compose up -d && KEELHAUL_E2E=1 cargo test --locked --test e2e" | newgrp docker
cargo build --release && ./target/release/keelhaul version
```

Scratch repo: `export KEELHAUL_REPO=$TMPDIR/repo KEELHAUL_PASSWORD_FILE=$TMPDIR/pw;
echo hunter2 > $TMPDIR/pw; keelhaul init`. Never point a repo or a restore target at real
data; restore into `$TMPDIR/out` and compare with `diff -r` plus a `stat` walk for
metadata.

## Platforms

| Platform | Ships | Review on | Notes |
|---|---|---|---|
| Linux | yes | Omarchy | Docker e2e (MinIO, openssh-server) runs here and in CI |
| macOS | yes | Mac Studio | no Docker e2e; APFS is case- and normalisation-insensitive; `/var` → `/private/var` symlink |
| Windows | no | | |

## What to attack

- **Damage:** flip bytes in packs, the index, snapshots, key slots and config, then
  truncate, delete, duplicate, swap two objects or append junk. `verify` must list
  exactly what you broke (exit 3). `restore` must never write a wrong byte to a target
  path and must still restore undamaged files. A tree with `..`, `/` or NUL in a name must
  never create a path outside the target.
- **Crashes:** SIGKILL `backup` and `prune` at random points. Then `check` must pass, the
  rerun must complete, and the restore must match. Also try a full disk, a read-only repo,
  and a backend that errors mid-write.
- **Locks:** two backups at once, prune during backup, a backup suspended (SIGSTOP) for
  longer than the 30-minute stale window while prune runs (time can be faked through the
  test clock seam), and `unlock` racing a live lock.
- **Input files:** files changing while they're read, files that shrink, sparse files,
  hardlinks across backed-up paths, symlink loops, non-UTF-8 names, names that collide
  under APFS, 0-byte and exactly-MIN/MAX-sized files, unreadable files (permission denied
  must warn, not abort, and the snapshot must record it).
- **Secrets:** a password must never appear in argv, logs, `--json` output or panic
  messages. Key slots with absurd Argon2 parameters must be refused before the KDF runs.
- **Spec drift:** write a reader from `docs/SPEC.md` alone (a short Python script) and
  check that it decrypts the test vectors and lists a real repository's snapshots.
