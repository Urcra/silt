# Contributing to Silt

- [docs/contributing.md](docs/contributing.md): building, the gate, the tests and their fixtures, the development loops, generated files and the sqlx workflow.
- [docs/architecture.md](docs/architecture.md): how Silt is built, crate by crate, with the decision behind each part.

The short version:

```sh
pnpm -C web install && pnpm -C web build   # the web UI is embedded in the binary
cargo build --workspace
scripts/gate.sh --quick                    # optional: lints and tests; CI runs the whole gate on the PR
git commit -s -m "feat(scope): what changed"   # Conventional Commits, signed off
```

Everyone taking part follows the [code of conduct](CODE_OF_CONDUCT.md). Open a pull request against `main`. CI runs the whole gate on it: formatting, lints, the licence check, the tests, the end-to-end tests and the performance budgets. Say in the PR what you ran and what you couldn't. For a change to the UI, put screenshots of each changed screen in the PR's description. For a bigger change, open an issue first to agree on the approach.

Never copy code or assets from closed-source products; open-source code may be used under its licence, credited in [NOTICE](NOTICE).

Sign off each commit you contribute with a `Signed-off-by` line (`git commit -s`), certifying the [Developer Certificate of Origin](https://developercertificate.org/): that you wrote the change, or otherwise have the right to submit it under the project's licence. Changes to the plugin SDK's crates and the plugin template are under MIT, the rest under AGPL-3.0 ([NOTICE](NOTICE)).
