# Backup pipeline: shutdown ordering, read consistency, chunk cuts

Governs `src/ops/backup/` and `src/chunker.rs`.

**Channels and shutdown.** The result channel holds `jobs + 8`: at most eight packs wait for an index flush, and the headroom lets every uploader report a failure while the assembler dispatches a batch, so neither side deadlocks. The file stage keeps its task receiver alive until the primary error is reported; otherwise the walker reports cancellation before the I/O cause and the user sees the wrong error. On an early return, dropping the receivers and the store's upload sender releases every blocked send before the thread scope joins. The upload sender must be dropped inside the scope, before it joins the uploaders, or they never finish.

**Read consistency.** Metadata is taken from the opened descriptor, not from the earlier walk, so a file swapped between walk and open is stored as read. Reads of a hard-linked inode are serialised so every alias gets one descriptor's metadata and bytes. Resource exhaustion (EMFILE and the like) fails the run; it is never a reason to omit a file.

**Change detection.** Size + mtime + ctime + inode. A content write changes ctime, so the incremental tests restore mtime to prove size + mtime alone would miss the change.

**Chunk cuts.** A discovered cut never depends on how much input has been read: the chunker exhausts the current tail before compacting its buffer, unless it needs more bytes to find a cut. Read sizes therefore never change chunk boundaries or deduplication.
