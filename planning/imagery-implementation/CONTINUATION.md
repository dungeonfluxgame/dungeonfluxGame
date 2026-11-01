# Imagery implementation continuation

## Active integration checkpoint (2026-10-29)

This section supersedes the historical status below. Work remains active on the shared
Desktop checkout; final acceptance has not been reached. Sol contributors own disjoint
source scopes. Root reviews, integrates, and serializes Cargo and disposable PostgreSQL
runs. The user authorized creator-account funding source changes and disposable tests.
No paid provider calls, deployment, push, or new commit occurred in this continuation.

Latest observed evidence in `artifacts/tmp/imagery-finish-20261029/`:

- `integration-check-19.log`: native `df-tools --all-targets --offline` passed. Later
  protocol and hosted composition changes require a new check and actual test binary.
- `imagery-lib-12.log`: all 820 library tests passed, zero ignored. Later changes need rerun.
- `execution/full-pg-8.log`: all 68 execution PostgreSQL tests passed, zero ignored.
- `publication/publication-current-pg-8.log`: all 28 publication PostgreSQL tests passed.
- `runtime/pg-default-stack-2.log`: all 11 runtime PostgreSQL tests passed on the normal
  thread stack, including actual provider-call crash recovery and private-result visibility.
  Production boxing reduced the run-next future from 37,872 to 152 bytes and start from
  41,184 to 3,584 bytes; no acceptance stack-size override is used.
- `identity/authority-pg-1.log`: all five actual current/refused/unproven authority and
  concurrent lock-order tests passed (6.35 seconds).
- `funded-adapter/cancellation-pg-9.log`: three of five actual funded tests passed.
  The two failures were an incorrect campaign ledger dimension assertion and missing
  admission-role SELECT access to source tombstone/variant facts. Narrow repairs are
  in progress; this is not a passing funded suite.
- Native HTTP Retry-After tests: 13 provider unit tests and three public integration tests
  passed with all existing transport ceilings unchanged.
- Strict image-boundary scanner and all 108 scanner regressions passed. Assets 189 tests,
  strict assets lint, and nine byte-identical legacy transform outputs have retained evidence.
  Current host consumer/logging/quota tests, full native/WASM builds, strict affected lint,
  and independent integrated acceptance remain outstanding.

Source mounts the unified service/runtime for generation, creator classification,
inspection, standalone analysis, transformations, canonical publication, usage projection,
and authorized delivery. Actual GETs recheck current source authority. Per-host quota and
operator controls survive worker rebuilds; completed timely images remain readable after
job deadlines, while late reconciliation remains private. These claims are bounded by the
specific test snapshots above; source-ready changes are not runtime acceptance.

Registered shared migrations: 0038 through 0047, then 0049 through 0053. 0048 is reserved
for same-campaign/catalog cross-run artifact reuse; 0054 for machine hosted-run authority;
0055 for durable submit-decline retry evidence. Do not infer completion from a maximum
version or silently skip holes. 0050 protects canonical image finance protocols from
legacy generic APIs; independent review found a multi-attempt lock-order issue, with a
union-lock repair pending. 0051 closes proven source-denied media delivery; 0052 provides
locked current/refused/unproven authority; 0053 grants read-only source facts to admission.

Remaining source work: safe submit-decline retry with old Unknown exposure held and fresh
independent admission; unsent quota/preflight terminal handling; atomic Book unsent marker
and execution acknowledgement; source-denied closure; same-catalog artwork reuse; and the
hosted composition/creator funding connection. All require actual integrated tests. No
fabricated Ready, supplier call, zero-cost Unknown, money release, or authority proof is allowed.

Two explicit approval questions are pending. The earlier shared NativeHttp ceiling relaxation
beyond 16 MiB / 180 seconds remains unapplied after automatic approval rejection. Bounded
Retry-After observation was separately implemented without changing those ceilings. A new
hosted campaign-funding mount was also rejected because automatic review interpreted prior
funding approval as creator portraits only. Its inert concrete patch and scope are retained in
`consumers/blocked-hosted-mount/`; the user has been asked to approve source changes and
scratch-database tests only. No rejected mount, production deployment, or paid call occurred.

Only disposable PostgreSQL at 127.0.0.1:55471 is authorized. Use the existing target
`artifacts/build/imagery-baseline`, two Cargo jobs, CARGO_INCREMENTAL=0 and test-tmp.
Require scratch mode for every actual PG test, with explicit finance/identity port settings.
Root serializes builds/tests and captures source snapshots before compilation; workers hold
shared interfaces until the executable is built. Keep workflow databases, evidence and the
latest verified build. Native Windows screenshot recorder status currently fails because its
Python tool imports Unix-only fcntl; no current gameplay screenshot is claimed.


Status: **current native/WASM/launcher builds and scoped runtime tests pass on 2026-10-28; full imagery integration and strict lint acceptance remain unfinished.**

Checkpoint commit: `6d0ce07e1d775d8bff9f569b23826a0503e13035`. The historical pause snapshot below remains useful; resumed runtime/attempt state is recorded in the local workflow database and contributor manifest.

The user resumed work and requested continuation until the project could be built. That build milestone is now verified. No push was requested. The resumed-integration record below supersedes older build/pause/process/attempt statements later in this document.

## Full completion resumed (2026-10-28)

The user requested completion of the crate beyond the earlier build milestone.
Six Muse Spark 1.3 CLI contributors are active on disjoint paths: typed development
configuration, cost projection, durable execution journal (migration 0040), authenticated
account-draft delivery, lossy WebP, and resumable/configurable private API backends.
Their exact sessions and scopes are retained in the contributor manifest. Root owns
shared exports, manifests, integration and verification; no new commit or push.

The prior successful builds below describe their recorded hashes, not these active edits.
The native libwebp 1.5.0 dependency builds; a transient imagery check during configuration
changes fails at incomplete config/codec mappings. Full execution/publication and all-consumer
migration remain unfinished. Host account-draft funding awaits the user's payer decision;
local execution, campaign execution and private delivery can proceed independently.
The final draft disclosure check uses one PostgreSQL statement snapshot over session,
account and catalog after bounded bytes load. Revocation committed before that authorization
snapshot denies delivery; later revocation cannot retract an already authorized response.

Windows Muse cross-session messaging is unavailable. Continue with recorded task handoffs
and sequential repair turns; do not assume feedback messages were delivered.

## Resumed integration after the build milestone (2026-10-28)

This section supersedes the milestone attempt/status statements below. The user asked to
continue and explicitly requires flexible development configuration. All changes remain on
shared `main`; root owns integration/checks and Muse Spark 1.3 CLI contributors own scoped
implementation. No new commit or push has been made since `6d0ce07e`.

Verified during resumed integration:

- Registered imagery suite after API A6, Codex A7, config A7 and service A4: **392 unit tests
  and 2 external API tests pass** (`wave2/registered-service-a4-tests.log`). This is not backend
  acceptance: independent Codex review subsequently found protocol/ownership defects.
- Journal A8: **27 real PostgreSQL tests pass** (`wave2/store-a8-tests.log`).
- Finance A5: **17 real PostgreSQL tests and strict all-targets Clippy pass**
  (`wave2/finance-a5-tests.log`, `wave2/finance-a5-clippy.log`).
- Account-draft A4: **158 asset unit tests pass** (`wave2/assets-draft-a4-tests.log`);
  earlier real media-catalog suite **41 pass** (`wave2/draft-pg-tests.log`). Strict
  assets/providers all-targets Clippy passes (`wave2/assets-providers-a4-clippy.log`).
- Config A7: bounded overlay robustness independently approved; all 76 submitted tests
  and current-contract probes pass. One preserved historical shipped-example success
  expectation fails because unsupported typed settings are explicitly refused. Profiles,
  quality, executable, cost and runtime-control consumption remain unfinished.
- Raster A4: **61 registered tests pass**; lossy WebP, consumption and full lint acceptance
  remain open. Codec prior bounded review and transport A2 evidence remain applicable.
- Architecture guard A6: **90 unit tests pass**, default tree scan passes with 13 exact legacy
  findings; strict scan correctly fails those 13. Independent review found three additional
  whole-expression/dependency bypasses; A7 repair is active.

New source: native-only public `ImageService` request/read/watch/cancel boundary and
PostgreSQL composition for immutable media publication/finance admission linkage (migration
0039). The service currently records Planning demand only. It does not dispatch providers,
claim Ready/cache hits, schedule work, publish assets, or replace the existing callers.
Migration 0039 requires the finance/media/journal schemas; it is deliberately separate from
the standalone journal migration sequence and still needs runtime composition wiring.

Latest verified integration snapshot:

- **Native workspace, actual df-tools WASM client, and launcher all build with locked,
  offline dependencies.** Final commands are the milestone commands below with `--locked`;
  logs: `wave2/final-workspace-build.log`, `wave2/final-wasm-build.log`,
  `wave2/final-launcher-build.log`. All exit 0 (35.03s, 0.47s, 0.15s final incremental
  refresh). The preceding full current rebuild also passed (2m29s, 10.15s, 2.02s).
  All **1,925** recorded Rust/manifest/lock/SQL file hashes stayed unchanged during the
  final builds (`wave2/final-build-source-manifest.json`, `wave2/final-build-hash-check.json`).
- **433 imagery unit tests and 2 external API tests pass**, including all **89 Codex
  tests** (`wave2/registered-codex-a10-tests.log`). Independent Codex review also passes
  all eight retained protocol/import/lifecycle probes. Root subsequently corrected only
  the stale cleanup-deadline comment, then rebuilt. Cleanup drains started work and can
  outlast the request deadline; the private caller must retain/await supported cooperative
  cancellation. Arbitrary future abort/drop and the eventual service supervisor remain
  integration gates. No live paid provider/CLI image turns were performed.
- ConfigControl A2 is registered and publicly exported with patch/provenance types.
  **17 owner + 76 overlay + 5 independent external API tests pass**. Independent bounded
  review approves base/launch/runtime precedence, cumulative partial updates, revision
  fences, reset preserving launch settings, and immutable prepared exports. Application is
  local only; actual service/scheduler/egress and launcher file-saving bindings remain target
  work. The parent plan deliberately adopts A7 precedence: new imagery environment wins
  over legacy environment; explicit launch sits below runtime and survives runtime reset.
- Service A8: **10/10 real PostgreSQL tests pass** (`wave2/service-a8-tests.log`),
  including forced concurrent authorization races, cross-owner isolation, cancellation,
  restart observations, and actual committed accept followed by injected Unknown and
  recovery of a single durable receipt. Bounded independent semantics review passes;
  this remains a demand-planning API, with no provider dispatch or Ready claim.
- Admission A8: **41/41 tests pass against real PostgreSQL**
  (`wave2/admission-a8-tests.log`). Independent bounded review passes audience rejection,
  immutable historical linkage, same-transaction finance/catalog rollback and replay,
  the NOLOGIN composition role, narrow timestamp-column privileges needed for row locks,
  meaningful-write denial, and true PUBLIC ACL detection. Deployment role/migration
  composition and actual run/session authority remain host integration work.
- Guard A7: **103 unit tests + 86 independent replayed checks pass**. Default tree scan
  passes; strict mode fails exactly **13 legacy findings**. Bounded guard repairs are
  approved; consumer migration and complete enforcement are not implied.
- Formatting of **all 70 changed/new Rust files passes**, as does `git diff --check`.
  Strict all-targets Clippy still fails: `df-imagery` reports 364 library diagnostics and
  60 test diagnostics (including overlapping unused-code and style findings).
  `df-persistence` lint is blocked at its imagery dependency. Logs:
  `wave2/current-imagery-clippy.log`, `wave2/current-persistence-clippy.log`.
  No warning suppression or public executor bypass was added to force acceptance.
  Earlier assets/providers and finance strict Clippy passes remain scoped evidence.
- Development configuration details are in
  [development-configuration.md](development-configuration.md), separating the implemented
  local owner from target profiles, fixture composition, typed backend/output/cost settings
  and full runtime disable propagation.

All Muse contributors have submitted; no contributor writer remains active. Exact latest
attempts: store A8, raster A4, codec A4, overlay A7, API A6, Codex A10, composition A3,
consumer guard A7, transport A2, finance A5, account drafts A4, service A8, admission A8,
development-config A1 (amended by config-control), and config-control A2. Reviews approve
bounded semantics only where stated; required integrated lint/runtime gates still prevent
whole-feature approval. No new commit or push has been made.

Independent reviewers: `project_analysis` (admission), `review_service_boundary` (service),
`review_recipe_key` (Codex/config), `review_imagery_plan` (guard/config design). Exact current
attempts, CLI session ids, exclusive paths and logs are in
`artifacts/tmp/imagery-20261027/wave2/contributors.json`; authoritative queue is
`development/workflow.sqlite3`. Ignored evidence files must be opened by exact path rather
than assuming a search omission means missing. The portable scratch PG is running at
127.0.0.1:55471; inspect the retained owner metadata before management. No game DB is used.

The final current build/test records above supersede the historical build below. Next work
is actual service/scheduler/backend/publication and host configuration composition, migration
of every consumer, cost reporting integration and strict lint completion. Preserve the existing
passing boundary checks while doing that work. The full objective below remains open.

## Historical build milestone (2026-10-28)

Actual source: Desktop checkout on `main`, base commit `6d0ce07e1d775d8bff9f569b23826a0503e13035` plus **uncommitted** contributor repairs and coordinator wiring. All contributors submitted before the final build check; no task-owned Muse writer was found during verification. The source manifest covers 1,843 Rust/manifests/toolchain files; every recorded hash matched after the native build completed.

Successful commands (PowerShell, from the Desktop checkout):

```powershell
$env:CARGO_TARGET_DIR = 'C:\Users\mreca\Desktop\dungeonfluxGame\artifacts\build\imagery-baseline'
$env:CARGO_BUILD_JOBS = '2'
cargo build --workspace --offline
cargo build -p df-tools --lib --target wasm32-unknown-unknown --offline
cargo build --manifest-path crates/df-launcher/Cargo.toml --locked --offline --target-dir artifacts/build/launcher
```

- Native workspace: exit 0, finished in 3m 31s; `wave2/workspace-build-third.log`.
- WebAssembly client: exit 0, finished in 1m 09s; `wave2/wasm-client-build.log`.
- Desktop launcher: exit 0, finished in 1m 13s; `wave2/launcher-build.log`.
- Outputs include `artifacts/build/imagery-baseline/debug/df-server.exe`, `artifacts/build/imagery-baseline/wasm32-unknown-unknown/debug/df_tools.wasm`, and `artifacts/build/launcher/debug/dungeonflux.exe`.

All evidence paths prefixed `wave2/` are relative to `artifacts/tmp/imagery-20261027/`. Source hashes are in `wave2/build-source-manifest.json`. These ignored files are retained local evidence; the successful results and commands above are preserved here independently of them.

The imagery config overlay, intent codec, native raster executor, and private native API/Codex backends are now registered. Dependency features and the assets/finance/provider integration exports are wired. Contributors repaired ownership/lifetime errors, Windows API imports/attribute types, and provider failure-type plumbing that blocked compilation. This is compilation evidence, not proof of complete runtime migration.

Quality gates remain open: rustfmt check of the 31 changed Rust files exited 1 (`wave2/build-milestone-format.log`). Strict all-targets Clippy for df-imagery, df-assets, df-finance, df-persistence, and df-providers exited 101 (`wave2/build-milestone-clippy.log`), with 512 imagery diagnostics including unused, not-yet-consumed internals and style findings. No warning suppressions were added to pass the build. Native compilation emitted 483 imagery warnings. Do not claim a clean lint gate, full test pass, feature acceptance, or runtime smoke test from these builds.

Latest submitted repair attempts: store A8, codec A4, config A5, raster A4, API backends A3, Codex backend A4, transport A2, finance A4, account drafts A3. Codec has 52/52 focused tests plus 59/59 independent tests; assets/drafts has 158/158 unit tests; transport has 11/11 focused tests and its scoped Clippy pass. Journal A8, finance A4, config A5, raster A4, API A3 and Codex A4 still need their latest full runtime/test gates. Real PostgreSQL media-catalog tests remain pending. The owned scratch PostgreSQL was restarted on port 55471; use the retained owner metadata before managing it.

Next work: resolve formatting/lint findings, run the remaining latest-attempt tests and independent reviews, settle service composition, migrate every image-generation consumer, remove bypasses, and verify integrated behavior. The consumer guard A3 and service composition A2 remain rejected; the old paused attempt table below is historical. Do not treat this build milestone as approval of any unfinished task.

## Repository and working agreement

- Actual source checkout: `C:\Users\mreca\Desktop\dungeonfluxGame`, branch `main`.
- The chat's `C:\Users\mreca\OneDrive\Documents\ChatGPT\DungeonFluxGame` directory is a separate planning checkout. Do not test or commit the wrong repository.
- Checkpoint base: `703267289d580d933e07b623fbea6fbba4ec4578`. The checkpoint commit containing this document includes incomplete source. Find it with `git log --oneline -- planning/imagery-implementation/CONTINUATION.md`.
- User explicitly authorized shared-main work, relevant source/prompts to Muse/Meta, and up to eight **Muse Spark 1.3 contributors through Muse Code CLI with `--contributor-mode on`**. Preserve exclusive file ownership; root owns integration, tests, shared manifests/lib registrations, Git and changelog. User requires high code quality. Frontier agents review independently; they are not substitutes for the requested Muse implementers.
- All running Muse contributors were stopped and the three independent reviewers interrupted. No task-owned Muse processes remained after verification. The owned scratch PostgreSQL server was stopped cleanly; its data and all evidence were retained.
- Two contributors were interrupted without a terminal submission: finance admission and account-draft storage. Their source can be incomplete, and their workflow DB attempts still say `active`; this is stale execution state, not a live process or approval.
- No wave-2 task is approved or integrated. A checkpoint commit does not change that.

## Full objective remains

Put project-wide image generation/editing behind one `df-imagery::ImageService` API with private OpenAI, fal, BFL and Codex CLI backends. Migrate creator portraits/classification/acceptance, canonical people/rooms, display plates and edits/ahead work, items/held items, expressions, murals, combat stills, and character-clip generated frames. Remove the generic fal/public executor bypasses. Preserve private creator drafts and creation without a portrait.

Include typed text/image inputs, resolution/format/alpha controls, configuration precedence and enable/disable/tuning, durable replay/join/cancel/recovery, asset publication and authorization, separately admitted paid stages, actual usage/cost tracking without fabricated zero cost, and deterministic image transformations. Respect the approved transition/animation plan when connecting rendering; generation and presentation remain separate responsibilities. Do not narrow completion to helper crates, local-only transforms, passing unit tests or a single consumer.

Governing plans: [generation](../image-generation-architecture.md), [transformations](../image-transformations-architecture.md), the presentation/transition plan already in `planning/`, and the project's storage/finance/runtime/coding-style documents. `service-composition.md` A3 is a **partially accepted composition contract**, not implementation approval. `operation-store.md` remains the concrete journal brief.

## Accepted baseline

- `10624f047f1dd2e99ead67441f049f7220b9e846`: versioned canonical recipe keys, legacy v2 preserved. 199 df-assets tests, WASM and strict style checks passed at that revision.
- `6fe8cce4358bf76b61787384b6f4719ad6abacbd`: validated imagery request/config/output/transform foundation. 86 tests passed then; a later journal-inclusive library run passed 92. Neither count covers the unregistered candidate modules.
- `703267289d580d933e07b623fbea6fbba4ec4578`: durable operation-store brief and truthful remaining prerequisites.
- Changelog catch-up preserves 425 missing commits in 21 historical work blocks; do not discard that uncommitted-at-pause catch-up when resuming.

## Paused implementation state

The exact attempt briefs and exclusive path lists are preserved in [paused-work-items.json](paused-work-items.json). This is a durable source snapshot of the ignored workflow DB, not a claim of approval. Local prompt/log/session metadata is under `artifacts/tmp/imagery-20261027/wave2/contributors.json`.

| Area | Latest attempt | Verified state and next action |
| --- | --- | --- |
| Durable operation journal | IMG-OPERATION-STORE-A7 | Submitted, not tested. A5 ran real PG: 24/25 passed; A6 fixed the ACL test's text-to-regclass binding. Independent SQL then proved the reverse FK still allowed phantom interests. A7 claims an exact four-column reverse FK, two regressions, and Clippy fixes. Run the full suite and independent probes again. |
| Canonical intent/config codec | IMG-INTENT-CODEC-A3 | Submitted, not retested. A2: 34 passed, 18 failed. Encoder omitted prompt template_version while decoder expected it; config literal lost three zero bytes. A3 claims fixes. Independent field-by-field vectors are retained; verify them rather than replacing expectations with encoder output. |
| Configuration overlays | IMG-CONFIG-OVERLAYS-A3 | All 56 isolated tests pass. Independent review remains unfinished: handwritten TOML parsing/unknown-section behavior and unbounded copying of recognized environment values remain concerns. Not wired to launcher. |
| Raster transformations | IMG-RASTER-EXECUTOR-A3 | Compiles in isolated harness: 53 pass, 6 fail. Failures listed below. Lossy WebP still unsupported; not accepted as a permanent scope reduction. |
| API image backends | IMG-API-BACKEND-EXTRACTION-A2 | Fails isolated compile: missing lifetime on fal confined_path and BflSuccess lacks Debug for an expect_err test. A2 only removed a stray brace. Source remains private/unregistered; old executors are still live. |
| Codex image backend | IMG-CODEX-BACKEND-A2 | Fails isolated compile with ten errors. Windows process-tree ownership is explicitly stubbed to direct kill. No approval; no paid/live image turn tested. |
| HTTP/media transport | IMG-IMAGE-TRANSPORT-PORT-A1 | Registered in df-providers; five loopback tests pass. Independent exported-API probes reject it: JSON 64 KiB cap limits a requested 6 MiB image download; failed delivery drops HTTP status. |
| Consumer map/guard | IMG-CONSUMER-MIGRATION-MAP-A3 | Submitted just before stopping; not retested. A2: 48/49 tests passed, default accepted 13 legacy sites, strict rejected 13. Independent bypass fixtures rejected A2. |
| Finance image-stage hook | IMG-FINANCE-IMAGE-STAGE-A1 | Interrupted with no submission. Changes in admit.rs/service.rs plus new image_stage.rs/tests. Existing modules reference crate::image_stage but lib.rs is not registered: expect a build break until completed/wired. |
| Account-draft storage | IMG-ACCOUNT-DRAFT-STORAGE-A1 | Interrupted with no submission. Partial df-assets catalog/object-store and persistence admission edits. Shared exports, recovery coverage and tests are incomplete/unverified. |
| Service composition | IMG-SERVICE-COMPOSITION-A2 | Independently rejected. No production ImageService exists yet. Reviewer was preparing a narrower service-boundary brief when interrupted; no completed brief was found at pause. |

## Concrete remaining defects

### Journal and migration

Keep 0038 as the imagery migration; 0037 is already the campaign-preferences migration. Both receipt-to-interest and interest-to-receipt links must enforce exactly `(owner_scope, operation_id, job_id, interest_id)` with deferred FKs, so accept can atomically create the cycle but no extra attached demand can commit. Test extra interests for the same job and another same-owner job. Maintain full-u32 snapshot revisions via bigint, all-or-none frozen facts even for retired jobs, zero-ID rejection, ReadCommitted admission and strict row-decoder linkage validation. A7 has not been executed. Previous strict Clippy failed a nine-argument parser helper and collapsible if; A7 claims both repaired.

### Codec

Independent expected Generate fixture: 129 bytes, SHA256 `007b57321501d9517e8209698d8f27c01bd6cf31d303cb00ee26e57e38eccfb8`.
Independent expected default config: 204 bytes, SHA256 `7453fdb8b64ddf8d204291e52da9e94599595b77279ac9898884e7444dcd2ea8`.
Keep template version, all material request/config fields, ordered references, set canonicalization, allocation-before-budget checks and negative-zero canonicalization. Independent review still wanted actual `to_bits()==0` decode proof, error Debug/Display privacy and isolated material-field sensitivity tests. A3 also changed an empty-transform test expectation; verify against accepted contracts.

### Raster

A3 failing tests: arbitrary_right_angle_matches_the_exact_quarter_turn; composite_half_opacity_blends_linearly; crop_then_resize_differs_from_resize_then_crop; encoded_output_over_limit_is_refused; encoded_output_stops_at_the_output_limit; mask_dimension_mismatch_is_refused.

The half-opacity fixture expects encoded 128 but implementation produces linear-light-correct approximately 188; resolve that expectation analytically. The right-angle fixture must distinguish fixed versus expanded canvas. Do not weaken failure assertions to make green. Verify bounded encoder errors map to OutputTooLarge, mask alignment semantics and truly noncommuting order fixtures. Preserve linear premultiplied resampling/composition, ICC conversion/refusal, memory accounting and cancellation. Existing image 0.25.10/image-webp encoder is lossless-only; evaluate one pinned lossy encoder if required, not silent permanent refusal. Full typed coverage/render recipe linkage remains unfinished.

### Transport and API backends

`fetch_bfl_delivery` currently reuses submit/poll NativeHttp response limits. Independent 128 KiB loopback body failed with the JSON 64 KiB limit despite caller requesting 6 MiB; it succeeded with dedicated delivery limits. Use validated delivery-specific response limits, preserve total remaining deadline, header/chunk/DNS controls and no credentials. Reject unsupported limits explicitly instead of silent clamping.

The error tuple loses observed HTTP status: a 403 became Unknown/fetch_failed. Coordinate a typed failure carrying refusal, static reason and optional HTTP status with the BFL backend caller. Retain allowlists, public-address pinning, remote-address checks, redirect/proxy refusal and fixtures. Raw NativeHttp fixture mode is not itself a loopback restriction; do not repeat stronger handoff claims.

API compiler repair: give fal confined_path a return lifetime tied to its URL; use redacting Debug or non-Debug-dependent test matching for BflSuccess. Recheck all loopback tests afterward. Split fal generation and paid alpha removal into separately admitted stages, never a hidden second paid call. Remove old public executors and generic fal submit_image only after every supported caller migrates through the service.

### Codex

A2 compiler failures: RpcId::Number expects i64 but request constants are u64 (three locations); owned_root moved into a blocking closure then reused; PngEncoder::write_image called on &encoder instead of consuming it (several fixtures); FakeSession lacks Debug for unwrap_err. Clean up genuinely unused imports/variables.

Replace the Windows direct-kill stub with real owned process-tree cleanup. Existing `df-tools/src/gameplay/codex_cli/windows_process.rs` has a race-free Windows job implementation to inspect/reuse at the correct dependency layer; do not import df-tools into imagery or copy text-only image prohibitions. Preserve bounded stdin/stdout/stderr, total deadlines, turn/item correlation, failure precedence, staged-reference/output containment, actual dimensions/format/alpha validation and usage retained through uncertain outcomes. Review blocking file verification cancellation and symlink replacement races. Do not expose a public backend bypass.

A read-only installed-CLI initialize/initialized handshake succeeded without starting a turn (evidence retained). Newline framing and initialization were observed; image capabilities, result encoding and fileId retrieval were not proven. The handoff's claim that framing was wholly unproven is stale. Do not fabricate fileId endpoints or costs. No live image-provider calls were made.

### Consumer guard

A2 bypasses: cfg(test) exemptions leaking into following production code; unrelated nearby loopback calls exempting live constructors; `_provider` route payload mistaken for wildcard; grouped aliases; indirect backend reexports; workspace-renamed forbidden dependencies; file-wide legacy markers laundering replacement callsites. A3 claims fixes, but no execution/review occurred. Preserve a default mode limited to exact legacy sites and strict mode that fails until migration is finished. The guard is not proof against arbitrary HTTP/script launches; audit dependencies/exports and full consumer inventory too.

### Finance, account drafts and full service

Finance contributor owns admit.rs, service.rs, image_stage.rs and focused tests. Add per-call injected transaction hook while preserving FinanceService's constructor/ordinary admit. NEW versus REPLAY comes from the finance receipt decision; replay verifies exact immutable linkage and never repairs/admit/sends. Bind job/stage/owner/asset/publication-attempt and child operation/attempt/effect/tenant/campaign/session into the digest before finance admission. Expose the same digest for quotes. Stamp media_asset_id in the same permitted finance plan/event. Real PG tests must prove rollback, exact replay, mismatch refusal and commit uncertainty. Never invent a parallel money ledger, free unknown cost or fake account-draft campaign/session.

Account-draft storage: derive `StorageOwner::AccountDraft(account)` from canonical catalog fields. Require account quota/origin/dedupe, OwnerPrivate audience, Personal retention and request key for generated no-campaign images. Key layout should be `<env>/users/<account>/draft-assets/<asset>/<attempt>/<write>/<variant>`. Existing schema supports this; no migration was needed in the reviewed brief. Validate service, fake and direct SQL admission; protect forced cross-owner dedupe conflicts. Thread ownership through advance/recovery, publication and missing-winner rewrites. Reuse account byte quotas and tombstones; no invented draft TTL. Storage admission is not spend authorization. Authenticated request/read/watch/cancel/discard/acceptance and hosted draft funding remain required service work.

Rejected service draft defects: sent marker must be durable before supplier I/O; complete finance refusal/Unknown types; local transforms must not require fake finance IDs; stage versus retry-attempt identity and lease/CAS constraints; catalog-sweep parent versus child attempt aggregation; authorization on stored reads/watch/cancel; Ready only from verified winning publication facts; one stable cost row per attempt with idempotent usage revisions; real PG transaction tests. These remain blocking full-service integration work.

## Integration ownership and resumption order

1. Confirm the user resumed; use the Desktop repo on main. Read AGENTS/coding-style and governing plans. Check Git status and the checkpoint; preserve partial edits.
2. Reconcile interrupted finance/account-draft attempts and leases using authoritative logs/process inspection. Neither is running. Retain failed/rejected attempt history; do not blindly rerun registration scripts or create duplicate owners. Other latest attempts are submitted, not approved. The local SQLite DB is ignored; paused-work-items.json preserves the unfinished briefs for another machine.
3. Restore the owned scratch PG server only when needed; validate paths/port before start. Run A7 journal tests and independent integrity probes, then Clippy/fmt and full migration/runtime-role checks. Do not reuse 24/25 as a current pass.
4. Refresh codec A3 harness and independently verify full vectors/roundtrips. Resolve config review findings despite 56 unit passes. Repair raster/API/Codex from current evidence. Source-hash snapshots before tests and recheck after shared dependencies change.
5. Complete the interrupted finance and draft-storage implementations, register only necessary narrow exports/modules, and run real PG plus compatibility tests. Do not interpret the paused workspace as compilable: finance module and draft helper exports are not wired.
6. Freeze and implement actual ImageService authorization/request/read/cancel and durable stage dispatch/publication composition. Integrate config, provider execution, transforms, finance and stable cost projections through it. Old gameplay paths are still present; no consumer is considered migrated merely because a backend was copied.
7. Migrate every consumer and remove direct image-provider/public bypasses. Run strict boundary checks, native/WASM lint/compile/tests and real browser creator/plate/item acceptance. Apply the architecture's performance/recovery checks. Mark the persistent goal complete only after full requirement-by-requirement evidence.

## Commands, evidence and environment

Muse executable: `C:\Users\mreca\AppData\Local\Programs\muse\muse.cmd`. At pause the actual launcher resolved to muse-bin-1.4.14-R7309.1.exe (earlier turns used 1.4.13). Inspect current CLI help rather than assume a version. Typical invocation uses `exec --model muse-spark-1.3 --contributor-mode on --reasoning-effort high --workspace <Desktop repo> --trust-workspace --approval-mode never --disable-shell --disable-reminders --disable-request-user-input --session-id <fresh UUID> --json --prompt-file <owned brief>` with bounded steps. Shell was disabled because the Muse Windows shell sandbox was unavailable; root runs checks. No model substitution was authorized by the user's Muse request.

Evidence root: `artifacts/tmp/imagery-20261027/`; wave files under `wave2/`. Retain all current evidence/builds/databases. `pause-state.json` and `pause-owned-processes.json` describe shutdown; `contributors.json` has prompt/log/session IDs. Some exec handles may be stale after pause: never infer liveness from them. The three accidentally written `tmp/imagery-20261027/wave2/*-handoff.md` files were preserved under artifacts as `pause-raw-*`, not discarded or committed as application source.

Relevant logs: store-a5-pg-tests.log; store-a6-independent-review.json/probes.log; integrated-clippy-first.log; codec-returned-tests.log; codec-a2-independent-fixtures.json; config-a3-tests.log; raster-next-tests.log; api-next-tests.log; codex-next-tests.log; transport-a1-tests.log; transport-port-a1-review.json; consumer-migration-a2-review.json; service-composition-a2-review.json. Partial later reviewer artifacts may exist; inspect timestamps/hashes before relying on them.

Root build directory: `artifacts/build/imagery-baseline`, CARGO_BUILD_JOBS=2. Isolated harnesses live at `wave2/{codec,config,raster,codex,api}-harness`. `snapshot-harness.py` supports codec/config/raster/codex; API snapshot was refreshed separately. These compile byte-identical private candidates with conventional module layout; they do not replace final registered-crate gates.

Scratch PostgreSQL 18.6: bin `artifacts/build/imagery-postgres-18.6/pgsql/bin`, data `artifacts/tmp/imagery-20261027/postgres-data`, localhost55471, postgres user, owned test-only trust configuration. Server is **stopped**; old PID in postgres-owner.json is historical. Never touch game/other database ports. For persistence tests set DUNGEONFLUX_RUNTIME_PG_PORT=55471, HOST=127.0.0.1, USER=postgres and DUNGEONFLUX_REQUIRE_SCRATCH_PG=1; use a unique TMPDIR under artifacts. Finance tests use DUNGEONFLUX_FINANCE_PG_PORT/HOST/USER; ensure tests really run, not skip.

Focused commands after restoring prerequisites:

```powershell
cargo test -p df-persistence --offline --test imagery_operations -- --test-threads=2
cargo test --manifest-path artifacts/tmp/imagery-20261027/wave2/codec-harness/Cargo.toml --offline --lib
cargo clippy -p df-imagery -p df-persistence -p df-providers --offline --all-targets -- -D warnings
```

No passing all-workspace or end-to-end result exists for this checkpoint. New unregistered modules and interrupted exports can break compilation. Do not run broad rustfmt over active contributor files on resume; root formats only quiescent owned/integrated paths.

The screenshot-journey recorder could not run on native Windows: development/screenshot_journey.py imports fcntl and raised ModuleNotFoundError. No screenshots were fabricated. Nonmutating cleanup inventory found no proven eligible deletion; evidence and local databases were retained.

## Owner funding decision (2026-10-28)

Hosted character portrait generation before campaign membership uses the creator account balance, with explicit consent and a spending limit. No platform allowance or invented campaign payer. Creator funding implementation is tracked as IMG-CREATOR-IMAGE-FUNDING-20261028; consent/quoted cap is required before paid dispatch.

## Integration checkpoint: text plan and Sol work (2026-10-28)

The user approved the exact creator-account funding source migration and disposable-PG tests in creator-funding-approval.md, and explicitly delegated implementation to Sol contributors with root reviewing/integrating. Muse exhausted weekly quota (429); no retries or spending bypass. Current actual source remains Desktop dungeonfluxGame shared main, HEAD6d0ce07 plus uncommitted work. No new commit/push.

New text-generation request is planned in planning/text-generation-architecture.md and text-generation-surface-map.md. Dedicated Sol author and independent source reviewer completed two passes; provider/feature/test coverage, phase dependency and semantic-qualification feedback reviewed. Artifacts are artifacts/tmp/text-generation-20261028, with R1 retained. Plans only; no text application implementation.

Image work remains incomplete. Current Sol owners: image_runtime (singleAPI runtime and migration0044 rawusage/results; crash ordering raw observation before complete, durable rawoutput recovery), atomic_publication (0042, byte proofs and concrete publisher; configurable proof limits, alpha requirement distinct from observed fact, output role guarded local allocation), creator_funding (0043 approved finance + catalog), identity_repair (0041 liveidentity/rootintegration), repair_execution (new recovery enumeration/claim extension0040), repair_cost (bounded cold-log reader/cost integration). Source files owned per contributors.json and workflow DB. Do not clobber these agents. Consumer migration still must be dispatched after runtime API stabilizes; strict13legacybypasses remain until actualcutover. No fullfeature/build/lint completion claim.

Verified bounded evidence: API56fixtures; config221+independent strictscopedlint; raster90plusnativeallocationprobes (metadatafix last); execution baseline32PG+18units, recoveryextension pending; hosted16PG+17config+packageClippy including realHosted/narrowroles/media-ledgeradoption; finance6PG+13pure+financeClippy, expandedtests pending; cost114actualBook+6neutral, boundedreaderfinalrerun pending. Logs/sourcehashes in wave2. These are not currentwholeworkspaceresults.

Root integrations now include native dependencies/exports, service legacy config-envelope regression, df-persistence media_catalog::open_existing and migrate_in shared canonicalmedia migration/adoption from historical hostedledger; hosted database delegates10..14. Root fixed delivery bodypermit field and fixture12argconstructor to DeliveryBasisFixture, portalUri/headerimports, proofexports, identitymodules+owned55471runtimeharness, financeevent/accountcode/allowlist patch (account2150 and unknown event), removed unnecessaryfinance dev-backedge. Root added outputpublisher module/native sha2, imagery runtime/draftadmission modules. Ensure currentCargo.lock refreshed by activechecks. Onlydisposable ownedPG127.0.0.1:55471postgres; never55911. SourcewriteDesktop needs require_escalated.

Next root steps: integrate any remaining shared hooks from activeworkers; independent reviews of finalimmutablehashes; assemble realhost/gameplayruntime and migrate all imageproviders/consumers; strictboundaryzero and appropriate endtoend actualbytes; fullnative/WASM/build/lint with no suppression, updatechangelog/docs/workflow. Fix df-tools Service startup bindings and consumerReady handles rather than declaring helpertests sufficient. Cost startup existingwhole-log replay memory is separatelyrecorded followup, not claimedbounded.
