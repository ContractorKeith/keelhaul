# Process startup: re-exec to scrub the environment, local offset before threads

Governs `src/main.rs` and `src/ops/forget.rs`.

**Environment.** Secrets passed in environment variables are removed by re-executing the binary with a cleaned environment, not by `set_var`: `unsafe` is forbidden, and mutating process-global state is racy. The re-exec also covers backend-specific variables without a second copy of their parsers.

**Local time.** The `time` crate can only look up the local UTC offset safely while the process is single-threaded. `forget::local_offset()` therefore runs at startup, before SSH reader threads or backup workers exist. Since `time` 0.3.55 the guarded TZ refresh is separate from the thread-safe lookups, so after the startup refresh, `local_offset_at` still gives historical DST offsets and falls back to the startup offset. Retention tests that use SFTP capture the offset before the reader starts for the same reason.
