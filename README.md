# CodeXe

CodeXe is a community fork of OpenAI Codex with native local
[Xenolith](https://github.com/simoneiacomino/xenolith) inference.
It preserves Codex's agent loop, tools, sandbox and approvals, while adding
local inference status and durable KV sessions.

## Quick start

On Linux x86_64:

```sh
curl -fsSL https://xenolith.it/install.sh | sh -s -- --interface codexe
```

The installer sets up CodeXe and Xenolith, guides you through model configuration
or reuses your existing setup, and starts CodeXe.

For later sessions, run this from your project directory:

```sh
codexe
```

## Build from source

On Linux/WSL, install **Rust 1.95.0**, **Python 3.11+**, a C/C++ compiler,
`pkg-config`, and OpenSSL, libcap and ALSA development headers
(`libssl-dev`, `libcap-dev`, `libasound2-dev` on Ubuntu).
Build [Xenolith](https://github.com/simoneiacomino/xenolith#build-from-source)
and obtain a supported model, then:

```sh
git clone https://github.com/crrisci/codexe.git
cd codexe
python3 scripts/install_codexe.py
xenolith config init --model /absolute/path/to/model.gguf
codexe
```

Skip `config init` if Xenolith is already configured. The engine must be on
`PATH`, or selected with `XENOLITH_BIN=/absolute/path/to/xenolith`.
The source installer builds the complete package and installs `codexe` in
`~/.local/bin`. See [build and install](XENOLITH.md#build-and-install) for
custom destinations, prebuilt local packages and runtime dependencies, or the
[container build guide](packaging/README.md) for standalone Linux archives.

## Native inference

- **Live status:** engine startup, prefill progress, generation speed and cache reuse.
- **Service discovery:** model, context, output limit and reasoning capabilities.
- **Vision:** image attachments and `view_image` with a configured projector.
- **Reasoning:** streamed thoughts and history across tools and resumed sessions.
- **Durable sessions:** checkpoints, KV restoration and interrupted-turn recovery.
- **Rewind:** an offline controller previews the context rebuild cost before applying it.

Xenolith is the default provider on Linux/WSL. Explicit provider selections take
precedence. CodeXe uses `~/.codexe` for configuration and conversations;
`CODEX_HOME` can select another home.

The native integration supports PNG/JPEG images, one per user message or tool
result. See [current limits](XENOLITH.md#current-limits) for tool compatibility,
context recovery and platform restrictions.

- [Engine configuration, images and reasoning](XENOLITH.md#start-and-configure)
- [Sessions and inference status](XENOLITH.md#sessions-and-local-telemetry)
- [Offline rewind](XENOLITH.md#offline-rewind)
- [Contributing](CONTRIBUTING.md) · [Upstream maintenance](UPSTREAM.md)

## License and attribution

CodeXe is licensed under [Apache-2.0](LICENSE), maintained independently of
OpenAI and unaffiliated with Intel. [NOTICE](NOTICE) preserves OpenAI and
third-party and CodeXe attributions; see the [license overview](docs/license.md).
The [upstream README](README-UPSTREAM.md) documents the integrated OpenAI Codex revision.

Project website: [xenolith.it](https://xenolith.it).
