# Contributing

Thanks for helping improve DungeonFlux. Issues are the public work queue: check for an
existing report, use the repository's existing labels, and include a clear outcome and
reproduction details. Do not put passwords, tokens, private player data or other secrets
in issues, pull requests, source code or logs. Report security vulnerabilities privately
using [SECURITY.md](SECURITY.md).

## Build and test

The workspace pins Rust 1.98.1 in `rust-toolchain.toml`. Install that toolchain and the
`wasm32-unknown-unknown` target before building; project scripts do not install
toolchains or dependencies. See [development/tooling.md](development/tooling.md) for
shared commands. Common checks from Git Bash at the repository root are:

```sh
development/df.sh check
development/df.sh test -p <crate> --lib <filter>
development/df.sh fmt
development/df.sh lint
development/df.sh wasm
```

Run focused tests for changed behavior. `df-tools` has a large suite and known failures;
use focused filters or the shard command described in the tooling guide. Follow
[planning/coding-style.md](planning/coding-style.md), including its formatting, lint and
verification requirements. Before submitting, run the applicable lane gate when one is
provided; the coordinator owns integration and final gate review.

## Pull requests

Keep changes focused and describe the behavior, affected paths and any compatibility
effect. List the checks you ran and include gate output where applicable. Use the
repository's descriptive commit subject style, such as `docs: clarify hosted account
status` or `deploy: update demo image owner`; Conventional Commit syntax is not required.
Review [planning/versioning-policy.md](planning/versioning-policy.md) when a change may
affect product versioning or release steps.
