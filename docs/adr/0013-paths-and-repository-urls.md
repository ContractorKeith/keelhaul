# Repository URLs and backup root paths

Governs `src/cli/repo.rs` and `src/ops/paths.rs`.

Only a lowercase URI scheme before the first slash (`sftp:`, `s3:`) selects remote storage. Any other string is a filesystem path, and colons in it stay ordinary filename characters.

A backup root is normalised by resolving its parent through the filesystem while preserving a final symlink. Historical roots whose parents no longer exist fall back to a cleaned absolute spelling.
