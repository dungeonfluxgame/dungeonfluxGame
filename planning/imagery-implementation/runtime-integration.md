# Native image runtime integration - IMG-RUNTIME-INTEGRATION-20261028-A1

Status: resumed full implementation and integration. The coordinator serializes native builds; prior partial evidence is preserved. Whole-feature acceptance remains pending observed checks of the graph below.

## Resumed full pipeline contract (2026-10-29)

The current attempt is `IMG-RUNTIME-INTEGRATION-20261028-RESUME-20261029-SOL`.
The prior evidence remains retained; the coordinator serializes builds. The target graph is:

1. Accept the immutable intent, configuration and a separate urgency hint with the first stage in
   one transaction. Rating-only metadata is valid; explicit regeneration never reuses a result.
2. If requested, run paid creator Classification while the job is still Planning. Its configured
   analysis account/model/endpoint are distinct from image credentials. Commit raw model response
   and usage, then a bounded versioned DecisionObservation with exact source/config/stage basis.
   General selects the configured general backend; adult nonsexual anatomy detail selects BFL;
   nudity/violence require the frozen Mature rating. Revise/Uncertain terminate visibly without
   rewriting the request or sending generation.
3. Run Generation/Edit, with immutable authorized references and explicit rating/quality/model.
   Commit raw image file bytes and usage before validating a Candidate receipt. Intermediate
   candidates never claim a catalog Ready asset or fabricate an object checkpoint.
4. Run any required separately paid Inspection against the exact candidate. Preserve photograph,
   empty-room/person count, party identity count, public hand custody and visible-feature evidence.
   One explicit Correction child may regenerate under a new dispatch/finance identity; its next
   Inspection retains the established visible-text exception. Every child has a same-job parent
   dispatch and an idempotent `(job,parent,kind)` logical-stage key.
5. When required, perform fal alpha Extraction as another admitted submit/checkpoint/resume child;
   generation and extraction are separate supplier liabilities. No merged billable call is hidden.
6. Select the winning candidate and exact interpreted decision evidence. A final LocalTransform
   freezes the full canonical execution recipe, performs configured normalization, writes the
   durable winning Master/optional explicit Display objects, and records their exact proofs.
   Intermediate decisions/candidates and final object publication use distinct typed completions.
7. Explicit final selection binds the final stage/dispatch to the existing planned publication
   pair and original allocation anchor. It cannot relabel a correction as the first generation,
   allocate a second quota hold, or substitute another candidate. Atomic publication changes
   catalog Ready and the imagery result together. Only then may authenticated read/watch expose
   Ready and the result.

Every billable child uses the same actual prepared CostRoute for finance admission and cost
projection. A borrowed DispatchAdmission supplies exact job/stage/dispatch/claim/config/quote;
finance can prepare that dispatch in its own admission transaction. Only SendAuthorization::New
permits egress; AlreadySent preserves Unknown. Raw usage persists before normalization/completion;
settlement and projection replay the same attempt/revision. Local CPU stages carry no supplier
attempt, including final normalization in Hosted mode, and must verify the existing funded anchor.

Recovery first retries retained usage projection, then exclusively reconciles the same dispatch.
It consumes durable raw responses/candidates/decisions and appends each next child once. Unknown
without a resumable receipt never resends. Cancel withdraws interest, prevents new children and
preserves sent liability; valid late paid output stays private. Recovery acknowledgements require
actual terminal settlement plus the correct decision/candidate/child or final Ready receipt.

The sole consumer boundary remains ImageService request/read/watch/cancel, optionally erased as
ImageServiceApi for one host injection. Safe execution states and published evidence are read
only after current stored authorization. Shared typed observer events are content-free and
non-gating; mandatory finance/execution journals remain separate egress gates.

## Implemented boundary

`ImageService<J,C,I,A,R=NoImageResults>` retains request/read/watch/cancel and adds `with_results(R)`. Authorized reads attach a durable `PublishedImage` only for attached interests and a frozen job. Withdrawn interests and private reconciliations expose no result. An independently read publication ahead of the journal snapshot returns Unknown with the stable handle. Planning-only construction never claims a result.

`PostgresRuntimeJournal` accepts demand and reserves its execution queue in one PostgreSQL transaction using `accept_with_execution_in`. Its explicit local-accounting selection must match the retained config. Service replay reads existing operation identity before minting reservation ids. The domain crate does not import persistence.

`NativeImageRuntime` consumes the durable queue through private OpenAI/fal/BFL/Codex/raster executors. Source bytes are resolved under the infrastructure authority and digest-checked before admission. Credentials are validated and pinned before admission. Every fresh transport follows admission, prepare and durable Sent. Unknown never creates another dispatch or falls back to another backend. Fal/BFL submit checkpoints are retained immediately and recovery uses only resume methods. Codex execution remains owned and awaited; no outer abort timeout discards its cleanup.

The runtime returns an owned work future plus `RuntimeHandle`; the host retains that future in its task owner and calls shutdown to await cleanup. No task is detached internally. Supplier refusal/uncertainty already recorded by the worker does not stop other queue work; infrastructure failures remain visible to the host owner.

`FrozenOutput` includes canonical intent/config, resolved backend/model, and raster/WebP/Codex adapter revisions. Current execution produces one delivered Master variant and explicitly freezes no Display variant. The publisher receives `PreparedImageOutput` with exactly those variants and never invents a resize policy.

## Raw usage and recovery

Migration 0044 and `PostgresImageUsage` retain immutable route, monotonically versioned units/evidence, actual provider output bytes and separate projection acknowledgement. Unknown projects route maxima. Observations commit before normalization, execution success or dropping the sole provider response. Raw output is redacted in Debug. One dedicated usage role can insert/read observations and update acknowledgement; it cannot read finance. Acknowledgement never deletes raw evidence.

Success order is durable raw response -> protected Candidate/Decision and child qualification -> normalize -> durable final object/proof -> execution checkpoint/success -> atomic publication Ready/result -> exact economic disposition of image supplier ancestors. Cost projection acknowledges the same retained raw revision independently; a projection failure never fabricates Unknown billing. Useful decision stages settle from a protected replay of their actual retained response, independently of image delivery. Lost raw-response acknowledgement leaves the same Sent dispatch recoverable. The execution owner's distinct recovery lease lets the worker reconcile expired/retired jobs without a fresh send. Actual persisted raw output is preferred to any supplier network recovery. The final output checkpoint names the durable winning Master object/version/proof.

## Prior verified evidence (before the resumed integration)

* `cargo check -p df-imagery -j 2 --target-dir artifacts/build/image-runtime-agent` passed before the last small guard refinements.
* `cargo check -p df-persistence -j 2 --target-dir artifacts/build/image-runtime-agent` passed before the last coordinated recovery changes.
* Focused `df-imagery` service tests: 29 passed. Includes authorized published-result read/watch, withdrawal filtering, and current-version envelope mismatch validation.
* Real scratch PostgreSQL runtime tests: two passed. First executes one real private raster transform through ImageService, durable PG queue, NativeFileStore and atomic catalog publication, then checks read/watch/restart, one dispatch and zero finance attempts. Second verifies raw response/units persistence, exact replay/conflict, restricted-role permissions and acknowledgement retention across restart.
* The new actual OpenAI loopback test uses a genuine 1024x1024 RGB PNG, real private HTTP execution, raw PG receipt and native publication. It forces lost raw acknowledgement, lost output acknowledgement, projection failure and cancellation. Its latest run FAILED during the first restart at `Execution(OperationConflict)`; the remaining failure scenarios have not passed.
* Mandatory Clippy was run and failed. Two owned runtime findings were repaired (eight-argument dispatch replaced by typed input and nested replay guard collapsed). Shared relocated backend/Codex dead-code and style findings remain. No lint suppression was added.

Latest retained logs are `artifacts/tmp/imagery-20261027/wave2/runtime-api.log`, `runtime-pg.log`, `runtime-build.log`, and `runtime-clippy.log`. PostgreSQL fixtures use only 127.0.0.1:55471, postgres, explicit scratch opt-in and unique df_runtime databases. Global role-install fixture DDL now uses the shared advisory lock. Failed test databases were retained; successful fixtures dropped their own database. Native image bytes remain under artifacts/tmp/runtime-test.

## Resumed observed baseline (2026-10-29)

The integrated imagery library passed 597/597 tests in `runtime/lib-baseline-2.log`.
The actual PostgreSQL runtime suite passed 3/3 tests in `runtime/pg-baseline-3.log`;
the HTTP test executes all four crash scenarios: raw acknowledgement loss, output
acknowledgement loss, projection failure and cancelled raw acknowledgement loss.
Each preserves one actual provider POST and dispatch, exact raw output and private late
reconciliation. The local transform test also verifies actual shared observer records and
that a diagnostic sink refusal cannot prevent durable publication. The tested PG binary
SHA256 is `D509E7A7C7BA5812343BD0CF7B467C4274BF33CECED2D33D8CE3EC867C46AF22`.
These results precede the mandatory exact source receipt and full graph extensions.

The acceptance adapter now rereads the demand view inside the same transaction after
queue reservation bumps the job revision. Strict observation revision checks remain.
Definitive unusable supplier results bind their retained usage revision; Unknown never
becomes a zero-cost or terminal failure inference. Known Codex failure usage is retained,
including its separate cumulative total, and never discarded because image bytes failed.

## Default-stack observed integration (2026-10-29)

`runtime/pg-default-stack-2.log` passed all 11 real PostgreSQL runtime tests in 50.72 seconds
with `RUST_MIN_STACK` unset. The binary SHA256 is
`55f229ced8681faf0d123b81c31729177cc6cc6a6e23bd99ef5ff41cca9cc523`.
The suite includes actual classification raw-ACK recovery, cancelled and expired qualified
candidate private finalization, local durable publication/observer refusal, immutable raw usage,
latest metadata usage projection, real historical v10/v11 Ready reuse under v12, and zero-backoff
queue fairness. The direct provider test executes five cases with one POST/dispatch each:
live raw ACK loss, output ACK loss, cost projection failure, cancelled raw ACK loss, and expired
raw ACK loss. Private late results remain hidden while timely public results remain readable.

The first enlarged-stack diagnostic exposed a real orchestration stack risk. Production owned
orchestration boundaries now use bounded heap allocation, without detached tasks. Actual measured
future sizes changed from 37,872 to 152 bytes for `run_next`, 41,184 to 3,584 for `start`, and
132,320 to 31,440 for the complete classification fixture. Regression budgets and all execution
cases pass on the default stack; the earlier 16 MiB diagnostic is not acceptance evidence.

This observed snapshot precedes the new queued operational notice, preflight rejection,
SourceDenied closure, atomic unsent cost-book marker and campaign artifact extensions. Those
require their own subsequent empirical gates. Core imagery tests previously passed 820/820;
new source is not attributed to that earlier binary.

## Current source and verification boundary (2026-10-29)

The source now connects the image graph, standalone typed analysis, exact source snapshots,
winning inspection/staging and actual usage projections, compatible Ready reuse, queue paging,
and guarded NoDelivery/unused cleanup. These are implementation statements, not whole-feature
acceptance. The latest coordinator native all-target check passed before the newly mounted
quota and finite-cleanup hooks; the current batch needs a new check and actual PostgreSQL run.

Quota admission uses one host-owned registry across jobs, priority pools and config rebuilds.
Its key is the canonical actual supplier origin plus the pinned credential or trusted custody
identity. New supplier stages check before finance and revalidate after capacity. Only an actual
new structured quota response opens the guard; only a valid new response can close a probe.
Cache reads and GET-only recovery have no permit. Pending file-backed cost projection retains
its exact raw revision and retries without a false durable acknowledgement, Ready, or Unknown.
Older raw revisions remain immutable but only the newest revision is pending projection.

Cancellation cleanup consumes bounded paged actual Prepared or finite no-dispatch proofs.
Live expired Prepared work retains its hold. A real terminal job is sealed through publication
before exact unused finance release; candidate dispositions come from immutable Ready/NoDelivery
lineage. Revoked authority remains pending and does not stop unrelated queue work. Independent
queue and cleanup cursors advance beyond blocked prefixes; the new actual PG zero-backoff
regression is source-ready and has not run.

Remaining acceptance work, in dependency order:

1. Compile the complete current source, then run full core and real native PG/host suites.
   The source-ready creator/classification, inspection/correction, analysis and historical-codec
   cache tests must produce actual evidence; prior three-test baseline does not prove them.
2. Connect and test explicit local recovery finalization after cancellation/deadline for an
   already qualified paid candidate, using prepare_reconciliation then the source-locked Local
   send gate. Missing inspection must not become a fabricated Pass or local checkpoint.
3. Preserve closed supplier failure causes independently of Unknown financial liability. Safe
   automatic retry still requires a retained proven-declined response and a fresh admitted
   attempt; Sent/Unknown may never be resent or treated as an estimated zero-use result.
4. Verify real quota cooldown/probe behavior across distinct intents and config rebuilds.
   Same-probe safe rate-limit retry remains unverified until the declined-response retry lane
   is implemented; the private registry tests alone do not prove runtime transport behavior.
5. Exercise actual NoOutput, NoDelivery, local negative candidate economics, every unsent cleanup
   state, source revocation and crash recovery. Run strict affected Clippy/native/WASM checks
   after behavioral gates; no dead-code suppression or fake completion is acceptable.

Builds remain serialized by the coordinator. Only owned scratch PostgreSQL55471 and
loopback supplier fixtures are authorized; no paid or production calls are used.

## Producer closure checkpoint (2026-10-29, after core 846)

The coordinator observed 846/846 core tests before this producer batch. The default-stack native
11/11 snapshot above remains the latest executed runtime suite; none of the following newer
source is credited to that binary.

The current source now retains exact schema-2 API failures including raw Retry-After values.
Only a protected documented rejected OpenAI/Fal submit response can complete as Declined.
Execution atomically selects either an eligible pending retry marker/reserve or a final decline;
a final visible failure cannot later acquire a retry marker. An iterative native sequence retains
one quota permit across retries, while each dispatch gets a fresh source/gate/demand check,
independent quote and admission. Unknown supplier maxima and all old finance holds remain.
Historical decline recovery projects the exact retained raw revision and performs no network I/O.

Protected preflight closure uses a real current claim with no dispatch, finance binding or raw
supplier record. Missing credentials and unsupported profiles have distinct finite reasons;
global invalid configuration remains fatal. Existing BestEffort inspection missing-credential
handling records only its separate local Unjudged checkpoint. Candidate/source denial closure
uses real queued claims or actual dispatch/recovery ownership, and native publication derives
current denial under authority/catalog locks. No caller supplies a denial boolean or fake attempt.

Never-sent cleanup now calls the actual Book unsent sealer outside database locks. Only the exact
durable tombstone produces the private receipt required by execution's cost-unsent ACK. Pending
or unavailable sealers do not acknowledge, create usage, or imply zero supplier cost. CPU-only
local transforms use execution's narrow existing kind/accounting exemption.

Known billing with missing Codex output remains Reported in raw storage and the Book. NoArtifact
or LostOutput does not convert it to unknown billing or authorize a resend. Projection precedes
a protected economic-pending response; finance remains fully held until actual disposition proof.

Remaining source dependencies are final-decline output-only NoDelivery (unknown finance retained)
and campaign-artifact allocation/election callbacks. Current preflight, decline, quota, Book and
source-closure producers still require fresh compile and actual native/host PostgreSQL evidence,
including two-job liveness and same-probe strict429 cap/backoff regressions. Default-sealer refusal
must remain visible in fixtures which do not compose the actual durable Book adapter.

### Final output-only decline and artifact admission source

The final-decline callback is now mounted for both the real original claim and actual recovery
lease. Native publication compares the exact retained decline receipt, requires no pending retry
marker and the actual graph tail, then seals only delivery capacity. Unknown supplier finance
remains held; actual candidate ancestors are settled only through the committed output proof.

The artifact port now returns Admitted, ArtifactPending or ArtifactResolved before prepare/Book/
send. It defaults to Unsupported for eligible artwork without an election owner. The runtime
uses the shared campaign semantic key while retaining its complete original execution recipe;
Pending/Resolved create no runtime dispatch or Book entry. Native/funded election writer and host
forwarding must close together before this extension is accepted. In particular, a catalog loser
must roll back the entire financial admission and recheck current authority in a new wait/read
transaction; commit uncertainty is not an election loss or retry authorization.
