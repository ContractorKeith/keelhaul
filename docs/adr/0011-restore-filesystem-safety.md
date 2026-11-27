# Restore writes relative to pinned descriptors

Governs `src/ops/restore/destination*`.

Every ancestor of a restored path is opened with `O_NOFOLLOW` and every write is relative to a pinned directory descriptor, so a symlink planted in the target cannot redirect bytes. Name collisions are detected by comparing filesystem identities within the pinned parent, not by assuming a case or Unicode folding rule. Opening an existing file for resume uses `O_NONBLOCK`, so a raced replacement with a FIFO cannot hang restore.

Where the filesystem lacks a no-replace rename, a hard link publishes the already verified inode atomically without replacing an existing name.

Durability on macOS: each file gets `fsync`, which keeps per-file ordering without macOS's expensive drive-cache flush, and restore issues one `F_FULLFSYNC` on the target directory after publication. Sudden power loss before that final barrier may lose completed files; that is the accepted cost.

Known gap: when restoring a hard link, another writer with access to the target can replace the source between the identity check and `linkat`. Tracked in #58.
