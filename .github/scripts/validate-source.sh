#!/usr/bin/env bash
set -euo pipefail
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/log.sh"

repo="$PWD"
inventory_only=false
output=""
while (($#)); do
  case "$1" in
    --repo) repo="${2:?--repo requires a path}"; shift 2 ;;
    --output) output="${2:?--output requires a path}"; shift 2 ;;
    --inventory-only) inventory_only=true; shift ;;
    *) log ERROR 'Unknown source validation option'; exit 2 ;;
  esac
done

trap 'log ERROR "Source validation did not complete"' ERR
repo=$(cd -- "$repo" && pwd -P)
work=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/kiaro-source-validation.XXXXXX")
trap 'rm -rf -- "$work"' EXIT
inventory="$work/source-files.nul"
git -C "$repo" ls-files -z > "$inventory"
count=0
while IFS= read -r -d '' path; do
  # Ignore rules do not protect files already committed to the source archive.
  # Public environment templates are the deliberate exceptions, not live data.
  case "${path,,}" in
    .env.example|*/.env.example|.env.sample|*/.env.sample|.env.template|*/.env.template) ;;
    .env|*/.env|.env.*|*/.env.*|*.session|*.session-*|*.pem|*.key|*.pfx|*.p12)
      log ERROR "Forbidden tracked credential/session path: $(printf '%q' "$path")"
      exit 1
      ;;
  esac
  case "$path" in
    .ai/*|.specify/*|specs/*|secrets.md|graphify-out/*|.gpg-private/*|bin/*|uv.lock|\
    __pycache__/*|*/__pycache__/*|*.pyc|*.pyo|.pytest_cache/*|*/.pytest_cache/*|\
    .mypy_cache/*|*/.mypy_cache/*|.ruff_cache/*|*/.ruff_cache/*|.venv/*|*/.venv/*|\
    target/*|*/target/*|logs/*|*.log|*.tmp|*.bak|.cargo/credentials*|*/.cargo/credentials*)
      log ERROR "Forbidden tracked build/cache path: $(printf '%q' "$path")"
      exit 1
      ;;
  esac
  count=$((count + 1))
done < "$inventory"

required=(
  Cargo.toml Cargo.lock LICENSE NOTICE README.md schema/end-to-end.tl
  migration/019-rust-anything-receipt.json docs/signing/publisher.json
  vendor/grammers/upstream-hashes.json vendor/grammers/local-patches.json
  vendor/grammers/grammers-client/Cargo.toml
  vendor/grammers/grammers-mtsender/Cargo.toml
  vendor/grammers/grammers-session/Cargo.toml
)
for path in "${required[@]}"; do
  if ! git -C "$repo" ls-files --error-unmatch -- "$path" >/dev/null 2>&1; then
    log ERROR "Required source path is not tracked: $path"
    exit 1
  fi
done
log INFO "Validated $count tracked source paths"
log DEBUG 'Source inventory is NUL-delimited; cache paths cannot hide behind whitespace'
if "$inventory_only"; then
  exit 0
fi
if [[ -z "$output" ]]; then
  log ERROR 'An output directory is required for distribution validation'
  exit 2
fi
if ! git -C "$repo" diff --quiet HEAD --; then
  log ERROR 'Commit tracked changes before validating their source archive'
  exit 1
fi
mkdir -p -- "$output"
output=$(cd -- "$output" && pwd -P)
revision=$(git -C "$repo" rev-parse --verify HEAD)
git -C "$repo" archive --format=tar "--output=$output/kiaro-secret-chat-source.tar" "$revision"
printf '%s\n' "$revision" > "$output/commit.txt"
cp -- "$inventory" "$output/source-files.nul"

extracted="$work/Source Archive With Spaces"
mkdir -p -- "$extracted"
tar -xf "$output/kiaro-secret-chat-source.tar" -C "$extracted"
log INFO 'Compiling the extracted source archive from a path containing spaces'
cargo check --locked --all-targets --manifest-path "$extracted/Cargo.toml"

consumer="$work/git-consumer"
mkdir -p -- "$consumer/src"
git_url="file://$repo"
quoted_url=$(jq -Rn --arg value "$git_url" '$value')
cat > "$consumer/Cargo.toml" <<EOF
[package]
name = "kiaro-git-consumer"
version = "0.0.0"
edition = "2024"

[dependencies]
kiaro-secret-chat = { git = $quoted_url, rev = "$revision" }
EOF
printf '%s\n' 'pub use kiaro_secret_chat::{grammers_client::Client, protocol::ChatRole};' > "$consumer/src/lib.rs"
log INFO 'Resolving a downstream Git dependency at the exact tested commit'
cargo generate-lockfile --manifest-path "$consumer/Cargo.toml"
cargo check --locked --manifest-path "$consumer/Cargo.toml"

log INFO 'Installing both maintenance commands from the exact Git revision'
install="$work/installed-tools"
cargo install --locked --git "$git_url" --rev "$revision" --root "$install" --debug --bins kiaro-secret-chat
"$install/bin/check-test-matrix" --help
"$install/bin/generate-schema" --schema "$extracted/schema/end-to-end.tl" --out "$work/schema.json"
"$install/bin/generate-schema" --check --schema "$extracted/schema/end-to-end.tl" --out "$work/schema.json"
(
  cd -- "$output"
  sha256sum kiaro-secret-chat-source.tar commit.txt source-files.nul > SHA256SUMS
  sha256sum --strict --check SHA256SUMS
)
log INFO 'Source archive, downstream Git consumer, and installed commands passed validation'
