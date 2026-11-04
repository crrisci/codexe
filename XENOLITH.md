# CodeXe native Xenolith provider

Copyright (c) 2026 Francesco Pio Crisci. Licensed under Apache-2.0.

CodeXe connects directly to Xenolith wire protocol v1 (text) or v2 (vision) over a Unix socket.
No HTTP adapter is required. CodeXe still owns the agent loop, shell tools, approvals
and conversation history. Xenolith owns inference, native sessions and KV snapshots.

## Build and install

Linux/WSL is the validated environment. Install Rust 1.95.0, Python 3.11+, a
C/C++ compiler, `pkg-config` and OpenSSL/libcap/ALSA development headers
(`libssl-dev`, `libcap-dev`, `libasound2-dev` on Ubuntu). Xenolith and its
model weights are installed separately.

```sh
git clone --branch main https://github.com/crrisci/codexe.git codexe
cd codexe
python3 scripts/install_codexe.py
```

This uses the existing package builder to compile the CLI, code-mode host, exec server and
matching bwrap, fetch verified ripgrep, and include
license notices. It installs the complete package below `~/.local/lib/codexe`
and exposes `~/.local/bin/codexe`. No manual symlink or helper layout is needed.

`--prefix /absolute/destination` or `PREFIX=/absolute/destination` selects another
root. `--package-dir /absolute/package` installs a previously assembled complete
package without Cargo or downloads. `--bin-dir /absolute/bin` selects the command
directory independently of the prefix. `--cargo-profile release` selects optimized
source builds; the default is `dev-small`. `just install-codexe` accepts the same
arguments. Repeating an installation reuses an identical package, or atomically
switches the command to a new package; previous versions remain for running
processes. Existing unrelated commands/directories and modified installed files
are not overwritten. Choose another prefix if an existing installation uses a
different installer. No configuration or conversations are migrated.

The Python command only builds and delegates to `scripts/install/install.sh`.
A prebuilt package needs no Python:

```sh
sh /absolute/package/install/install.sh --codexe --package-dir /absolute/package
# Or, with the installer from this checkout or another native package:
sh scripts/install/install.sh --codexe --package-archive /absolute/package.tar.gz
```

The explicit `--codexe` mode accepts only local packages, verifies their complete
checksum manifest and native capability metadata, then uses the existing shell
installer's lock and release activation. It stores releases below
`PREFIX/lib/codexe/standalone/releases`, with `current` selecting the active one,
and exposes `BIN_DIR/codexe` as an ordinary symlink to `current/bin/codex`.
It never downloads upstream Codex, edits profiles, runs an updater or launches a
client. Without `--codexe`, the shell script retains upstream Codex behavior.
The manifest checks package integrity; obtain the package and installer from a
trusted source and verify the distribution checksum before running them.

Before activating `codexe`, the installer probes the CLI, code-mode host,
exec server, bwrap and ripgrep. A missing helper runtime library fails without switching
an existing command. The checked Ubuntu 24.04 package needs glibc 2.39 and its
runtime packages `libssl3t64`, `libcap2`, `libtinfo6` and `libgcc-s1`; CA
certificates support HTTPS features. Audit each new package against its actual
helper dependencies; source builds use the host's ABI;
installing OS prerequisites remains an explicit user action.

For development without installation, `cd codex-rs` then
`cargo build -p codex-cli --bin codex` builds the CLI alone. The complete package
is required for its bundled tools. The optional offline session controller can
be built with `cargo build -p codex-xenolith-wire --bin xenolith-session`; its
artifact is `codex-rs/target/debug/xenolith-session`.

The tagged upstream workspace may regenerate package versions in Cargo.lock.
Current engine integration targets Linux/WSL; macOS and native Windows remain
unsupported for this installation path. A native GNU/Linux source build uses the
host's libraries and is not a portable release artifact for every distribution.

## Start and configure

On Linux/WSL, Xenolith is built in and selected when no provider is explicitly
configured. With a Xenolith build supporting native configuration, configure
it once, using the same file as Xenolith's own CLI and other native clients:

```sh
xenolith config init --model /absolute/path/to/model.gguf
codexe
```

For vision, add `--mmproj /absolute/path/to/mmproj-F16.gguf --ctx 65536` to
initial configuration. Keep the model, projector, context, socket, engine state
and cache settings in Xenolith's native configuration. CodeXe does not parse or
copy that file; it asks `xenolith config inspect --json` to resolve the endpoint
and operational paths. Inspection is bounded to five seconds and 64 KiB per
output stream and can be cancelled. Invalid configuration or discovery output
fails explicitly instead of silently selecting a different service.

The engine must be on `PATH`, or selected through `XENOLITH_BIN`. An optional
absolute `XENOLITH_CONFIG` selects a different engine configuration file. The
inspection and startup use the same selected file and resolved socket/state/cache
paths. Model parameters are loaded by the engine when it starts; changing a file
does not reconfigure an already running engine. The running service's `describe`
response remains authoritative for model and capabilities.

A pre-existing explicit provider endpoint connects directly, even if the engine
binary or its configuration is unavailable. Without an executable, CodeXe can
also connect to the conventional default socket when no custom config was
selected. Custom endpoints known only to engine configuration require a working
engine executable for discovery. Existing explicit provider, environment and
projector settings keep their precedence; CodeXe passes only explicit legacy
inference overrides on native-config startup. No configuration, binding or
conversation is migrated automatically. `--help` and configuration-only commands
do not perform engine discovery.

For older engine builds, explicit model-based startup remains available. The
known legacy CLI usage response permits that fallback; malformed configuration,
unsupported discovery schemas, and permission failures do not. The legacy command
from your coding workspace is:

```sh
XENOLITH_BIN=/absolute/path/to/xenolith \
XENOLITH_MODEL=/absolute/path/to/model.gguf \
codexe
```

Omit `XENOLITH_BIN` when `xenolith` is on `PATH`. Omit both variables when an
engine is already running on the default socket. The interactive native path
uses the fork's embedded server, so no `--no-daemon` flag or managed daemon
installation is needed. Explicit remote/shared-server workflows are separate
from this local path.

You can also connect to an engine you start yourself:

```sh
/path/to/xenolith serve /path/to/model.gguf --ctx 65536 \
  --socket /absolute/private/wire.sock \
  --state /absolute/private/engine-state \
  --cache /absolute/private/engine-cache
```

For that custom socket, start CodeXe with
`XENOLITH_SOCKET=/absolute/private/wire.sock`. Provider registration, model and
context discovery need no configuration file. Native sessions automatically
disable provider-hosted web search.

Invoking the fork as `codex` uses the normal Codex home (`~/.codex`). Installing
or symlinking that executable under the command name `codexe` selects `~/.codexe`
by default; an explicit `CODEX_HOME` wins for either name. The selected alias
home is inherited by helper and child processes, including re-executions under
another name. Native bindings default to `$CODEX_HOME/xenolith/bindings`; `XENOLITH_STATE_DIR` optionally
overrides that absolute path. Keep the home, bindings and engine state/cache to
resume conversations and restore KV across processes. Do not share a binding
directory between unrelated homes.

An optional separate home can keep CodeXe apart from another Codex installation:

```sh
export CODEX_HOME=/absolute/private/codex-home
mkdir -p "$CODEX_HOME"
```

For optional preferences, edit `$CODEX_HOME/config.toml`. For example, enable
reasoning and override the default socket:

```toml
model_reasoning_effort = "low"
show_raw_agent_reasoning = true

[model_providers.xenolith]
base_url = "unix:///absolute/private/wire.sock"
```

Then start the compiled CLI from your coding workspace:

```sh
codexe
```

Use the same home, binding directory and engine state/cache when resuming.
No API key is required for this provider. An explicit `model` must match the
served model. An explicit positive `model_context_window` may reserve a smaller
client budget but cannot exceed the service capacity. It does not resize the engine.
`model_auto_compact_token_limit`, if set, must fit that budget. A changed engine
context after discovery stops inference before session mutation; restart CodeXe
to rediscover the new capacity.

Explicit provider selection through configuration, profiles or CLI takes
precedence over the Xenolith default. Existing configurations that relied on
implicit OpenAI selection must add `model_provider = "openai"`, including those
that only specified an OpenAI model. The reserved built-in provider `xenolith`
always uses the native transport and no OpenAI authentication; socket and retry
settings can still be overridden. Other providers keep their own capabilities.

### Optional engine startup

Native engine configuration supplies startup settings automatically. The
following explicit environment overrides remain available for existing setups
and older engine builds:

```sh
export XENOLITH_BIN=/absolute/path/to/xenolith
export XENOLITH_MODEL=/absolute/path/to/model.gguf
export XENOLITH_SOCKET=/absolute/private/wire.sock
export XENOLITH_ENGINE_STATE_DIR=/absolute/private/engine-state
export XENOLITH_ENGINE_CACHE_DIR=/absolute/private/engine-cache
export XENOLITH_CONTEXT=65536
```

The provider's `base_url` takes precedence over `XENOLITH_SOCKET`; omit it to use
the environment socket. If neither is set, native configuration supplies the
socket; older engines use the runtime/state directory default. `XENOLITH_BIN`
defaults to `xenolith` on `PATH`. A model must be configured to start an absent
service, either in native configuration or through `XENOLITH_MODEL`.
Set `XENOLITH_NO_SPAWN=1` to require an already running service.

CodeXe always attempts connection first. On startup it acquires Xenolith's
global weight-owner lock and hands it to the child, preventing duplicate weight
owners even when another client uses a different socket. A successfully started
engine outlives CodeXe and remains available for resume or other clients;
`XENOLITH_IDLE_SHUTDOWN` overrides idle shutdown in minutes. Native startup uses
the engine configuration by default; legacy model-based startup defaults to 30.
An optional `XENOLITH_CACHE_BUDGET`, such as `4GiB`, is passed to the engine.
Startup failures include the engine's log tail. An unrelated engine is never
terminated. Engine state/cache variables are distinct from `XENOLITH_STATE_DIR`,
which continues to hold CodeXe conversation bindings.

### Images

Use CodeXe's image attachment controls, `codexe --image /path/image.png`,
or ask the agent to open a local image with its `view_image` tool. Vision is
advertised only when the connected engine reports valid v2 image capabilities.
No new attachment syntax or tool is introduced.

Keep the projector in native engine configuration to share it with other clients.
An optional CodeXe-specific projector override and image budget can be set in
`$CODEX_HOME/config.toml`:

```toml
[model_providers.xenolith]
mmproj = "/absolute/path/to/mmproj-F16.gguf"
image_budget = 280
```

Projector selection is `XENOLITH_MMPROJ` first, then the provider's `mmproj`, then
Xenolith's native configuration. Remove both CodeXe overrides to use the shared
projector. `XENOLITH_IMAGE_BUDGET` overrides the provider's image budget.
The projector override is passed as `--mmproj` only when CodeXe starts an engine;
an existing service remains authoritative. These are startup CLI arguments,
not wire requests that reconfigure a running service. Restart the engine to
enable a projector. Omit the image budget to use the service default. Current
budgets are 70, 140, 280, 560 and 1120;
vision requires a 65536 or 131072 engine context. A changed budget rebuilds the
affected visual history on the next request.

The adapter accepts inline PNG/JPEG bytes, with one image per user message or
tool result. Send additional images in separate turns. Text blocks retain their
order as the image caption. Multiple images, remote URLs, file IDs, unsupported
formats and oversized requests fail explicitly; no images are silently removed.
Codex retains its usual image preparation, resizing and permission checks.

Codex history retains the image bytes, so resume, fork and reconstruction do not
need the original file. Existing local compaction receives actual images and
replaces active context with its summary. The native binding records the image
budget; conservative replay after an interrupted operation also restores images.
The adapter sends inline bytes when needed and otherwise reuses its existing
incremental session path; it adds no separate media archive or download service.

The native status area shows `Preparing image` and elapsed time while Xenolith
prepares an image. This is not token progress or an encoder percentage. Escape
cancels the client operation and prevents generation; synchronous engine encoding
may still finish before the engine can process cancellation/disconnection. The
next request rebuilds uncertain native history from Codex's retained transcript.

### Reasoning and output

Set `model_reasoning_effort` to a service-supported level (`low`, `medium`,
`high`, `max`); omitted or `none` disables reasoning. Compatibility aliases
`minimal` and `xhigh` map to `low` and `high`, respectively, as in XenoPi.
Only advertised levels appear in the discovered model's reasoning choices.
Use `show_raw_agent_reasoning = true` to display the native reasoning channel
in interfaces that hide raw reasoning by default. It is stored separately from
assistant text and is not presented as a provider-generated summary.

| Variable                     | Contract                                                                                                                                                            |
| ---------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `XENOLITH_MAX_OUTPUT_TOKENS` | Optional positive output limit, no greater than `describe.max_output`. Omit to use the service limit; there is no fixed 4096-token cap.                             |
| `XENOLITH_REASONING_BUDGET`  | Optional nonnegative signed-32-bit reasoning token budget; requires enabled reasoning and advertised budget support. Reasoning shares the output budget.            |
| `XENOLITH_REASONING_HISTORY` | `discard` (default) or `preserve_tool_calls`, validated against advertised capabilities. This controls engine context projection, not deletion from CodeXe history. |

As specified by Xenolith, omitting a reasoning budget at unchanged effort retains
the session's existing engine override; changing effort resets it. Generation
does not reseed the sampler on every request. Output/reasoning capabilities are
revalidated before each inference. A response ending at its output limit is
terminal; CodeXe does not automatically retry the identical transcript. Incomplete
tool calls are never executed, and the next changed transcript rebuilds safely.

## Sessions and local telemetry

Bindings are durable, atomic and locked. The native path never uses ephemeral
requests or deletes sessions belonging to another client. It reopens unchanged
history, appends continuations and explicitly rebuilds after changed context or an
uncertain interrupted mutation. A divergent committed marker stops for investigation.
Reasoning can be recorded when the stream transitions to visible text, before
the engine commits the full generation. Recovery reconciles that partial history
by rebuilding; executable tool items are still published only after validated
completion and a durable binding write.

The TUI keeps native progress in a dedicated area above the composer, separate
from conversation text and reasoning. Startup shows connection/loading phases
and elapsed time, without inventing a model-loading percentage. During inference,
the area shows prefill completion, output tokens and measured throughput. Decode
completion changes the phase to finalization while the engine finishes and
persists its work. A successful response leaves a compact performance summary;
cancelled or failed work clears its active progress.

Rates use engine-reported tokens and active phase time. Missing measurements are
omitted rather than shown as zero or diagnostic placeholders. Checkpoint and
rewind reports never insert messages into the conversation. Full native reports
remain available through `codex_core::xenolith` debug logging. Only actual
`loaded: true` confirms snapshot restoration; an opening zero-prefill hint alone
does not. Snapshot persistence can require an explicit checkpoint because
autosave is rate-limited.

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
  node --test --test-concurrency=1 xenolith/tests/test-*.mjs
```

The CLI acceptance tests require Node >=22 and the rebuilt fork. They use isolated
homes and simulated services, exercising real tool execution, discovery, reasoning,
limits, compaction, optional startup and resume after an interrupted tool-result
append. They do not prove real KV restoration. GitHub CI runs wire/backend tests,
builds the CLI and executes these acceptance tests on Linux. It does not load a
real model or execute the full upstream suite.

For the opt-in real-model smoke, including live/restarted conversation resume
and a separate probe that verifies actual KV snapshot loading, see
[the smoke-test instructions](xenolith/tests/README.md).

## Current limits

The native integration targets Linux/WSL; macOS and native Windows are unsupported.
Compaction is verified manually; automatic saturation of the 65536-token context
remains untested.

Audio, multiple images per message, hosted/deferred tools, nested namespaces and
constrained JSON are rejected.
Function/custom tools with one namespace are supported. Native temporary requests are not
exposed. Crash recovery conservatively rebuilds KV and cannot guarantee uncommitted
streamed output. GGUF incompatibility remains untested.

## Keeping upstream syncs small

Wire translation, persistence and service lifecycle live in the isolated
`xenolith-wire` crate. Capability discovery uses Codex's existing custom model
catalog; native settings stay in `core/src/xenolith/`. Keep the configuration
resolution at inference runtime boundaries: generic config loading, help and
configuration-only commands must not start an engine. Keep those hooks and
provider dispatch small. Do not add native protocol fields to shared
Codex schemas unless the interface actually needs them. Interactive rewind and deferred tools remain future work.
