# Standalone Linux container build

CodeXe owns its Rust toolchain and package build dependencies, including ALSA.
The container uses Ubuntu 24.04 and Rust 1.95.0; it contains no Xenolith, Intel
GPU compiler or inference dependencies. It calls the canonical
`scripts/build_codex_package.py`, preserving helpers, licenses and package
checksums. Docker is a local maintainer prerequisite, never an installer action.

From a selected CodeXe checkout:

```sh
python3 scripts/build_linux.py --build-image \
  --cargo-profile release --package-version 0.1.0-rc.1 \
  --output /absolute/output/codexe-build
```

Choose the package version being qualified, independently of the engine and
XenoPi. Output must be empty and outside the source checkout. `--source` selects
another checkout; `--image` reuses an already built environment. The default
`dev` profile is intended for local iteration; select `release` for candidates.
The version is derived inside a disposable source snapshot and workspace lockfile,
using the same transformation as release CI. The tracked upstream development
Cargo version is preserved. The embedded CLI and package versions are built
together; a version label never relabels an existing binary.

The container mounts only the output directory, which holds the source snapshot,
manifest, build script, image identity/provenance, canonical `artifact/` package
and `codexe-v<VERSION>-linux-x86_64-unknown-linux-gnu.tar.gz`, plus archive
`SHA256SUMS` and final lockfile/archive hashes in build provenance. It does not mount
your home or Docker socket. Network access is needed to acquire build dependencies;
no engine/model is started. Image identity and source hashes are recorded before
the build so a failure remains inspectable. A mutable Rust image tag and apt
repositories can change: retain the recorded image ID for an identical environment.
This recipe does not claim bitwise reproducibility across dependency mirrors.

Verify a completed package's offline activation using its canonical installer:

```sh
sh /absolute/output/codexe-build/artifact/install/install.sh --codexe \
  --package-archive /absolute/output/codexe-build/codexe-v0.1.0-rc.1-linux-x86_64-unknown-linux-gnu.tar.gz \
  --prefix /absolute/smoke/prefix --bin-dir /absolute/smoke/bin
/absolute/smoke/bin/codexe --version
```

These checks need no model. For fixture tests without Docker or Rust compilation:

```sh
python3 -m unittest discover -s scripts -p 'test_build_linux.py'
python3 -m unittest discover -s .github/scripts -p 'test_verify_codexe_candidate.py'
```

The [manual release workflow](../UPSTREAM.md) remains the candidate/publication
entrypoint. It verifies offline installation and the embedded version, records
checksums/provenance, then promotes only an explicitly approved existing artifact.
Publication creates a draft, attaches all assets, and publishes only after every
upload succeeds, supporting GitHub immutable releases. A failed upload leaves a
draft for inspection; no step overwrites an existing release/tag or asset.
The exact official tag namespace remains `codexe-v<VERSION>`, for example
`https://github.com/crrisci/codexe/releases/download/codexe-v0.1.0-rc.1/codexe-v0.1.0-rc.1-linux-x86_64-unknown-linux-gnu.tar.gz`.
Real-engine qualification and maintainer approval are separate from package tests.
