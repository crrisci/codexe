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
