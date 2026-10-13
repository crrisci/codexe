# Codex with native Xenolith inference

Use Codex CLI with a local Xenolith engine through its native wire protocol.
This community fork preserves Codex's agent loop, tools, sandbox and approvals,
while exposing the state of local inference and keeping durable KV sessions.

[Setup guide](XENOLITH.md) · [Modification notices](XENOLITH-MODIFICATIONS.md)

## Why native integration?

On a local machine, inference speed and the cost of rebuilding context matter.
The Xenolith provider communicates directly over a Unix socket and carries local
inference information into Codex's interface. It does not require an HTTP bridge.

- **Live prefill progress** and measured prefill/decode speed in the TUI.
- **Durable sessions and checkpoints**, with actual KV snapshot restoration reports.
- **Explicit cache state:** reused context, loaded snapshot or reconstruction reason.
- **Rewind cost before applying it**, expressed in tokens to prefill.
- **Interrupted-turn recovery**, including pending tool results without automatically
  repeating filesystem effects whose execution was already recorded by Codex.

The existing upstream Responses provider remains available.

## Build and run

This is a development source distribution. Build this branch to get the Xenolith
provider; official Codex installers and npm packages contain the upstream version.

The complete CLI and real-model tests were run on Linux/WSL. Build prerequisites:
Rust **1.95.0**, a C/C++ compiler, OpenSSL development headers and a compatible
`bwrap` for Linux sandboxed tools. Xenolith and model weights are separate.

```sh
git clone --branch xenolith-native https://github.com/crrisci/codex.git codex-xenolith
cd codex-xenolith/codex-rs
cargo build -p codex-cli --bin codex
```

1. Start your compiled Xenolith engine with a Unix socket and persistent state/cache.
2. Create a dedicated `CODEX_HOME` and set `XENOLITH_STATE_DIR` to a private binding directory.
3. Configure the served model, context capacity and `wire_api = "xenolith"` in that home.
4. Run the rebuilt `codex-rs/target/debug/codex` from your coding workspace.

For example, after following the [configuration guide](XENOLITH.md#start-and-configure):

```sh
CODEX_HOME=/absolute/private/codex-home \
XENOLITH_STATE_DIR=/absolute/private/codex-bindings \
/absolute/codex-xenolith/codex-rs/target/debug/codex
```

Keep the Codex home, native bindings and Xenolith state/cache to resume conversations.
The provider connects to an existing engine; it does not implicitly start or stop it.

## Current limits

- Rewind uses an **offline controller**, with the Codex thread closed; an interactive picker is not implemented.
- Model and context selection are explicit; automatic discovery in the model picker is not implemented.
- Text, function/custom tools and one namespace level are supported. Reasoning items,
  images/audio, hosted/deferred tools, nested namespaces and constrained JSON are rejected.
- Maximum output is **4096 tokens**. Crash recovery conservatively rebuilds context;
  uncommitted streamed output is not guaranteed to survive.
- Compaction is verified manually; automatic saturation of the 65536-token context remains untested.
- Native Windows requires WSL. Wire/backend tests passed on macOS; a full macOS CLI build remains unverified.

## Fork and companion bridge

This branch is based on upstream **`rust-v0.162.1`**, commit
`092d3acd6bec3e3a14bdc7e7a2810ab628ab759d`. Native changes are concentrated in
`codex-rs/xenolith-wire`, the provider integration and dedicated inference events.

**Xenolith Bridge**, formerly Xenocodex, is the separate HTTP adapter for clients
that need API translation. Its current frontend speaks Responses HTTP/SSE; native
Codex can connect directly to Xenolith through this fork. Other harness protocols
require their own bridge frontends.

This fork is maintained independently of OpenAI. It retains the **Apache-2.0**
[LICENSE](LICENSE), upstream [NOTICE](NOTICE) and [modification notices](XENOLITH-MODIFICATIONS.md).
The [original upstream README](README-UPSTREAM.md) is retained for reference.
