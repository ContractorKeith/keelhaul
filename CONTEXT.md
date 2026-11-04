# keelhaul

The words keelhaul uses for a deduplicating, encrypted backup repository and what lives in it. One definition each.

## Language

**Repository**:
The storage location (a local directory, an SFTP path or an S3 bucket prefix) holding every encrypted object keelhaul writes. One repository has one master key.
_Avoid_: repo store, vault, archive, bucket

**Backend**:
The code that reads, writes, lists and deletes named objects in one kind of storage (local, SFTP, S3). It knows nothing about the format.
_Avoid_: driver, provider, remote, storage adapter

**Chunk**:
A variable-length piece of a file's contents, cut by content-defined chunking so unchanged data produces the same chunks.
_Avoid_: block, segment, piece

**Chunk ID**:
The keyed BLAKE3 hash of a chunk's plaintext, keyed with the repository's ID key. Two chunks with the same ID are stored once.
_Avoid_: hash, digest, fingerprint

**Master key**:
The random secret that encrypts every object and keys every chunk ID. It never leaves the repository unwrapped.
_Avoid_: repo key, data key

**Key slot**:
One stored copy of the master key, wrapped by a password (through Argon2id) or by a key file. Adding or removing a slot never rewrites data.
_Avoid_: keyfile entry, credential, unlock key

**Blob**:
One stored unit inside a pack: a chunk of file data or a tree, compressed (or not) then encrypted, addressed by its ID.
_Avoid_: object, record, entry

**Pack**:
One backend object holding many encrypted blobs and an encrypted header listing them. Packs are written once and never modified; prune replaces them.
_Avoid_: segment, bundle, archive file

**Index**:
Encrypted objects mapping every blob ID to its pack, offset and length. It can be rebuilt from pack headers.
_Avoid_: catalog, manifest, database

**Tree**:
A blob describing one directory: its entries' names, types, metadata, and the blob IDs of their contents or subtrees.
_Avoid_: directory listing, node, manifest

**Snapshot**:
An encrypted object recording one backup run: time, host, paths, tags and the root tree ID. Deleting a snapshot frees nothing until prune.
_Avoid_: backup, archive, version, generation

**Lock**:
An object in the repository saying which host and process is using it, shared or exclusive, refreshed while held. Stale after 30 minutes without refresh; dead when its process is gone on this host. Only dead locks are ignored; stale ones wait for `unlock`.
_Avoid_: lease, mutex, lockfile

**Parent snapshot**:
The earlier snapshot of the same host and paths that a backup compares against to skip unchanged files.
_Avoid_: base, previous backup, reference snapshot

**Retention policy**:
The keep-last/hourly/daily/weekly/monthly/yearly and keep-tag rules `forget` applies to each group of snapshots sharing host and paths.
_Avoid_: schedule, rotation, keep rules

**Forget**:
Delete snapshot objects chosen by a retention policy or by ID. Frees no space by itself.
_Avoid_: delete, expire, remove

**Prune**:
Delete packs no snapshot reaches and repack packs that are mostly unused, then rewrite the index. The only operation that frees space.
_Avoid_: gc, compact, cleanup, vacuum

**Check**:
A structural audit of the repository: every index, pack header, tree and snapshot decrypts and every reference resolves. Reads metadata, not file data.
_Avoid_: fsck, scan, validate

**Verify**:
A full read of the repository: every pack downloaded, every blob decrypted and re-hashed against its ID, every file in every snapshot resolved. The proof a backup is restorable.
_Avoid_: check --read-data, audit, test restore

**Damage**:
Any object that is missing, truncated, undecryptable, mis-hashed or inconsistent with what references it. Check and verify list each one and exit 3.
_Avoid_: corruption error, bad data, failure
