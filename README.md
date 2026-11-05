# keelhaul

Encrypted, deduplicating backups for Linux and macOS. Back up directories to local
disk, SFTP or S3-compatible storage; restore whole snapshots or selected files.
Keyed FastCDC splits changed data into chunks, zstd compresses them, and
XChaCha20-Poly1305 encrypts them. Password slots use Argon2id; key files are also supported.

**0.1.0 is in preparation.** Release archives appear on each tag; Homebrew and crates.io
installs follow shortly after (see `docs/RELEASING.md`). The repository format is draft until
the tag.
MIT licensed.

## Install

Homebrew (Linux or macOS):

```sh
brew tap ContractorKeith/tap
brew install ContractorKeith/tap/keelhaul
```

From crates.io (Rust 1.98 or newer):

```sh
cargo install keelhaul --locked
```

Release archive (Linux x86_64 example; alternatives: `linux_arm64`,
`darwin_amd64`, `darwin_arm64`):

```sh
version=0.1.0
archive="keelhaul_v${version}_linux_amd64.tar.gz"
base="https://github.com/ContractorKeith/keelhaul/releases/download/v${version}"
curl -fLO "$base/$archive"
curl -fLO "$base/SHA256SUMS"
grep "  $archive\$" SHA256SUMS | sha256sum -c -
tar -xzf "$archive"
install -m 755 keelhaul ~/.local/bin/keelhaul
```

On macOS use `shasum -a 256 -c` in place of `sha256sum -c`. Ensure the installation
directory exists and is on `PATH`. Downloads include the binary, LICENSE and README.

## Quick start

Local disk (interactive commands prompt for a password):

```sh
export KEELHAUL_REPO=/mnt/backup/keelhaul
keelhaul init
keelhaul backup ~/Documents ~/Pictures --exclude '*.tmp' --tag nightly
keelhaul snapshots
keelhaul ls latest
keelhaul restore latest --target /mnt/restore
keelhaul verify
```

Restore recreates the saved absolute path beneath the target directory. Test a
restore before relying on a backup. Store your password or key file separately;
losing all unlocking secrets makes the backup unrecoverable.

The hostname groups snapshots for `latest`, parent selection and retention, and
identifies lock ownership. Pass `--host` to backup (and to `ls`/`restore` when
selecting `latest`) on machines whose hostname changes, such as macOS with DHCP,
or on cloned VMs. Give cloned machines distinct stable hostnames for lock ownership.

For SFTP, configure a system SSH alias in `~/.ssh/config`:

```sshconfig
Host backup-nas
    HostName nas.example.net
    User backup
    IdentityFile ~/.ssh/backup_ed25519
```

```sh
export KEELHAUL_REPO=sftp:backup-nas:/srv/backups/laptop
keelhaul init
keelhaul backup ~/Documents
```

The system `ssh` supplies authentication, host-key checking and alias resolution.
Connect with `ssh backup-nas` first to verify your SSH setup.

For S3-compatible storage, provide credentials through your environment (for
example, loaded by your credential manager):

```sh
export AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY
export AWS_REGION=us-east-1
# Export AWS_SESSION_TOKEN too when using temporary credentials.
export KEELHAUL_REPO=s3:https://s3.example.net/my-backups/laptop
keelhaul init
keelhaul backup ~/Documents
```

The bucket must exist. URLs are `s3:https://endpoint/bucket[/prefix]`; endpoint
path prefixes are unsupported. Servers without conditional PUT cannot guarantee
create-once semantics during concurrent initialization; initialize a repository once.

## Secrets and global options

Use one secret source: `--password-file PATH` / `KEELHAUL_PASSWORD_FILE`,
`--password-command COMMAND` / `KEELHAUL_PASSWORD_COMMAND`, or `--key-file PATH` /
`KEELHAUL_KEY_FILE`. Otherwise keelhaul prompts on a TTY. A password file’s single
trailing `\n` or `\r\n` is stripped. Pass a file path or a
credential-manager command, never the password itself in argv. Protect secret
files with mode 600. Scheduled runs require a noninteractive secret source.

`-r/--repo` overrides `KEELHAUL_REPO`; flags override environment settings.
`--json` emits one JSON document; `-q/--quiet`, `-v/--verbose` and `--jobs N`
(default 4) control output and parallelism. See `keelhaul <command> --help` for flags.

## Commands

| Command | Purpose |
|---|---|
| `init` | Create an encrypted repository. |
| `key list` | List unlocking slots. |
| `key add` | Add a password or `--new-key-file PATH` without rewriting backups. |
| `key remove <slot-id>` | Remove an unlocking slot, retaining at least one. |
| `backup <path>...` | Save a snapshot; unchanged chunks are reused. |
| `snapshots` | List snapshots, optionally filtered by host, tag or path. |
| `ls <snap> [path]` | List a snapshot's saved files. |
| `restore <snap> --target DIR` | Restore files, optionally selected with `--include PATH`. |
| `check` | Check repository structure and metadata. |
| `verify [--subset N/M]` | Read and authenticate stored backup data. |
| `forget` | Select snapshots to remove using explicit IDs or retention rules. |
| `prune` | Reclaim unreferenced data and repack partly used packs. |
| `unlock` | Remove stale locks. |
| `version` | Print the version. |

Snapshot selectors accept a unique ID prefix or `latest`. Incremental backups
use parent metadata; `--force-rehash` forces file contents to be read again.
Filters include `--exclude`, `--exclude-file` and `--one-file-system`.

## Retention

`forget` removes snapshot records; `prune` reclaims their unreferenced data.
Policies apply separately to each host and set of source paths by default.
Combine `--keep-last`, `--keep-hourly`, `--keep-daily`, `--keep-weekly`,
`--keep-monthly`, `--keep-yearly` and `--keep-tag`: a snapshot matching any rule
is kept. Calendar rules keep the newest snapshot in each of the last N occupied
buckets, using local time (UTC with a warning if unavailable).

Preview a policy, then apply it and reclaim space in one operation:

```sh
keelhaul forget --keep-daily 7 --keep-weekly 4 --keep-monthly 12 --dry-run
keelhaul forget --keep-daily 7 --keep-weekly 4 --keep-monthly 12 --prune
```

You can instead forget explicit snapshot IDs, without policy flags, and prune
later. `prune --max-unused PCT` repacks packs whose unused blob bytes exceed the
percentage (default 20); entirely unused packs are reclaimed subject to the
in-flight backup grace period. Preview pruning before changing storage:

```sh
keelhaul prune --max-unused 20 --dry-run
keelhaul prune --max-unused 20
```

Conflicting locks or loss of an operation's lock cause exit 4. Run
`keelhaul unlock` only when the process holding the lock is gone; a stale lock
may belong to a sleeping laptop. Check the owning machine first. `unlock`
removes locks that are stale or belong to a dead process on this host; age alone
does not establish that it is safe to interrupt another machine's backup.

## How to verify a backup

If `verify` reports data damage, run `keelhaul backup --heal <same paths>` then
`prune`. Healing re-reads the source files, verifies stored chunks before
deduplicating, and stores fresh copies of damaged chunks; prune can then discard
the damaged copies.

```sh
keelhaul check
keelhaul verify
keelhaul verify --subset 1/7
```

`check` audits structure and metadata; it does not authenticate every data byte.
`verify` reads stored data and checks authentication, chunk IDs and references.
`--subset N/M` deterministically selects one partition of packs: run partitions
`1/7` through `7/7` to cover all packs across seven runs. A subset alone is not a
full verification. Also periodically restore to a scratch directory and compare
against known source files. Damage reports name the affected objects.

| Exit | Meaning |
|---|---|
| 0 | Success |
| 1 | I/O, authentication or refused-operation error |
| 2 | Invalid usage |
| 3 | Repository damage found |
| 4 | Repository locked |

## Scheduling

A daily cron job (adjust paths; this example uses a password file):

```cron
0 2 * * * KEELHAUL_REPO=/mnt/backup/keelhaul KEELHAUL_PASSWORD_FILE=/home/alice/.secrets/keelhaul /home/alice/.local/bin/keelhaul backup /home/alice/Documents >>/home/alice/keelhaul.log 2>&1
```

For a systemd user timer, save `~/.config/systemd/user/keelhaul.service`:

```ini
[Unit]
Description=Back up documents with keelhaul

[Service]
Type=oneshot
Environment=KEELHAUL_REPO=/mnt/backup/keelhaul
Environment=KEELHAUL_PASSWORD_FILE=%h/.secrets/keelhaul
ExecStart=%h/.local/bin/keelhaul backup %h/Documents
```

Save `~/.config/systemd/user/keelhaul.timer`:

```ini
[Unit]
Description=Daily keelhaul backup

[Timer]
OnCalendar=*-*-* 02:00:00
Persistent=true

[Install]
WantedBy=timers.target
```

```sh
systemctl --user daemon-reload
systemctl --user enable --now keelhaul.timer
journalctl --user -u keelhaul.service
```

User timers require an active user manager; enable lingering if backups must run
while logged out. Schedule full verification separately and inspect failures.

## Format and development

[Repository format](docs/SPEC.md) · [Build plan and release checklist](docs/PLAN.md)

Run `scripts/accept.sh` for formatting, linting, tests and a live backup/restore
round trip. Docker e2e and longer crash loops are described in the plan.
