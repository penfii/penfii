# Contributing to Penfii

Thank you for helping. This guide is short on purpose; if something is unclear, open an issue and ask.

Please read the [Code of Conduct](CODE_OF_CONDUCT.md). **Security problems are reported privately, see [SECURITY.md](SECURITY.md); do not open a public issue for them.**

## Before you start

- For a bug, search the existing issues first and use the bug report form; include the steps to reproduce it. For the web app, include the browser and the signed-in role.
- For a new feature or a larger change, open an issue and describe the problem first. Penfii is small and opinionated; a discussion saves you from writing code that cannot be merged.
- The project is at **v0.1.0**, an early release. Behavior, the HTTP API and the database schema can change; say so in your pull request if you change one of them.

## Set up

Everything you need is in [docs/development.md](docs/development.md): prerequisites, running the server and the web app from source, and the checks. The repository has four areas, and most changes touch one:

| Area | Where | Language |
|---|---|---|
| Server | [`server/`](server) | Rust |
| Web app | [`client/`](client) | TypeScript, React |
| Schema | [`database/`](database) | SQL |
| Docker stack | [`docker/`](docker) | Compose, Dockerfiles |

A change to the API touches `server/api/openapi.json` and the client's copy of it; see [Changing the API](docs/development.md#changing-the-api).

## Checks that must pass

Run the checks of the area you changed before you open or update a pull request. CI runs all of them.

```sh
# server/
make tools && make check     # rustfmt, clippy, licenses, unused dependencies, notices, cargo-deny, the full test suite (needs Docker)

# client/
make check                   # npm ci, licenses, typecheck, lint, knip, tests, build, npm audit, in a Node 22 container (needs Docker)
```

Never point the server tests at a database you care about: every integration test creates and drops a database of its own. `make check` in `server/` uses throwaway containers on random ports.

## Conventions

- **Tests.** Add or update tests with the change. For anything that touches permissions, also test the refusal: another workspace, a plain member, a person who was removed, and a signed-out caller. Client tests run under jsdom and must not open real sockets or networks.
- **Permissions are decided in the server**, from PostgreSQL, on every request. Do not add a cache or a client-side check that replaces one. Hidden things are "not found", not "forbidden". The UI may hide controls a role cannot use, but must show the server's refusals in a way a person can act on.
- **Migrations.** Add the next numbered file under `database/migrations/`; never edit one that is already part of a release. See [Changing the schema](docs/development.md#changing-the-schema).
- **Text is translated.** Every user-visible string in the client has both a Korean and an English value.
- **Configuration.** Document new environment variables, with their defaults, in [docs/configuration.md](docs/configuration.md).
- **Comments and messages** are for a reader who only has this repository: do not refer to private documents, tickets of other systems or internal hostnames.
- **Secrets.** Never commit credentials, tokens or real personal data, including in tests and fixtures.
- **Dependencies** must be compatible with GPL-3.0-only. Regenerate `THIRD-PARTY-NOTICES.md` (`make notices` in `server/` or `client/`) when `Cargo.lock` or `package-lock.json` changes.

## Commits and pull requests

- Keep a pull request focused on one change; unrelated refactoring belongs in its own pull request.
- Write commit messages that say what changed and why. There is no required format.
- Fill in the pull request template, and update the documentation in [`docs/`](docs) when behavior changes. A screenshot helps for UI changes.
- A maintainer will review; expect questions and requests for changes.

## License

Penfii is licensed under **GPL-3.0-only** (see [LICENSE](LICENSE)). By contributing you agree that your contribution is licensed under the same terms. No contributor agreement or sign-off is required at the moment; if that changes, this file will say so.
