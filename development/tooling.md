# Developer tooling

The repository pins Rust 1.98.1 in `rust-toolchain.toml`. Install that toolchain and
its `wasm32-unknown-unknown` target before running the commands below. Commands do
not install dependencies or toolchains. Cargo dependencies must already be cached
when working offline.

From the repository root, use `development/df.sh` (Git Bash on Windows, Bash on
Linux):

| Command | Purpose |
| --- | --- |
| `development/df.sh check` | Check all workspace targets on the native host |
| `development/df.sh test -p <crate> --lib <filter>` | Run a focused crate test |
| `development/df.sh fmt` | Check Rust formatting using the root `rustfmt.toml` |
| `development/df.sh lint` | Run Clippy with warnings denied |
| `development/df.sh wasm` | Check `df-tools` for `wasm32-unknown-unknown` |
| `development/df.sh test-tools --only '<module-regex>'` | Run selected `df-tools` shards |
| `development/df.sh test-tools` | Run the full shard matrix with strict known-failure checks |
| `development/df.sh python-check` | Compile Python tools using isolated mode and an artifacts cache |
| `development/df.sh run --print-config` | Preview launch settings without starting the game |
| `development/df.sh run [game options]` | Start the local game and PostgreSQL as documented below |

`df-tools` has known failures and a large suite. Use focused Rust filters during
development; use the shard runner to get per-module results and compare failures
against `development/df-tools-known-failures.txt`. The full matrix takes much longer
than a local smoke run. See `planning/coding-style.md` for required style and test
gates. `development/run-gameplay.sh --print-config` is a dry run; an ordinary run
starts the server and may start the owned PostgreSQL instance.

## PostgreSQL

`development/postgres.sh setup` builds the pinned PostgreSQL 18.6 source release
under `artifacts/`. It verifies the archive checksum but downloads the archive on a
fresh setup, so bootstrap needs network access. The local lifecycle commands are
`development/postgres.sh start`, `status`, and `stop`. The runner uses the same
artifact-owned installation and does not manage a system PostgreSQL service.

## Optional pre-commit check

The opt-in hook checks staged Rust formatting and shell/Python syntax. Documentation-
only changes pass without running a workspace-wide Rust format check. It does not stage
or modify files. Enable it with:

```sh
git config core.hooksPath development/hooks
```

Disable it with `git config --unset core.hooksPath`. CI and the lane gate remain the
authoritative checks.

## Dependency review

`development/df.sh deny` skips clearly until `cargo-deny` and an approved
`development/deny.toml` are both present. No license allowlist or advisory policy is
invented here; use `planning/dependency-license-policy.md` and the dependency owner
to establish one before adding the config. After approval, install the reviewed
`cargo-deny` version with `cargo install --locked cargo-deny --version <version>`;
the command itself never installs tools. Workspace-wide lint/profile policy also
needs a scoped root-manifest change by its owner.

## Commit gate and artifact pruning

`development/gate.sh [crate ...]` runs the commit gate in one command: `cargo fmt`
check, Clippy with warnings denied, the wasm Clippy check when `df-playground` is
included, and `cargo test --lib`. It defaults to `df-table`, `df-table-host` and
`df-playground`, uses `artifacts/build/start-s00` unless `CARGO_TARGET_DIR` is set,
and stops at the first failing stage. `--quick` skips tests, and `GATE_TEST_ARGS`
passes extra arguments to the test harness. Stage logs go to `artifacts/tmp/gate/`.

`development/prune-artifacts.sh [--dry-run] [--hours N] [--force]` removes stale
output from the shared target: `df_*` and `df-*` files in the native and wasm
`debug/deps` directories, workspace `.exe` and `.pdb` test binaries (default: older
than 2 hours), and `*-target` or `*_target` directories in `artifacts/tmp` untouched
for 24 hours. It never touches third-party crate files, `artifacts/cache`,
`artifacts/playground`, the PostgreSQL or imagery build roots, or `development/`. It
refuses to delete while `cargo` or `rustc` is running unless `--force` is given. Run
`--dry-run` first to see counts and sizes.

## Handoff and release notes

Lane workers hand off exact changed paths, checks and retained evidence to the
coordinator. The coordinator reviews and integrates changes, owns commits, and
records integrated commit hashes and subjects in `CHANGELOG.md` in six-hour blocks.
For product version impact and release tags, follow
[`planning/versioning-policy.md`](../planning/versioning-policy.md); version bumps
and release tags remain owner-approved operations.
