# Keyed chunk IDs and a keyed gear table

Chunk IDs are BLAKE3 keyed with a secret derived from the master key, and the FastCDC gear table is derived from the same key, so whoever holds the storage can neither confirm that a known file was backed up nor infer it from chunk boundaries. The cost is that two repositories never deduplicate against each other, and copying snapshots between repositories means re-chunking. Chosen over plain hashes because the S3 or SFTP host is the threat model, and reversing it later would change every ID in every repository.
