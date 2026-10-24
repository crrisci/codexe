# Next steps after Panther Lake validation

Updated 2026-10-24. The bridge feedback/KV roadmap and CodeXe sandbox/network
fixes are published on their respective `main` branches. This checklist keeps
the remaining work separate from those completed checks.

| # | Work | Status | Completion criteria |
|---|---|---|---|
| 1 | Close CodeXe CI for the published changes | Deferred (tracked) | Review blocking, postmerge and Xenolith native jobs; resolve actual failures and distinguish Linux/PTL evidence from Windows/Bazel/container checks. |
| 2 | Consolidate real text/vision acceptance | Completed (PTL, 2026-10-24) | Keep real-weight tests opt-in; verify actual image bytes, successful `view_image`, exact fixture answers, live/restarted conversation resume, owned-process cleanup and reports identifying source/binaries. |
| 3 | Extend real recovery acceptance | Supported paths validated; encoder cancellation pending | Cancellation during image encoding, crash with a pending image tool, rewind/fork and image-budget changes recover explicitly without duplicate effects. Demonstrate native KV loads separately from conversation resume. |
| 4 | Qualify Panther Lake in the engine | Pending | Record PCI/device, driver, engine/model and repeatable measurements; prepare minimal upstream evidence and evaluate XMX only in the engine project. Do not infer capabilities in clients or publish upstream reports automatically. |
| 5 | Prepare bridge 0.4.0-rc.2 | Pending | Package the published bridge fixes, verify archive/checksum and clean installation/API acceptance, review CI and release notes; keep persistent bindings experimental. |

Item **2 is complete**. Current focus: **3**; item 1 is deferred at the
owner's request, with its known failures retained. Items 4 and 5 remain saved
for subsequent work. Completion of item 2 does not close platform CI, failure
recovery or hardware qualification. Bridge release work belongs to
[crrisci/xenolith-bridge](https://github.com/crrisci/xenolith-bridge); hardware
qualification belongs to the engine rather than CodeXe or the bridge.

## Acceptance scope for item 2

The existing `xenolith/tests/model-smoke.mjs` covers a real shell read, reasoning,
conversation resume and an independent native snapshot probe. The vision smoke
must add direct attachment, live resume, resume after engine restart and a
fresh conversation that calls `view_image` on a different fixture. A correct
color word alone does not establish that image bytes or the tool were used.

Fixture/trace/report data stay in ignored `.test-artifacts/` directories. Real
weights are never downloaded by CI or committed. Passing simple color fixtures
establishes this integration path, not general image understanding. Failure and
interruption reports must remain inspectable; cleanup may signal only processes
owned by the test. Ordinary CI continues to use simulators.


## Item 2 validation record

Implementation tested: `6f5bfb7b1d5d0c088a262d1cc659c798d9bfc091`.
On native Linux/Panther Lake with Node 22.23.4, engine
`1e7025331fdb6bff4d9411e72bc526a86d73b0a0`, Gemma-4-26B-A4B-it-qat
UD-Q4_K_XL and its F16 projector:

- Real vision: **4/4**, actual PNG attachment, live/restarted resume and a fresh
  `view_image` conversation with different fixture bytes/color and matching tool ID.
- Real text smoke: **passed**, successful shell nonce read, raw reasoning,
  live/restarted resume and separate native KV snapshot probe.
- Native simulator/harness suite: **48/48**, no skips on Linux. Mac harness
  checks: 7 passed, one deliberately Linux-only preflight check skipped.
- Reports retain source/binary/model/projector hashes, wire observations and
  owned-process cleanup. Raw reports/fixtures stay private and ignored.

The independent probes report actual snapshot loading. In this vision run the
restarted CLI's own wire terminal also reported `resume.loaded:true` with 9801
cached tokens; this is a recorded observation for this configuration, not a
guarantee inferred from conversation resume. The initial configuration error
(disabled reasoning plus an explicit reasoning budget) is retained in private
failure evidence; the harness now removes inherited reasoning knobs for its
explicit no-reasoning vision calls. No production Rust changes were needed.


## Item 1 progress

CI logs identify separate formatting, runner/Fence, Windows compiler-wrapper,
resource/runfiles and remote-test failures. The formatting corrections use the
locked Prettier version. Fence stays in audit mode with a verified complete
Action pin including upstream resolver-layout fixes. Windows wrapper flags use
response files independently of compiler flags; upstream wrapper tests remain
in the Bazel matrix alongside the unchanged product target set.

The HTTP-client follow-up isolates native-TLS fallback fixtures in subprocesses
without inherited CA overrides: Cargo can populate `SSL_CERT_FILE` after the
workflow unsets it. It also fixes typed TLS failure classification through
nested I/O wrappers and adds a real Rustls untrusted-certificate handshake
regression. Native Linux/PTL validation passes all 113 library and 10 custom-CA
integration tests; the seven fallback fixtures also pass with deliberately
invalid inherited CA paths. These checks do not establish hosted CI success.

The Windows setup-manifest fixture now resolves its helper through the existing
Cargo/Bazel runfiles-aware locator rather than treating Bazel's logical runfile
name as a filesystem path. The resource assertions remain unchanged; this fix
still requires execution in the hosted Windows matrix.

The dependency gate additionally reports RUSTSEC-2026-0387 in crypto-bigint
0.5.5. The patched 0.7.6 line is not semver-compatible with the current
elliptic-curve/p256 dependencies reached through secrets, AWS credentials,
user verification and RTC. Item 1 remains open until compatible coordinated
upstream updates and the platform/test failures are resolved. No vulnerability
ignore, product-feature removal or failed-test suppression is used to close it.


## Item 3 validation record

Opt-in `vision-smoke.mjs --recovery` tested on native Linux/Panther Lake with
the same engine/model/projector as item 2. All **five recovery probes passed**:

- Offline rewind retains the first image and native session while discarding
  later turns, with the tool's immutable rollout backup.
- Changing and restoring the image budget explicitly rebuilds retained media
  in the same session; byte hashes and exact fixture answers match.
- Killing the owned CLI after the engine accepts a tool image, while losing
  the ACK, resumes from the real rollout with one retained tool result and no
  repeated `view_image` call.
- Interrupting the client before the image reply reconciles its in-flight
  binding and recovers the retained image. A tagged PNG fixture avoids reusing
  identical media bytes from an earlier probe.
- A real app-server fork replays the original image into a distinct native
  session, accepts a different image in the fork, and preserves the parent
  conversation without the fork's image bytes.

The run completed **10/10 CLI vision cases**, both app-server turns and the
independent post-restart native KV load probe. Focused harness checks passed
**13/13** on Linux, without weights; the Mac run passed 11 and skipped the two
Linux-only preflight cases. Reports include source/binary hashes, fault traces,
fixture bytes and owned-process cleanup. An initial harness failure incorrectly
classified a user-message notification as a tool effect; its evidence is kept
and the corrected classifier has regression coverage.

Item 3 remains partial for **cancellation during image encoding**. The current
engine's protocol v2 prepares images synchronously before replying and exposes
no observable cancellable encoder boundary. Client interruption before an ACK
must not be claimed as encoder cancellation. Closing that criterion requires
engine support, followed by a targeted real acceptance case. No engine change
or upstream report was published as part of this work. The CI failures in item 1
remain deferred; they do not block using these opt-in recovery checks.
