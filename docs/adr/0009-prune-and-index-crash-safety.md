# Prune and index rebuilds survive a crash at any point

Governs `src/ops/prune.rs`, `src/ops/prune/`, `src/ops/rebuild_index.rs`, `src/ops/check/mod.rs`.

Prune plans fully before mutating anything, publishes its replacement index before deleting, and includes any additive indexes the pack writer flushed, which stay valid after a crash. Partial replacement indexes are additive; only the final object supersedes old indexes. `rebuild-index` follows the same rule: the final commit supersedes all old names, including unreadable ones, and earlier batches stay live after it is published.

Before the first deletion, prune can roll back by withdrawing only its own replacement objects, which restores old indexes that a backup running while prune waited still needs (lock fencing, SPEC #21).

Age-based cleanup needs ages. Young orphan packs, and packs whose modification time is unknown, keep their 30-minute grace period, so prune never treats blobs recovered from an unindexed pack as indexed data. A missing modification time disables both age-based cleanups, of orphan packs and of temporary names.

An unreadable pack header is different from a new pack: verified indexed ranges inside it can be rescued into new packs, but the damaged physical pack is never kept. Physical damage to one of several duplicate copies is recoverable; an unreadable logical graph is not, and prune refuses.

The crash tests use durability sentinels to make the two ordering mutations deterministic: a stalled first pack must not enter an index, and a stalled ninth pack leaves the first eight-pack index reusable. Random kills cover ordinary I/O.
