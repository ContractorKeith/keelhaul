#!/usr/bin/env bash
# scripts/accept.sh — the acceptance for keelhaul. Worker, reviewer and CI run this exact
# script; the orchestrator never retypes it. Each milestone appends its live checks below.
# Bash 3.2 compatible (macOS): no mapfile, no ${var,,}, no associative arrays.
set -eu

cd "$(dirname "$0")/.."
scratch_root="${TMPDIR:-/tmp}"
scratch="$(mktemp -d "$scratch_root/keelhaul-accept.XXXXXX")"
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

echo "accept: ok"
