# AGENTS.md

Behavioral guidelines to reduce common LLM coding mistakes. Merge with project-specific instructions as needed.

**Tradeoff:** These guidelines bias toward caution over speed. For trivial tasks, use judgment.

## 1. Think Before Coding

**Don't assume. Don't hide confusion. Surface tradeoffs.**

Before implementing:
- State your assumptions explicitly. If uncertain, ask.
- If multiple interpretations exist, present them - don't pick silently.
- If a simpler approach exists, say so. Push back when warranted.
- Resolve routine implementation choices within the agreed design. For missing
  requirements or conflicting contracts, name the uncertainty and seek a decision
  while continuing work that does not depend on it.

## 2. Simplicity First

**Minimum code that solves the problem. Nothing speculative.**

- No features beyond what was asked.
- No abstractions for single-use code.
- No "flexibility" or "configurability" that wasn't requested.
- No error handling for impossible scenarios.
- If you write 200 lines and it could be 50, rewrite it.

Ask yourself: "Would a senior engineer say this is overcomplicated?" If yes, simplify.

## 3. Surgical Changes

**Touch only what you must. Clean up only your own mess.**

When editing existing code:
- Don't "improve" adjacent code, comments, or formatting.
- Don't refactor things that aren't broken.
- Match existing style, even if you'd do it differently.
- If you notice unrelated dead code, mention it - don't delete it.

When your changes create orphans:
- Remove imports/variables/functions that YOUR changes made unused.
- Don't remove pre-existing dead code unless asked.

The test: Every changed line should trace directly to the user's request.

## 4. Goal-Driven Execution

**Define success criteria. Loop until verified.**

Transform tasks into verifiable goals:
- "Add validation" → "Write tests for invalid inputs, then make them pass"
- "Fix the bug" → "Write a test that reproduces it, then make it pass"
- "Refactor X" → "Ensure tests pass before and after"

For multi-step tasks, state a brief plan:
```
1. [Step] → verify: [check]
2. [Step] → verify: [check]
3. [Step] → verify: [check]
```

Strong success criteria let you loop independently. Weak criteria ("make it work") require constant clarification.

---

**These guidelines are working if:** fewer unnecessary changes in diffs, fewer rewrites due to overcomplication, and clarifying questions come before implementation rather than after mistakes.

Source: [Karpathy-inspired guidelines](https://github.com/multica-ai/andrej-karpathy-skills/blob/main/CLAUDE.md) (MIT).

## Planning and project documentation

`planning/` contains the project's design documents. Agents MUST read and reference
the design documents relevant to their assigned task before implementation or
evaluation. Task briefs must identify those documents and the sections that govern
the work. Follow the approved design, and record missing requirements or conflicting
instructions in the devlog for the coordinator to resolve.

`docs/` contains the GitHub Pages project site: public project information, pages,
styles, and site assets. Keep design documents in `planning/` and development
strategies in `ADR/`.

`dungeonflux.old` is a read-only reference archive, not this project's spec. Honor
the requested work phase: planning and review do not authorize prototypes,
dependency installation, or importing old game files. Concept art guides visual
direction; it does not add gameplay requirements.

## Mandatory coding style

Every agent MUST read and follow [Coding style](planning/coding-style.md) before
implementing, repairing, or evaluating code. The root `rustfmt.toml` governs
formatting; agents must not create their own style variants. Task briefs include
this document and the applicable formatting, Clippy, target, and behavior checks.

Use explicit names, private crate internals, typed contracts/errors, deliberate
ownership, and bounded owned async work. Avoid speculative abstractions, silent
error/default handling, input-driven panics, and ad hoc logging. Preserve the
server-authoritative thin-client architecture and shared OTEL conventions.

Formatting and affected native/WASM lint/compile gates are required before approval.
Evaluators inspect both automated results and the
semantic style rules. Necessary exceptions are narrow, justified, and disclosed;
do not disable gates or reformat unrelated files. Code style cannot substitute
for the independent running-output evaluation required below.

## Application architecture

The new application is full-stack Rust, with a native backend and WebAssembly
browser clients. Do not add Go, JavaScript, or TypeScript application code.
Generated JavaScript glue for loading WebAssembly and browser API bindings is
allowed; application logic and UI source must remain Rust.

The server owns game state, rules, action validation, outcomes, and AI/media
orchestration. Browser clients stay thin: send input, render server-provided views,
and play server-provided audio. Do not duplicate game rules in clients or send
private game data for clients to hide. Read `planning/client-architecture.md`.
Read `planning/runtime-reliability.md` for authority, ownership, cancellation,
reconnect, stream, and persistence requirements extracted from the old devlog.

Each subsystem is a separate Rust crate with a small public interface and private
implementation. Read `planning/subsystem-architecture.md`,
`planning/subsystem-interfaces.md`, and `planning/feature-inventory.md` for the
concrete crate map, planned contracts, and legacy feature coverage. Read
`planning/runtime-directors.md`, `planning/narrative-engine.md`,
`planning/interaction-engine.md`, `planning/tempo-engine.md` and
`planning/asset-engine.md` for the separate
story, NPC/world, knowledge, intent, encounter/combat, experience and tempo
subsystems. All propose pure staged changes through the engine/session owner;
rules retain mechanical authority and clients only render audience-safe plans.
Follow
`planning/implementation-roadmap.md` for prerequisites and delivery order. Freeze
concrete shared types and resolve applicable open gates before dispatching dependent
implementation. Preserve contracts when refining a crate and coordinate consumers.
Read `planning/gap-analysis.md` and the applicable campaign-authoring,
long-horizon-state, rules-effect-model and expansion-boundaries documents for
source-backed import/memory/effect contracts and conditional platform scope.
Summaries and semantic indexes never replace canonical truth; imported content
never becomes executable rules. Required standard progression/inventory stay on
the core campaign backlog. Hosted remote/mixed-room play is required core;
read planning/feature-refinement.md and remote-play.md. Read campaign-cinematics.md
for observer-safe recaps/trailers/critical cues and generated-content.md for
standard-compatible templates plus explicitly disclosed versioned custom opt-in.
Never let provider failure withhold required rewards, or let cosmetic cinema
change source outcomes/timing. Do not dispatch unselected async/community work.

Standard 2024 fifth-edition D&D rules support is required. Read
`planning/rules-support.md` and `planning/rules-coverage.md`; coverage families are
planned, while exact book/catalog/source revisions remain an implementation gate.
Do not silently substitute custom rules, demo shortcuts, or mixed editions.

Observability is required from the first implementation. Read
`planning/observability.md` and include the relevant telemetry and diagnostic
requirements in subsystem plans, task acceptance criteria, and evaluations.

Use OpenTelemetry for structured logs/traces with a dedicated SQLite telemetry
database and read-only agent review access. Capture all emitted logs at enabled
levels without log sampling, propagate correlation through asynchronous work,
and make ingestion lag/loss visible. Use the shared instrumentation path, durable
spooling, bounded export, and tested recovery; do not create competing ad hoc
logging paths except the planned emergency startup/exporter-failure diagnostics.

PostgreSQL is the required backend database for durable gameplay state. The
development queue/devlog stays SQLite and is separate from telemetry storage.
Read `planning/storage-architecture.md` before persistence or telemetry work.
Agents must not mutate telemetry or treat runtime logs as authoritative game data.

Client/server game communication uses generated Rust RPC contracts through the
planned native gRPC-over-WebSocket bridge port. Read `planning/rpc-transport.md`.
Preserve all four RPC modes and transport semantics; verify the browser/WASM
feasibility gate before dependent implementation. Read `planning/rpc-api.md` for
the planned service/message shape, authorization, durability and stream semantics.
Concrete protobuf numbering/generation is a shared-owner prerequisite. Do not
silently replace this with a custom RPC envelope or assume a
native Tonic channel works in a browser without checking its runtime requirements.

After initial integration, every subsystem crate must undergo the optimization
pass in `planning/optimization.md`. Use representative baselines, profiles, and
measured targets; preserve correctness and contracts, verify integrated results,
and track remaining performance gaps. Avoid speculative optimization before this
gate or rewrites without measurable benefit.

## Development strategies

Read applicable decisions in `ADR/` before implementation or evaluation:

The source-backed project plans live in `development/workflow.sqlite3`; read
[Plan database](planning/plan-database.md) and its manifest/query index. Complete
plan coverage does not make every blueprint a dispatchable atomic task. Resolve
applicable gates and preserve a bounded execution brief before claiming work.
Keep this durable database and its WAL/SHM outside artifact cleanup; coordinator
authority and independent evidence-based completion still apply.
Read [Development quality](planning/development-quality.md) for the installed
quality skill, separate review cache and minute heartbeat. Only the coordinator
uses its idempotent findings-intake command; quality review never approves work.
Commercial cost/budget work follows [Pricing and costs](planning/pricing-and-costs.md):
dated rates are research inputs, not pinned provider availability or profitable
capacity promises; selected bounds and payment/usage models require verified gates.

- [ADR 0001](ADR/0001-sqlite-agent-workflow.md): SQLite task queue, worker/evaluator
  separation, and verified completion.
- [ADR 0002](ADR/0002-resource-scheduling-and-cleanup.md): run as many useful agents
  as memory and other limits permit, reserve build/review capacity, and dispatch
  a lightweight stale-artifact cleanup worker every 30 minutes during active runs.
- [ADR 0003](ADR/0003-agent-devlog.md): subagents append development observations
  to SQLite for later process refinement.
- [ADR 0004](ADR/0004-development-reliability.md): concrete task briefs,
  integration ownership, meaningful verification, and build-specific handoffs.
- [ADR 0005](ADR/0005-frontier-output-evaluation.md): coordinator-spawned frontier
  evaluators with computer use and vision, independent running-output checks,
  and evidence-based approval.

## Implementation and evaluation

- Use a frontier coordinator; prefer Luna/Muse for ordinary bounded work, sparing
  Terra/Sol for harder logic/interfaces, and Sonnet 5.5+ for UI with Opus sparingly.
  These are routing preferences subject to actual model/tool availability, not
  assumed capabilities. Follow ADR 0001 and record necessary substitutions.
- Delegate each user work message or scoped atomic work-item message to one
  implementing subagent owning the whole result. Do not split that single item
  among multiple parallel implementation agents or create recursive delegation
  of the same item. Separate independently scoped messages may use different
  workers when prerequisites, edit areas and resource limits permit. The
  independent frontier evaluator remains a separate completion gate; this rule
  does not remove output evaluation or authorize duplicate implementation.
- The coordinator spawns an independent frontier evaluator for each submission,
  with computer use and vision actually enabled. Follow ADR 0005: operate and
  visually inspect running user-facing results, execute affected boundaries for
  internal tasks, and perform integrated feature acceptance. Code review alone
  cannot approve work. Map every required criterion to current observed evidence;
  missing capabilities or unperformed checks keep review pending. Audio needs an
  appropriate playback observation; vision alone cannot verify it.
- After two unsuccessful implementation/repair attempts on the same task by
  Luna-, Terra-, or Muse-class workers, the coordinator spawns Sol for logic,
  interfaces, or computer-use work; Sol or Opus for UI/UX/gameplay experience.
  The initial attempt counts. Switching economical workers does not reset the
  count. Carry forward defects, tried approaches, evidence, and devlog references
  under ADR 0001; fix unavailable prerequisites rather than replaying blocked
  attempts. The escalated worker still needs independent frontier evaluation.
- Work from one bounded task/attempt. The brief identifies why, governing design,
  source revision, owning crate, permitted paths, acceptance/failure cases,
  integration hooks and their owners, verification, and relevant known pitfalls.
  Preserve that brief for the attempt. Report missing prerequisites precisely;
  do not invent public contracts or silently substitute production stubs.
- Use isolated worktrees for source implementation. Include shared schemas,
  composition files, manifests, and layout foundations in edit-area conflicts.
  Re-read affected source before edits or resumed work, and stage only the task's
  paths. The coordinator
  integrates sequentially and owns authoritative task state.
- Make connecting work explicit. Registered executors, reachable callers,
  complete projections, mounted views, and real asset bytes are part of an
  integrated feature. A tested helper alone does not prove the feature works.
- Use focused checks plus verification at the affected boundary. Timing and
  concurrency checks use controlled clocks and synchronization. Ordinary provider
  checks use faithful fakes/local fixtures; live calls require a scoped task with
  deadlines and spend limits. Do not weaken an assertion without verifying the
  approved behavior changed.
- Browser changes need browser-target checks and a real browser smoke test.
  Verify same-phase updates, reconnect, asset failure, and audio playback when
  relevant. CLI success, coverage totals, and critic scores are not substitutes
  for the user behavior being evaluated.
- Identify the commit, build, configuration, and relevant asset revision tested.
  Do not test stale binaries or silently build another revision. Use your own
  preview port/data directory and owned processes. Keep builds fixed during
  playtests and performance measurements; preserve the last verified preview.
- Handoff includes task/attempt, commit, changed paths, delivered behavior,
  integration status, check commands/results, retained evidence, known gaps,
  distinct follow-up proposals, and devlog IDs. State unperformed checks plainly.
  Evaluation verifies the submission; integration checks verify the resulting
  source before the coordinator marks the task done.
- Track workers and child processes, bound searches to relevant project paths,
  and stop only processes you own. A process exit without a submission and
  handoff is not a completed task.

## Agent devlog

Every spawned worker, evaluator, and cleanup agent records meaningful confusion,
conflicting instructions, errors, defects, blockers, challenges, and discoveries
through the SQLite devlog write command when available. Link entries to the task
and attempt; state what happened, what you tried, and the observed outcome.
Append linked resolutions or corrections instead of rewriting earlier entries.
Record significant unresolved issues before handoff. Keep entries concise,
separate facts from hypotheses, and exclude secrets and routine progress noise.
Devlog writes never mark work complete or change queue state.
The planning-phase append command is `development/devlog.py --context <json>
--entry <json>`. The coordinator supplies the execution-context file; the entry
contains the observation, with optional retained evidence/link. Same-ID retries
are idempotent and changed observations use new linked rows. This local role
contract does not replace the future runner's trusted identity/access binding.

## Generated files and artifacts

Use `artifacts/` for all project-controlled caches, generated build output, scratch
files, and temporary staging files that do not need to be saved in Git. Point tool
cache and temporary-output settings there; do not scatter generated files through
source directories or the project root. Use subfolders such as `artifacts/cache/`,
`artifacts/build/`, and `artifacts/tmp/<attempt-id>/`.

The entire `artifacts/` folder is gitignored. Source files, ADRs, the changelog,
and durable workflow records must not depend on disposable files there. Ignored
files are not automatically stale: preserve active output and retained devlog
evidence according to ADR 0002 and ADR 0003.

## Changelog

The coordinator maintains `CHANGELOG.md` in six-hour work blocks. Capture every
integrated commit with its hash and subject, plus the block's timestamps and Git
revision range. Close a shorter final block when a work session ends. Use
integration order and revision ranges so no commit is missed or counted twice.

## Hosted-service authority

Read planning/commerce-service.md, service-operations.md and commercial-validation.md
before account/payment/provider admission, deployment/security/lifecycle or commercial
work. Pure df-commerce policy and native facade are distinct; existing auth and
provider ports delegate to one tenant/entitlement/ledger authority. Current grants,
hierarchical liability and dispatch uncertainty are enforced at durable admission
and native egress; never release Unknown spend because a lease expired. Respect
redaction/backup tombstones and reviewed RightsGrant. Canonical speech slots and
listener-safe context cannot be replaced by a model critic's assurance. Design
scores, planning PASS and assumed cash scenarios are not runtime/customer evidence.

## Visual development journey

The coordinator maintains one screenshot recorder during active development.
Read [Screenshot journey](development/screenshot-journey.md), register only the
current passive public/synthetic DungeonFlux preview, and start/renew its bounded
active lease when actual work starts or continues. Stop it when the development
run ends. Scheduled minute ticks must never renew activity; agents must not start
competing timers. Report preview/source/task changes so the coordinator can update
its registration. A local game preview exists via `development/run-gameplay.sh`;
earlier static-site captures remain a project-site baseline.

Retain an actual browser screenshot and timestamp/source/task metadata each active
minute, even when unchanged. The recorder appends scoped devlog entries; routine
photo evidence is explicitly requested and does not imply approval or game progress.
Never capture the user's desktop, unrelated tabs, credentials, private player views
or a page that auto-joins, purchases or invokes providers. Missing/late captures
remain explicit outcomes. Protect `development/evidence/journey/` from cleanup;
the disk cap stops capture instead of deleting history. Browser profiles/staging
belong in `artifacts/tmp/journey/`; captures use an isolated owned headless browser.
