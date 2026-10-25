# DungeonFlux

DungeonFlux is an AI Dungeon Master for a living-room table. The adventure plays on a
shared TV or laptop display, and each player joins from their own phone, tablet or
laptop. A Rust server owns the story, the dice and the rules, which follow the D&D
SRD 5.2.1. The look is photographic: scenes, people and places are generated to read
like a live-action fantasy TV drama.

The project is in active development. A local game is playable end to end on a
development machine, but there is no packaged release, no hosted service and no
accounts. Project site: [dungeonfluxdnd.com](https://dungeonfluxdnd.com/) (concept
page; see [docs/README.md](docs/README.md) for its publication status).

## What works today

- A local game server with PostgreSQL storage, a shared table display and player
  screens, run from one script or from a desktop launcher.
- A fixed harbor adventure, randomly rolled adventures and persistent campaigns, plus
  a free-roam pack. Content lives in [content/](content/).
- Level 1 Fighter (Soldier background) character creation with SRD 5.2.1 species,
  dice rolled by the server, and typed, real-dice combat.
- An AI storyteller, optional spoken narration and character voices, speech input,
  adaptive music, generated pictures, and optional video and Gaussian-splat battle
  scenes. Each is behind a switch and needs its own provider key.
- A development command line and structured logs for driving and inspecting a running
  game ([development/README.md](development/README.md)).

What is not built: a production `df-server` binary (the game server today is the
`df-transport-fixture` binary in `crates/df-tools`), accounts and real sign-in,
billing, hosted play, and rules coverage beyond the SRD 5.2.1 subset the game uses.
The `df-server` crate directory holds source but is not a workspace member. Detailed
status and open bugs are in [planning/issue-triage.md](planning/issue-triage.md) and
[planning/implementation-roadmap.md](planning/implementation-roadmap.md).

## Run it locally

Prerequisites, from the scripts and `rust-toolchain.toml`:

- Rust 1.98.1 with the `wasm32-unknown-unknown` target (pinned in `rust-toolchain.toml`).
- `wasm-bindgen` 0.2.129 (the run script refuses any other version).
- PostgreSQL 18.6 built locally by `development/postgres.sh setup`, which downloads
  the official source archive, checks its SHA256 and installs under `artifacts/`.
- A POSIX shell (Git Bash on Windows), plus Python 3 for helper scripts.

The run script keeps its own `rustup` and `cargo` homes under `artifacts/cache/` and
builds with `--offline --locked`, so dependencies must already be fetched there. A
from-scratch bootstrap is not scripted.

```sh
development/run-gameplay.sh
```

The script builds the server and the wasm client, starts PostgreSQL if needed, and
prints a `READY` line. Open the `display` URL on the shared screen and the `player`
URL in a browser tab or on a phone. Ctrl-C stops the server. Useful flags:

```sh
development/run-gameplay.sh --print-config      # show nonsecret launch inputs, start nothing
development/run-gameplay.sh --lan               # let phones on your network join
development/run-gameplay.sh --no-build --database <name>   # resume a saved game
development/run-gameplay.sh --harbor            # the fixed harbor adventure
development/run-gameplay.sh --campaign <id>     # a persistent campaign
```

Every flag and environment variable is listed in the
[launch and configuration reference](development/launch-reference.md), and the play
walkthrough is in [development/README.md](development/README.md#play-the-game-locally).
The local preview has no accounts: anyone on your network with the room code can take a
seat, so use it on a network you trust.

### Desktop launcher (Windows)

`crates/df-launcher` builds `dungeonflux.exe`, a window for starting, resuming and
stopping games and editing settings. It is a standalone crate (its own `Cargo.lock`,
not a workspace member) and uses an existing game build.

```sh
cargo build --manifest-path crates/df-launcher/Cargo.toml --target-dir artifacts/build/launcher
artifacts/build/launcher/debug/dungeonflux.exe --help
```

### Provider keys

With no keys the game still runs; features that need a provider stay off. Keys go in the
Git-ignored `.env` at the repository root, in your shell environment, or in the
launcher's Git-ignored `config/dungeonflux.toml` (copy options from
`config/dungeonflux.example.toml`). The names the code reads:

| Variable | Used for |
| --- | --- |
| `OPENAI_API_KEY` | Storyteller, story memory, scene pictures, character portraits |
| `ELEVENLABS_API_KEY` | Narration voices, music, speech input |
| `DF_JEV_API_KEY` | Reading player actions, sorting portrait requests |
| `FAL_KEY` (or `DF_FAL_KEY`) | Generated video clips, the FLUX picture option |
| `BFL_API_KEY` | Alternate portrait route for mature character appearances |
| `WORLDLABS_API_KEY` | Explorable 3D places |

Never commit keys or paste them into logs. Generated video is off by default because of
cost; see [Pricing and costs](planning/pricing-and-costs.md) for the dated cost research.

## Architecture

```text
Player phones and shared display (Rust compiled to WebAssembly)
      |  generated protobuf RPC over one binary WebSocket
      v
Rust server: authorization -> session owner -> rules and engine
      |                                  |
      | PostgreSQL commit                | committed effect intents
      v                                  v
Durable game state                 AI and media services -> provider ports
      |                                  |
      +-------- permitted views, audio and assets --------> clients

Instrumentation -> OpenTelemetry spans and logs -> telemetry store
```

Key rules:

- The server is authoritative. It owns state, dice, legal actions, rules outcomes and
  AI orchestration, and filters private information before building a view.
- Clients are thin Rust/WebAssembly presenters. They send input and show
  server-confirmed results; they never duplicate rules or receive data to hide. Only
  generated glue is JavaScript.
- Transport is gRPC over a binary WebSocket, ported from the GoGRPCBridge idea
  (`df-rpc-bridge`, tonic and prost).
- One session owner serializes decisions, and PostgreSQL commits state and effect
  intents atomically. The design requires that replaying a saved game never calls a
  model or repeats a paid generation (see runtime reliability).
- Telemetry follows OpenTelemetry conventions (`df-observe`, `df-telemetry`).

`crates/` holds 41 crates: 39 workspace members plus `df-launcher` and `df-server`, which
are outside the workspace. Each keeps its internals private behind a small typed
boundary; the [crate map](planning/subsystem-architecture.md) is authoritative.

| Role | Crates |
| --- | --- |
| Foundations | `df-types`, `df-model`, `df-content`, `df-locale`, `df-testkit` |
| Rules and engine | `df-rules`, `df-engine`, `df-combat`, `df-encounter` |
| Commercial policy | `df-commerce` |
| World and narrative | `df-world`, `df-knowledge`, `df-intent`, `df-interaction`, `df-narrative`, `df-experience` |
| Presentation policy | `df-tempo`, `df-presentation` |
| Session and API | `df-auth`, `df-session`, `df-api`, `df-server` (contract only, not a workspace member) |
| Transport | `df-protocol`, `df-rpc-bridge` |
| AI, media and storage | `df-provider-api`, `df-providers`, `df-ai`, `df-media`, `df-assets`, `df-persistence` |
| Browser clients and render | `df-client`, `df-ui`, `df-render`, `df-audio`, `df-player`, `df-display`, `df-web` |
| Telemetry | `df-observe`, `df-telemetry` |
| Tools and launcher | `df-tools` (the game server binary, dev CLI and fixtures), `df-launcher` |

Design documents that explain the larger pieces: [client architecture](planning/client-architecture.md),
[narrative engine](planning/narrative-engine.md), [interaction engine](planning/interaction-engine.md),
[tempo engine](planning/tempo-engine.md), [asset engine](planning/asset-engine.md),
[RPC transport](planning/rpc-transport.md), [storage](planning/storage-architecture.md),
[observability](planning/observability.md) and [runtime reliability](planning/runtime-reliability.md).
An LLM only expresses narration and dialogue that the rules engine and story directors
have already validated; it never decides dice or outcomes.

## Rules

The game targets the 2024 fifth-edition rules and uses the System Reference Document
5.2.1 as its source for species, creatures and mechanics; the PDF is kept in
`development/rules-sources/`. The SRD is a subset of the full rulebooks, so full
standard coverage is a goal tracked in [rules coverage](planning/rules-coverage.md) and
[rules support](planning/rules-support.md), not a current claim. Content that departs
from SRD values is flagged as custom rules in the game.

This work includes material from the System Reference Document 5.2.1 by Wizards of the
Coast LLC, available at https://www.dndbeyond.com/srd. The SRD 5.2.1 is licensed under the
[Creative Commons Attribution 4.0 International License](https://creativecommons.org/licenses/by/4.0/legalcode).

## Development

Read [AGENTS.md](AGENTS.md) and the [coding style](planning/coding-style.md) before
changing code. The style is mandatory: explicit names, private crate internals, typed
errors, bounded async work, no dead code, LF endings, and rustfmt from the root
`rustfmt.toml`. Application and UI code stays Rust.

```sh
cargo build --locked -p df-tools --bin df-transport-fixture
cargo build --locked -p df-tools --lib --target wasm32-unknown-unknown
cargo clippy -p <crate> --all-targets -- -D warnings
cargo check -p df-tools --target wasm32-unknown-unknown
cargo test -p <crate>
```

Test caveats:

- `df-tools` has thousands of tests. Run filtered tests only, for example
  `cargo test -p df-tools --lib <name-filter>`, as the coding style requires. The full
  `df-tools` suite currently has a known crash, so do not rely on it.
- Tests must not sleep, spend money on providers or leave processes behind.
- `python development/test-launch-settings.py` checks the launch scripts with fake
  tools; it does not start PostgreSQL or call providers.

Where things live:

| Path | Contents |
| --- | --- |
| `planning/` | Design documents; start at [planning/README.md](planning/README.md) |
| `development/` | Run and launch scripts, dev CLI notes, schemas, browser checks |
| `ADR/` | Decisions on workflow, resources, devlog and evaluation |
| `content/` | Adventure tables, packs, voice casting, pricing data |
| `config/` | Launcher settings (`dungeonflux.example.toml` is the clean copy) |
| `docs/` | The static project site |
| `CHANGELOG.md` | Integrated commits in six-hour blocks |

## Status, roadmap and costs

The [implementation roadmap](planning/implementation-roadmap.md) orders the remaining
work: broader rules coverage, campaign continuity, a production server and accounts,
and optional video fidelity. Open bugs are listed in
[planning/issue-triage.md](planning/issue-triage.md), and recent work is in
[CHANGELOG.md](CHANGELOG.md). Concurrency and latency findings are in
[server concurrency research](planning/server-concurrency-research.md).

Running the game with provider keys spends real money on text, voice, music, pictures
and optionally video. Commercial pricing is research only, with no launch price or
billing system; see [Pricing and costs](planning/pricing-and-costs.md).

## License

The workspace manifest declares MIT, but the repository does not yet include a
`LICENSE` file, so treat the licensing terms as unconfirmed until one is added.
Third-party code is credited in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md), and
the [dependency license policy](planning/dependency-license-policy.md) covers
dependencies.
