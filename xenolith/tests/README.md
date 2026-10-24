# Linux test setup

For repeatable Linux/WSL checks, install the Rust toolchain pinned in
`codex-rs/rust-toolchain.toml`, Node >=22, Python 3, curl, a C/C++ compiler,
pkg-config, bubblewrap, and the OpenSSL, libcap and ALSA development packages.
On Ubuntu these development packages are `libssl-dev`, `libcap-dev` and
`libasound2-dev`. Existing private toolchains or a pkg-config sysroot can be
used; the script does not install system packages or change shell profiles.

From the fork root:

```sh
bash xenolith/scripts/test-linux.sh setup
bash xenolith/scripts/test-linux.sh native
bash xenolith/scripts/test-linux.sh network
```

`setup` prepares the test environment and builds the actual CLI,
`codex-linux-sandbox` and `codex-code-mode-host`. `native` additionally runs
wire, provider/config, native TUI, plugin startup and CLI simulator checks.
`network` runs Linux sandbox unit tests and the core network-approval tests.
These modes do not load a real model or run the entire upstream suite.

The script sets at least 16 MiB of Rust test-thread stack, as in CI, and
downloads the Codex-built V8 archive and bindings. It first checks the release
manifest against the repository's pinned manifest hash, then verifies both
artifact hashes. The default Deno V8 URL may not publish the required variant.
The verified cache is private and versioned under
`.test-artifacts/linux-tests/v8/`; set `XENOLITH_TEST_CACHE_DIR` to use another
cache. Completed downloads are reverified on every run.

For additional Cargo tests, use the generated environment:

```sh
source .test-artifacts/linux-tests/env.sh
cargo test --manifest-path codex-rs/Cargo.toml -p codex-core --lib
```

The private environment file retains the selected toolchain paths and any
provided pkg-config/sysroot flags, so it can also be sourced in a fresh shell.

Building `codex-code-mode-host` is required by hook fixtures that execute code
mode. A missing helper can make those tests fail even when runtime unit tests
pass. Network fixtures select their shell explicitly; their approval metadata
must describe that same executable. A denial must release the process lock
without waiting for test teardown.

# Explicit real-model smoke

The ordinary `test-*.mjs` acceptance suites use simulators. `model-smoke.mjs`
loads real weights and is deliberately excluded from that wildcard and CI.
Run it only while the machine has no Xenolith weight owner:

```sh
XENOLITH_BIN=/absolute/path/to/xenolith \
XENOLITH_MODEL=/absolute/path/to/model.gguf \
XENOCODEX_NATIVE_CODEX_BIN="$PWD/codex-rs/target/debug/codex" \
  node xenolith/tests/model-smoke.mjs
```

Linux/WSL, Node >=22, Python 3, `pgrep`, a working Codex shell sandbox and a
rebuilt native CLI are required. The smoke uses built-in native provider/web
defaults and ordinary CLI invocation; only its intended reasoning settings are
passed as configuration overrides. The script refuses an existing engine or held
shared engine lock, requires at least 21 GiB `MemAvailable`, and checks free disk
against the KV cache budget plus 1 GiB. Context defaults to 16384 tokens and cache
budget to `4GiB`; explicit `XENOLITH_CONTEXT` and `XENOLITH_CACHE_BUDGET` override
those values. CodeXe auto-starts the real engine with isolated engine state,
cache, Codex home and bindings; reasoning uses low effort, budget 128,
`preserve_tool_calls`, and a 1024-token output limit.

The smoke requires an actual successful `exec_command` read of a random file,
its exact contents as the final answer, nonempty raw reasoning in the rollout,
and the same conversation resumed by a second CLI process. It stops only the
engine identified by the supplied executable, its unique private socket and
process start time, then resumes the thread through another automatic startup.

CLI telemetry does not persist a structured `done.resume` report. To verify
actual KV loading, the smoke separately creates and checkpoints a small native
probe session before the engine restart, then requires that probe's generation
to report `done.resume.loaded: true` afterward. It never directly mutates
CodeXe's bound session. This proves snapshot restoration for the separate probe;
it does not claim a specific cache load for the CLI conversation. Missing or
rejected checkpoints or loads fail explicitly with restoration unverified.

Logs, probe frames, configuration, available resources, repository/protocol
identities and a final `report.json` remain in private
`.test-artifacts/model-smoke-*` directories, including after failure. Cleanup
signals only the owned engine. Each CLI invocation has a ten-minute timeout;
Ctrl-C or SIGTERM interrupts it and proceeds to owned-engine cleanup. SIGKILL
cannot run cleanup; the report's private socket identifies the test owner.

The simulator startup suite also exercises fresh configuration with no provider
flags and ordinary `codex exec`: automatic startup via explicit `XENOLITH_BIN`,
fallback to a `xenolith` executable on `PATH`, and connection to an existing
engine without `XENOLITH_MODEL`. These cases omit manual model/context and web
settings or `XENOLITH_STATE_DIR`, so the public defaults must discover the
engine and persist bindings under `CODEX_HOME/xenolith/bindings`. A PTY case
opens the ordinary interactive TUI, waits for its discovered model to render,
and exits through keyboard interrupts while checking that no managed daemon
package or PID file was created. Relative binding overrides are rejected before
inference.

A second PTY case starts with an empty home and a delayed engine. It requires
the startup indicator to appear before engine readiness and Ctrl-C to exit
promptly during that wait. Focused Rust TUI tests (`cargo test -p codex-tui
--lib xenolith`, from `codex-rs`) exercise the dedicated progress row, stale
event filtering and lifecycle cleanup, plus actual transcript consolidation
across native telemetry and terminal resize without duplicating the answer.


# Explicit real-vision smoke

`vision-smoke.mjs` promotes the private Panther Lake experiment into an opt-in,
portable Linux/WSL driver. Like `model-smoke.mjs`, it is excluded from
`test-*.mjs` and ordinary CI; it never downloads weights. See
[the remaining roadmap](../ROADMAP.md) for subsequent recovery and hardware work.

From the fork root, with no Xenolith weight owner:

```sh
XENOLITH_BIN=/absolute/path/to/xenolith \
XENOLITH_MODEL=/absolute/path/to/model.gguf \
XENOLITH_MMPROJ=/absolute/path/to/matching-projector.gguf \
XENOCODEX_NATIVE_CODEX_BIN="$PWD/codex-rs/target/debug/codex" \
  node xenolith/tests/vision-smoke.mjs
```

Build the native binaries first with `bash xenolith/scripts/test-linux.sh setup`.
Node >=22, Python 3, `pgrep`, Linux/WSL and matching engine/model/projector files
are required. The driver requires 21 GiB available RAM and 5 GiB free artifact
disk, refuses an existing engine/held shared weight lock, and launches one owned
engine with a 65536-token context and a 4 GiB cache budget. All four paths must
be explicit absolute paths; no private machine paths are embedded in the script.

The four required cases are:

1. Attach a generated PNG, require its exact color answer and observe the actual
   PNG bytes and advertised image budget in native `append`/`rebuild` requests.
2. Resume the same conversation from another CLI process, with unchanged native
   binding and a newer committed epoch.
3. Stop the owned engine, start a different process with the same private state,
   then resume that same conversation and observe the native session reopen.
4. Start a fresh conversation in a separate Codex home/binding directory, ask
   `view_image` to inspect a different-colored PNG, and require exactly one
   actual tool call plus image bytes returned under its matching native call ID.
   No attachment seeds this conversation and no other tools may execute.

A transparent Unix-socket observer forwards bytes unchanged and retains actual
requests/responses in private `wire.jsonl`. It verifies payload hashes, not just
a correct answer or a `view_image` name found in older conversation history.
Stdin is closed; the `--` separator prevents variadic `--image` parsing from
swallowing the prompt. Image filenames/prompts do not disclose the chosen color.
These small color fixtures establish integration, not general visual quality.

A separate native session is checkpointed before the restart. Its generation
after restart must report `resume.loaded:true` and positive native cached-token
accounting. The report distinguishes this independent probe from the CLI image
conversation: conversation resume alone is not evidence of a specific KV load.
CLI terminal usage/resume fields are retained without converting a valid
history rebuild into a cache-restoration claim.

Expect four `VISION_PASS` lines and `VISION_SMOKE_PASSED 4/4`. Missing vision
capabilities, wrong answers, missing actual media/tool output, failed snapshots,
nonzero CLI exits, timeouts or interruptions fail the run. Each CLI invocation
keeps the existing ten-minute budget. Interruptions terminate its owned process
group and clean up the exact spawned engine, never another user's engine.

Private `.test-artifacts/vision-smoke-*` reports include source revisions/dirty
state, runner/helper/model/projector/CLI/engine SHA256 hashes, fixture hashes,
capabilities, resource checks, native frames, CLI logs and cleanup outcomes.
Supplied binary hashes identify what ran; a repository revision by itself does
not certify how an externally supplied executable was built. Engine logs and
reports persist on failure. Private temporary socket directories are retained
for ownership diagnosis; no model or session state is committed.

The default CI wildcard runs `test-vision-harness.mjs` without weights to check
fixture pixels, closed stdin and image argument boundaries, UTF-8 framing,
byte-preserving observation, and rejection of missing/wrong media, unrelated
sessions/call IDs or extra tools. It is harness coverage, not real inference.
