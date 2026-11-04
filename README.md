# keelhaul

A deduplicating, encrypted backup tool. keelhaul backs up directories to local disk, SFTP
or S3-compatible storage, and never stores the same data twice. Every snapshot is
encrypted and can be verified byte for byte.

**Status:** planned, no code yet. See [`docs/PLAN.md`](docs/PLAN.md) for the build plan
and [`docs/SPEC.md`](docs/SPEC.md) for the repository format.

```sh
keelhaul -r /mnt/nas/backup init
keelhaul -r /mnt/nas/backup backup ~/docs --exclude '*.tmp'
keelhaul -r /mnt/nas/backup snapshots
keelhaul -r /mnt/nas/backup restore latest --target /tmp/restore
keelhaul -r /mnt/nas/backup verify
keelhaul -r /mnt/nas/backup forget --keep-daily 7 --keep-weekly 4 --keep-monthly 12 --prune
```

- Content-defined chunking (keyed FastCDC) with BLAKE3 IDs: only changed data is uploaded.
- XChaCha20-Poly1305 encryption, unlocked by a password (Argon2id) or a key file.
- `check` audits the repository structure. `verify` reads every byte back and lists any damage.
- A written, versioned repository format, so others can write a compatible reader.
- Linux and macOS. MIT licensed.
