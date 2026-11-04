# keelhaul repository format, version 1

Status: draft until release 0.1.0 tags; then frozen. A change to this file lands as its own
PR before the code that implements it. Words in **bold** are defined in `CONTEXT.md`.

This document is the contract. Someone with only this file, a JSON parser, BLAKE3,
XChaCha20-Poly1305, Argon2id and zstd must be able to write a reader that lists snapshots
and restores files byte for byte. Test vectors in `testdata/vectors/` (§12) pin every
algorithm; when this text and a vector disagree, that is a bug in one of them, fixed in
the same PR.

Conventions: integers in binary framing are unsigned little-endian. `hex` is lowercase
hexadecimal. `b64` is standard base64 with padding (RFC 4648 §4). Times are RFC 3339 UTC
with exactly nine fractional digits and a `Z` suffix, `2026-11-04T14:03:22.123456789Z`;
readers refuse any other form. JSON is UTF-8, RFC 8259. Readers MUST
ignore JSON object members they do not know; v1 writers MUST NOT write members not listed
here. Sizes: KiB = 1024 bytes, MiB = 1024 KiB.

## 1. Layout

A **repository** is a flat namespace of named objects stored by a **backend**. Names use
`/` as separator regardless of backend.

| Name | Content | Encrypted |
|---|---|---|
| `config` | repository config (§5) | yes |
| `keys/<id>` | one **key slot** (§3) | no (wraps the master key) |
| `data/<id[0..2]>/<id>` | one **pack** (§7) | yes |
| `index/<id>` | one **index** object (§8) | yes |
| `snapshots/<id>` | one **snapshot** (§10) | yes |
| `locks/<id>` | one **lock** (§11) | yes |

`<id>` is 64 hex characters. For `keys/` it is 32 random bytes. For every other directory
it is the unkeyed BLAKE3-256 hash of the object's exact stored bytes (the ciphertext), so
any reader can detect a damaged object by re-hashing it, without the password. `<id[0..2]>`
is the first two hex characters of the pack ID. Names that do not match these patterns
are not part of the repository; `check` reports them as unknown objects (notes), never as
damage, except partial uploads (§13), which it ignores. A file with a valid name whose
bytes do not hash to it is damage, including a zero-length one.

Objects are written once and never modified. Prune and `key remove` delete objects;
nothing rewrites one in place.

## 2. Primitives

- Hash: BLAKE3 (32-byte output unless stated).
- AEAD: XChaCha20-Poly1305 (24-byte nonce, 16-byte tag), from RFC 8439 plus the
  XChaCha extension (draft-irtf-cfrg-xchacha-03).
- KDF: Argon2id, version 0x13 (RFC 9106), 32-byte output.
- Compression: zstd frames (RFC 8878). Writers use level 3; readers accept any frame.
- Randomness: the operating system CSPRNG for every key, salt, nonce and slot ID.

## 3. Key slots and the master key

The **master key** is 32 random bytes made at `init`. Subkeys come from BLAKE3
`derive_key(context, master_key)`:

| Subkey | Context string |
|---|---|
| encryption key | `keelhaul 2026-11 v1 encryption` |
| chunk ID key | `keelhaul 2026-11 v1 chunk id` |
| gear key | `keelhaul 2026-11 v1 gear` |
| key-slot check key | `keelhaul 2026-11 v1 key slot check` |

A key slot is a plaintext JSON object at `keys/<random id>`:

```json
{
  "version": 1,
  "kind": "password",
  "created": "2026-11-04T14:03:22.123456789Z",
  "host": "omarchy",
  "username": "keith",
  "kdf": { "alg": "argon2id", "m_kib": 65536, "t": 3, "p": 4, "salt": "<b64 of 16 bytes>" },
  "wrapped": "<b64 of an envelope>",
  "check": "<hex of 32 bytes>"
}
```

`kind` is `password` or `keyfile`.

- Password slot: KEK = Argon2id(password UTF-8 bytes, salt, m_kib, t, p). Writers use
  m_kib 65536, t 3, p 4. Readers MUST refuse a slot outside m_kib 8192..=4194304, t
  1..=10, p 1..=16, or a salt shorter than 16 bytes, before running the KDF.
- Key-file slot: `kdf` is absent. A key file is exactly 32 random bytes, stored raw.
  KEK = BLAKE3 `derive_key("keelhaul 2026-11 v1 key file", file_bytes)`.

`wrapped` is an envelope (§4) under the KEK with associated data `keelhaul/key-slot` and
plaintext exactly the 32-byte master key (no compression byte). Opening a repository tries
the supplied secret against every slot. Exactly one secret is supplied per command. Each
slot is tried in name order until one decrypts. No slot decrypting is a wrong-password
error (exit 1), not damage.

`check` lets anyone holding the master key detect damage in slots they cannot unlock. It is
the BLAKE3 keyed hash (key = key-slot check subkey) of these bytes concatenated: the ASCII
`kind`, one `0x00` byte, then for a password slot `m_kib`, `t`, `p` as u32 LE and the decoded
salt, and finally the decoded `wrapped` bytes. A reader that opened the repository through
another slot MUST recompute `check` for every slot and report a mismatch as damage of that
slot. A reader unlocking through a slot verifies its `check` after unwrapping.

Removing the last slot is refused. A repository never stores the master key unwrapped.

## 4. Envelope

Every encrypted object or blob is an envelope:

```
nonce (24 bytes, random) || ciphertext || tag (16 bytes)
```

AEAD key: the encryption subkey (or KEK for slots). Associated data: an ASCII label naming
what the envelope is, so a valid envelope moved to the wrong place fails to decrypt:

| Label | Used for |
|---|---|
| `keelhaul/key-slot` | §3 |
| `keelhaul/config` | §5 |
| `keelhaul/blob/data` | data blob (§6) |
| `keelhaul/blob/tree` | tree blob (§6) |
| `keelhaul/pack-header` | §7 |
| `keelhaul/index` | §8 |
| `keelhaul/snapshot` | §10 |
| `keelhaul/lock` | §11 |

Except for key slots, the decrypted plaintext starts with a one-byte compression flag:
`0x00` raw, `0x01` one zstd frame. Any other flag value is damage. The **payload** is the
remaining bytes, decompressed when the flag is `0x01`. A writer compresses and uses
`0x01` only when the compressed bytes are shorter than the raw ones, so a `0x01` envelope
never holds an empty payload. A reader MUST reject a zstd frame that decompresses to more
than 64 MiB or to zero bytes, a frame whose window log exceeds 27, and any bytes after the
single frame.

## 5. Config

`config` is an envelope whose payload is:

```json
{
  "version": 1,
  "id": "<hex of 32 random bytes>",
  "created": "2026-11-04T14:03:22.123456789Z",
  "chunker": { "alg": "fastcdc-gear-keyed-v1", "min": 524288, "avg": 1048576, "max": 8388608 },
  "pack_target": 16777216
}
```

A reader MUST refuse any `version` other than 1, and any unknown `chunker.alg`, with an
error naming the version or algorithm found. `chunker` min/avg/max MUST equal the values
above for `fastcdc-gear-keyed-v1`. `pack_target` is writer policy; readers accept any value
from 1 MiB to 256 MiB inclusive and refuse others as damage. `id` names the repository in
logs and lock objects; it is not secret.

## 6. Chunking, chunk IDs, blobs

### 6.1 Gear table

The gear table is 256 u64 values. Take the BLAKE3 keyed hash (key = gear subkey) of the
empty input. Read 2048 bytes of its extendable output. Entry `i` is the u64 LE at bytes
`8i..8i+8`.

### 6.2 Cut points (FastCDC, normalized level 1)

For a file's contents `b` with `n` bytes remaining from offset `s`:

```
MIN = 524288, NORMAL = 1048576, MAX = 8388608
MASK_S = 0xFFFF_F800_0000_0000   (top 21 bits)
MASK_L = 0xFFFF_E000_0000_0000   (top 19 bits)

if n <= MIN: chunk = b[s .. s+n]
else:
  end = min(n, MAX); normal = min(end, NORMAL)
  h = 0
  for i in MIN .. end:
     h = (h << 1) wrapping_add gear[b[s+i]]          (u64 arithmetic)
     mask = MASK_S if i < normal else MASK_L
     if h & mask == 0: return chunk b[s .. s+i+1]
  return chunk b[s .. s+end]
```

The chunker restarts with `h = 0` at each cut. An empty file has zero chunks.

### 6.3 Chunk and blob IDs

The **chunk ID** of a data blob is the BLAKE3 keyed hash (key = chunk ID subkey) of the
chunk's bytes. The ID of a tree blob is the same keyed hash of the tree's exact JSON
payload bytes (§9). Readers never re-serialize JSON to check an ID; they hash the bytes
they decrypted.

A **blob** is an envelope with label `keelhaul/blob/data` or `keelhaul/blob/tree`, whose
payload is the chunk or tree bytes. Two blobs with the same ID and type are the same blob;
writers store each one once per repository.

## 7. Pack

```
blob_0 || blob_1 || ... || blob_{k-1} || header_envelope || header_len (u32 LE)
```

Blobs are concatenated envelopes with no padding. `header_len` is the byte length of
`header_envelope`, which has label `keelhaul/pack-header` and payload:

```json
{
  "version": 1,
  "blobs": [
    { "id": "<hex>", "type": "data", "offset": 0, "length": 524331, "raw_length": 524288 }
  ]
}
```

`offset` and `length` locate the blob's envelope inside the pack. `raw_length` is the
payload length after decompression. Entries appear in file order; together they cover
bytes `0 .. pack_len - 4 - header_len` exactly, with no gaps or overlaps. `header_len` must
be at most 16 MiB and at most `pack_len - 4`. Writers close a pack when it reaches
`pack_target` bytes of blobs, or when the backup ends. A pack holds at least one blob. A
pack never holds two blobs with the same ID and type.

## 8. Index

An index object is an envelope (label `keelhaul/index`) whose payload is:

```json
{
  "version": 1,
  "supersedes": ["<index id>"],
  "packs": [
    { "id": "<pack id>", "blobs": [ { "id": "<hex>", "type": "tree", "offset": 0, "length": 812, "raw_length": 2230 } ] }
  ]
}
```

The live index is the union of every index object, minus every object named in a
`supersedes` list. Entries for a pack must equal that pack's header. If one blob appears in
more than one pack, any copy may be read; `prune` removes duplicates. The index is a
cache of pack headers: it can always be rebuilt by reading every pack header, and
`check` treats a mismatch between the two as damage of the index object.

Backup writes an index object after every 8 packs or 5 minutes, whichever comes first, and
one at the end. Only these flushed indexes count for a rerun after a crash (§13).

## 9. Tree

A tree blob's payload describes one directory:

```json
{
  "version": 1,
  "entries": [
    { "name": "notes.txt", "type": "file", "mode": 420, "mtime": "...", "ctime": "...",
      "uid": 1000, "gid": 1000, "user": "keith", "group": "keith",
      "inode": 1234567, "device": 2049, "links": 1, "size": 13,
      "content": ["<chunk id>"] },
    { "name": "src", "type": "dir", "mode": 493, "mtime": "...", "ctime": "...",
      "uid": 1000, "gid": 1000, "user": "keith", "group": "keith",
      "inode": 1234568, "device": 2049, "links": 2, "subtree": "<tree id>" },
    { "name": "latest", "type": "symlink", "mode": 511, "mtime": "...", "ctime": "...",
      "uid": 1000, "gid": 1000, "user": "keith", "group": "keith",
      "inode": 1234569, "device": 2049, "links": 1, "target": "notes.txt" }
  ]
}
```

- Exactly one of `name` (the UTF-8 name) or `name_b64` (raw bytes, when the name is not
  valid UTF-8) is present. Likewise `target` or `target_b64` for symlinks. Names are a
  single non-empty path component: no `/`, no NUL byte, not `.` or `..`. A reader MUST
  treat a violating name as damage and never create it.
- Entries are sorted by raw name bytes, ascending, with no duplicates.
- Trees have no depth limit. Readers walk them iteratively and treat a tree that names one
  of its own ancestors as damage.
- `mode` holds only the permission bits (`0o7777` mask).
- `user` and `group` may be absent when the IDs have no names.
- `content` lists chunk IDs in file order. Their `raw_length`s sum to `size`.
- Hardlinks: on restore, regular files with `links > 1` and equal `(device, inode)`
  within one snapshot are restored as hardlinks to the first one restored.
- `xattrs` is reserved for a later version; v1 writers never write it.
- Sockets, FIFOs and device files are not stored.

## 10. Snapshot

A snapshot object is an envelope (label `keelhaul/snapshot`) whose payload is:

```json
{
  "version": 1,
  "time": "2026-11-04T14:03:22.123456789Z",
  "host": "omarchy",
  "username": "keith",
  "paths": ["/home/keith/docs"],
  "tags": ["nightly"],
  "excludes": ["*.tmp"],
  "tree": "<root tree id>",
  "parent": "<snapshot id or null>",
  "program": "keelhaul 0.1.0",
  "summary": { "files_new": 3, "files_changed": 1, "files_unchanged": 812,
               "dirs": 40, "files_skipped": 0, "data_blobs_added": 7, "bytes_added": 6220351, "bytes_processed": 912003311 }
}
```

`paths` are absolute, cleaned and sorted. The root tree has one entry per backed-up path,
named by the path with every `%` replaced by `%25` first, then every `/` replaced by
`%2F` (so `/a%2Fb` becomes `%2Fa%252Fb`). Restore reverses that encoding (`%2F` → `/`, then
`%25` → `%`) to rebuild the original path; any other `%` sequence in a root entry name is
damage. `files_skipped` counts entries the
backup could not read (permission denied, vanished mid-walk) or did not store (sockets,
FIFOs, devices). Each one is warned about on stderr and listed under `skipped` in
`backup --json`. A skipped entry never aborts the backup. A snapshot ID is shown by its
shortest unique prefix, at least 8 hex characters.

## 11. Lock

A lock object is an envelope (label `keelhaul/lock`) whose payload is:

```json
{ "version": 1, "exclusive": false, "host": "omarchy", "pid": 4242, "username": "keith",
  "created": "...", "refreshed": "..." }
```

- **Acquire:** write the lock, then list `locks/`. Shared conflicts with any other
  non-stale exclusive lock. Exclusive conflicts with any other non-stale lock. On
  conflict, delete your own lock and fail with exit 4. Two writers that race both see
  each other and both fail, which is safe.
- **Refresh:** every 5 minutes, write a new lock with a new `refreshed`, then delete the
  old one.
- **Stale:** `refreshed` older than 30 minutes, or `host` equals this host and `pid` is
  not a running process. `unlock` deletes stale locks only.
- **Self-check:** before writing a snapshot, backup lists `locks/`. If its current lock
  object is gone, or its last successful refresh is older than 30 minutes, backup aborts
  without writing the snapshot.

| Operation | Lock |
|---|---|
| backup, restore, check, verify, snapshots, ls | shared |
| forget, prune, key remove | exclusive |
| init, key add, key list, unlock | none |

## 12. Test vectors

`testdata/vectors/` holds JSON vectors with a fixed master key, nonce and input for:
subkey derivation, the gear table (first 8 entries and a hash of all 256), cut points on a
seeded 32 MiB stream (`seed`, `len`, list of chunk lengths), chunk IDs, one envelope of
each label, a password slot (fixed salt and cheap KDF parameters m_kib 8192, t 1, p 1),
a key-file slot, a two-blob pack, and an index, tree and snapshot payload. The
implementation's tests read these files; a third-party reader can use them unchanged.

## 13. Write ordering and crash safety

1. A pack is uploaded completely before any index names it.
2. An index is uploaded before any snapshot that depends on its blobs.
3. A snapshot is the last object a backup writes.

Backends make each write atomic. Local and SFTP write `<name>.tmp-<16 hex>` in the target
directory, then rename. Local backends also fsync the file and the directory. S3 uses a
single PUT. A reader ignores names ending in `.tmp-<16 hex>`. `prune` deletes them once
they are older than 30 minutes.

After a crash, the repository may hold packs that no index names (orphans) and indexes
that no snapshot needs. Both are valid. `check` reports them as notes, not damage, and
`prune` removes them. A rerun of backup reuses every blob named by a flushed index.

## 14. Prune

Under an exclusive lock:

1. Read every snapshot and walk its trees to get the used blob set. Any unreadable
   snapshot or tree aborts prune with exit 3. Prune never deletes data on a guess.
2. A pack with no used blobs is deleted, unless an index names none of its blobs and it
   is younger than 30 minutes (it may belong to an in-flight backup on another host).
3. A pack whose unused bytes exceed `--max-unused` (default 20%) of its blob bytes is
   repacked: its used blobs are copied into new packs.
4. Write new index objects whose `supersedes` lists every old index. Only then delete the
   old indexes, the repacked packs and the unused packs, in that order.

A crash at any step leaves a repository that `check` passes.
