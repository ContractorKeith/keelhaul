# Portable acceptance, release workflow and platform-bound tests

Governs `scripts/accept.sh`, `.github/workflows/release.yml` and platform-specific tests.

**accept.sh** runs unchanged on release machines, including macOS: Bash 3.2 (no `mapfile`, `${var,,}` or associative arrays) and no jq or Python. JSON fields are extracted with `sed`, a byte is flipped with Bash arithmetic and `od`/`dd`, and the metadata walk uses portable `stat`; it compares times to the second, while the Rust tests pin nanoseconds.

**Release.** crates.io publishing is manual (`docs/RELEASING.md`); the workflow only runs `cargo publish --dry-run` to prove the package builds. Zig supplies the C cross-compiler for bundled zstd as well as the linker. A static PIE may have a DYNAMIC section but must have neither a program interpreter nor any shared-library dependency; `readelf` checks both.

**Docker e2e.** `compose.yaml` mounts `${KEELHAUL_SSHD_KEYS:-./.tmp/sshd}` into the sshd container; the SFTP e2e tests set `KEELHAUL_SSHD_KEYS` to their scratch key directory.

**Platform-bound tests.** APFS rejects non-UTF-8 names, so those cases are Linux-only while shared-chunk salvage still runs on macOS. Case-folding filesystems alias the uppercase-ID fixture, so only that fixture is omitted there. Exclude semantics are pinned against `git check-ignore --no-index` (braces are literal, unescaped trailing spaces are trimmed, a backslash keeps one). The `/dev` fixture whitelists only the empty `/dev` directory; no device, home or user file is opened. Root-run harnesses drop privileges only for the CLI subprocess so permission checks are real. Retention fixtures use America/New_York across DST: 366 local days, because the spring and fall hours cancel, and the ISO-week expectations are spelled out in `tests/m5_forget.rs`. A disconnected socket makes BrokenPipe deterministic. Fixtures restore permissions before asserting, so cleanup always succeeds.
