# Imagery implementation continuation

Status: **paused at the user's request; unfinished checkpoint, not a release or accepted integration.**

The user requested a continuation document and then a commit of the current work. No push was requested. Do not resume implementation until the user resumes it. This document records the paused state; repository instructions remain subject to the user's directions.

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

Governing plans: [generation](../image-generation-architecture.md), [transformations](../image-transformations-architecture.md), the presentation/transition plan already in `planning/`, and the project's storage/finance/runtime/coding-style documents. `service-composition.md` is a **rejected draft**, not a frozen implementation contract. `operation-store.md` remains the concrete journal brief.

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
