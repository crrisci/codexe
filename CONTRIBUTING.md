# Contributing to CodeXe

CodeXe is maintained independently of OpenAI and Intel. Use the
[CodeXe issue tracker](https://github.com/crrisci/codexe/issues) for bugs,
feature requests and design discussions about this fork.

## Development branch

`main` is the development branch and the target for pull requests. It contains
the native Xenolith integration, including the vision work from PR #2.
The former `xenolith-native` branch has been consolidated into `main`.

## Reporting bugs

Search existing issues first. Include reproduction steps, expected and actual
behavior, the commit you built, operating system, and relevant logs with secrets
removed. For native inference issues, include the Xenolith version and model
configuration. The current engine integration targets Linux and WSL.

## Proposing changes

Open an issue first for substantial changes. Keep pull requests focused, explain
the behavior they change, and describe the checks you actually ran. Target
`main` and follow the surrounding code's conventions. See the
[setup guide](XENOLITH.md) and [native test guide](xenolith/tests/README.md).
Do not remove tests or relax assertions just to make CI green.

## Licensing and attribution

CodeXe's project license is [Apache-2.0](LICENSE). Preserve existing copyright,
license and third-party notices when modifying or incorporating code. See the
[license overview](docs/license.md) for the distinction between the project
license and dependency licenses.

The OpenAI contribution policy and CLA retained as historical documentation
are not CodeXe's contribution policy. CodeXe does not ask contributors to sign
OpenAI's CLA for contributions to this fork.

Be respectful, and keep credentials and private data out of issues and patches.
