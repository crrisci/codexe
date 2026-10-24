#!/usr/bin/env bash
# Copyright (c) 2026 CodeXe contributors. SPDX-License-Identifier: Apache-2.0
# Reproduce the native Linux test environment without modifying system packages.
set -euo pipefail
umask 077

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
mode=${1:-setup}
case "$mode" in setup|native|network) ;; *) echo "Usage: $0 [setup|native|network]" >&2; exit 2 ;; esac
[[ $(uname -s) == Linux ]] || { echo "Run this script on native Linux or WSL." >&2; exit 1; }
for required_tool in rustup cargo node python3 curl pkg-config cc c++ bwrap; do
    command -v "$required_tool" >/dev/null || { echo "Missing prerequisite: $required_tool (see xenolith/tests/README.md)" >&2; exit 1; }
done
node -e 'if (Number(process.versions.node.split(".")[0]) < 22) process.exit(1)' || { echo "Node >=22 is required." >&2; exit 1; }
pkg-config --exists openssl libcap alsa || { echo "Missing development packages: libssl-dev, libcap-dev, libasound2-dev." >&2; exit 1; }
channel=$(python3 - "$repo/codex-rs/rust-toolchain.toml" <<'PY'
import pathlib, re, sys
print(re.search(r'channel\s*=\s*"([^"]+)"', pathlib.Path(sys.argv[1]).read_text())[1])
PY
)
export RUSTUP_TOOLCHAIN="$channel"
rustup run "$channel" rustc --version
RUST_MIN_STACK=$(python3 -c 'import os; print(max(16777216, int(os.environ.get("RUST_MIN_STACK", "16777216"))))')
export RUST_MIN_STACK
export CARGO_INCREMENTAL=${CARGO_INCREMENTAL:-0}
export CARGO_PROFILE_DEV_DEBUG=${CARGO_PROFILE_DEV_DEBUG:-0}
export CARGO_PROFILE_TEST_DEBUG=${CARGO_PROFILE_TEST_DEBUG:-0}
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-2}

cache=${XENOLITH_TEST_CACHE_DIR:-$repo/.test-artifacts/linux-tests}
mkdir -p "$cache"
cache=$(cd "$cache" && pwd)
target=$(rustc -vV | sed -n 's/^host: //p')
version=$(python3 "$repo/.github/scripts/rusty_v8_bazel.py" resolved-v8-crate-version)
v8_dir="$cache/v8/$version/$target"
mkdir -p "$v8_dir"
archive="librusty_v8_ptrcomp_sandbox_release_${target}.a.gz"
binding="src_binding_ptrcomp_sandbox_release_${target}.rs"
manifest="rusty_v8_ptrcomp_sandbox_release_${target}.sha256"
trusted="$repo/third_party/v8/rusty_v8_${version//./_}_release_manifests.sha256"
url="https://github.com/openai/codex/releases/download/rusty-v8-v${version}"
fetch() {
    if [[ ! -f "$v8_dir/$1" ]]; then
        local download="$v8_dir/$1.download.$$"
        curl --fail --location --retry 2 --proto '=https' --tlsv1.2 "$url/$1" -o "$download"
        mv "$download" "$v8_dir/$1"
    fi
}
fetch "$manifest"
python3 - "$trusted" "$v8_dir/$manifest" <<'PY'
import hashlib, pathlib, sys
trusted, manifest = map(pathlib.Path, sys.argv[1:])
entries = dict((name.lstrip("*"), digest) for digest, name in (line.split() for line in trusted.read_text().splitlines()))
if hashlib.sha256(manifest.read_bytes()).hexdigest() != entries.get(manifest.name):
    raise SystemExit("V8 manifest checksum mismatch or unsupported target")
PY
fetch "$archive"
fetch "$binding"
python3 - "$v8_dir/$manifest" "$archive" "$binding" <<'PY'
import hashlib, pathlib, sys
manifest = pathlib.Path(sys.argv[1])
entries = dict((name.lstrip("*"), digest) for digest, name in (line.split() for line in manifest.read_text().splitlines()))
if set(entries) != set(sys.argv[2:]):
    raise SystemExit("Unexpected V8 artifact manifest")
for name, expected in entries.items():
    if hashlib.sha256((manifest.parent / name).read_bytes()).hexdigest() != expected:
        raise SystemExit("V8 checksum mismatch: " + name)
print("V8 archive and bindings verified")
PY
export RUSTY_V8_ARCHIVE="$v8_dir/$archive"
export RUSTY_V8_SRC_BINDING_PATH="$v8_dir/$binding"
for name in PATH RUSTUP_HOME CARGO_HOME PKG_CONFIG_SYSROOT_DIR PKG_CONFIG_PATH CFLAGS CXXFLAGS LIBRARY_PATH LD_LIBRARY_PATH CARGO_TARGET_DIR RUSTUP_TOOLCHAIN RUST_MIN_STACK CARGO_INCREMENTAL CARGO_PROFILE_DEV_DEBUG CARGO_PROFILE_TEST_DEBUG CARGO_BUILD_JOBS RUSTY_V8_ARCHIVE RUSTY_V8_SRC_BINDING_PATH; do
    if declare -p "$name" &>/dev/null; then
        printf 'export %s=%q\n' "$name" "${!name}"
    fi
done > "$cache/env.sh"

cd "$repo"
manifest_path="$repo/codex-rs/Cargo.toml"
cargo build --manifest-path "$manifest_path" -p codex-cli -p codex-code-mode-host -p codex-linux-sandbox --bins
target_dir=$(cargo metadata --manifest-path "$manifest_path" --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
export XENOCODEX_NATIVE_CODEX_BIN="$target_dir/debug/codex"
printf 'export XENOCODEX_NATIVE_CODEX_BIN=%q\n' "$XENOCODEX_NATIVE_CODEX_BIN" >> "$cache/env.sh"
# Simulator tests must not accidentally discover a user's real model installation.
unset XENOLITH_BIN XENOLITH_MODEL XENOLITH_SOCKET XENOLITH_STATE_DIR XENOLITH_MMPROJ
case "$mode" in
    native)
        cargo test --manifest-path "$manifest_path" -p codex-xenolith-wire
        cargo test --manifest-path "$manifest_path" -p codex-model-provider-info -p codex-config --lib xenolith
        cargo test --manifest-path "$manifest_path" -p codex-tui --lib xenolith
        cargo test --manifest-path "$manifest_path" -p codex-core-plugins --lib startup_sync
        node --test --test-concurrency=1 xenolith/tests/test-*.mjs
        ;;
    network)
        cargo test --manifest-path "$manifest_path" -p codex-linux-sandbox --lib
        cargo test --manifest-path "$manifest_path" -p codex-core --test all network_approval -- --test-threads=2
        ;;
esac
printf 'Linux test setup ready. For custom cargo commands: source %q\n' "$cache/env.sh"
