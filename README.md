# DungeonFlux

A browser-based tabletop campaign game with a shared TV/laptop display and personal
phone, tablet or laptop clients, including hosted remote and mixed-room campaigns. The new application is planned as **full-stack
Rust**: a native authoritative server and Rust/WebAssembly presentation clients.

Project site: [dungeonfluxdnd.com](https://dungeonfluxdnd.com/).

## Current state

The playable local game runs with one command: `./development/run-gameplay.sh`. Open
the printed display URL once and the player URL in two tabs. See [Play the game
locally](development/README.md#play-the-game-locally) for details. Gameplay tuning
is in progress, with design in [planning/gameplay-tuning.md](planning/gameplay-tuning.md)
and data planned for `content/harbor-journey/`.

The playable two-player harbor journey covers room join, Level 1 Dwarf Fighter
(Soldier) creation, Lantern Wharf, typed courier dialogue, a real-dice bandit fight,
a short rest and arrival at the Harbor Inn. It uses the 39-crate Rust workspace,
real session actor and engine, generated gRPC-over-WebSocket services and durable
PostgreSQL state; the fixture binary is `df-transport-fixture --gameplay-demo` in
`crates/df-tools`.

A production `df-server` binary, live-data `df-web`/`df-player`/`df-display` screens,
create-room/ready, real guest sign-in, telemetry wiring and the wider 2024 rules
catalog are not implemented. The `crates/df-server` policy modules are not yet a
workspace crate. The earlier S00 transport lab and its desktop pressure checks remain
available; see [fixture setup](development/start-s00.md) and
[G02 qualification](development/qualify-g02.md). Physical-device and whole-process
resource evidence still block full G02 approval.

The sibling `dungeonflux.old/` preserves the earlier implementation as a read-only
reference. Its website was explicitly moved into [docs/](docs/); the remaining
archive stays untouched. Old code, todos and devlog inform the design, while demo
restrictions and vendor choices are not the specification. The static site is
prepared for GitHub Pages at `dungeonfluxdnd.com`; publication and DNS remain pending.
See the [site setup guide](docs/README.md).

Start with the [planning index](planning/README.md),
[implementation roadmap](planning/implementation-roadmap.md) and
[agent instructions](AGENTS.md).

## System architecture

```text
Player browser / shared display
  Rust/WASM: input, permitted views, animation, audio capture/playback
       |
       | Generated protobuf RPC over persistent binary WebSocket/WSS
       v
Rust server: authorization -> session owner -> rules/engine
       |                                     |
       | PostgreSQL decision commit          | committed effect intents
       v                                     v
Durable game state                    AI/media services -> provider ports
       |                                     |
       +---------- permitted views/audio/assets ----------> clients

Native/browser instrumentation -> OTEL ingestion/spool -> telemetry SQLite
Development plans/attempts/devlog -> workflow SQLite
Changed-source quality reviews   -> separate quality SQLite
```

The server owns state, dice, legal actions, rules resolution, progression and
AI/media orchestration. It filters private information before projecting a view.
Clients send input and present server-confirmed outcomes; they never duplicate
the rules engine or hide unrestricted server state as an access-control technique.
Host controls are a permission-gated panel on either client role, not a third
required device or a permission granted by a URL.

Each subsystem is a crate with private internals and a small typed public boundary.
Crates initially compose into one backend process rather than independently
deployed services. New features are modules, content and bounded tasks within those
boundaries; a spell, scene, vendor or component does not automatically need a crate.

| Family | Planned boundaries |
| --- | --- |
| Shared | `df-types`, `df-protocol`, `df-observe`, `df-locale`, `df-rpc-bridge`: IDs/units, generated wire schemas, instrumentation, localization and transport |
| Game foundations | `df-model`, `df-content`, `df-rules`, `df-engine`: versioned state/content, pure mechanics and composition of staged decisions |
| World and interaction | `df-world`, `df-knowledge`, `df-intent`, `df-interaction`: causal world updates, truth/belief/memory, input proposals and validated NPC/entity behavior |
| Runtime directors | `df-narrative`, `df-encounter`, `df-combat`, `df-experience`: story structure, challenges, legal tactical proposals and broad pacing |
| Presentation policy | `df-tempo`, `df-presentation`: audience-safe moment intensity, cue plans and predictive asset demand |
| Session/API | `df-auth`, `df-session`, `df-api`, `df-server`: access, session actors, permitted RPC projections and composition |
| AI/media/storage | `df-provider-api`, `df-providers`, `df-ai`, `df-media`, `df-assets`, `df-persistence`: provider ports, validated output, predictive asset generation, durable assets and PostgreSQL adapters |
| Browser | `df-client`, `df-ui`, `df-render`, `df-audio`, `df-player`, `df-display`, `df-web`: connection, reusable presentation and role-specific shells |
| Development | `df-telemetry`, `df-testkit`, `df-tools`, `df-workflow`: diagnostics, controlled fixtures, developer commands and the future Rust coordinator |

The evolving [crate map](planning/subsystem-architecture.md) is authoritative;
this family summary does not freeze later refinements. Consumer-owned ports prevent
adapters from creating dependency cycles. Browser dependencies exclude server
domain/rules/provider/PostgreSQL code. [Subsystem interfaces](planning/subsystem-interfaces.md)
define ownership, typed errors, bounds, cancellation, privacy and recovery.

Private campaign authoring/import/template contracts, bounded memory continuity,
source-grounded effect/trigger testing and conditional platform expansion are
refined in the [gap analysis](planning/gap-analysis.md) and the later
[feature refinement](planning/feature-refinement.md). The latter adds required
hosted remote play, observer-safe recaps/trailers and explicitly opted-in
source/handler-approved custom content while retaining standard 2024 support. These remain plans and
owned evidence gates; standard progression/inventory is core campaign scope.

## Rules and campaign behavior

The target is **standard D&D 2024 fifth edition**, with source-linked mechanics and
content. Pin required books/catalogs, access/rights and errata before claiming full
support; an SRD or an early legal subset is not the entire rulebook. Track supported,
unsupported and pending entries explicitly in [rules coverage](planning/rules-coverage.md)
and [rules support](planning/rules-support.md).

Campaign planning must represent the narrative outline explicitly: beats, established
facts, unresolved threads, backstory hooks, player intent, threat clocks, tension,
convergence and generation budgets. A director chooses story significance and pacing
within permitted committed facts; the rules engine determines outcomes; the LLM
expresses validated narration/dialogue. Steering cannot overwrite dice, invent
resources, reveal withheld knowledge or force player choices. This is the current
planning direction. The [runtime authority map](planning/runtime-directors.md),
[narrative engine](planning/narrative-engine.md),
[interaction engine](planning/interaction-engine.md),
[tempo engine](planning/tempo-engine.md) and
[asset engine](planning/asset-engine.md) define candidate models and interfaces.
Concrete source schemas, campaign content, replay compatibility and calibration
remain gated. Directors propose changes; one session owner commits them. Saved
campaign replay must not rerun an LLM or repeat a paid generation.

## Persistent Rust browser presentation

Both roles use mounted reusable Rust components. Ordinary views, phase changes,
navigation, overlays, late assets and reconnect update the application in place
without full-page reloads. Generated JavaScript for WASM loading/browser bindings
is allowed; authored application and UI logic remains Rust.

Typed server views select supported layouts, presentation variants and bounded
scene/camera/animation cues. Existing variants can change through data; new behavior
requires a Rust build. Stable component/entity identity preserves valid focus,
drafts and resources while disposing replaced listeners, subscriptions and buffers.
Animation uses elapsed frame time, supports interruption/reduced motion, and never
delays authoritative input or resolves gameplay locally. Hidden-tab recovery skips
obsolete cues rather than replaying an unbounded backlog.

Audio/captions follow server timelines and observed browser playback. Target devices,
codec/microphone support, frame-time tails, memory and input-to-render budgets require
actual measurement. See [client architecture](planning/client-architecture.md) and
[client presentation](planning/client-presentation.md).

## RPC, durability and observability

The planned transport ports the GoGRPCBridge idea: established HTTP/2 gRPC over a
binary WebSocket byte stream, with generated Rust/protobuf contracts. Preserve
unary, server-streaming, client-streaming and bidirectional calls, metadata,
trailers/status, deadlines, cancellation, half-close and bounded flow control.
The S00 experiment uses tonic/prost generated contracts, a local h2 browser driver
and wasm-bindgen bindings. Its narrow browser clock patch retains upstream h2 reset
semantics. Full browser/device/resource qualification must pass before dependent
production implementation; desktop smoke evidence alone does not close G02. See [RPC transport](planning/rpc-transport.md)
and [RPC API](planning/rpc-api.md).

A session owner serializes decisions. PostgreSQL commits state, operation result,
facts and effect intents atomically with revision/fence checks. A confirmed mutation
means durable acceptance, not enqueue. A lost response is resolved by operation ID;
it does not automatically repeat a paid call or cancel an already accepted job.
Asset publication requires complete verified bytes before durable visibility.
See [storage architecture](planning/storage-architecture.md) and
[runtime reliability](planning/runtime-reliability.md).

OTEL is part of the first integrated slice. Correlate input, rules, persistence,
async jobs, projection and browser playback/rendering. Capture emitted logs at
enabled levels without sampling; store structured logs and correlated spans in
dedicated telemetry SQLite with durable spooling, bounded queues and read-only
agent queries. Report lag, retention gaps and losses explicitly. Telemetry failure
must not freeze gameplay, and telemetry does not replace authoritative state.
See [observability](planning/observability.md).

After initial integration, every crate gets a measured optimization pass covering
its relevant CPU, memory, I/O, latency and resource costs. Preserve contracts,
correctness, privacy and instrumentation; an evidenced within-budget result can
close a pass without a speculative rewrite. See [optimization](planning/optimization.md).

## Inspect the existing plans

From this directory, with SQLite installed, these commands are read-only:

```sh
sqlite3 -readonly development/workflow.sqlite3 < development/queries.sql

sqlite3 -readonly development/workflow.sqlite3 \
  'SELECT kind,status,count(*) FROM features GROUP BY kind,status ORDER BY kind,status;'

sqlite3 -readonly development/workflow.sqlite3 \
  'SELECT id,title FROM dispatch_ready_tasks ORDER BY priority DESC,id LIMIT 20;'

python3 development/provision.py --check-only
```

The five workflow tables are `features`, `tasks`, `dependencies`, `attempts` and
append-only `devlog`. [Schema](development/schema.sql),
[plan manifest](development/plan-manifest.json) and [query examples](development/queries.sql)
preserve reproducibility. Python here is an existing planning-data helper, not
application code or the future Rust agent runner. `provision.py` without
`--check-only` writes planning data and belongs to the coordinator.

Dependency-ready plans are not necessarily dispatchable atomic tasks. The
coordinator freezes a bounded execution brief and applicable gates before dispatch;
source fingerprints and planned coverage do not prove completion.
See [plan database](planning/plan-database.md).

## Development workflow

Agents must read [AGENTS.md](AGENTS.md), relevant planning and
[mandatory coding style](planning/coding-style.md). Shared types, schemas, migrations
and composition have one active edit owner. Source workers use isolated worktrees
and the coordinator integrates sequentially.

- A frontier coordinator delegates each user work item to one implementing agent.
  Prefer economical Luna/Muse workers, with Terra/Sol for harder logic and
  Sonnet 5.5+ for UI, Opus sparingly, subject to verified runner/model availability.
- After two unsuccessful economical implementation/repair attempts on the same
  task, escalate to Sol for logic/interfaces/computer use, or Sol/Opus for UI,
  UX and gameplay. A worker cannot approve its own work.
- An independent frontier evaluator executes the affected boundary and operates
  real user-facing outputs with computer use and vision. Audio requires actual
  playback observation. Missing evidence remains pending; code review alone is
  insufficient. The coordinator records completion after verified integration.
- Dispatch as many useful agents as measured memory, edit independence, ready
  work and actual runner slots permit, reserving build/browser/evaluation capacity.
  Planned cleanup runs every 30 active-development minutes and protects live
  output, retained evidence, operational databases and spools.
- Workers retain meaningful confusion, errors, defects and challenges in devlog
  through scoped commands when available. [CHANGELOG.md](CHANGELOG.md) captures
  every integrated commit in six-hour blocks.

The installed `dungeonflux-quality-review` personal skill has a configured
one-minute heartbeat. It claims bounded new/changed authored Rust files and caches
MD5, relevant review context and successful review time in separate
`development/quality.sqlite3`. It reviews smells, complexity, duplication, bugs,
test evidence and Rust idioms. Unchanged successful reviews are skipped; MD5 is
change detection, not a security guarantee. The S00 workspace now contains authored
Rust source and an attributed vendored h2 clock adaptation to review.
Findings enter the workflow through coordinator-only deduplicated intake, not
automatic task approval. New runners must install/verify the personal skill;
see [development quality](planning/development-quality.md).

The [ADRs](ADR/0001-sqlite-agent-workflow.md) define queue ownership and
[resource cleanup](ADR/0002-resource-scheduling-and-cleanup.md),
[devlog](ADR/0003-agent-devlog.md), [reliable delivery](ADR/0004-development-reliability.md)
and [output evaluation](ADR/0005-frontier-output-evaluation.md).
Disposable caches/builds belong in ignored `artifacts/`; durable SQLite databases,
their WAL/SHM, telemetry and evidence never belong to routine stale-build cleanup.

## Implementation order and commercial limits

Prove Rust/WASM transport first, then deliver one durable two-role flow with
PostgreSQL, privacy, reconnect and queryable OTEL. Add source-faithful character
creation/rules, story/voice, combat, campaign continuity and complete catalog
coverage through the [roadmap](planning/implementation-roadmap.md). Developer
operations grow with these slices; optional renderer/video fidelity follows the
reliable game path.

Campaign membership and simultaneous sessions need representative capacity tests;
there is no approved four-player cap or unlimited-capacity claim. Provider budgets
reserve and reconcile every paid branch. Prepared assets/replay never silently
start a live provider on a miss. The dated [pricing research](planning/pricing-and-costs.md)
proposes bounded campaign allowances and separate optional video credits; it is
not a launch-price commitment or a deployed billing system. Compare the
[asset-provider shortlist](planning/asset-provider-research.md) and
[competitive research](planning/competitive-research.md) before fixing routes or
customer allowances. Published latency claims still require a measured voice/media
qualification run, and host bundles are meaningful pricing competitors.
