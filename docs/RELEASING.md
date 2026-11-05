# Releasing keelhaul

The release workflow (`.github/workflows/release.yml`) builds and publishes the four archives
and `SHA256SUMS` when a `v*` tag is pushed. Homebrew and crates.io are manual follow-ups. Do
them in this order, the same session.

1. **Gate.** `main` is green in CI, `skeptic-review` on Linux and macOS has no open
   `skeptic` issues for this version, and `scripts/accept.sh` passes on both machines at the
   commit you will tag.
2. **Changelog.** Replace `## <version> — unreleased` with `## <version> — <YYYY-MM-DD>` and
   remove any "draft"/"unreleased" wording from that section; it becomes the release notes.
   For 0.1.0 also change the SPEC status line to "Frozen as format version 1 at keelhaul
   0.1.0". Commit `chore: release <version>` and push.
3. **Tag.** `git tag -a v<version> -m "keelhaul <version>" && git push origin v<version>`.
   Watch the `Release` workflow: gate, four builds, checksums, publish.
4. **Verify the release.** Download `SHA256SUMS` and one archive; `sha256sum -c` passes; the
   binary's `keelhaul version` prints the version.
5. **Homebrew.** In `ContractorKeith/homebrew-tap`, update `Formula/keelhaul.rb` with the four
   sha256 values from `SHA256SUMS` and the version, `brew style` and `brew audit`, merge. Then
   on Omarchy (Linuxbrew) and the Mac Studio: `brew update && brew install
   ContractorKeith/tap/keelhaul && keelhaul version`.
6. **crates.io.** From the tagged commit: `cargo publish --locked`. Then
   `cargo install keelhaul --locked --version <version>` in a scratch `CARGO_HOME` and run
   `keelhaul version`.
7. **Track.** Close the release's milestone issue, complete the Basecamp card's step and move
   the card to Done, record the session in the worklog.
