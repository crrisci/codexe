# Codex package builder

This package contains the implementation behind `scripts/build_codex_package.py`.
The top-level script is the stable executable entry point; these modules keep the
package-building logic split by responsibility.

Run the builder through `just`:

```bash
just assemble-codex-package --help
just assemble-codex-package --variant codex-app-server
just assemble-codex-package --target x86_64-unknown-linux-gnu
```

The builder creates a canonical Codex package directory:

```text
.
├── codex-package.json
├── bin
│   ├── <entrypoint>[.exe]
│   ├── codex-code-mode-host[.exe]
│   └── codex-exec-server[.exe]
├── codex-resources
│   ├── bwrap                             # Linux only
│   ├── codex-command-runner.exe          # Windows only
│   └── codex-windows-sandbox-setup.exe   # Windows only
└── codex-path
    └── rg[.exe]
```

The package directory is the primary artifact. Archive formats such as
`.tar.gz`, `.tar.zst`, and `.zip` are serializations of that directory.
The `execServer` field in `codex-package.json` advertises the packaged executor.
Installers use its presence to repair a missing executor while accepting older
packages that predate the standalone binary.

If `--target` is omitted, the builder uses the release target for the current
host platform. On Linux, that default is a musl target to match Codex release
artifacts; pass a GNU Linux target explicitly for native glibc local builds. If
`--package-dir` is omitted, the builder creates a new temporary directory and
prints its path after the package is built.

The `--variant` flag selects the package entrypoint. Supported variants are
`codex` and `codex-app-server`. The `--package-version` flag sets the version in
`codex-package.json`; it defaults to `[workspace.package].version` in
`codex-rs/Cargo.toml`.

## Source-built artifacts

Artifacts built from this repository are built by the package builder in one
grouped `cargo build` command per package when they are needed and no prebuilt
override was provided:

- all targets: the selected entrypoint, unless `--entrypoint-bin` is provided
- all targets: `codex-code-mode-host`, unless `--code-mode-host-bin` is provided
- all targets: `codex-exec-server`, unless `--exec-server-bin` is provided
- Linux targets: `bwrap`, unless `--bwrap-bin` is provided
- Windows targets: `codex-command-runner` and `codex-windows-sandbox-setup`,
  unless the corresponding prebuilt helper flags are provided

The default cargo profile is `dev-small` because local iteration should favor
fast, small builds. Release jobs should pass `--cargo-profile release` and an
explicit target. Release jobs that already built and signed/notarized the
entrypoint should pass `--entrypoint-bin` so the package contains that exact
binary instead of rebuilding it.

Release jobs should likewise pass `--code-mode-host-bin` and `--exec-server-bin`
so the package preserves the signed host and executor beside the entrypoint.

For Linux packages, maintainers can add `--strip` to remove unnecessary ELF
symbols from staged package copies with the system binutils command
`strip --strip-unneeded`. The builder checks the target and tool availability
before creating output or building. It stages the executable inputs, metadata,
notices, and install support, strips all staged ELF files (including bundled
helpers), and only then generates the native checksum manifest and archives.
Original prebuilt inputs and Cargo artifacts are preserved. A strip failure
aborts packaging without a successful native checksum manifest. Omitting the
flag preserves current behavior; this option is unavailable for non-Linux
packages. Never strip an already checksummed package in place: reassemble a new
package with `--strip` and the verified inputs instead. Prebuilt native CLIs still
require `--attest-xenolith-native-config`; stripping does not imply capability.

Release jobs that already built package resource binaries should also pass the
corresponding resource flags: `--bwrap-bin` for Linux packages, and
`--codex-command-runner-bin` plus `--codex-windows-sandbox-setup-bin` for
Windows packages. This keeps package archive creation as a pure staging step
after signing instead of rebuilding resources.

When the builder source-builds an entrypoint for a Darwin or Linux target, it
downloads and verifies the matching Codex-built V8 release pair before invoking
Cargo and sets `RUSTY_V8_ARCHIVE` plus `RUSTY_V8_SRC_BINDING_PATH` for that
build. Windows targets keep Cargo's release-build MSVC artifact path. Explicit
overrides remain authoritative when both variables are already set. Set
`V8_FROM_SOURCE=1` to leave the build with the `v8` crate source-build path.

`rg` is not built from this repository, so the builder fetches it from the
DotSlash manifest at `scripts/codex_package/rg`. Downloaded archives are cached
under `$TMPDIR/codex-package/<target>-rg` and are reused only after the recorded
size and SHA-256 digest have been verified. Pass `--rg-bin` to use a local
ripgrep executable instead.

## Install CodeXe from source

From the repository root, `python3 scripts/install_codexe.py` builds this complete
package for the native Linux GNU target and installs the `codexe` command under
`~/.local/bin`. `just install-codexe` invokes the same installer. Use `--prefix`
or `PREFIX` for another installation root, and `--package-dir` to reuse an already
assembled package. No shell startup or client configuration files are modified.
The installer preserves unrelated existing commands and keeps installed package
versions separate so updating the command does not modify a running executable.

The builder includes project and bundled-helper license notices. Helper notices
are checked against `notices/pins.json`; when changing dependency versions,
review and update their notices together. See `notices/README.md` for provenance.

Native Linux CodeXe source builds add `xenolith.nativeConfig: 1` to the canonical
`codex-package.json`, bundle `install/install.sh` and `install/codexe-local.sh`,
and create `codexe-package_SHA256SUMS` covering all regular package files. The
shell installer consumes this canonical schema (field order and names included),
checks its complete file set and hashes, and probes all five executables before
activation. End users can run `sh PACKAGE/install/install.sh --codexe
--package-dir PACKAGE` without Python, Cargo or network access. A local `.tar.gz`
archive can instead be supplied through `--package-archive`; `--prefix` and
`--bin-dir` control storage and the ordinary `codexe` command separately.

When supplying `--entrypoint-bin`, native capability is **not** inferred from
its version or name. Maintainers must pass `--attest-xenolith-native-config` only
for a binary verified to contain this fork's native configuration integration.
The option requires native integration in the builder checkout and a Linux
Codex package. Keep the verified binary hashes with build evidence. Archives
with no native marker remain ordinary upstream-compatible packages and cannot
be installed with `--codexe`. The Python source-install helper delegates all
validation, locking, collision handling and activation to the same shell path.
