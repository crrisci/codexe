# Next steps after Panther Lake validation

Updated 2026-10-24. The bridge feedback/KV roadmap and CodeXe sandbox/network
fixes are published on their respective `main` branches. This checklist keeps
the remaining work separate from those completed checks.

| # | Work | Status | Completion criteria |
|---|---|---|---|
| 1 | Close CodeXe CI for the published changes | Pending | Review blocking, postmerge and Xenolith native jobs; resolve actual failures and distinguish Linux/PTL evidence from Windows/Bazel/container checks. |
| 2 | Consolidate real text/vision acceptance | In progress | Keep real-weight tests opt-in; verify actual image bytes, successful `view_image`, exact fixture answers, live/restarted conversation resume, owned-process cleanup and reports identifying source/binaries. |
| 3 | Extend real recovery acceptance | Pending | Cancellation during image encoding, crash with a pending image tool, rewind/fork and image-budget changes recover explicitly without duplicate effects. Demonstrate native KV loads separately from conversation resume. |
| 4 | Qualify Panther Lake in the engine | Pending | Record PCI/device, driver, engine/model and repeatable measurements; prepare minimal upstream evidence and evaluate XMX only in the engine project. Do not infer capabilities in clients or publish upstream reports automatically. |
| 5 | Prepare bridge 0.4.0-rc.2 | Pending | Package the published bridge fixes, verify archive/checksum and clean installation/API acceptance, review CI and release notes; keep persistent bindings experimental. |

Current focus: **2**. Finish it before resuming the other development items.
CI review (1) can proceed independently. Bridge release work belongs to
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
