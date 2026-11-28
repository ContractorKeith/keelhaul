# The Backend contract and the storage layout

Governs `src/backend/mod.rs`, `local*.rs`, `foreign.rs`, `fault.rs`, `sftp/scan.rs`, `sftp/status.rs`.

**Writes are create-once.** An identical retry succeeds; different bytes under the same name fail. Ranged reads are clipped at EOF, including offsets beyond EOF. Lists are sorted by name. Both delete operations are idempotent when the target is missing. Every backend passes the same conformance suite; that suite writes in reverse order so insertion order can never satisfy the sorted-list contract by accident, crosses S3's 1000-key ListObjectsV2 page, and uses different payloads for racing writers, because a concurrent reader must see either NotFound or one complete immutable payload, never a prefix.

**Local writes.** Comparing an idempotent retry can read a whole pack, so it happens after the directory lock is released. A failed publication may clean up an object that never existed; deletes return early on NotFound so they never wait out a second directory-lock deadline. Existing directories need no fsync, and creating the repository root tolerates an unreadable parent. NFS and CIFS may not support locks (ENOLCK, EOPNOTSUPP): that is treated as success, because rename stays atomic and content-addressed writes are identical. Create-once there is best effort (accepted 2026-11-04).

**Layout traversal.** The storage layout is trusted, so symlinks are followed (users symlink `data/` to other disks), but only at the fixed layout depths; object-depth directories are foreign entries and are never descended into. Layout directories are traversed only by their own kind. On SFTP, READDIR attributes describe the link itself, so following a link takes an explicit STAT. SFTP v3 has no ELOOP status; OpenSSH reports loops as generic FAILURE (4), which is treated like NO_SUCH_FILE for a broken link.

**Foreign entries.** Raw names are repository-relative and may never resolve outside their kind (no `..`, backslash or NUL). A name that parses as a valid object must go through the typed delete instead. Direct children of `data/` may be shard directories: listing them as foreign does not grant deletion.

**Fault injection.** `FaultBackend` fires delete faults after the durable effect, so a test catches code that deletes references before publishing their replacement index.

**Secret destinations.** `contains_destination` resolves symlinked parents before deciding whether a new key file would land inside a local repository; remote backends answer `false`.
