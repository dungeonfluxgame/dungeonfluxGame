# Planning

Store project design documents here. Agents must read and reference the documents
relevant to their tasks before implementing or evaluating changes.

Task briefs should link to the governing documents and sections. Add links to
approved design documents below as they are created.

Each document distinguishes agreed requirements, proposals, and open decisions.
Use current user decisions and these documents as the source of truth; the old
archive and historical devlog are supporting evidence. Runtime details and
subsystem interface shapes are now planned in the documents below. Resolve their
remaining gates and freeze concrete types before dependent implementation. Plan
the first integrated game flow and connecting tasks before expanding a large
backlog; see ADR 0004 for task briefs. This folder is planning, not an implementation
or a claim that source, generated schemas or game/runtime services already exist.
The authorized development planning database is now provisioned; its records are
plans, not implemented or verified game features.

- [Game refinement](game-refinement.md): ongoing refinement decisions and open launch-flow discussion.
- [Mandatory coding style](coding-style.md): required Rust formatting, naming,
  contracts, ownership/errors, async behavior, instrumentation, and review gates.
- [Browser client architecture](client-architecture.md): agreed shared-display and
  player-client roles, thin presentation clients, server authority, full-stack
  Rust direction, and open design decisions.
- [Client presentation](client-presentation.md): persistent programmable Rust/WASM
  shells, incremental mounted updates, animation/resource lifetimes, recovery,
  accessibility and measured device acceptance.
- [Subsystem architecture](subsystem-architecture.md): separate Rust crates,
  a 41-crate map including tooling, and allowed direct dependencies.
- [Subsystem interfaces](subsystem-interfaces.md): public operations, ownership,
  durable commit/effect contracts, browser boundaries and integration guarantees.
- [Feature inventory](feature-inventory.md): 44 capability areas mapped from the
  old implementation/todos and supplied runtime outlines into the new architecture, with gaps and exclusions.
- [Runtime directors](runtime-directors.md): ten separate pure runtime subsystems,
  concrete model contracts, single-owner composition and replay/budget boundaries.
- [Narrative engine](narrative-engine.md): structural beats/facts/threats/threads,
  believable convergence, agency and authoring validation.
- [Interaction engine](interaction-engine.md): NPC autonomy, provenance, social
  commitments, affordances and bounded compound action plans.
- [Tempo engine](tempo-engine.md): continuous server-owned intensity profiles,
  music/visual/UI cues, inertia/fatigue and accessible audience-safe projection.
- [Asset engine](asset-engine.md): predictive media orchestration in df-media,
  canonical reference packs, latency classes, fair bounded demand and spend/fallback policy.
- [D&D rules support](rules-support.md): required standard rules support,
  selected 2024 fifth edition, planned crate contracts and source-pinning gates.
- [Rules coverage](rules-coverage.md): mechanics/content families, source gates,
  provenance and staged implementation versus complete standard support.
- [Observability](observability.md): OTEL logs and correlated spans in SQLite,
  agent review access, metrics, diagnostics, and ingestion reliability.
- [Storage architecture](storage-architecture.md): PostgreSQL game backend,
  separate SQLite workflow and telemetry stores, and durability/scale requirements.
- [RPC transport](rpc-transport.md): Rust gRPC-over-WebSocket direction,
  structured streaming, latency requirements, and browser feasibility gate.
- [RPC API](rpc-api.md): typed services, actions/views, auth, recovery, audio/assets,
  reports, privileged diagnostics and versioning.
- [Post-integration optimization](optimization.md): required profiling and measured
  optimization of every crate after the first integrated game flow.
- [Runtime reliability](runtime-reliability.md): failure patterns from the old
  game translated into requirements for server ownership and thin clients.
- [Implementation roadmap](implementation-roadmap.md): ordered integrated slices,
  prerequisite owners, explicit open gates and feature acceptance/optimization.
- [Plan database](plan-database.md): durable five-table SQLite corpus, complete
  model/feature coverage, bounded dispatch, refinement evidence and read-only queries.
- [Development quality](development-quality.md): bounded cached source review,
  minute cadence and coordinator-only deduplicated findings intake.
- [Pricing and costs](pricing-and-costs.md): dated supplier/hosting research,
  variable campaign capacity, bounded allowances/video and commercial acceptance gates.
- [Gap analysis](gap-analysis.md): every supplied assessment/research row mapped
  to canonical plans, priority corrections, refinement coverage and honest evidence gaps.
- [Campaign authoring](campaign-authoring.md): private immutable packs/templates,
  bounded import/extraction, source/rights validation and activation/migration.
- [Long-horizon state](long-horizon-state.md): source-backed memory retrieval,
  contradiction/summary safety, simulation tiers and bounded pause-aware catch-up.
- [Rules effect model](rules-effect-model.md): source-linked typed effects,
  trigger windows, exceptional mechanics and meaningful property/golden tests.
- [Expansion boundaries](expansion-boundaries.md): owned remote/async/community/
  homebrew decisions and core degraded-mode guarantees without new launch claims.

- [Engine overview](engine-overview.md): derived Mermaid maps of the full engine,
  authority, staged decisions, runtime directors and storage/presentation flows.

- [Requested feature refinement](feature-refinement.md): exact new 15-idea/15-priority
  mapping, required table stakes/phases, contextual phones and consented spotlight.
- [Hosted remote play](remote-play.md): required core remote/mixed-room authority,
  admission/presence/private capture/audio topology and network/device acceptance.
- [Campaign cinematics](campaign-cinematics.md): recap/trailer/critical cues,
  consented scoped exports and committed location/item/media continuity.
- [Generated content](generated-content.md): source-compatible templates and
  explicitly approved custom ruleset/handler admission, with standard mode preserved.

## Hosted service readiness

- [Customer, commerce and admission](commerce-service.md): recoverable tenants,
  subscriptions/entitlements, one hierarchical exact ledger and payment state.
- [Service operations](service-operations.md): routing/dispatch, isolation, bounded
  workloads, recovery, telemetry placement, support and deletion/backup lifecycle.
- [Commercial validation](commercial-validation.md): rights-complete launch,
  acquisition/retention experiments and reproducible cash/contribution sensitivity.

All current 47 game families remain required; these contracts add service readiness
without claiming runtime evidence, commercial clearance, customer demand or profit.
