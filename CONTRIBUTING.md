# Contributing to CodeXe

CodeXe is maintained independently of OpenAI and Intel. Use the
[CodeXe issue tracker](https://github.com/crrisci/codexe/issues) for bugs,
feature requests and design discussions about this fork.

## Development branch

`main` is the development branch and the target for pull requests.
See [UPSTREAM.md](UPSTREAM.md) for upstream synchronization and CI policy.

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

Use Conventional Commits for new downstream commits and PR titles: a lowercase
type, optional scope and concise English imperative description. Types are
`feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `build`, `ci` and `chore`.
Mark incompatible changes with `!` and explain migration. Keep a body when it
helps explain motivation. Imported upstream history and generated merge/revert
messages are exempt; preserve authorship and existing attribution trailers.

Require relevant checks before ordinary merges. Request additional review when useful.
Use merge commits by default; agree any feature squash with its contributors.
Never squash upstream sync ancestry. Use temporary `sync/codex-YYYY-MM-DD`
branches for upstream integrations; merge only after validating the native delta.
A merge does not publish CodeXe. Select and qualify exact artifacts before a
separate publication decision; see [UPSTREAM.md](UPSTREAM.md) for versioning and
release requirements. For inclusion in the Xenolith installer, see its
[distribution guide](https://github.com/simoneiacomino/xenolith/blob/main/installer/RELEASING.md).

## Licensing and attribution

CodeXe's project license is [Apache-2.0](LICENSE). Preserve existing copyright,
license and third-party notices when modifying or incorporating code. See the
[license overview](docs/license.md) for the distinction between the project
license and dependency licenses.

The OpenAI contribution policy and CLA retained as historical documentation
are not CodeXe's contribution policy. CodeXe does not ask contributors to sign
OpenAI's CLA for contributions to this fork.

Be respectful, and keep credentials and private data out of issues and patches.
