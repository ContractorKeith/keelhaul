#!/usr/bin/env bash
# scripts/accept.sh — the acceptance for keelhaul. Worker, reviewer and CI run this exact
# script; the orchestrator never retypes it. Each milestone appends its live checks below.
# Bash 3.2 compatible (macOS): no mapfile, no ${var,,}, no associative arrays.
set -eu

cd "$(dirname "$0")/.."
scratch_root="${TMPDIR:-/tmp}"
scratch="$(mktemp -d "$scratch_root/keelhaul-accept.XXXXXX")"
scratch="$(cd "$scratch" && pwd -P)"
trap 'rm -r "$scratch"' EXIT

echo "== gate"
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked

echo "== build"
cargo build --locked --release
bin="$PWD/target/release/keelhaul"

echo "== isolate"
export HOME="$scratch/home" XDG_CACHE_HOME="$scratch/cache" XDG_CONFIG_HOME="$scratch/config"
mkdir -p "$HOME" "$XDG_CACHE_HOME" "$XDG_CONFIG_HOME"
unset KEELHAUL_REPO KEELHAUL_PASSWORD_FILE KEELHAUL_PASSWORD_COMMAND KEELHAUL_KEY_FILE || true

echo "== live checks"
"$bin" version | grep -q '^keelhaul '
# M1+: init / key checks. M2+: backup → restore → diff. M3+: corrupt one pack → verify exits 3.

echo "== M1 key rotation"
printf 'pw1\n' > "$scratch/password"
"$bin" -r "$scratch/repo" --password-file "$scratch/password" init --json
"$bin" -r "$scratch/repo" --password-file "$scratch/password" key list --json > "$scratch/slots.json"
# Extract the fixed JSON field without requiring jq or Python on release machines.
original_id="$(sed -n 's/.*"id":"\([^"]*\)".*/\1/p' "$scratch/slots.json")"
test "${#original_id}" -eq 64
"$bin" -r "$scratch/repo" --password-file "$scratch/password" key add --new-key-file "$scratch/key" --json
"$bin" -r "$scratch/repo" --key-file "$scratch/key" key remove "$original_id" --json
"$bin" -r "$scratch/repo" --key-file "$scratch/key" key list --json > "$scratch/slots.json"
test "$(grep -o '"id":' "$scratch/slots.json" | wc -l | tr -d ' ')" -eq 1
grep -q '"kind":"keyfile"' "$scratch/slots.json"
status=0
"$bin" -r "$scratch/repo" --password-file "$scratch/password" key list || status=$?
test "$status" -eq 1

echo "== M2 backup and restore"
fixture="$scratch/fixture"
mkdir -p "$fixture/nested/empty"
: > "$fixture/empty"
printf 'hardlink contents\n' > "$fixture/hard-a"
ln "$fixture/hard-a" "$fixture/hard-b"
dd if=/dev/urandom of="$fixture/nested/random3" bs=1048576 count=3 2>/dev/null
dd if=/dev/urandom of="$fixture/random9" bs=1048576 count=9 2>/dev/null
ln -s nested/random3 "$fixture/link"
if test "$(uname)" = Linux; then
  raw_name="$(printf 'raw-\377')"
  printf 'non-UTF-8\n' > "$fixture/$raw_name"
fi
chmod 640 "$fixture/hard-a"
"$bin" -r "$scratch/repo" --key-file "$scratch/key" backup "$fixture" --tag accept --json > "$scratch/first.json"
"$bin" -r "$scratch/repo" --key-file "$scratch/key" snapshots --tag accept --json > "$scratch/snapshots.json"
grep -q '"paths":' "$scratch/snapshots.json"
"$bin" -r "$scratch/repo" --key-file "$scratch/key" restore latest --target "$scratch/out"
restored="$scratch/out$fixture"
diff -r "$fixture" "$restored"
# Portable lstat walk: times are compared to seconds here; Rust tests pin nanoseconds.
stat_walk() {
  root="$1"
  find "$root" -print0 | while IFS= read -r -d '' path; do
    relative="${path#"$root"}"
    if test "$(uname)" = Darwin; then
      details="$(stat -f '%Lp %m' "$path")"
    else
      details="$(stat -c '%a %Y' "$path")"
    fi
    target=""
    if test -L "$path"; then target="$(readlink "$path")"; fi
    printf '%s %s %s\n' "$relative" "$details" "$target"
  done | LC_ALL=C sort
}
stat_walk "$fixture" > "$scratch/before.stat"
stat_walk "$restored" > "$scratch/after.stat"
diff "$scratch/before.stat" "$scratch/after.stat"
if test "$(uname)" = Darwin; then
  test "$(stat -f '%i' "$restored/hard-a")" = "$(stat -f '%i' "$restored/hard-b")"
else
  test "$(stat -c '%i' "$restored/hard-a")" = "$(stat -c '%i' "$restored/hard-b")"
fi
"$bin" -r "$scratch/repo" --key-file "$scratch/key" backup "$fixture" --json > "$scratch/second.json"
grep -q '"data_blobs_added":0' "$scratch/second.json"
"$bin" -r "$scratch/repo" --key-file "$scratch/key" restore latest --target "$scratch/single" --include "$fixture/nested/random3"
cmp "$fixture/nested/random3" "$scratch/single$fixture/nested/random3"
test ! -e "$scratch/single$fixture/random9"

echo "== M4 incremental read counter"
touch "$fixture/random9"
"$bin" -r "$scratch/repo" --key-file "$scratch/key" backup "$fixture" --json > "$scratch/incremental.json"
grep -q '"files_read":1' "$scratch/incremental.json"

echo "== M3 check and verify"
"$bin" -r "$scratch/repo" --key-file "$scratch/key" check --json > "$scratch/check.json"
grep -q '"damage":\[\]' "$scratch/check.json"
"$bin" -r "$scratch/repo" --key-file "$scratch/key" verify --json > "$scratch/verify-clean.json"
pack="$(find "$scratch/repo/data" -type f | LC_ALL=C sort | head -n 1)"
# XOR one byte without a Python/jq dependency (Bash arithmetic and portable od/dd).
byte="$(od -An -tu1 -N1 "$pack" | tr -d ' ')"
flipped=$((byte ^ 1))
octal="$(printf '%03o' "$flipped")"
printf "\\$octal" | dd of="$pack" bs=1 count=1 conv=notrunc 2>/dev/null
status=0
"$bin" -r "$scratch/repo" --key-file "$scratch/key" verify --json > "$scratch/damage.json" || status=$?
test "$status" -eq 3
grep -q "$(basename "$pack")" "$scratch/damage.json"
echo "accept: ok"
