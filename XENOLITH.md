# CodeXe native Xenolith provider

Copyright (c) 2026 Francesco Pio Crisci. Licensed under Apache-2.0.

CodeXe connects directly to Xenolith wire protocol v1 over a Unix socket.
No HTTP adapter is required. CodeXe still owns the agent loop, shell tools, approvals
and conversation history. Xenolith owns inference, native sessions and KV snapshots.

Baseline: `rust-v0.162.1`, commit `092d3acd6bec3e3a14bdc7e7a2810ab628ab759d`.
Current limitations are listed below.

## Build

Linux/WSL is the environment used for the complete CLI and real-model validation.
Install Rust 1.95.0, a C/C++ compiler, OpenSSL development headers and a compatible
`bwrap` for Linux sandboxed tool execution. These commands do not install Xenolith.

```sh
git clone --branch xenolith-native https://github.com/crrisci/codexe.git codexe
cd codexe/codex-rs
cargo build -p codex-cli --bin codex
cargo build --manifest-path xenolith-wire/Cargo.toml --bin xenolith-session
```

The CLI is `codex-rs/target/debug/codex`; the controller is
`codex-rs/xenolith-wire/target/debug/xenolith-session`.
The tagged upstream workspace may regenerate package versions in Cargo.lock;
use the build command above rather than `--locked` for this development branch.
The transport is Unix-only; use WSL on Windows. A full macOS CLI build has not been
validated in this milestone, although wire/backend tests passed on macOS.

## Start and configure

Start your compiled Xenolith explicitly with isolated state/cache directories.
Use the served model and context capacity reported by its `describe` operation.
For example, with a GGUF model and a configured context of 65536:

```sh
/path/to/xenolith serve /path/to/model.gguf --ctx 65536 \
  --socket /absolute/private/wire.sock \
  --state /absolute/private/engine-state \
  --cache /absolute/private/engine-cache
```

Use a new private CodeXe home and native binding directory. Keep these, as well as
engine state/cache, to resume conversations and restore KV across processes.
Do not share a native binding directory between unrelated CodeXe homes.

```sh
export CODEX_HOME=/absolute/private/codex-home
export XENOLITH_STATE_DIR=/absolute/private/codex-bindings
mkdir -p "$CODEX_HOME" "$XENOLITH_STATE_DIR"
```

Write `$CODEX_HOME/config.toml` with the exact model identifier returned by Xenolith:

```toml
model = "YOUR_SERVED_MODEL"
model_provider = "xenolith"
model_context_window = 65536
model_reasoning_effort = "none"
web_search = "disabled"

[model_providers.xenolith]
name = "Xenolith"
base_url = "unix:///absolute/private/wire.sock"
wire_api = "xenolith"
requires_openai_auth = false
supports_websockets = false
request_max_retries = 0
stream_max_retries = 0
```

Then start the compiled CLI from your coding workspace:

```sh
/absolute/codexe/codex-rs/target/debug/codex
```

Use the same home, binding directory and engine state/cache when resuming. The
native backend connects to an existing engine; it does not start/stop it implicitly.
No API key is required for this provider. Model/context discovery in the picker is
not implemented; the example model name must match your own engine.

## Sessions and local telemetry

Bindings are durable, atomic and locked. The native path never uses ephemeral
requests or deletes sessions belonging to another client. It reopens unchanged
history, appends continuations and explicitly rebuilds after changed context or an
uncertain interrupted mutation. A divergent committed marker stops for investigation.

TUI status displays actual wire prefill progress, prefill/decode rates, reused cache
and snapshot/reconstruction reports. Missing measurements stay unavailable. Actual
`loaded: true` confirms restoration; an opening zero-prefill hint alone does not.
Snapshot persistence can require an explicit checkpoint because autosave is rate-limited.

## Offline rewind

Close CodeXe for the thread before using this controller:

```sh
xenolith-session SOCKET STATE_DIR THREAD
xenolith-session SOCKET STATE_DIR THREAD MARKER
xenolith-session SOCKET STATE_DIR THREAD MARKER CODEX_ROLLOUT_JSONL
```

The first command lists committed boundaries; the second previews Xenolith's
`rewind_cost`; the third applies it, checkpoints and writes Codex's rollback event.
Use absolute paths. `prefill` is a number of tokens, not a promised duration.
A private audit backup is retained. Filesystem effects are never undone or rerun.
Pending tools, uncertain state and a live competing owner are rejected. No interactive
rewind picker is included yet. Compaction invalidates old rewind anchors.

## Tests

From the fork root:

```sh
cargo test --manifest-path codex-rs/xenolith-wire/Cargo.toml
XENOCODEX_NATIVE_CODEX_BIN="$PWD/codex-rs/target/debug/codex" \
  node --test xenolith/tests/test-codex.mjs xenolith/tests/test-crash-tool.mjs
```

The CLI acceptance tests require Node >=22 and the rebuilt fork. They use isolated
homes and a simulated wire server, exercising real tool execution and resume after
an interrupted tool-result append. They do not prove real KV restoration.
GitHub CI checks the isolated wire/backend tests and acceptance script syntax;
it does not load a real model or execute the full upstream suite.

## Unsupported in this milestone

Reasoning and historical reasoning items, media, hosted/deferred tools, nested
namespaces and constrained JSON are rejected. Function/custom tools with one namespace
are supported. Output is limited to 4096 tokens. Native temporary requests are not
exposed. Crash recovery conservatively rebuilds KV and cannot guarantee uncommitted
streamed output. Automatic context saturation and GGUF incompatibility remain untested.
