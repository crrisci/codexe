# Maintaining CodeXe alongside Codex

CodeXe follows [OpenAI Codex](https://github.com/openai/codex). The exact upstream
revision used by this integration is recorded in [upstream-base.txt](upstream-base.txt).
Git ancestry and that file must describe code that was actually integrated.

To inspect the downstream source changes:

```sh
git diff "$(cat upstream-base.txt)" HEAD
```

## Keep the downstream changes small

Keep the native transport in `codex-rs/xenolith-wire`, with narrow integration
points in provider selection, configuration, inference events and the TUI. Keep
the canonical upstream builders and generators. Extend them for CodeXe rather
than maintaining a second build system or a copied generated protocol.

Preserve explicit provider selections, upstream authentication and the existing
sandbox/approval behavior. Linux and WSL currently support native Xenolith;
Windows compilation is a maintenance target, while native Windows inference is
future work. A Windows CLI build does not qualify a Windows engine or installer.

The two CodeXe entrypoints are `codexe-ci.yml` and `codexe-release.yml`.
CI contains Linux native acceptance, Windows CLI compatibility, dependency checks
and a required summary job. The upstream
`blocking-ci` and `postmerge-ci` entrypoints retain their definitions but run only
in `openai/codex`. Other upstream recipes remain available for targeted/manual
checks. V8 canary keeps its own change detector. Do not copy old runner fixes or
restore retired dependencies without checking the new upstream implementation.

Do not publish upstream `rust-v*`, `python-v*`, `rusty-v8-v*` or `codex-zsh-v*`
tags as CodeXe releases. Those namespaces belong to inherited release recipes.
The manual release workflow takes a package version and builds a Linux GNU
archive, checksums and provenance, including an offline installation check.
With `publish=false` it builds and uploads review artifacts only. After CI and
real-engine qualification, run the same workflow again at the same source
revision with `publish=true`, the original `candidate_run_id`, the same
`package_version` and the explicitly approved `approved_archive_sha256`.
The publisher downloads that run's artifact without rebuilding. It checks the
successful run's repository, workflow and source SHA, candidate provenance,
version and archive digest before creating a `codexe-v<VERSION>` tag and GitHub
release. It creates an unpublished draft, uploads all selected files, then
publishes the draft so immutable releases receive a complete asset set. Failed
uploads leave a draft; publication never overwrites files or existing tags.
Existing tags or releases are rejected. Prerelease versions are marked as such, and releases are not automatically promoted to latest. The publishing
job alone has `contents: write`. Publication requires a dispatch on `main` or a temporary
`release/*` branch and the `release` environment. Configure that environment
to allow only those branches and require a maintainer's approval before using
the publisher. No push or tag event
publishes a CodeXe release.

The approved digest must identify the archive actually qualified; a checksum
check alone is not model acceptance. Retain the candidate while its Actions
artifact is available. An expired or replaced candidate must be rebuilt and
qualified as a new artifact. This workflow does not configure required reviews
or claim that remote CI or publication has already been executed.

Run CI and the real-engine acceptance described below on the selected revision
before publishing. The recipe currently distributes Linux GNU only; Windows
compilation in CI is preparation for future Windows support.

## Regular synchronization

Keep the previous qualified package available while validating an upstream update.

Configure a read-only source remote once if it is missing:

```sh
git remote add upstream https://github.com/openai/codex.git
git fetch --no-tags upstream main
```

Start with a clean working tree. Fix the target SHA before integrating:

```sh
git switch main
git switch -c sync/codex-YYYY-MM-DD
git merge --no-ff upstream/main
```

Resolve changes against the current native behavior and the new upstream APIs.
Do not choose an entire old file merely to remove conflict markers. Never use
an `ours` merge to record upstream as integrated without incorporating its code.
Do not squash upstream history: ordinary ancestry makes the next merge smaller.
Review `git diff upstream/main` as the intentional downstream delta.

Update `upstream-base.txt` to the integrated SHA, regenerate config/app-server
schemas with the canonical commands in `justfile`, and run:

- Linux native wire/provider/configuration/CLI/TUI simulator tests.
- Package builder, offline installer and source installation helper tests.
- Windows CLI compilation and provider tests; investigate shared-code regressions.
- Dependency/advisory checks and upstream tests for the areas changed.
- V8 canary when V8, code mode or their build inputs change.

Before selecting a release package, build that exact committed revision and test
native startup, tools, images and persisted conversation resume against a stated
engine/model/device. A simulator pass does not replace real inference acceptance.
Record unresolved failures and supported platforms. Merge the tested sync branch
into local `main`; publishing it is a separate operation.

For an isolated local build of the same canonical package, see
[the Linux container recipe](packaging/README.md). This repository owns the Rust
toolchain and ALSA build dependencies; Xenolith consumes the completed archive.

## Component releases

CodeXe, Xenolith and XenoPi have independent versions. The Xenolith distribution
manifest selects exact packages and checksums; runtime protocol/capabilities
decide compatibility. A CodeXe sync does not require a new engine release.
Keep source revision, package version, artifact checksum and validation results
together. Do not use the upstream development Cargo version as a unique CodeXe
release identity.

The release workflow derives `[workspace.package].version` only in its temporary
build checkout. It records that transformation and the final lockfile hash,
checks that CLI and package versions agree, and tests offline activation. The
tracked development workspace keeps the upstream version to avoid recurring
version-only merges. A metadata label alone does not change a prebuilt CLI's
embedded version; native installation rejects mismatched versions.
