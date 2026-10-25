# Subsystem crate architecture

Date: 2026-09-29
Status: Concrete planning baseline; implementation and remaining gates pending

## Design

Use one Cargo workspace, separate subsystem crates, and initially one backend
process. Crates are not separate deployed services. Thousands of features become
modules/data/tasks inside these boundaries, not thousands of crates. Provider
brands, story phases, D&D classes/spells and UI components stay modules/data unless
measured coupling justifies a split.

The user has now requested detailed planning. This map and
[Subsystem interfaces](subsystem-interfaces.md) replace the earlier deferral of
names and interface shapes. [RPC API](rpc-api.md) defines wire shapes;
[Feature inventory](feature-inventory.md) maps the archive's capabilities.
These are planned contracts, not compiled source. Open prerequisites in the
[roadmap](implementation-roadmap.md) must be resolved for affected tasks before
implementation; workers must not silently substitute incompatible assumptions.

Use `df-` names in Cargo and `df_` in Rust imports. The complete baseline contains
41 crates including development tools and independent commerce policy. Create them
in delivery order, not as empty skeletons. Tables specify allowed direct production dependencies on
project crates; external libraries are omitted.

## Shared and transport crates

| Crate | Target and ownership | Public boundary | Direct project dependencies |
| --- | --- | --- | --- |
| `df-types` | Native/WASM: small IDs, revisions, units and provenance | SessionId, OperationId, AssetId, Revision, LocaleTag, BuildIdentity | none |
| `df-protocol` | Native/WASM: protobuf and generated RPC clients/services | public/admin schemas; no gameplay implementation | none |
| `df-observe` | Native/WASM: common OTEL conventions/export hook | OperationContext, instrumentation, bounded diagnostics | df-types |
| `df-locale` | Native/WASM: catalogs/locale negotiation | Catalog, TextKey, format/settle | df-types |
| `df-rpc-bridge` | Native/WASM target modules: binary WebSocket byte adapter | TunnelStream, limits/lifecycle, native accept/browser connect | df-types, df-observe |

`df-types` must not accumulate character models or service-specific errors.
`df-protocol` may contain privileged schemas, but public bootstrap never registers
those services. DTOs are explicitly mapped to domain/client types.

## Server crates

| Crate | Ownership | Public boundary | Direct project dependencies |
| --- | --- | --- | --- |
| `df-model` | Domain state/inputs/effects and persisted document types | GameState, GameCommand, Effect, GameFact, versioned checkpoint | df-types |
| `df-content` | Versioned campaign/rules catalogs and asset references | ContentPack, RulesCatalog, validate, NPC/context/scene definitions | df-types, df-model, df-locale |
| `df-rules` | Pure D&D mechanics, legal choices and spatial rules | Ruleset, ResolveRequest/Outcome, rolls, legal choices | df-types, df-model, df-content |
| `df-world` | Pure world causality, logical time, travel/weather/resources/faction movement | WorldSimulator::advance/affordances; WorldDelta | df-types, df-model, df-content, df-rules |
| `df-knowledge` | Truth-linked knowledge/beliefs, witness provenance, memory/rumor policy | KnowledgeSystem::perceive/query/propagate/decay; KnowledgeDelta | df-types, df-model, df-content |
| `df-intent` | Classify input and validate bounded semantic action plans | IntentEngine::classify/validate; IntentDisposition, ActionPlan | df-types, df-model, df-content, df-rules |
| `df-interaction` | NPC cognition, social policy, relationships/conversations/obligations | InteractionEngine::react/plan; InteractionProposal | df-types, df-model, df-content, df-rules, df-knowledge |
| `df-narrative` | Story arcs/beats/threads/hooks, gravity and convergence policy | NarrativeDirector::evaluate; NarrativeProposal | df-types, df-model, df-content, df-knowledge |
| `df-encounter` | Combat/social/chase/hazard/puzzle/survival challenge preparation | EncounterDirector::propose/validate; EncounterPlan | df-types, df-model, df-content, df-rules |
| `df-combat` | Legal NPC tactics, battlefield objectives/escalation policy | CombatDirector::choose; TacticalProposal | df-types, df-model, df-content, df-rules, df-knowledge |
| `df-experience` | Macro pacing and voluntary spotlight opportunity policy | ExperienceDirector::observe/recommend; PacingRecommendation | df-types, df-model, df-content |
| `df-tempo` | Continuous moment intensity, inertia/fatigue and bounded impulse policy | TempoEngine::advance; TempoFrame, TempoCueIntent | df-types, df-model, df-content |
| `df-presentation` | Audience-safe audiovisual/UI plans and anticipatory asset demand | PresentationDirector::compose/forecast; PresentationPlan | df-types, df-model, df-content, df-knowledge |
| `df-engine` | Pure composition of rules and directors; game modes and staged transitions | Engine::decide/apply/read_model; typed effects/offers | df-types, df-model, df-content, df-rules, df-world, df-knowledge, df-intent, df-interaction, df-narrative, df-encounter, df-combat, df-experience, df-tempo, df-presentation |
| `df-commerce` | Native pure customer/tenant, entitlement and spend-ledger policy | CommerceTransition; CommerceRepository, PaymentGateway ports; native facade supplied by server | df-types |
| `df-auth` | Identity, membership access and role/capability policy | Authenticator, CredentialStore port, Principal, AuthorizedAudience | df-types, df-observe |
| `df-session` | Actor/directory, durable decisions, timers/jobs/subscriptions | SessionHandle, SessionRepository and EffectExecutor ports, ReadSnapshot | df-types, df-model, df-engine, df-auth, df-observe |
| `df-assets` | Immutable durable media bytes/manifests/access | AssetStore and AssetMetadataStore ports, resolve/open/publish | df-types, df-observe |
| `df-provider-api` | Provider contracts, budgets and job context | text/STT/TTS/image/video/sound ports, BudgetStore, RecordingStore | df-types, df-model, df-observe |
| `df-providers` | Actual vendor/native HTTP adapters; provider modules | factories implementing provider ports | df-types, df-observe, df-provider-api, df-commerce |
| `df-ai` | Validated narrative/intent/flavor, cache/replay/policy | AiService::run; AiResult with permitted text/proposals | df-types, df-model, df-content, df-observe, df-provider-api |
| `df-media` | Asset Engine: predictive media, live speech, audio routing, visual/sound jobs | AssetEngine::admit/status/cancel/complete/reconcile; MediaService streams | df-types, df-model, df-content, df-assets, df-observe, df-provider-api |
| `df-persistence` | PostgreSQL adapters and migrations | implementations of repository/auth/asset/budget/recording ports | df-types, df-model, df-session, df-auth, df-assets, df-provider-api, df-observe, df-commerce, df-customer |
| `df-customer` | Native customer-account facade: signup, verification, sessions, password change/reset, email change, closure/export, throttles, dev mail outbox ([Account service](account-service-research.md)) | CustomerService; AccountStore and EmailSender ports | df-types, df-auth, df-observe |
| `df-portal` | Server-rendered customer portal: HTTP routes, escaped HTML, cookies, CSRF, static CSS; no database, gateway or game types | router, dev_routes, PortalState, PortalOrigin | df-types, df-observe, df-customer |
| `df-api` | RPC handlers, audience projection and debug surfaces | generated services; PublicServiceSet/AdminServiceSet | df-types, df-protocol, df-model, df-session, df-auth, df-assets, df-media, df-provider-api, df-locale, df-observe, df-commerce |
| `df-server` | Hosted composition binary: typed configuration (`toml`), health, metrics exporter, ownership contracts for the control plane, campaign runtime and egress dispatcher; dev surfaces behind the `local-dev` feature | startup/configuration; no reusable game logic. Today it depends only on df-session; as the production root it will wire (launch decision 15) df-tools as the runtime library, df-persistence stores (`PostgresAccountStore` with a `TimeoutPolicy`), df-customer with `HostConfig` and `IdentitySource` (df-workos in hosted production), `df_portal::router_with_app`, df-campaign, df-finance and df-runner | df-session (planned: df-types, df-persistence, df-customer, df-workos, df-portal, df-campaign, df-finance, df-runner, df-observe, df-telemetry) |
| `df-workos` | WorkOS User Management adapter for df-customer's `IdentityProvider` port, plus an in-memory WorkOS for tests; HTTPS transport with no redirects, proxies or retries | `WorkOsIdentity`, `ReqwestTransport`, `FakeWorkOsServer` (test-fakes) | df-customer, df-observe |
| `df-account-app` | The account web app (wasm32 cdylib) and its JSON wire types; df-portal's `app_api` uses the wire types only | routes, update loop, views, wire types | none native (web-sys on wasm32) |
| `df-campaign`, `df-campaign-wire` | Campaigns, membership, invitations, lobby and display pairing; wire types shared with the account app | CampaignService, ports, JSON routes (feature `routes`) | df-types, df-auth, df-observe, df-campaign-wire |
| `df-finance` | Money core: journal, lots, pots, admission, settlement (finance-ledger-research.md) | ledger stores and services | df-types, df-commerce |
| `df-runner` | Trusted runner that starts one isolated campaign runtime process | runner, Docker proxy fake (test-fakes) | df-observe |

Async ports belong to their consumer's crate; adapters depend on that contract,
never the reverse. `df-session` does not import `df-persistence`, `df-ai`, or
`df-media`. `df-server` supplies an executor matching every Effect variant and
calling those services. `df-ai` neither owns state nor commits rules outcomes.
Pure rules/engine/director code returns diagnostic facts to its caller without an SDK.
The ten directors/simulators are concrete separate subsystem boundaries, not new
processes, database writers, tick loops or provider clients. Their versioned state
lives in `df-model`; authoring definitions live in `df-content`. `df-engine` supplies
ordered inputs and stages one candidate; `df-session` alone commits it. Policies
exchange typed `df-model` proposals through composition instead of importing each
other. See [Runtime directors](runtime-directors.md), [Narrative engine](narrative-engine.md),
[Interaction engine](interaction-engine.md) and [Tempo engine](tempo-engine.md).
`df-rules` retains initiative, action economy, dice, legality and resources;
`df-combat` selects tactics, never replaces that authority. `df-media` remains the
content-generation executor; `df-presentation` only chooses permitted plans/demand.

`df-api` uses only the narrow `BudgetStore::inspect` port from `df-provider-api`
for authorized operator cost inspection; it cannot dispatch providers. Observation
context carries correlation and deadlines, never trusted authorization. `df-auth`
supplies authorization separately; the engine receives a pure `df-model` audience
scope constructed after authorization, not an auth or telemetry dependency.

## Browser crates

| Crate | Ownership | Public boundary | Direct project dependencies |
| --- | --- | --- | --- |
| `df-client` | RPC/connection, view freshness, bounded assets/reporting | ClientSession, ViewStore, AssetCache | df-types, df-protocol, df-rpc-bridge, df-observe |
| `df-ui` | Shared Rust tokens/widgets, join/about/host panels/accessibility | components, theme, presentation helpers | df-types, df-protocol, df-locale |
| `df-render` | Flat/tactical scenes, tokens/animations and optional 3D | SceneRenderer, RendererCapabilities, PresentationOutcome | df-types, df-protocol, df-observe |
| `df-audio` | Browser mixer/playback and microphone adapters | AudioPlayer, CaptureSession, outcomes/cancel | df-types, df-protocol, df-observe |
| `df-player` | Personal responsive screens and interaction mapping | PlayerScreen::mount/update | df-types, df-protocol, df-client, df-ui, df-render, df-audio, df-observe |
| `df-display` | Group scene, party/initiative/captions/dice/cinematics | DisplayScreen::mount/update | df-types, df-protocol, df-client, df-ui, df-render, df-audio, df-observe |
| `df-web` | WASM boot/routes/composition; generated JS glue only | startup and role/panel mounting | df-types, df-client, df-ui, df-player, df-display, df-observe |

Host controls are a permission-gated panel mountable on either role, not permission
granted by a URL or host-looking layout. Browser crates must never transitively
depend on `df-model`, `df-content`, `df-rules`, `df-engine`, `df-session`, `df-auth`,
provider adapters or PostgreSQL. Protocol DTOs contain permitted views, not full
server state. Camera/layout/decoding/input drafts can be local; legal actions,
paths, rolls, private filtering and consequences are server-owned.

## Tooling crates

| Crate | Ownership | Public boundary | Direct project dependencies |
| --- | --- | --- | --- |
| `df-telemetry` | Rust OTLP ingestion, spool/SQLite writer and read-only review | ingest/query/pin/export, TelemetryService, retention/readiness | df-types, df-protocol, df-auth, df-observe |
| `df-testkit` | Test-only fixtures/fakes, clocks/dice/replays | VirtualHarness, FixtureProviders, contract assertions | df-types, df-model, df-content, df-rules, df-engine, df-session, df-auth, df-assets, df-provider-api |
| `df-tools` | Rust dfctl/xtask: build/preview/content/media/test tools | CLI via RPC; owned process/build manifests | df-types, df-protocol, df-content, df-assets, df-provider-api, df-providers, df-ai, df-media, df-observe |
| `df-workflow` | SQLite queue/devlog, dispatch/evaluation/escalation/cleanup | scoped commands and coordinator loop | df-types, df-observe |

Use `df-testkit` for integration suites or dev-dependencies only where the graph
remains acyclic; its own dependencies cannot depend back on it. Browser preview
fixtures use protocol data, never a client engine or server testkit. Keep workflow
development-model routing independent of the game's provider policy. `docs/`
is the GitHub Pages site and does not require a game crate.

## Integration and refinement

Architecture checks enforce these allowed direct edges, no cycles, no server code
in WASM, and no production testkit imports. Justified edge changes update this
document and consumer checks together. Entry points own configuration; libraries
take explicit settings instead of reading environment globals.

Schema generation, manifests, workspace setup, migrations, shared UI foundations
and composition registration each have one active edit owner. Startup validates
required effects/providers/storage/content/fallback assets. Missing required wiring
fails readiness; optional capabilities explicitly report a fallback/unavailability.

Contracts specify validation, errors, ownership, bounds, cancellation, ordering,
versioning, telemetry and focused tests. Preserve [Coding style](coding-style.md).
Every crate gets its measured [optimization pass](optimization.md) after initial
integration. A split or internal replacement preserves behavior and must improve
measured development/runtime results.

## Commercial service composition

[Commerce service](commerce-service.md) defines the one new native policy boundary.
Commerce IDs use df-types; commercial records live once in df-commerce, not a
second game/provider ledger. df-provider-api BudgetStore remains a consumer port
implemented by df-server/persistence with the commerce ledger; models use opaque
reservation/grant IDs. The native CommerceService facade owns I/O, separately from
pure policy. df-auth owns EntitlementReader and its persistence projection; current
capability plus atomic reservation/dispatch revalidation protects paid admissions.
No df-commerce dependency enters df-session, df-engine/directors or browser closure.
The customer portal is `df-customer` (native facade; owns its store and mail ports) plus
`df-portal` (HTTP and markup only). Composition wires `df-persistence`'s `PostgresAccountStore`
into them and merges the portal router into the running server; neither enters a browser closure.
[Service operations](service-operations.md) fixes dispatch/tenant/recovery limits;
[Commercial validation](commercial-validation.md) owns future income evidence.
