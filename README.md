# CodeXe

CodeXe is a community fork of OpenAI Codex with native local Xenolith inference.
The name combines Codex with a nod to Intel Xe processors.
CodeXe preserves Codex's agent loop, tools, sandbox and approvals,
while exposing the state of local inference and keeping durable KV sessions.

[Setup guide](XENOLITH.md) · [Modification notices](XENOLITH-MODIFICATIONS.md)

## Why native integration?

On a local machine, inference speed and the cost of rebuilding context matter.
The Xenolith provider communicates directly over a Unix socket and carries local
inference information into CodeXe's interface. It does not require an HTTP bridge.

- **Dedicated inference status** for engine startup, live prefill progress and
  measured generation speed, separate from conversation text.
- **Service discovery** for the served model, context, output limit and reasoning capabilities.
- **Vision** through existing image attachments and `view_image`, when Xenolith serves a projector.
- **Native reasoning**, including streamed thoughts and history across tools and resume.
- **Optional engine startup**, sharing Xenolith's weight-owner lock with other clients.
- **Durable sessions and checkpoints**, with actual KV snapshot restoration reports.
- **Cache diagnostics:** reused context, loaded snapshot or reconstruction reason.
- **Rewind cost before applying it**, expressed in tokens to prefill.
- **Interrupted-turn recovery**, including pending tool results without automatically
  repeating filesystem effects whose execution was already recorded by Codex.

Xenolith is the built-in default provider on Linux/WSL. Explicitly selected
upstream providers remain available.

## Build and run

This is a development source distribution. Build this branch to get the Xenolith
provider; official Codex installers and npm packages contain the upstream version.

The complete CLI and real-model tests were run on Linux/WSL. Build prerequisites:
Rust **1.95.0**, a C/C++ compiler, OpenSSL development headers and a compatible
`bwrap` for Linux sandboxed tools. Xenolith and model weights are separate.

```sh
git clone --branch xenolith-native https://github.com/crrisci/codexe.git codexe
cd codexe/codex-rs
cargo build -p codex-cli --bin codex
```

From your coding workspace, start the rebuilt CLI with your engine and model:

```sh
XENOLITH_BIN=/absolute/path/to/xenolith \
XENOLITH_MODEL=/absolute/path/to/model.gguf \
/absolute/codexe/codex-rs/target/debug/codex
```

If `xenolith` is on `PATH`, omit `XENOLITH_BIN`. If the engine is already running
on the default socket, the CLI needs neither variable. Set `XENOLITH_SOCKET` for
a different socket. Provider registration, model/context discovery, native
binding storage and disabling provider-hosted web search are automatic.

The executable remains `codex` and uses the usual `CODEX_HOME` (`~/.codex` by
default). An optional separate home keeps CodeXe configuration and conversations
apart from another Codex installation. Native bindings default to
`$CODEX_HOME/xenolith/bindings`. Native interactive sessions use the fork's
embedded server; `--no-daemon` is not required.

Explicit provider selections in config, profiles or CLI remain authoritative.
If an existing configuration relied on the implicit OpenAI default, add
`model_provider = "openai"` to retain that selection, including when it only
specified an OpenAI model. See the [configuration guide](XENOLITH.md#start-and-configure).

Keep the CodeXe home, native bindings and Xenolith state/cache to resume conversations.
The provider connects to an existing engine first. Optional startup is enabled by
setting a model path; `XENOLITH_NO_SPAWN=1` enforces connect-only operation.
An automatically started engine remains available until its idle shutdown.

## Current limits

- Rewind uses an **offline controller**, with the CodeXe thread closed; an interactive picker is not implemented.
- Text, reasoning, function/custom tools and one namespace level are supported.
  PNG/JPEG images are supported, one per user message or tool result. Audio,
  hosted/deferred tools, nested namespaces and constrained JSON are rejected.
- Output defaults to the service's advertised limit, with an optional lower client limit.
  Reaching that limit terminates the response without an automatic retry.
  Crash recovery conservatively rebuilds context;
  uncommitted streamed output is not guaranteed to survive.
- Compaction is verified manually; automatic saturation of the 65536-token context remains untested.
- The Xenolith integration targets **Linux and WSL**. macOS and native Windows are not targets for the current engine.

## CodeXe and companion bridge

This branch is based on upstream **`rust-v0.162.1`**, commit
`092d3acd6bec3e3a14bdc7e7a2810ab628ab759d`. Native changes are concentrated in
`codex-rs/xenolith-wire`, the provider integration and dedicated inference events.

**Xenolith Bridge**, formerly Xenocodex, is the separate HTTP adapter for clients
that need API translation. Its current frontend speaks Responses HTTP/SSE; CodeXe can connect directly to Xenolith through this fork. Other harness protocols
require their own bridge frontends.

CodeXe is maintained independently of OpenAI and is not affiliated with Intel.
It retains the **Apache-2.0**
[LICENSE](LICENSE), upstream [NOTICE](NOTICE) and [modification notices](XENOLITH-MODIFICATIONS.md).
The [original upstream README](README-UPSTREAM.md) is retained for reference.
