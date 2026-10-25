# Hosted runtime and production composition: design note (O1 round 0g)

Date: 2026-10-24. Author: launch agent O1. Status: design, revised after the round 0 and 0b
critics (REVISE, 14, 13, 12, 8, 7, then F alignment) and coordinator rulings D21-D27; no code changed. Money
core references are to F (D27); B keeps identity, the portal and processor records. F is
authoritative for every finance table and function (`planning/finance-ledger-research.md`); this
note states only what O1 needs from F (10.2).
Governing text: `planning/master-launch-plan.md` (cited as "plan N.N"), coordinator decisions in
`artifacts/tmp/agents/LAUNCH.md` (cited as "D1".."D20"), `planning/coding-style.md`,
`planning/runtime-reliability.md`, `planning/server-concurrency-research.md`,
`planning/service-operations.md`, `planning/subsystem-architecture.md`, and the peer notes
`planning/account-service-research.md` (B), `planning/campaigns-and-table-research.md` (S1),
`planning/asset-storage-research.md` (O2, section 14 is its contract with this note),
`artifacts/tmp/agents/S2/round-0.md`, `artifacts/tmp/agents/H4/metrics-gap.md`.

Scope (LAUNCH.md, row O1): the `df-server` production composition root, a trusted campaign
runner, the campaign owner directory and its fence, edge routing to the current owner,
drain/checkpoint/transfer, session lifecycle states and meter segments (plan 4.1), the
blocking-wait signal, the save/resume contract (plan 13.7), the egress dispatcher contract (D18),
the local-campaign importer contract (D19) and the production metrics surface (D20). Plan 1.3,
6.2, 12.1, 12.2, 12.5-12.7, 13.7, 13.10, 14.1, 15, 16.1-16.3, 17 (TIME, SAVE, OPS, AUTH-01/02),
18 and journeys J04, J07, J08, J10.

Values marked **(proposed)** are owner-pending; they ship as configuration with these defaults.

## 1. What exists today (verified in the tree)

1. **`df-server` is a contract skeleton, never compiled.** `src/` and `tests/`, no `Cargo.toml`;
   created 2026-10-10 (`b19e85fb`, `a0af961c`, `efd5a54d`) as `composition_registry.rs` (an
   ownership witness whose `native_admission` always refuses), `deployment.rs`
   (`MultiNode` refused as `MultiNodeUnqualified` "until durable owner routing and fencing are
   qualified") and `shutdown_policy.rs` (`AdmissionPolicy` Serving/Incident/ShuttingDown). Tests
   include the library through `#[path = "../src/lib.rs"]`. It was never wired because the game
   grew inside `df-tools`; RP-141 in `planning/issue-triage.md` records it. This note restores it.
2. **The running server is `df-tools` `gameplay::serve`** (`gameplay.rs:1164`) behind the
   `df-transport-fixture` binary. `serve_run` (`gameplay.rs:1266-3188`, about 1,900 lines) is
   composed from `DF_GAMEPLAY_*` variables and positional arguments. `run_restart::serve` moves a
   new adventure to a successor database (`df_gameplay_demo_next_<hash>`).
3. **One owner thread per process** draining the bounded inbox (256 items, 2 MiB) over 24
   `actor::Call` variants (`actor.rs:372-521`); about 125 non-test process-wide statics,
   `OnceLock`s and `thread_local!`s. One process per campaign (D6) avoids auditing all of them.
4. **Durable game state** is `df_game` (`0001_game_decisions.sql`), keyed by
   `(tenant_id, session_id)` with `FORCE ROW LEVEL SECURITY` but no policies and no `campaign_id`
   column. A full envelope is written per revision and never deleted. Commits recheck the fence
   and a database-clock lease in one transaction (`df-session/src/submission.rs:213-221`).
5. **Story memory** is `df_story` (`0003`, `0004`, pgvector optional): keyed by `run_id text`, no
   tenant column, no row security, and its DDL runs on every start
   (`story_memory.rs:436-456`, advisory lock 7283410001).
6. **The fence is a random 16-byte value** (`gameplay.rs:1664`), claimed in
   `local_demo_scope/startup.rs` `restore` when the old lease has expired; lease 300 s, renewed on
   the owner thread (`actor.rs:2170`) whether or not the game is progressing.
7. **Identity is local development authority:** constants `TENANT`, `CAMPAIGN`, `PLAYER`,
   `DISPLAY`; per-run random credentials sent by the browser as `x-df-local-binding`; room-code
   join and seat reclaim after 30 s away (`seats.rs`); database name must start with
   `df_gameplay_demo_`. A1 fixed restore of expired grants (`RETAINED_GRANT_SQL`).
8. **Durable game facts on local disk:** campaign records (`adventure/campaign_store.rs`),
   notebook (`notebook.json`), social ledger, local lore, the cost log
   (`development/runs/<database>/cost.jsonl`), hero library caches.
9. **Every PostgreSQL connection is `NoTls`** (`df-persistence`: `account_store.rs`,
   `hero_library.rs`, `native_bridge.rs`, `native_connection.rs`, `story_memory.rs`,
   `local_demo_scope/{recovery,restart}.rs`; `df-tools`: `gameplay.rs`, `run_restart.rs`,
   `adventure/startup.rs`, `hero_library.rs`, qualification harnesses).
10. **Environment reads:** 39 non-test `df-tools` source files read the environment (inventory in
    section 8.4), including provider keys at the call sites (`imagery.rs`, `figure_masks.rs`,
    `character_clips.rs`, `home_video.rs`, `memory/startup.rs`, `speech_input.rs`,
    `place_music/resource.rs`, `creator_portraits.rs`).
11. **Reusable pieces:** operation lookup for retried commands (`DurableOwner`, SAVE-02); fenced
    idempotent commits; durable intents with `UNIQUE (tenant_id, effect_id)`;
    `EffectRepository::claim_dispatch` (`df-session/src/effects.rs`); the rewind validation and
    commit path in `df-session` (`validate_rewind`, `commit_owner_rewind`); seat presence with
    `last_seen_millis`; host pause (`df-engine/src/host_pause.rs`); B's `IdentityProvider`
    (`df-customer/src/identity.rs`); S1's `campaign_event` feed and display tickets.

The in-story machine (`journey/session_machine.rs`: Lobby, Story, Conversation, Fight,
Aftermath, Ended, Preparing) says where the story is; plan 4.1 says whether the table is being
served. They stay separate types (section 5).

## 2. Target shape

```text
browser (portal, player page, shared display)
   | HTTPS + WebSocket, one public origin
edge / control plane    df-server control-plane   B's portal router (S1 routes mounted through it),
   |                                               player and display edge paths, owner lookup and
   |                                               recheck, WS proxy, static wasm, sweeper, failover
   |                                               check, media gateway (internal mTLS endpoint)
   | internal network: only the edge reaches a runtime
campaign runtime        df-server campaign-runtime one process per active campaign; df-tools game
   |                                               engine; lifecycle task; meter segments
   |-> egress dispatcher   df-server egress        holds provider credentials (D18); rechecks the
   |                                               claimed attempt and the fence before every send;
   |                                               fetches provider output (D23)
   |-> media gateway       (control plane)         every media byte in or out of a runtime (D23)
   |-> telemetry ingestor  df-telemetry per host   OTLP logs, traces and pushed metrics (D23)
trusted runner          df-runner                  one per worker host; placement, leases, launch
PostgreSQL              one database (D7), one schema per owner
object storage          O2 (runtimes hold no object-store credentials and get no presigned URLs)
```

Network (D18 amended by D23; S2 implements). Internal connections use mTLS and are authorized by
certificate identity, the URI SAN, not by chain alone: the runtime's game listener accepts only
the edge (`urn:dungeonflux:edge:<instance>`); the media gateway and the dispatcher accept a
runtime only when its SAN `urn:dungeonflux:placement:<placement_id>` names a placement that is
current and matches the request's campaign and fence; the control plane's internal listener
accepts dispatchers by `urn:dungeonflux:dispatcher:<worker_id>`. A runtime accepts connections only
from the edge. It may open connections only to PostgreSQL, the egress dispatcher, the control
plane's internal mTLS listener (the media gateway and F's finance endpoint, D30; client
certificate issued per placement) and the host's telemetry ingestor.
Runtimes cannot reach each other, the runner, the Docker proxy or any object-store or provider
host. Media the game needs (reference images for masks, audio for ffmpeg, rehydrated assets) is
read and written through the media gateway, which authorizes against O2's catalog with the
runtime's placement identity; provider output is fetched by the dispatcher and handed to the
gateway, never fetched by the runtime. Prometheus scrapes the per-host ingestor, never a runtime.
The runner reaches only PostgreSQL and the Docker proxy.

## 3. Requirement map

| Plan requirement | Crate / module | Types | Tables | Tests (acceptance id) |
|---|---|---|---|---|
| 1.3, 15.3 production root without dev flags, credentials or room reclaim | `df-server`; `df-tools` feature `local-dev` (D15) | `ServerConfig`, `Mode` | none | `production_router_mounts_no_dev_reclaim_or_room_code_routes`; `df_tools_builds_without_local_dev`; `every_environment_read_is_inside_the_local_dev_module`; `hosted_run_configuration_is_exactly_the_descriptor` |
| 12.5 one isolated process per campaign, trusted runner | `df-runner` | `RuntimeSpec`, `RuntimeLauncher` | `df_runtime.workers`, `df_runtime.placements` | `two_runners_racing_for_one_campaign_start_one_owner`; `a_runner_crash_after_placement_lets_another_runner_place` |
| 12.5, 15.1 RuntimeOwner, fences | `df-persistence::runtime_directory`, `df-session::ownership` | `OwnerFence { recovery_epoch, claim_seq, placement_id }` | `df_runtime.campaign_owners` | `each_claim_strictly_increases_the_claim_sequence`; `a_lost_superseding_claim_halts_service_while_the_old_owner_is_alive`; `a_commit_under_an_old_fence_is_refused`; `renewals_and_commits_never_deadlock` |
| 12.5 edge routes to the current owner | `df-server::edge` | `RouteLookup`, `ConnectionTicket` | `df_runtime.connection_tickets` | `edge_closes_an_upstream_whose_fence_changed`; `a_redeemed_ticket_cannot_be_reused` (AUTH-01) |
| 12.5 drain, checkpoint, transfer; idle release | `df-server::drain` | `DrainPlan` | `df_runtime.save_points` | OPS-01; `an_all_away_table_releases_its_placement_after_the_idle_timeout` |
| 4.1 lifecycle, meter segments | `df-session::lifecycle`, `df-persistence::runtime_sessions` | `TablePhase`, `TableHolds`, `MeterMode`, `PaidWork` | `table_sessions`, `session_transitions`, `meter_segments`, `meter_credits` | TIME-01, TIME-03; `an_orphaned_open_segment_is_closed_by_the_sweeper` |
| 4.1 blocking-wait signal | `df-session::lifecycle::BlockingWaitBoard` | `BlockingReason`, `BlockingWaitGuard` | `meter_credits` | TIME-02 |
| 4.1, J08 total party disconnect | `df-session::lifecycle::Presence` | `PresenceSnapshot` | as above | `the_last_player_leaving_stops_paid_work_at_once_and_the_meter_after_the_window` |
| 16.3 owner progress | runtime supervisor | `OwnerProgress` | `campaign_owners.last_progress_at` | `a_stalled_owner_stops_advancing_proven_progress_and_is_unhealthy`; `a_reduce_stuck_past_the_bound_is_not_progress` |
| 13.7 save/resume | `df-persistence::save_points`, hosted scope | `SavePoint`, `Compatibility` | `save_points`, `side_documents`, O2 references | SAVE-01..05 |
| 14.1 tickets, display, revocation | `df-server::edge`, hosted scope | `TicketRole`, connection principal | `connection_tickets`; S1 `campaign_event` | AUTH-01, AUTH-02; `a_removed_member_loses_open_streams` (J07) |
| D18 egress dispatcher | `df-server::egress` | `DispatchRequest`, `DispatchOutcome`, attempt latch | `finance.provider_attempt` (F), `df_runtime.campaign_owners`, `df_runtime.dispatch_outputs`, `df_runtime.refetch_requests` | `dispatch_under_a_stale_fence_sends_nothing`; `a_second_dispatch_of_one_attempt_sends_nothing` (JOB-01, JOB-02) |
| 12.2 TLS | `df-persistence::postgres_tls` | `PostgresTls` | none | `plaintext_to_a_non_loopback_address_is_refused`; `verify_full_rejects_another_ca` |
| 12.6 migrations | `df-persistence::migrations`, `df-server migrate` | `Migration` | `df_meta.schema_migrations` | `a_changed_applied_migration_is_refused` |
| 12.6 feature flags | `df-server::flags` | `GenerationCategory` | `df_runtime.feature_flags` | `a_stopped_category_admits_no_new_work_and_keeps_the_session` |
| 12.7, 14.1 tenant isolation | `0018_runtime_roles.sql` | none | all granted tables | `every_table_granted_to_a_runtime_role_forces_row_security_with_a_policy` |
| D20 metrics | `df-server::metrics` | `MetricsRegistry` | none | `the_exporter_reports_five_separate_health_signals` |

## 4. Database, schemas, roles and row security

### 4.1 One database (D7)

`df_meta` (migration ledger), `df_runtime` (directory, placements, sessions, segments, credits,
save points, side documents, tickets, flags), `df_game` (canonical gameplay), `df_story` (story
memory), B's account and processor schemas, F's finance schemas (D27), S1's `campaigns`, O2's
`media` and `library`. Local
development keeps its database per run; the local demo scope is not changed.

### 4.2 Hosted migration set (D8: O1 owns 0015-0019)

Applied by `df-server migrate` in this order with B 0005-0007 (and F's numbers, D27), S1
0008-0009, O2 0010-0014:

| File | Content |
|---|---|
| `0001_game_decisions.sql` | existing `df_game`, unchanged |
| `0003_story_memory.sql`, `0004_story_memory_vector.sql` | existing `df_story`; in hosted mode applied only by the migration runner. `0004` needs the `vector` extension: availability and `CREATE EXTENSION` rights on the selected DigitalOcean edition are checked in Round 1 against the provider's documentation (plan S19) and qualified on staging; without it the runtime reports `VectorState::Unavailable` as today |
| `0015_runtime_directory.sql` | workers, placements, placement secrets, campaign owners, connection tickets |
| `0016_table_sessions.sql` | table sessions, transitions, meter segments, meter credits, feature flags |
| `0017_save_points.sql` | save points, side documents, checkpoint compaction expand step |
| `0018_runtime_roles.sql` | roles, grants, connection limits, row policies on `df_runtime`, `df_game`, `df_story` |
| `0019_story_memory_scope.sql` | expand: `tenant_id`, `campaign_id` on `df_story.story_memory`, `df_story.memory_recalls` and `df_story.entity_dossiers`, backfilled by the importer, then `NOT NULL`; indexes; the maintenance escape restricted to `df_story_maintenance` (4.4) |

`0002_local_room_reclaim.sql` is local-only and never applied to the hosted database.
`story_memory.rs` gets a `SchemaMode::{ApplyOnStart, Managed}` parameter: hosted composition
passes `Managed` (no DDL from the runtime role); local keeps `ApplyOnStart`. `btree_gist` (for the
meter-credit exclusion constraint, 5.2) is created by `0016`; its availability on the selected
edition is checked in Round 1 with `vector`.

### 4.3 Roles, pools and connection limits (plan 12.7)

`df_migrator` (owns the schemas; used only by `migrate`), `df_runtime_definer` (`NOLOGIN`,
owns the definer functions),
`df_control` (control plane, edge, sweeper, failover check, media gateway), `df_runner`,
`df_campaign_runtime`, `df_egress`, `df_story_maintenance` (index rebuild and erasure only), F's
finance worker, a read-only support role. None is superuser, `BYPASSRLS` or owner of a table it
uses at runtime.

Each process uses one bounded pool and no other connection. The runtime's pool is
`runtime.database_pool_size` (proposed 4): one connection pinned to the game owner's repository,
one for the lifecycle task (renewal, transitions, segments), one for the stream layer (membership
reads, `campaign_event`, display tickets, tickets), one for the save-point and side-document
writer. Today's separate clients (repository, grant issuer, hero library, story memory) move
onto this pool in the seam (8.2). Role limits are sized from the pools, cluster-wide:
`df_campaign_runtime CONNECTION LIMIT` = pool size times total configured runtime slots plus 10
percent; `df_control` 24, `df_runner` 2 per worker, `df_egress` 8 per worker (all proposed).
`statement_timeout` and `idle_in_transaction_session_timeout` are set on each role, and `TEMP` on
the database is revoked from `PUBLIC` and from every runtime role, so no runtime can create a
`pg_temp` object for a definer function to resolve. One runaway
process exhausts its own pool; one runaway role exhausts its own limit, not the cluster.

### 4.4 Row security for the shared runtime login

All runtimes log in as `df_campaign_runtime`; every runtime transaction begins
`SET LOCAL df.placement_token = $token`.

1. **Lease state lives in `df_runtime.placement_secrets`** (`placement_id` PK, tenant, campaign,
   `token_digest`, `recovery_epoch`, `claim_seq`, `is_current`, `lease_until`). It has no row
   security and is granted to no runtime role (only to `df_runtime_definer`); the placement,
   claim, renew, release and supersede transactions update it together with `campaign_owners`
   (6.2). `is_current` is true only for
   the placement named in `campaign_owners.placement_id`, so a superseded owner's token stops
   matching anywhere at once.
2. **Definer functions** are owned by `df_runtime_definer` (`NOLOGIN`, granted only what the
   functions read and write), declared `SECURITY DEFINER` with
   `SET search_path = pg_catalog, df_runtime, df_meta, pg_temp` (`pg_temp` named last, so it is
   never searched before the trusted schemas). The directory tables (`campaign_owners`,
   `placements`, `placement_secrets`, `workers`, `df_meta.service_state`) are granted to no runtime
   role: runtimes change them only through `claim`, `renew`, `release` and `fence_current`, which
   verify the caller's token themselves. Under `FORCE`, those tables carry one policy
   `TO df_runtime_definer USING (true)` and `TO df_control`/`df_runner` policies for their own
   operations, so a function owner is never filtered by a runtime policy. `df_game.sessions` (FORCE
   row security) carries the same `TO df_runtime_definer` policy, because the directory functions
   project the fence and lease into it; a projection that updates fewer rows than the
   `campaign_game_sessions` mapping lists raises an error (6.2). The scope functions read
   only `placement_secrets`, which has no row security, so no policy calls back into itself:
   - `df_runtime.leased_campaign() -> (tenant_id, campaign_id)`: the campaign of the placement
     whose `token_digest` matches `current_setting('df.placement_token', true)` with
     `is_current` and `lease_until > clock_timestamp()` (`placing` placements get a lease for the
     claim itself); otherwise no row.
   - `df_runtime.leased_game_sessions() -> setof (tenant_id, session_id)` and
     `df_runtime.leased_table_sessions() -> setof (tenant_id, session_id)`: the game and table
     sessions of that campaign, from the definer-only mapping tables
     (`campaign_game_sessions`, 6.2) maintained by the transactions that create sessions.
3. **One scope convention for every schema a runtime transaction touches** (F, S1, O2 adopt it
   for their tables granted to `df_campaign_runtime`): campaign-keyed rows use
   `USING ((tenant_id, campaign_id) = (SELECT * FROM df_runtime.leased_campaign()))`;
   game-session-keyed rows (`df_game`) use
   `USING ((tenant_id, session_id) IN (SELECT * FROM df_runtime.leased_game_sessions()))`;
   table-session-keyed rows (S1's `display_ticket`, F's session funding rows) use
   `leased_table_sessions()`. The initplan form runs each function once per statement. Other roles
   (control plane, F's worker) keep their own transaction-local scope settings.
4. **Every table granted to a runtime role** (`df_campaign_runtime`, `df_egress`) has `ENABLE` and
   `FORCE ROW LEVEL SECURITY` and a policy; a test walks the catalog (`pg_class`,
   `has_table_privilege` for each runtime role, `relrowsecurity`, `relforcerowsecurity`,
   `pg_policy`) and checks `proconfig` of every `SECURITY DEFINER` function for a fixed
   `search_path` ending in `pg_temp`, and that no runtime role holds `TEMP`. A later migration cannot add an unprotected granted table.
5. **`df_story`** is covered on all three tables (`story_memory`, `memory_recalls`,
   `entity_dossiers`) by 0019. The append-only trigger's maintenance escape requires both
   `df_story.maintenance = on` and membership of `df_story_maintenance`
   (`pg_has_role(current_user, 'df_story_maintenance', 'member')`, the trigger being
   `SECURITY INVOKER`), so a runtime setting the variable gains nothing.
6. `NOTIFY` payloads are hints: channels are global and any role can notify. Consumers re-read.

This gives row confidentiality and integrity per campaign and refuses a superseded owner on every
table. CPU, memory and connections are bounded by the pools and limits (4.3), the runner (7) and
S2's network policy. The scope function's cost is measured in Round 2; per-campaign roles are the
fallback, not the default.

## 5. Session lifecycle, meter segments and the blocking-wait signal

### 5.1 State model (`df-session::lifecycle`, pure, injected time, closed tables)

Phases (one at a time): `Scheduled`, `Lobby`, `Preparing`, `Active`, `Paused`, `Settling`,
`Ended`. Holds (a small set; several can hold at once): `NoPlayersConnected`, `PlayersAway`,
`BlockingWait`, `LowBalance`, `FundingRequired`, `OwnerUnhealthy`, `MoneyReconstructing` (F's
automatic per-campaign reconstruction running, 6.3), `MoneyFrozen` (reconstruction missing or
failed, J10, OPS-02), `Draining`.

| Phase + holds | `MeterMode` | `PaidWork` (new paid requests) | Plan 4.1 row |
|---|---|---|---|
| Scheduled, Lobby | Free | `SetupOnly` | draft/scheduled/lobby |
| Preparing | Free | `PreparationBudget` | preparing |
| Active, no hold | Metered | `WithinCaps` | active |
| Active + LowBalance only | Metered | `EssentialOnly` | low balance |
| Active + NoPlayersConnected (inside the presence window) | Metered | `None`, at once | 4.1 "stop new work immediately" |
| Active + PlayersAway (window elapsed) | Free from the last-seen time | `None` | 4.1 "stop time billing promptly" |
| Active + FundingRequired (F's funds state, or `meter_funded_until` not covering the next interval) | Free | `None` | funding required |
| Active + BlockingWait (past grace) | Free from the wait start | `RecoveryOnly` | blocking provider failure |
| Active + OwnerUnhealthy | Free from `last_progress_at` | `None` | unhealthy owner |
| Active + MoneyReconstructing (seconds) | Free | `None` | J10 |
| Active + MoneyFrozen | Free | `None`; read-only campaign access | J10 |
| Active + Draining | Free | `None` | transfer (OPS-01) |
| Paused | Free | `None`; no background generation | paused by host |
| Settling, Ended | Free | `None` | settling / ended |

Events: `OpenLobby`, `BeginPreparation`, `PreparationReady`, `Start`, `Pause`, `Resume`,
`PresenceChanged(PresenceSnapshot)`, `BlockingWaitsChanged(oldest_start)`, `FundsChanged`,
`OwnerStalled`, `OwnerProgressed`, `MoneyReconstructionStarted`, `MoneyReconstructed`, `MoneyFrozen`,
`DrainBegan`, `IdleReleaseDue`, `End{reason}`, `SettlementFinished`. One closed `next(state,
event)` table; an illegal event is refused with a typed reason and one audit line. Start, pause
and end need host or co-host authority from S1's matrix, rechecked by the runtime from the
database, never from a client claim.

### 5.2 Meter segments and credits

- **Segments** cover `MeterMode::Metered` time. Each change of meter mode opens or closes a
  segment in the same transaction as its `session_transitions` row, under the fence (6.1,
  `'game'`). F prices (D27); O1 never computes money.
- **Rolling, bounded by proven progress:** every `meter.segment_roll_seconds` (proposed 60) the
  open segment is closed and a new one opened, inside a renewal transaction (6.2). The roll closes
  at `GREATEST(opened_at, last_progress_at)` (the renewal's returned value), never later, and the
  new segment opens at the same instant; a segment opened by a mode change after the last proven
  progress therefore closes empty rather than violating `closed_at >= opened_at`. The meter
  writes run in a `BEGIN ... EXCEPTION WHEN OTHERS THEN ... END` sub-block of the plpgsql renewal
  function (its implicit subtransaction; plpgsql has no `SAVEPOINT` statement); an error rolls
  back only that block, raises one `NOTICE` the runtime records as an audit line
  (`METER roll_failed`) and is retried at the next renewal, so a meter error never fails a lease
  renewal.
- **Funded before metered (F 13.1).** Every segment open, and every roll (including the one inside
  the renewal sub-block), first reads `finance.meter_funded_until(tenant, campaign, session)`, F's
  lock-free definer read (no finance row lock is taken inside O1's transaction, so D25/D29 hold).
  If the next interval (`opened_at + meter.segment_roll_seconds`) is not covered, no metered
  segment is opened, the open one is closed at the roll instant as usual, and the lifecycle raises
  `FundingRequired` (meter free, plan 4.1); it clears when a later read covers the next interval. If
  F's meter worker falls behind, the table goes free, never unfunded-charged.
- **Tail.** A closed segment never extends past proven progress, so the tail
  of a crash or stall is never in a closed, priced segment; it stays in the open segment, which
  the sweeper, the runner's release or the next claim closes at `last_progress_at`. This is
  TIME-03's tick: F sees consumed time within one roll.
- **Database clock only:** `opened_at`/`closed_at` are `clock_timestamp()` or a stored
  `last_progress_at`; `CHECK (closed_at >= opened_at)`; a back-dated close is
  `GREATEST(opened_at, clock_timestamp() - lag)`, with the lag measured on the monotonic clock and
  bounded by its window. Each transition and roll carries the expected `transition_seq`; a replay
  returns the recorded row (TIME-03).
- **Credits** cover only back-dated stops of a live, progressing owner (presence window,
  blocking-wait grace) that reach into a segment already closed by a roll; crash and stall tails
  need none (previous bullet). The runtime has no `INSERT` on `df_runtime.meter_credits`; it calls
  `df_runtime.record_meter_credit(fence, session, from_at, to_at, reason)`, a definer function that
  locks the session's table-session row, clips the range to closed metered time of that session not
  already credited, and inserts what remains (possibly nothing). An exclusion constraint
  `EXCLUDE USING gist (tenant_id WITH =, session_id WITH =, credited WITH &&)` makes overlapping
  credits for two reasons unrepresentable; it needs `btree_gist`, whose availability on the
  selected edition is checked in Round 1; without it the function's lock and clipping are the only
  guard, and a test pins them.
- **Back-date window:** `meter.backdate_window_seconds` = the larger of the presence window and the
  gated grace, plus one renewal interval (proposed 45 s); the credit function refuses a `from_at`
  older than that.
- **Contract for F (pricing):** F prices a closed segment, minus its credits, only once its
  `closed_at` is older than `meter.backdate_window_seconds`; it never prices an open segment; it
  reads segments and credits in its own transactions (finance rows first, D25) and treats
  `NOTIFY df_runtime_meter` as a hint. With rolls bounded by proven progress, no credit can arrive
  after F has priced a segment, including after a host loss.
- **Sweeper:** the control plane runs one owned task every 30 s under the directory row lock
  (`FOR UPDATE` on `campaign_owners`): it closes open segments of sessions whose owner is
  `released` or whose lease expired, at that owner's `last_progress_at` (reason `owner_lost`), and
  ends tables that have had no runtime and no interactive player for `presence.away_end_hours`
  (5.3). Runner release (7) and every claim (6.2) also close their campaign's orphan in their own
  transaction.
- **Uncertain tail (proposed):** the company absorbs all time between the last proven progress and
  the stop; with a 10 s renewal and a 15 s stall bound that is at most about 25 s per incident.

### 5.3 Presence (TIME-01, J08)

- Interactive presence counts player, host and co-host connection principals with a live view
  stream (the display does not count). The owner thread never sends presence events: the stream
  layer keeps a `watch<PresenceSnapshot>` (latest value wins, `send_replace`, never blocks, never
  queues) holding `{ interactive_connected, last_interactive_seen_at }`.
- The lifecycle task reacts: count reaches zero -> `NoPlayersConnected` at once (no new paid work,
  the meter still runs); still zero after `presence.window_ms` (proposed 30,000) -> `PlayersAway`,
  segment closed at the last-seen time with a credit for any rolled part, and a save point of kind
  `all_away`. A returning player clears both.
- **Idle release (proposed `presence.idle_release_seconds = 900`):** an all-away table that stays
  away writes a save point of kind `idle`, releases its placement (6.5 without the reconnect
  step) and exits. The session stays `Active` with `PlayersAway`; the next connection asks for a
  placement and the new runtime resumes. With no runtime left, the control-plane sweeper (5.2)
  ends a table still away after `presence.away_end_hours` (proposed 12) with reason `abandoned`,
  under the directory row lock, refusing if a placement appeared meanwhile: it writes the `End`
  transition and a `session_end` save point naming the last committed revision, calls S1's
  `revoke_display_tickets(session, session_ended)`, and leaves settlement to F.

### 5.4 Blocking-wait signal (TIME-02)

- `BlockingWaitBoard`: a bounded slot list (`MAXIMUM_CONCURRENT_WAITS = 32`) of
  `{ wait_id, reason, started: Instant }` behind a `std::sync::Mutex` held only to insert or
  remove one entry (no I/O, no allocation after construction). `begin(reason) ->
  BlockingWaitGuard`; dropping the guard removes the entry. The board exposes the start time of the
  oldest live wait, which counters could not. A begin with every slot full is refused with one
  audit line (`LIFECYCLE wait_refused reason=slots_full`) and the wait is not credited; 32
  concurrent table-gated waits in one table indicate a defect, not normal play. A `Notify` wakes
  the lifecycle task; it never waits on the game.
- `BlockingReason` (closed): `PlaceGeneration`, `AdventurePreparation`, `RequiredNarration`,
  `ProviderOutage`, `ProviderRetryBackoff`. Blocking means the whole table is gated on the result
  or a required provider's circuit is open; an ordinary NPC reply is not blocking.
- Grace (proposed): `blocking_wait.gated_grace_ms = 10,000`, `blocking_wait.outage_grace_ms = 0`.
  Once the oldest wait passes its grace the meter is free from that wait's start
  (proposed `blocking_wait.blocked_interval_free = true`), with a credit for any rolled part.
- The lifecycle state, including `BlockingWait{reason}`, is projected into every view (W5 renders
  it). Call sites belong to W3 (travel, adventure preparation), N (required narration) and C/W6
  (provider circuits); each adds a guard around the gated await.

### 5.5 Where the lifecycle owner runs

One owned async task in the runtime (handle held by the runtime supervisor, aborted on shutdown),
with its pooled connection (4.3). Inputs: the presence `watch`, the wait board, the owner-progress
atomics (6.2: the start time of the reduce in progress and the completed count), host commands from the edge
(async callers; a bounded channel of 16 where the caller awaits capacity under a deadline and gets
a typed busy refusal, never the owner thread), and F's funds state (`NOTIFY` hint plus re-read).
Output: `watch<TableGate>` (meter mode, `PaidWork`, background generation, funds state) read at
the game's admission and generation points; it replaces a separate funds `watch` (question F-3).

## 6. Owner directory, fence, routing, drain and transfer

### 6.1 Fence contract (for F, S1, O2 and the dispatcher; D24, D25, D27)

The fence is `OwnerFence { recovery_epoch: i64, claim_seq: i64, placement_id: [u8; 16] }`, all
three compared everywhere.

```sql
-- The first statement of every transaction that uses the fence or the directory, before any row lock.
CREATE FUNCTION df_runtime.enter_service() RETURNS void
LANGUAGE sql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, df_meta, pg_temp AS $$
  SELECT pg_advisory_xact_lock_shared(df_meta.service_lock_key())
$$;
-- Called first by every fenced function. Never waits: if this transaction already holds the
-- shared lock it returns at once from the backend's own lock table; if not, it takes it only when
-- no halt holds or waits for it. False means a halt holds or is waiting.
CREATE FUNCTION df_runtime.require_service() RETURNS void
LANGUAGE plpgsql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, df_meta, pg_temp AS $$
BEGIN
  IF NOT pg_try_advisory_xact_lock_shared(df_meta.service_lock_key()) THEN
    RAISE EXCEPTION 'service halting' USING ERRCODE = 'DF002';   -- ServiceHalting
  END IF;
END $$;

CREATE TYPE df_runtime.fence_path AS ENUM ('game', 'paid');
CREATE FUNCTION df_runtime.fence_current(
    p_tenant_id bytea, p_campaign_id bytea,
    p_recovery_epoch bigint, p_claim_seq bigint, p_placement_id bytea,
    p_path df_runtime.fence_path)
RETURNS boolean
LANGUAGE plpgsql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, df_meta, pg_temp AS $$
BEGIN
  PERFORM df_runtime.require_service();
  RETURN EXISTS (
    SELECT 1 FROM df_meta.service_state s, df_runtime.campaign_owners o
     WHERE s.serving = 'serving'
       AND o.tenant_id = p_tenant_id AND o.campaign_id = p_campaign_id
       AND o.recovery_epoch = p_recovery_epoch AND o.claim_seq = p_claim_seq
       AND o.placement_id = p_placement_id
       AND o.state IN ('serving', 'draining') AND o.lease_until > clock_timestamp()
       AND (p_path = 'game' OR o.money_state = 'reconciled')
     FOR SHARE OF o);
END $$;
```

- **Halt without a global row lock.** `df_meta.service_state` is read, never row-locked, by
  `fence_current`, `place`, `claim`, `renew` and both releases. Each starts with
  `require_service()`, which uses `pg_try_advisory_xact_lock_shared` and therefore never waits:
  when the caller took `enter_service()` first (the convention, before any row lock) it returns at
  once; when not, it takes the lock only if no halt holds or awaits it, and otherwise raises
  `DF002` (`ServiceHalting`), which the caller maps to a typed retryable refusal. No caller holding
  row locks can queue behind a waiting halt, so the cycle "row holder, halt, shared holder, row
  holder" cannot form, and no call reads `pg_locks`. The halt (6.3) runs
  `SET LOCAL lock_timeout = '2s'` (proposed) and takes the same key with `pg_advisory_xact_lock`
  (exclusive); on timeout it rolls back, waits with backoff and retries, so it never holds a queue
  position for long, and `enter_service()` callers wait at most the lock timeout. Once it holds the
  lock, it updates `serving`. A fenced transaction therefore either finished before the halt or
  sees it; none slips past, and concurrent fenced writes share no row and build no multixact.
- **Pooling.** The lock and every `SET LOCAL` are transaction-scoped, so they work with session-
  or transaction-mode pooling; statement-mode pooling is forbidden (request to S2).
- **Idle holders.** Every role that takes the shared lock (`df_campaign_runtime`, `df_control`,
  `df_runner`, `df_egress`, F's roles) has `idle_in_transaction_session_timeout` of 5 s
  (proposed), so one idle transaction cannot hold the shared lock and stall a halt and, behind it,
  all fenced work.
- **Who calls it with which path.** Only paid admission (F's reservation transaction) and the
  dispatcher's send mark and pre-egress recheck pass `'paid'`. Game-side writes of S1 and O2 inside
  runtime transactions, transitions, segments and save points pass `'game'`. F's settlement,
  reconstruction and credit postings are keyed by the shared attempt id (D22) and never call
  `fence_current`: a late result after a release, or the reconstruction that clears a freeze,
  settles without a live owner. S1's present two-part `(owner_epoch, placement_id)` check becomes
  this call.
- **The game decision commit does not call it.** It checks the projection in `df_game.sessions`
  (`owner_fence`, `lease_until`, existing commit SQL), which every directory transaction rewrites
  (6.2), so the owner thread's commit keeps its one row lock.
- **Lock order (D25, extended by D29):** the service advisory lock (shared) before any row lock;
  then accounts and finance rows (B, F); then O1's directory and session rows (`campaign_owners`,
  `placement_secrets`, `df_runtime` session tables, `df_game.sessions` and later `df_game` rows);
  then `campaigns.campaign`, then campaign child rows (S1); then media (`quota_usage`, `asset` by
  asset id, references, grants). No O1 transaction locks finance or account rows; a transaction that
  starts in media never afterwards locks earlier rows. F's admission transaction (run by F's
  finance endpoint, D30) is therefore: `enter_service()`, finance rows,
  `fence_current(..., 'paid')`, then O2's `admit_generation`;
  S1's fenced writes are `enter_service()`, `fence_current(..., 'game')`, then
  `campaigns.campaign`. The game commit locks only `df_game.sessions` and later `df_game` rows. A
  Round 2 test runs renewals against a stream of commits and F-style admissions and asserts no
  deadlock and no failed commit.

### 6.2 Directory and its SQL

Each SQL block below is the body of a definer function (`df_runtime.place`, `claim`, `renew`,
`release_clean`, `release_after_exit`) owned by `df_runtime_definer` (4.4), declared with
`SET search_path = pg_catalog, df_runtime, df_meta, pg_temp`, and beginning with
`PERFORM df_runtime.require_service()` (never waits; `DF002` during a halt); the caller takes
`enter_service()` as the first statement of its transaction. A runtime's call passes its placement token, which the
function matches against `placement_secrets` before acting.

```sql
CREATE TABLE df_runtime.campaign_owners (
  tenant_id bytea NOT NULL CHECK (octet_length(tenant_id) = 16),
  campaign_id bytea NOT NULL CHECK (octet_length(campaign_id) = 16),
  recovery_epoch bigint NOT NULL CHECK (recovery_epoch >= 1),
  claim_seq bigint NOT NULL CHECK (claim_seq >= 0),
  state text NOT NULL CHECK (state IN ('released', 'placing', 'serving', 'draining')),
  placement_id bytea CHECK (placement_id IS NULL OR octet_length(placement_id) = 16),
  worker_id bytea REFERENCES df_runtime.workers,
  placement_lease_until timestamptz,
  runtime_endpoint text,
  lease_until timestamptz NOT NULL,
  last_progress_at timestamptz,
  released_cleanly boolean NOT NULL DEFAULT true,
  money_state text NOT NULL DEFAULT 'reconciled'
      CHECK (money_state IN ('reconciled', 'reconstructing', 'frozen')),
  reconstruction_id bytea,   -- written by the claim that starts a reconstruction
  PRIMARY KEY (tenant_id, campaign_id),
  CHECK ((state = 'placing') = (placement_lease_until IS NOT NULL)),
  CHECK (state = 'released' OR placement_id IS NOT NULL));
CREATE TABLE df_runtime.placements (
  placement_id bytea PRIMARY KEY, tenant_id bytea NOT NULL, campaign_id bytea NOT NULL,
  worker_id bytea NOT NULL REFERENCES df_runtime.workers,
  created_at timestamptz NOT NULL, ended_at timestamptz, end_reason text);
CREATE UNIQUE INDEX placements_one_live ON df_runtime.placements (tenant_id, campaign_id)
  WHERE ended_at IS NULL;
-- Definer-only: which df_game sessions belong to a campaign (written where sessions are created).
CREATE TABLE df_runtime.campaign_game_sessions (
  tenant_id bytea NOT NULL, campaign_id bytea NOT NULL, game_session_id bytea NOT NULL,
  PRIMARY KEY (tenant_id, game_session_id));
```

**Row creation.** The edge's placement request inserts the row if absent:
`INSERT INTO df_runtime.campaign_owners (tenant_id, campaign_id, recovery_epoch, claim_seq,
state, lease_until) VALUES ($t, $c, df_runtime.current_recovery_epoch(), 0, 'released',
'-infinity') ON CONFLICT DO NOTHING`, then its `placement_requests` row the same way.

**Placement (runner),** one transaction, after closing any orphaned open segment:

```sql
UPDATE df_runtime.campaign_owners
   SET state = 'placing', placement_id = $placement, worker_id = $worker,
       placement_lease_until = clock_timestamp() + make_interval(secs => $placing_lease_s),
       released_cleanly = (state = 'released' AND released_cleanly)
 WHERE tenant_id = $t AND campaign_id = $c
   AND (SELECT serving FROM df_meta.service_state) = 'serving'
   AND (   (state = 'released')
        OR (state = 'placing' AND placement_lease_until <= clock_timestamp())
        OR (state IN ('serving', 'draining') AND lease_until <= clock_timestamp()))
RETURNING recovery_epoch, claim_seq;
-- zero rows: a live placement exists or service is halted; the runner moves on.
UPDATE df_runtime.placements SET ended_at = clock_timestamp(), end_reason = 'superseded'
 WHERE tenant_id = $t AND campaign_id = $c AND ended_at IS NULL;
UPDATE df_runtime.placement_secrets SET is_current = false
 WHERE tenant_id = $t AND campaign_id = $c;
INSERT INTO df_runtime.placements
  VALUES ($placement, $t, $c, $worker, clock_timestamp(), NULL, NULL);
INSERT INTO df_runtime.placement_secrets
  (placement_id, tenant_id, campaign_id, token_digest, is_current, lease_until)
  VALUES ($placement, $t, $c, $digest, true,
          clock_timestamp() + make_interval(secs => $placing_lease_s));
```

`placing_lease_s` is proposed 60: a runner that crashes before launch leaves a placement that
expires and can be superseded; a second runner cannot overwrite a live one (the `WHERE` clause and
the partial unique index). Superseding an expired `serving` placement keeps
`released_cleanly = false` (the owner never released).

**Claim (runtime, on start):**

```sql
UPDATE df_runtime.campaign_owners
   SET claim_seq = claim_seq + 1,
       money_state = CASE
           WHEN money_state IN ('reconstructing', 'frozen')
             OR recovery_epoch < df_runtime.current_recovery_epoch() OR NOT released_cleanly
             THEN 'reconstructing'
           ELSE money_state END,
       reconstruction_id = CASE
           WHEN money_state IN ('reconstructing', 'frozen')
             OR recovery_epoch < df_runtime.current_recovery_epoch() OR NOT released_cleanly
             THEN $new_reconstruction_id
           ELSE reconstruction_id END,
       recovery_epoch = df_runtime.current_recovery_epoch(),
       state = 'serving', runtime_endpoint = $endpoint, placement_lease_until = NULL,
       lease_until = clock_timestamp() + make_interval(secs => $lease_s),
       last_progress_at = clock_timestamp(), released_cleanly = false
 WHERE tenant_id = $t AND campaign_id = $c AND state = 'placing'
   AND placement_id = $placement AND placement_lease_until > clock_timestamp()
   AND (SELECT serving FROM df_meta.service_state) = 'serving'
RETURNING recovery_epoch, claim_seq, lease_until, money_state, reconstruction_id;
```

Same transaction, in lock order: update `placement_secrets` (`recovery_epoch`, `claim_seq`,
`lease_until`); close the previous owner's open segment at its `last_progress_at`; project into
`df_game.sessions` for every session in `campaign_game_sessions` of this campaign (`owner_fence` =
first 16 bytes of SHA-256 over `recovery_epoch || claim_seq || placement_id`, big-endian, and the
returned `lease_until`), raising an exception when the projection updates fewer rows than the
mapping lists, so a silent no-op cannot pass. After commit and before readiness, append
`OwnerClaimed { campaign, fence }` through O2's journal submitter (6.3) and wait for its
confirmation. Zero rows: exit with `ClaimRefused`. A `reconstructing` result starts F's
reconstruction (6.3); the runtime serves play meanwhile with `PaidWork::None`.

**Renew (lifecycle task, every `lease.renew_every_seconds`, proposed 10; lease proposed 30):**

```sql
UPDATE df_runtime.campaign_owners
   SET lease_until = clock_timestamp() + make_interval(secs => $lease_s),
       last_progress_at = CASE WHEN $progressed THEN clock_timestamp() ELSE last_progress_at END
 WHERE tenant_id = $t AND campaign_id = $c AND recovery_epoch = $re AND claim_seq = $seq
   AND placement_id = $placement AND state IN ('serving', 'draining')
   AND lease_until > clock_timestamp()
   AND (SELECT serving FROM df_meta.service_state) = 'serving'
RETURNING lease_until, last_progress_at;
-- same transaction, the returned values, lock order kept:
UPDATE df_runtime.placement_secrets SET lease_until = $returned_lease
 WHERE placement_id = $placement AND is_current;
UPDATE df_game.sessions AS g SET lease_until = $returned_lease
  FROM df_runtime.campaign_game_sessions m
 WHERE m.tenant_id = $t AND m.campaign_id = $c
   AND g.tenant_id = m.tenant_id AND g.session_id = m.game_session_id
   AND g.owner_fence = $projected_fence;
-- fewer rows than the mapping lists: RAISE (the renewal fails and is retried; see below)
-- when this renewal is also a roll (5.2), inside a sub-block:
--   BEGIN
--     close the open segment at GREATEST(opened_at, $returned_last_progress);
--     IF finance.meter_funded_until($t, $c, $session) >= roll_at + roll_interval
--       THEN open the next segment ELSE record FundingRequired (no segment) END IF;
--   EXCEPTION WHEN OTHERS THEN RAISE NOTICE 'meter roll failed: %', SQLSTATE;
--   END;
```

Zero rows on the first statement: `RevisionConflict` (superseded or halted); stop serving, exit.
A projection that updates fewer rows than expected is a typed `ProjectionMismatch` error, one audit
line, readiness false, and the renewal is retried; it never reports success.

**Progress.** The owner thread publishes two atomics: `reduce_started_at` (monotonic nanoseconds
of the reduce in progress, 0 when idle) and `reduces_completed`. `$progressed` is true only when
no reduce has been in progress longer than `owner.stall_seconds` (proposed 15) and either a reduce
completed since the last renewal or the inbox was empty for the whole interval. A reduce in
progress past the bound, or queued items with no completion for that long, raises
`OwnerUnhealthy` (meter free from `last_progress_at`, no paid work, readiness false); past
`owner.stall_exit_seconds` (proposed 60) the supervisor exits and the runner restarts the
campaign. The lease is still renewed while stalled, so no second owner starts while it lives.

**Local lease.** The runtime records the monotonic instant each renewal was sent;
`local_lease_deadline = sent + lease - lease.safety_margin_seconds` (proposed 3). Without a
successful renewal by then it stops commits, closes every client stream with the reconnect status
and exits.

**Release.** Two functions, both in lock order, both requiring all three fence parts:
`release_clean` (the runtime itself after a drain or idle release, 6.5) sets
`released_cleanly = true`; `release_after_exit` (the runner, after it observed the child exit
without a clean release) sets `released_cleanly = false`. Both set `state = 'released'`,
`lease_until = LEAST(lease_until, clock_timestamp())`, `placement_id = NULL`; close the open
segment at `last_progress_at`; end the placement row; set `placement_secrets.is_current = false`;
and project the expired lease into `df_game.sessions` (same mapping and row-count check).

### 6.3 Failover, restore and money reconstruction

**What the fence guarantees.** In any one database state only one fence is valid. It does not
survive the loss of committed writes: if a failover to an asynchronous standby, or a restore,
loses the write that superseded an owner (the placement or the claim), the database returns to the
old owner's fence. If that process is still inside its local lease, its next renewal succeeds and
it serves again while the newer owner's checks fail, and effects carried out under the lost fence
are unknown to the database. The fence cannot detect that; the following does.

**Mirrored facts.** `OwnerClaimed { campaign, fence }` (before a runtime becomes ready) and the
dispatcher's `Dispatching` entries (section 10) go to O2's protected recovery journal through its
submitter model (D22): the submitter inserts `journal.pending_entry` in its own transaction, and
the control-plane writer stamps, seals and confirms it.

**Detection.**
- `df_meta.service_state(serving, checked_at)` and `df_meta.failover_cursor(journal_sequence)`
  live in the same database, and the cursor advances in the same transaction as `checked_at`. A
  failover or restore therefore rolls the cursor back together with the lost writes, and every
  journal entry after the restored cursor is compared again.
- Independently of the cursor, on each control-plane reconnect the check compares a window of
  `failover.recheck_window_seconds` (proposed 600, at least the maximum replication lag the
  selected tier documents) of confirmed journal entries before the cursor.
- The comparison: a journaled claim the directory does not hold (higher claim sequence or recovery
  epoch, or the same with another placement), or a journaled `Dispatching` attempt that F's rows do
  not know, means committed writes were lost.
- On detection the control plane takes the service advisory lock exclusively (6.1), sets
  `serving = 'halted_regression'` and commits. Every renewal, placement, claim and `fence_current`
  then refuses, so all runtimes stop at their local lease and the dispatcher sends nothing. A clean
  check advances `checked_at` and the cursor.
- **Reconnect gate.** Every process treats a broken database connection as a possible failover.
  On the new connection it reads `clock_timestamp()` as `reconnected_at`, and writes nothing
  (renewal, commit, placement, claim, dispatch, reservation, settlement) until
  `service_state.checked_at > reconnected_at`, both database clock readings. Past
  `failover.check_wait_seconds` (proposed 30) it exits. The gate binds the runtimes, runners,
  dispatchers, the control plane's other tasks and F's worker processes (settlement,
  reconciliation). The one exemption is the control plane's own failover-check transaction, which
  is what advances `checked_at`; it reads the journal and the database and writes only
  `service_state`, `failover_cursor` and, on detection, the halt.
- A process that sees a regression of a fact it saw committed (its own claim gone, a lease shorter
  than granted, a lower sequence) writes a `df_meta.regression_reports` row and stops; the control
  plane treats a report as a detection.

**Resuming.** Only through the restore path (service-operations "Composite revision after
disaster restore"): an operator step allocates a strictly greater recovery epoch through the
journal head compare-and-set and sets `serving = 'serving'`; each next claim raises its row's
`recovery_epoch` and sets `money_state = 'reconstructing'`. If the selected DigitalOcean tier gives
a synchronous standby, a failover loses no committed write and the detection never fires; whether
it does depends on the account's eligibility (owner question). The detection stays either way.

**Money reconstruction (J10, ruling (b)).** Paid work stops only while money reconstruction for
that campaign is running, missing or failed; there is no blanket freeze on every crash.
- A claim sets `money_state = 'reconstructing'` when the recovery epoch rose or the previous owner
  did not release cleanly. The runtime serves play with `PaidWork::None` (lifecycle hold
  `MoneyReconstructing`, meter free) and asks F to reconstruct.
- **Reconstruction is an F endpoint call (D30).** After its claim the runtime calls F's
  `reconstruct_campaign { campaign, fence, reconstruction_id }` on the control plane's internal mTLS
  listener (authorized by the placement certificate). F's endpoint may read the confirmed journal
  tail from O2's journal (F 22.2: replay of missing entries, recreation of journaled `Dispatching`
  attempts as `OutcomeUnknown`) and makes no provider call; status lookups belong to F's later
  reconciliation of unknown attempts. It ends, in F's final transaction, with
  `df_runtime.record_money_state(tenant, campaign, reconstruction_id, outcome)`, `Reconstructed` or
  `Failed { reason }`. There is no `clear_paid_work_freeze`: `record_money_state` is the only way
  out of `reconstructing` or `frozen` (request to F, 10.2). The call is idempotent per
  `reconstruction_id`; on a timeout or error the runtime retries it with the same id, and the next
  claim retries with a new one. Proposed p95 under 5 s for a campaign with up to 64 open attempts,
  journal head and tail reads included, measured in F's round.
- **`record_money_state(tenant, campaign, reconstruction_id, outcome)` is a compare-and-set**
  (O1's definer function; `EXECUTE` granted only to F's finance service role, revoked from
  `PUBLIC` and from every other role, `df_campaign_runtime` included):
  `UPDATE campaign_owners SET money_state = CASE WHEN $outcome = 'reconstructed' THEN 'reconciled'
  ELSE 'frozen' END WHERE tenant_id = $t AND campaign_id = $c AND reconstruction_id = $id AND
  (money_state = 'reconstructing' OR (money_state = 'frozen' AND $outcome = 'reconstructed'))`.
  Zero rows means the result is stale (a newer claim wrote a newer `reconstruction_id`) or
  arrives after a success; it is ignored with one audit line.
- **`record_reconstruction_deadline(tenant, campaign, reconstruction_id)`** is the runtime's only
  way to change `money_state` (O1's definer function; `EXECUTE` granted only to
  `df_campaign_runtime`). It first checks that the caller's placement token
  (`df.placement_token`) matches the campaign's current placement in `placement_secrets`, then
  runs `UPDATE campaign_owners SET money_state = 'frozen' WHERE tenant_id = $t AND campaign_id = $c
  AND reconstruction_id = $id AND money_state = 'reconstructing'`: it can only move
  `reconstructing` to `frozen`, never reach `reconciled`, never touch another campaign. The runtime
  calls it when `money.reconstruction_deadline_seconds` (proposed 30) passes without a result.
  Because both functions are conditional on `reconstructing` for the same id, the deadline can
  never overwrite a `Reconstructed` that committed first, and a late `Reconstructed` for the same
  id still lifts `frozen` through `record_money_state`.
- `frozen` keeps paid work off and the campaign read-only (J10: "missing financial reconstruction
  keeps paid work frozen while read-only campaign access is restored") until a `Reconstructed`
  for the current `reconstruction_id` arrives (F's reconciliation retrying) or the next claim
  starts a new one; an operator alert fires (plan 16.3 P1).

Tests: `a_lost_superseding_claim_halts_service_while_the_old_owner_is_alive` (P2's claim
journaled, the database restored to before it with the cursor, P1 still inside its local lease:
after the check P1's renewal is refused, nothing is dispatched, and no claim succeeds until the
epoch is raised); `a_restored_cursor_rechecks_the_lost_entries`;
`a_reconnecting_runtime_writes_nothing_until_a_check_after_its_database_reconnect_time`;
`a_crash_without_release_starts_reconstruction_and_paid_work_resumes_when_it_succeeds`;
`a_failed_reconstruction_freezes_paid_work_and_play_continues`;
`a_halt_waits_for_in_flight_fenced_transactions_and_refuses_later_ones`;
`a_fenced_call_during_a_waiting_halt_raises_service_halting_without_waiting`;
`a_late_deadline_failure_never_overwrites_a_committed_reconstruction`;
`a_stale_reconstruction_result_under_a_newer_claim_is_ignored`;
`the_runtime_role_cannot_execute_record_money_state`;
`a_deadline_with_a_superseded_placement_token_changes_nothing`;
`the_failover_check_runs_after_its_own_reconnect`.

### 6.4 Edge paths

**Players.** `GET /sessions/:id/play` and its tunnel need B's session cookie; the edge loads S1's
membership through S1's adapter, mints a connection ticket (32 random bytes, stored as a SHA-256
digest, account, campaign, member id, role `Player | Host | CoHost`, device label, 60 s, single
use) and passes it to the runtime in an internal header. The ticket never leaves the server: the
browser never sees it. The runtime redeems it under its fence and binds the authenticated
principal to the tunnel connection; every RPC on that connection carries that principal (a tonic
request extension), so no bearer binding sits in browser storage. The member id is S1's
`MemberId` (S1 O1-2); the runtime mints none. The edge mints every ticket from S1's
`member_snapshot(account, campaign)` (`MemberSnapshot { member_id, account_id, campaign_id,
tenant_id, role, holds_seat, delegations, status, member_revision, campaign_status, seat_count,
event_seq }`); S1 mints none. Hosted players always authenticate (D9). The runtime seats only
members with `holds_seat = true`, never more than `MemberSnapshot.seat_count`, and never more
than `runtime.maximum_seats` (a ceiling; both default 6); a member past either bound gets a typed
"table full", and a member without `holds_seat` can still watch where S1's role allows it.
Seated players never pay unless they fund (D9). A host transfer rotates outstanding `Host`
tickets and does not transfer campaign ownership (D11).

**Display.** `GET /sessions/:id/display` and its tunnel take S1's two host-only cookies
(`__Host-df_display_ticket`, `__Host-df_display_device`), a separate type and digest domain from a
web session (S1 4.6, 8.5). The edge calls S1's `verify_display_ticket` at connect; the runtime
calls it again at the start of each publication batch and on S1's renewal window, and attaches a
`Display` principal that carries only `ViewPublicProjection`. Both are on one public origin, so
the host-only cookies reach the edge (S1 8.5 step 6, first shape; answer to S1 O1-8). Renewal is
S1's `POST /display/renew` on the same origin, which extends the same ticket (same id, same
digest), so an open stream is never closed by it; O1 builds the client side in the display
client: during S1's renewal window it posts with both cookies and CSRF, and on refusal it shows
the pairing page. The runtime's batch-start check then sees the later expiry on the same ticket
id.

**Routing and recheck.** The edge reads the directory row; `serving` with a live lease: open an
internal WebSocket (mTLS, edge certificate) to `runtime_endpoint`, after checking that the
runtime's certificate SAN names the placement in the directory row, and copy binary messages both ways within the bridge
bounds (`df_rpc_bridge::MESSAGE_BYTES`, backpressure, no unbounded buffer). Anything else: create
a placement request and answer "table starting" for the client to retry with backoff. The edge
keeps each upstream's fence and rechecks it on `LISTEN df_runtime_owner` hints and on a timer
(proposed every 5 s); an upstream whose stored fence no longer matches the row is closed, and the
client reconnects through the edge. Reconnect uses operation lookup and a snapshot, never Join.

**Membership feed as a gate (J07, D24, S1 request 6-8).** S1's check-at-use is the revocation
guarantee, and the feed is part of it.
- Every stream is admitted, and every publication batch starts, with a membership revalidation
  through `member_snapshot` (players and hosts) or `verify_display_ticket` (displays), on the
  stream layer's pooled connection, never on the owner thread. A batch is the publication of one
  committed revision to the streams it addresses and must finish within
  `publication_batch_maximum_ms` (proposed 500); a batch that runs longer is cut off and the next
  one revalidates again. The staleness bound for a revoked member is one batch plus the feed lag
  bound below.
- **No durable cursor.** For each campaign it serves, the consumer subscribes (`LISTEN
  campaign_event`, a hint only), then takes a fresh `MemberSnapshot` and starts from its
  `event_seq`, holding the position in memory. On a gap or a restart it takes a new snapshot rather
  than resuming.
- **Finding changes.** It polls `campaigns.campaign` on the indexed (`last_event_at_ms`,
  `campaign_id`) keyset, starting each poll `feed_overlap_seconds` (15) before the last
  `last_event_at_ms` it saw, then reads that campaign's `campaign_event` rows with `event_seq`
  greater than its position, in order; `event_seq` is gapless per campaign.
- **Lag gate.** Lag is the time since the last poll that reached the head. Above
  `feed_maximum_lag_seconds` (5) the runtime admits no new stream and publishes no private data
  (player and host views pause with a "reconnecting" notice; the display's public projection may
  continue) until a poll reaches the head again.
- **Revoking means re-check, not deny.** A revoking member row drops that member's cached
  authority and re-checks it against a fresh snapshot; the consumer keeps
  `highest_revoking_revision[member]`. A campaign-level revoking row (`archived`, `delete_*`) has no
  member: every grant records the snapshot `event_seq` it was made under, and a campaign-level
  revoking row with a higher `event_seq` forces re-authorization of every grant on the campaign. A
  host transfer arrives as one row per member. A `display_ticket_revoked` row closes the streams of
  that ticket id.
- Every write still rechecks in its own transaction.

### 6.5 Drain and transfer (OPS-01)

1. The runner or an operator sets `state = 'draining'` and calls `POST /internal/drain`
   (authenticated by the placement token); `SIGTERM` maps to the same.
2. Readiness false at once; `Draining` hold; `AdmissionPolicy::begin_shutdown`; new commands get a
   typed "table is moving" refusal.
3. In-flight effects get up to `drain.effect_wait_seconds` (proposed 30); those still
   `Dispatching` remain so for the next owner (unknown, never resent).
4. One transaction: close the segment (reason `drain`) and release (6.2); the save writer has
   already written a save point of kind `drain` naming the last committed revision (9.2); the
   release is `release_clean`. Then close streams with the reconnect status and exit 0. Hard
   limit `drain.maximum_seconds` (proposed 90).
5. A new runtime claims with `claim_seq + 1` and loads the checkpoint. Target under two minutes
   (plan 13.10), measured in OPS-01.

**Session end and revocation writes.** Every session end (the `End` transition, a drain that ends
the session, the control-plane sweeper's `abandoned` end) calls S1's `session_ended(session)` and
`revoke_display_tickets(session, session_ended)`. Neither is fence-gated: S1 accepts them from any
authenticated placement of the campaign, or from the control plane, and stores the fence for
audit, so a stale or frozen placement can still revoke. The control plane also calls
`revoke_display_tickets` on every placement release (D29).

Crash (J10): the runner releases early with `release_after_exit` (not clean) only after it has
observed the child's exit; otherwise the lease expires. Either way the next claim sets
`money_state = 'reconstructing'` and F's automatic reconstruction (6.3) runs; paid work resumes
when it succeeds and stays frozen, with read-only campaign access, only if it fails.

## 7. Trusted runner (`df-runner`)

- One per worker host under the host's init system (S2); no game, payment or provider credentials.
- Loop: heartbeat `df_runtime.workers`; take placement requests it has a slot for by the plan 12.5
  formula (`minimum(memory, CPU, connection, database, provider slots)` from configured,
  measured values); place (6.2); launch; supervise; release on exit.
- `RuntimeLauncher` port: `start(RuntimeSpec)`, `drain(&handle)`, `stop(&handle, grace)`,
  `wait(&handle)`, `usage(&handle)`. Adapters: `FakeLauncher` (tests), `NativeLauncher` (local:
  child process, explicit arguments, no inherited environment), `DockerProxyLauncher` (staging and
  production, through S2's body-checking proxy; never run against a real daemon by O1).
- The proxy policy S2 enforces (D18): paths limited to create/start/stop/kill/inspect/wait/delete;
  create bodies refused unless `Privileged` is false, `Binds` is empty except the descriptor and
  spool mounts, `CapAdd` is empty with `CapDrop: ["ALL"]`, `NetworkMode` is the runtime network,
  `Devices` is empty, `PidMode`/`IpcMode` are private, `ReadonlyRootfs` is true and the image is a
  digest in the allow-list. Images are verified with `cosign verify` by digest at deploy (D18); the
  runner starts only digests present in the deployed allow-list.
- `RuntimeSpec`: image digest, campaign and tenant, placement id and token, internal port,
  database endpoint and CA path, dispatcher, media gateway and ingestor endpoints, the
  placement's mTLS certificate with URI SAN `urn:dungeonflux:placement:<placement_id>`, used as
  client certificate toward the media gateway and the dispatcher and as server certificate on the
  game listener (issued by the control plane at placement, valid for the placement lease plus its
  renewals, revoked at release), limits, lease
  settings; delivered as a read-only mounted descriptor file, never as environment variables.
- Placeholder limits until Round 6 measures p95: 1,536 MiB, 1.0 CPU, 256 pids, `/tmp` 512 MiB.

## 8. Production composition root (`df-server`)

### 8.1 Crate, binary, configuration

- `crates/df-server/Cargo.toml` and workspace membership (coordinator owns the root manifest).
  Keep `composition_registry`, `deployment`, `shutdown_policy`; move their tests in-crate.
  `DeploymentMode` gains `CampaignPerProcess`; anything else stays refused (D6).
- Subcommands: `control-plane`, `campaign-runtime --descriptor`, `egress`, `migrate
  {status|plan|apply}`, `healthcheck`, `config check`; `df-runner` is its own binary.
- One TOML file per role, typed, `deny_unknown_fields`, secrets as `*_file` paths, validated at
  load with typed errors; staging and production reject every local-only key.

### 8.2 Reusing the game engine (D15, D16)

D15: `df-server` depends on `df-tools`; dev surfaces live behind `local-dev` (default on for local
builds, off in production images). D16: O1 performs the `serve_run`/actor split after W4's
concurrency stages 0 and 1 land on those files.

1. `local-dev` gates: `dev_control` and `/dev/*`, `dev_cli`, every qualification module, test
   fights and dev overrides (`journey/tactical.rs`), the dev rewind *surface*, room-code join and
   seat reclaim (`room.rs`, `room_code.rs`, `Call::{AwaySeats, Reclaim}`), `launcher_status`, the
   campaign CLIs (`adventure/cli.rs`), `RunConfiguration::from_environment`, preview and fixture
   modes.
2. `serve_run` splits into `RunConfiguration` and `compose_run(RunConfiguration, ScopeAuthority)`;
   the hosted path builds the configuration from the descriptor and the TOML file.
3. `ScopeAuthority` is an enum over `LocalDemoScopeIssuer` and `HostedScopeIssuer`
   (`df-persistence::hosted_scope`, O1).
4. A hosted new adventure allocates a new `game_session_id` in `df_game`, not a database.

### 8.3 What the control plane mounts

B's `df_portal::router` and `webhook_router` (S1 routes through B), the player and display edge
paths, static wasm and content assets, internal health and metrics. One origin for portal, play
and display.

### 8.4 Environment reads: inventory and enforcement

Every non-test `df-tools` file that reads the environment (39; counts of read sites):

| File | Reads | Hosted treatment |
|---|---|---|
| `gameplay.rs` (29), `gameplay/startup_configuration.rs` (32), `gameplay/timing.rs` (3), `gameplay/media_policy.rs` (1), `gameplay/tempo.rs` (1), `gameplay/display_tuning.rs` (1), `gameplay/splat_world.rs` (1), `gameplay/adventure/campaign_startup.rs` (2) | `DF_GAMEPLAY_*` settings, timing, gates, tuning, seeds, campaign | fields of `RunConfiguration`; the read moves into `from_environment` (local-dev) |
| `gameplay/cost.rs` (1) | `DF_GAMEPLAY_PRICING` | configuration field (price table path), C's file |
| `gameplay/imagery.rs` (2), `gameplay/imagery/figure_masks.rs` (1), `gameplay/character_clips.rs` (3), `gameplay/home_video.rs` (3), `gameplay/memory/startup.rs` (2), `gameplay/speech_input.rs` (2), `gameplay/place_music/resource.rs` (3), `gameplay/creator_portraits.rs` (7) | provider keys, models, caches, `DF_FFMPEG` | keys removed from the runtime (dispatcher, section 10); models and cache paths become configuration fields; owners W2, W6, C, A4/O2 |
| `gameplay/living_scenes/canonical.rs` (1), `gameplay/hero_library.rs` (1) | `DF_GAMEPLAY_IMAGERY_CACHE` | cache path field (O2 bounds it) |
| `gameplay/imagery/photo_check.rs` (1) | `DUNGEONFLUX_ARTIFACT_ROOT` | field, or local-dev only (W2 decides) |
| `gameplay_browser/presentation_ingress.rs` (1) | `DF_GAMEPLAY_DEV_CONTROL` | local-dev only |
| `gameplay/journey/tactical.rs` (2) | `DF_GAMEPLAY_TEST_LEVEL`, `DF_GAMEPLAY_DEV_OVERRIDES` | local-dev only |
| `gameplay/launcher_status.rs` (1), `gameplay/adventure/cli.rs` (5), `dev_cli.rs` (4), `preview.rs` (3), `main.rs` (2), `gameplay/dev_control/{mod,timeline,reads,direct}.rs` (3, 2, 1, 1) | launcher token, CLIs, dev control | local-dev only |
| `gameplay/{recovery,courier_npc,courier_process,restart,rest,inn}_qualification.rs` (4, 4, 3, 3, 3, 3) | qualification switches | local-dev only |
| `gameplay/startup_contract.rs` (1), `gameplay/cache_replay_locale_contract.rs` (1) | fixture roots, `TMPDIR` | test-only; confirm `#[cfg(test)]` or move under local-dev |

Enforcement:
1. All environment access in `df-tools` goes through one module, `df_tools::environment`, that
   exists only under `#[cfg(feature = "local-dev")]`. Calls to it from code outside `local-dev`
   fail to compile in the `--no-default-features` build. A direct read left outside the module is
   caught by the scan test `every_environment_read_is_inside_the_local_dev_module`, which fails on
   `env::var`, `env::var_os`, `env::vars`, `env::args`, `env::temp_dir`, `std::env::home_dir`,
   any `from_env` constructor (the OpenTelemetry SDK reads `OTEL_*` variables there), and any
   `Command::new` outside one helper, `df_tools::child_process::command(program)`, which always
   calls `env_clear()` and then sets only the variables its caller passes explicitly (child
   processes such as ffmpeg would otherwise inherit the environment).
2. `df_tools_builds_without_local_dev` (`cargo check -p df-tools --no-default-features`, in the
   gate).
3. `hosted_run_configuration_is_exactly_the_descriptor`: a descriptor and TOML file set every field
   of `RunConfiguration` to a valid value that differs from its default; the hosted composition's
   effective configuration is dumped as a canonical field listing and compared field by field. A
   field still taken from the environment or from a default shows as a difference. Timing, tempo,
   pricing and gate fields are part of the dump. Temporary directories and the telemetry exporter
   are configured fields too (explicit `temp_dir`, explicit OTLP endpoint and resource), so the
   dump covers what `env::temp_dir` and `OTEL_*` would otherwise decide.
4. `the_runtime_is_launched_with_a_cleared_environment`: the runtime reports the names (never
   the values) of the environment variables it saw at startup in its first audit line. For
   `NativeLauncher`, which spawns with `env_clear()`, the test asserts the names are a subset of
   `{PATH, HOSTNAME, HOME}`. For `DockerProxyLauncher`, which is never run against a real daemon,
   the test asserts the create body recorded by the fake proxy has an empty `Env`; Docker itself
   adds `PATH` and `HOSTNAME`, so the same subset rule applies on staging, and S2 checks that the
   runtime image declares no `ENV` (request to S2). Child processes go through the helper in item
   1.
5. The same scan covers every workspace crate in `df-server`'s normal dependency graph (from
   `cargo metadata`), against a checked-in allowlist of test-only reads, each confirmed under
   `#[cfg(test)]` in Round 1: `df-assets/src/native_file_store.rs` (`TMPDIR`),
   `df-persistence/src/hero_library.rs` and `local_demo_scope/restart.rs` (owned-PostgreSQL test
   harness variables), `df-providers/src/media_container.rs` (`DF_PROVIDER_MP4_SAMPLE`),
   `df-rules/src/coverage.rs` (`DF_R01_COVERAGE_REPORT`). The runtime read at
   `df-persistence/src/decision_rows.rs:515` (`DF_GAMEPLAY_DEMO_DATABASE`, choosing
   `action=new-game` on a rules-pin mismatch) becomes the `PinMismatchPolicy` parameter, and hosted
   composition passes `Compatibility` (9.5, SAVE-05). Third-party crates are covered by the cleared
   environment (item 4), not by the scan.

## 9. Save and resume contract (plan 13.7)

### 9.1 "Saved" means committed

Views carry `saved_revision`/`saved_at` from confirmed commits only and `backup_copied_revision`
from O2's checkpoint-bundle catalog event (O2 14). The client shows "Saved" and "Backup copied"
separately (W5).

### 9.2 Save points and asset references (O2 contract, D24)

A save point is written in its own fenced transaction, off the owner thread, by the runtime's
save writer task, naming a revision that is already committed. It never shares a transaction with
a player decision, so an archive race in O2's catalog can fail a save point and never an accepted
decision, and the owner's commit never takes per-asset locks.

- Trigger: the writer watches committed checkpoints (the existing `watch` channel) and writes a
  save point at every scene boundary, and at most every 50 decisions or 10 minutes of play
  (proposed), plus the kinds `all_away`, `idle`, `drain`, `session_end`, `host_request`.
- Transaction: `fence_current(..., 'game')`; check that the named revision is committed and is the
  newest one this owner wrote at or below it; insert `df_runtime.save_points` (basis, fence, kind,
  schema and codec versions, build/rules/content labels and digests, state digest = SHA-256 of the
  canonical envelope, last event sequence, draw count, pending-intent count and digest,
  `committed_at`); call O2's
  `media_catalog::record_save_references(&transaction, save_point, &references)` with the **full**
  asset list of that revision (O2 section 14), and store the digest O2 returns as
  `asset_manifest_digest`. Dropping a save point (compaction, 9.4) calls O2's
  `drop_save_references` in the dropping transaction; O2's garbage collection keeps every asset
  any remaining save point holds.
- A `ReferenceRefusal` (an asset no longer `Ready`/`Archived`) fails that save point with one
  audit line (`SAVE save_point_refused reason=asset_state asset=`); the writer retries at the next
  trigger with the then-current revision. The game is unaffected; the previous save point remains
  the resume point.
- Asset lists are extracted from the committed checkpoint on the writer task, not the owner
  thread. If FILE-03 (more than 512 assets) measures the full list as a problem, a delta form is
  proposed to O2 then; not before.
- "Saved" in the UI follows the decision commit (9.1), not the save point.

### 9.3 Side documents (D17)

`df_runtime.side_documents(tenant_id, session_id, kind, revision, digest, bytes,
written_at_revision)` for campaign records and lore (W3) and the social ledger and notebook (N);
the cost log is C's usage data, not a side document (D17). Rules:

- A side document is written in the same transaction as the checkpoint commit it belongs to
  (`SessionRepository::commit_decision` takes an optional bounded list of changed documents), or,
  for a change made outside a decision (an async social shift), in its own fenced transaction that
  records the current checkpoint revision.
- Only changed documents are written; owners keep a dirty flag and an expected revision
  (optimistic). Each document is bounded (proposed 1 MiB, per kind in configuration); the commit
  path never encodes an unchanged document.
- The file implementation stays for local play behind the same port.

### 9.4 Compaction (SAVE-04)

Expand step in `0017`: a checkpoint row may be compacted (`complete_envelope` NULL,
`compacted_at` set) when it is neither a save point nor among the latest 20 revisions; operations,
facts and intents keep their rows. A runtime-owned job runs between scenes. SAVE-04 drives a
six-player fake campaign to a large state and bounds storage per hour and recovery time.

### 9.5 Resume, compatibility, branch and restore

- **Resume (J04, SAVE-01):** membership plus a fresh ticket replace retained bearer grants, so
  credential expiry cannot strand a campaign. Claim, check compatibility, rebuild pending work from
  durable intents, restore views; no provider call during rehydration (an expired optional
  narration file is omitted, never regenerated).
- **Compatibility (SAVE-05):** `Compatibility::{Compatible, Migrate(plan), ReadOnly(reason)}` from
  schema, codec and pins against the runtime's supported set. A rules-pin, content-pin or build-pin
  mismatch is `Migrate` when a validated migration exists and `ReadOnly` otherwise: views and
  export, typed refusal of commands, no writes, old rows intact. It is never "start a new game":
  today's `decision_rows.rs:515` path (`CHECKPOINT recovery cause=rules-pin-mismatch
  action=new-game`, chosen by reading `DF_GAMEPLAY_DEMO_DATABASE`) becomes a parameter
  (`PinMismatchPolicy::{LocalNewGame, Compatibility}`) passed by composition; hosted passes
  `Compatibility` (8.4).
- **Restore is a production feature, not the dev rewind surface.** `local-dev` gates the dev
  control surface that calls rewind; the commit path (`validate_rewind`, `commit_owner_rewind`)
  lives in `df-session`, which has no such feature. Restore calls it with a new policy marker
  (`RewindPolicy::PlayerRestore`, beside `DEV_REWIND_POLICY`), host authority and the save point id.
  Intents and attempts:
  - Intents of the restored point that were never claimed for dispatch are dropped from the
    restored state; work the game still needs is requested again on resume with a new effect id
    and, if paid, a fresh quote.
  - Intents and attempts that are `Dispatching` or `Unknown` at restore time stay in
    reconciliation, untouched: their F dispatch rows, reservations and journal entries remain, keyed
    by the shared attempt id (D22).
  - A late result from any attempt made before the restore is rejected by the game as stale: the
    restored revision's job generation does not contain that attempt id, so the completion is
    discarded with one audit line (runtime-reliability "Validate session/run generation and
    operation identity"). F settles that attempt, keyed by attempt id and without a live owner,
    under D21/D22 (plan 6.3: output not delivered to
    the table is not charged to the customer; the supplier cost is company loss).
  - `UNIQUE (tenant_id, effect_id)` is the backstop against re-arming. No money row is read or
    written by the restore.
- **Branch:** S1 creates the new campaign; one transaction copies the save point's envelope (new
  session identity and recovery namespace), the side documents as of that revision, and `df_story`
  rows under the new campaign; O2's `copy_save_references` adds holders without copying bytes.
  - `df_story` rows with a `game_revision` at or below the save point's revision are copied; rows
    with `game_revision` NULL are not copied by revision: `rebuildable` ones are rebuilt in the
    branch from its canonical sources by the existing run index rebuild, and the rest are copied
    only when `created_at <= save_points.committed_at`, with their count in the branch audit line.
  - Member-private material (private clues in `df_story`, private state in the checkpoint) stays
    server-side under the original member ids. It becomes visible in the branch only when the same
    account joins it, through a `branch_member_map(original_member, account)` the branch writes;
    otherwise it is never projected.
  - The branch has no funding, consent or allocation until a new session is funded.

### 9.6 Local campaigns (D19)

Existing local campaigns are importable through a one-shot offline importer (`df-server import
--source <local database> --campaign-owner <account>`), never part of hosted startup. Order: this
contract (Rounds 2 and 5), then W3/N/C adopt the PostgreSQL stores, then the importer (O1, after
them), then SAVE-01. The importer reads the local run database and the run directory, writes the
hosted rows under a new campaign with a fresh recovery namespace, backfills `df_story` tenant and
campaign (0019), and refuses a source whose pins it cannot validate.

## 10. Egress dispatcher contract (D18, D21-D23, D25, D27)

O1 builds the dispatcher in Round 4 (coordinator ruling).

### 10.1 Process, identity and routes

- `df-server egress` on each worker host is the only holder of provider credentials (key files
  from S2's secrets layout). It has no durable local path.
- Inbound: runtimes call it over the internal network with mTLS. It authorizes each caller by
  certificate identity, not by chain alone: the client certificate's URI SAN
  `urn:dungeonflux:placement:<placement_id>` (issued per placement, 7) must equal the
  `placement_id` of the request's fence, and that placement must be current in
  `placement_secrets`; the placement token is checked as well.
- Outbound: allow-listed provider hosts (service-operations fetch policy: HTTPS, host allowlist,
  redirect limit, no private addresses), PostgreSQL, and the control plane's internal mTLS listener
  on the data network (S2 builds it), used for staging delivery to the media gateway and for the
  journal submission calls O2 exposes there; the control plane authorizes the dispatcher by its
  certificate's URI SAN `urn:dungeonflux:dispatcher:<worker_id>`. There is no route from the
  control plane to a dispatcher (D23): work for dispatchers arrives as database rows (10.4).
- Every process start draws a fresh 16-byte `dispatcher_instance_id`.
- **Admission is F's endpoint, not the dispatcher's (D30).** The runtime sends its
  `AdmissionRequest` (F 18.1) to F's finance endpoint on the control plane's internal mTLS
  listener. That endpoint checks that the client certificate's URI SAN equals
  `fence.placement_id` and that the placement is current; F's finance service role takes
  `enter_service()` first in each transaction, is bound by the reconnect gate (6.3) and the 5 s
  idle-in-transaction limit (6.1), and holds `EXECUTE` on `df_runtime.fence_current`. The attempt
  starts as `Reserved` at admission and moves to `DispatchPrepared` through F's
  `prepare_dispatch(attempt, fence)` when the runtime claims the effect, before it asks the
  dispatcher to send.

### 10.2 Needs from F (`finance.provider_attempt`, F 11.3 and 18.2)

F is authoritative for the attempt table, its states, grants and policies. O1 needs:


1. **`dispatcher_instance_id`** on `finance.provider_attempt`, written by `mark_attempt_sent`
   from `df_runtime.current_dispatcher()` (the caller's registered instance, below), never from a
   parameter.
2. **`mark_attempt_sent(attempt, fence)`** accepts only attempts whose campaign's current
   placement (`df_runtime.campaign_owners.worker_id`) runs on the calling dispatcher's worker
   (`dispatcher_instances.worker_id`), in addition to its existing lock-then-`fence_current(...,
   'paid')` order; and it checks `current_dispatcher()` is not NULL.
3. **`Sent -> VerifiedUnsent`**, a new `dispatch_state`, by a definer such as
   `mark_attempt_unsent(attempt)`: allowed only when the attempt is `Sent` and its
   `dispatcher_instance_id` equals `current_dispatcher()`, and it releases the reservation and the
   supplier exposure in the same call.
4. **`Sent -> OutcomeUnknown | ResultKnown` by F's reconciliation** when the sending instance is no
   longer live in `dispatcher_instances` (restart, host loss) or the attempt has been `Sent` for
   more than `dispatch.sent_reconcile_after_seconds` (proposed 300); provider key or status lookup
   decides, else `OutcomeUnknown`.
5. **D25's limit (proposed 24 h)** applies to attempts stuck in `Sent` as it does to
   `OutcomeUnknown` (to `FailedUnknown`, customer credited per D21).
6. **`current_dispatcher()` checked inside every F definer a dispatcher calls**
   (`mark_attempt_sent`, `mark_attempt_unsent`, `record_transport_outcome`), so only the sending
   instance records the transport outcome of its own attempt.
7. **No `clear_paid_work_freeze`**: reconstruction ends with `df_runtime.record_money_state`
   (6.3); F 22.2 step (4) should call that instead.
8. **Pricing**: F prices `[opened_at, closed_at)` of closed segments minus credits; O1 closes rolls
   at proven progress (5.2), so F's `proven_until` is the segment's `closed_at` and no extra column
   is needed (answer to F 13.2).

**Dispatcher instances (O1, `df_runtime`).** `df_runtime.dispatcher_instances(instance_id PRIMARY
KEY, worker_id, token_digest, started_at, last_seen_at, ended_at)`. At start a dispatcher draws an
instance id and a 32-byte token, and registers through the definer
`df_runtime.register_dispatcher(worker_id, instance_id, token_digest)`, authorized by its
certificate identity at the control plane and by `df_egress`. Every dispatcher transaction sets
`SET LOCAL df.dispatcher_token = $token`; `df_runtime.current_dispatcher()` (definer, reads the
table, which is granted to no login role) returns the instance whose `token_digest` matches and
whose `ended_at` is NULL and `last_seen_at` is within 30 s (heartbeat every 10 s), else NULL. A
stopping dispatcher sets `ended_at`.

### 10.3 Send sequence (every metered route, Jev included, D21)

1. One transaction: `enter_service()`; `SET LOCAL df.dispatcher_token`; F's
   `mark_attempt_sent(attempt, fence)` (locks the attempt, `fence_current(..., 'paid')`,
   `DispatchPrepared -> Sent` with this instance, 10.2); insert the `Dispatching`
   `journal.pending_entry` (O2's submitter model); commit.
2. Wait for the writer to confirm that entry. This needs O2's writer to drain on every `NOTIFY`,
   batching every entry pending at that moment into one object-store write (request to O2).
3. Immediately before writing request bytes, a short read transaction: `enter_service()`,
   `fence_current(..., 'paid')`. Then the send path moves the latch to `Sending`.
4. Hand the request to the HTTP client, then record the outcome through F's
   `record_transport_outcome` (`ResultKnown` or `OutcomeUnknown`).

**The latch.** Each attempt has one in-memory gate with three states, `Armed`, `Sending`,
`Disabled`, changed only by compare-and-swap. The send path moves `Armed -> Sending` immediately
before it hands the request to the HTTP client, because the dispatcher cannot observe when the
client writes the first byte; if the compare-and-swap fails it hands nothing over. From the hand-off
on the attempt counts as possibly sent. The timeout or refusal path
moves `Armed -> Disabled` **before** it commits `VerifiedUnsent`; if that compare-and-swap fails
(the send already started), it records nothing and the attempt proceeds as sent. So a confirmation
that arrives at 2,001 ms cannot send an attempt already recorded as unsent. `VerifiedUnsent`
therefore means the request was never handed to the HTTP client, which is the only way the
dispatcher can know no request bytes reached any connection, pooled connections included.

**Unconfirmed or refused.** When confirmation does not arrive within
`dispatch.journal_confirm_timeout_ms` (proposed 2,000), or the recheck fails, the dispatcher
disables the latch, then calls F's `mark_attempt_unsent` (`Sent -> VerifiedUnsent`, same
instance only, reservation released in the same call, 10.2). The runtime gets `JournalUnavailable`, `FenceStale`,
`MoneyNotReconciled` or `ServiceHalted`. The pending journal entry may still be confirmed later;
restore reconciliation prefers a database `VerifiedUnsent` and treats the attempt as possibly sent
only when that record is missing. After a dispatcher restart its old `Sent` rows carry another
instance id, cannot become `VerifiedUnsent`, and are moved by F's reconciliation (10.2 items 4-5). No path leaves a permanent Unknown.

**Latency budget (ruling (c)).** Journal-before-send may add at most p99 25 ms per call
(proposed); measured in Round 1 with the batched writer on staging-shaped storage and reported to
the owner. A miss is reported, not worked around by weakening the order.

A second request for the same attempt never sends (JOB-01, JOB-02).

### 10.4 Provider output and re-fetch

- **Handle storage.** The provider fetch handle is persisted before the response is acknowledged
  to the runtime, in `df_runtime.dispatch_outputs(attempt_id PRIMARY KEY, tenant_id, campaign_id,
  route, writer_instance, key_id, nonce, handle_ciphertext, handle_expires_at, staging_state,
  staging_attempts, last_error)`, written in the same transaction as the `ResultKnown` outcome (F's `record_transport_outcome`).
  - Encryption: AES-256-GCM (`ring`) under one dispatcher output key shared by every dispatcher
    (S2 secrets), identified by `key_id`; associated data `attempt_id || key_id`, so a ciphertext
    copied to another row fails to open; a random 96-bit nonce per row.
  - Rotation: new rows use the newest key; older keys stay available for decryption until no row
    with their `key_id` still has a ciphertext (at most the longest handle lifetime), then they are
    retired.
  - Clearing: `handle_ciphertext` is set to NULL when `staging_state` reaches `quarantined` or
    `already_staged`, and by the dispatcher's sweep when `handle_expires_at` passes.
  - Access: granted only to `df_egress`, `FORCE ROW LEVEL SECURITY`, with a `writer_instance`
    column and one policy for all commands:
    `USING (writer_instance = df_runtime.current_dispatcher() OR EXISTS (SELECT 1 FROM
    df_runtime.refetch_requests r WHERE r.attempt_id = dispatch_outputs.attempt_id AND
    r.claimed_by_instance = df_runtime.current_dispatcher() AND r.outcome IS NULL AND
    r.claimed_until > clock_timestamp()))`, `WITH CHECK (writer_instance =
    df_runtime.current_dispatcher() OR <the same claim condition>)`. A dispatcher reads a handle only
    for an output it wrote or a re-fetch it currently holds; no runtime or control-plane role can
    read the table. The handle is never logged.
- **Staging.** The dispatcher streams the output through the control plane's internal listener to
  the media gateway's staging endpoint with `{asset, attempt}` and retries with backoff until it
  answers `Quarantined` or `AlreadyStaged` (or a terminal refusal: attempt no longer current, over
  the size bound). It re-fetches through the handle while it is valid, so no new provider call.
- **Re-fetch requests (ruling (a)).** `df_runtime.refetch_requests(attempt_id PRIMARY KEY,
  requested_at, request_count, attempts, claimed_by_instance, claimed_until, outcome, finished_at)`.
  - Request (O2's sweeper, `df_control`): `INSERT ... VALUES ($a, clock_timestamp(), 1, 0, NULL,
    NULL, NULL, NULL) ON CONFLICT (attempt_id) DO UPDATE SET requested_at = clock_timestamp(),
    request_count = refetch_requests.request_count + 1, attempts = 0, claimed_by_instance = NULL,
    claimed_until = NULL, outcome = NULL, finished_at = NULL WHERE refetch_requests.outcome IS NOT
    NULL`, so a finished row is reset for a later request and an open one is left alone; then
    `NOTIFY df_refetch` as a hint.
  - Claim (dispatchers poll every 5 s, proposed): `UPDATE df_runtime.refetch_requests SET
    claimed_by_instance = df_runtime.current_dispatcher(), claimed_until = clock_timestamp() +
    interval '60 seconds', attempts = attempts + 1 WHERE attempt_id = $a AND outcome IS NULL AND
    attempts < $maximum_attempts AND (claimed_until IS NULL OR claimed_until < clock_timestamp())
    RETURNING attempt_id`. The holder renews `claimed_until` every 20 s while it works.
  - Finish: the holder sets `outcome` (`staged`, `already_staged`, `handle_expired`,
    `attempt_not_current`, `failed`) and `finished_at`. A row whose `attempts` reaches
    `refetch.maximum_attempts` (proposed 5) without success is finished as `exhausted`; O2's sweeper
    then applies its `output_lost` rule. Finished rows are deleted after
    `refetch.retention_days` (proposed 7) by the dispatcher sweep.
  - Double work is harmless: if a holder dies or stalls past its claim, another instance may
    download again, and the staging endpoint answers `AlreadyStaged` to the second write, with no
    new provider call.
  - Policies (`FORCE ROW LEVEL SECURITY`): `CREATE POLICY control_request ON
    df_runtime.refetch_requests FOR INSERT TO df_control WITH CHECK (true)` and `FOR UPDATE TO
    df_control USING (outcome IS NOT NULL) WITH CHECK (outcome IS NULL)` (the reset only);
    `FOR SELECT TO df_egress USING (true)`; `FOR UPDATE TO df_egress USING (outcome IS NULL AND
    (claimed_until IS NULL OR claimed_until < clock_timestamp() OR claimed_by_instance =
    df_runtime.current_dispatcher())) WITH CHECK (claimed_by_instance =
    df_runtime.current_dispatcher())`; `FOR DELETE TO df_egress USING (outcome IS NOT NULL AND
    finished_at < clock_timestamp() - make_interval(days => 7))`.
- **Inline bytes.** Routes that return bytes inline (no handle) hold them in memory, bounded by
  `egress.maximum_unstaged_bytes` (proposed 512 MiB), until staging is acknowledged; if the
  dispatcher dies first, O2's `staging_deadline` makes the row `Rejected(output_lost)` with a
  company-loss settlement row (D21).
- **Late results** (plan 6.3 reconciliation) go through the same staging endpoint while the
  attempt is still current; afterwards the endpoint refuses and the result is discarded.

### 10.5 Failure surfacing and runtime side

- Every failure returns a typed `DispatchOutcome::Failed { class }`, `class` in
  `{ ProviderUnavailable, ProviderRefused, Timeout, OutcomeUnknown, InvalidOutput, CircuitOpen,
  JournalUnavailable, FenceStale, MoneyNotReconciled, ServiceHalted }`. The runtime maps it to the
  plan's notice for that capability ("Narration provider unavailable; no new charge"), never to
  in-world speech, narration or a durable finding (D21). A sent call that produced no usable output
  is recorded by F as company loss and not charged to the customer.
- Streaming routes (speech) are relayed in bounded chunks. The dispatcher owns the circuit breaker
  per provider (JOB-03) and the company provider caps (configuration, default off).
- Each provider client in the runtime takes a base URL and no key; hosted composition points it at
  the dispatcher with the route name. Only metered routes are dispatchable; a `Metering::Pending`
  route is refused in hosted mode.

## 11. TLS to PostgreSQL (Phase B requirement)

- `df-persistence::postgres_tls`: `PostgresTls::VerifyFull { ca_file, server_name }` and
  `PostgresTls::LoopbackPlaintext`, implementing `tokio_postgres::tls::MakeTlsConnect` over
  `tokio-rustls` (rustls 0.23.45, tokio-rustls 0.26.6 and ring 0.17.14 are already in
  `Cargo.lock`). Verification of chain and host name always on; trust store is the configured CA
  file only; TLS 1.2 minimum; channel binding where offered.
- Plaintext only when the configured host is a literal loopback address (127.0.0.0/8 or `::1`,
  parsed as an IP, no DNS lookup) or a local socket path, and `tls.mode = "loopback-plaintext"` is
  explicit. Any other host with plaintext is a typed startup refusal; staging and production
  configurations cannot express plaintext.
- Every connection site takes a `PgConnector` from composition (list in 1.9); `df-tools` sites are
  part of the seam (8.2), qualification harnesses stay loopback under `local-dev`.
- Tests: an in-process rustls server on loopback with committed test certificates (test values
  only) accepts the right CA and rejects another CA and a wrong host name;
  `plaintext_to_a_non_loopback_address_is_refused`.

## 12. Production metrics and telemetry (D20, D23)

- One Prometheus text exporter per process (control plane, each runtime, dispatcher, runner),
  hand-written (no new dependency), backed by a `MetricsRegistry` of counters, gauges and
  fixed-bucket histograms with bounded label sets (route templates and closed enums, never ids or
  player text). Runtimes push their exposition to the host's telemetry ingestor every 10 s
  (proposed) over the allowed ingestor connection; Prometheus scrapes the ingestor, never a
  runtime (D23). The control plane, dispatcher and runner expose `/metrics` on their internal
  listeners for the ingestor or Prometheus as S2 configures.
- Five separate signals (plan 16.3): `live` (process and event loop answer), `ready`
  (dependencies, schema range, lease current at my fence, failover check passed since my last
  reconnect, not draining), `database` (round trip, pool in use), `providers` (circuit state per
  provider, from the dispatcher), `progress` (owner progress age, longest reduce in progress, inbox
  depth). The control plane, dispatcher and runner serve them as `/healthz/<signal>` on their
  internal listeners; a runtime serves them only to the edge on its internal listener and pushes
  them with its metrics. `df-server healthcheck --address --signal` reads them inside a container
  (exit 0 healthy, 1 unhealthy, 2 unreachable).
- Series O1 emits: `df_owner_reduce_ms`, `df_owner_queue_wait_ms`, `df_owner_inbox_items`,
  `df_owner_progress_age_seconds`, `df_owner_reduce_in_progress_seconds`,
  `df_lease_renew_failures_total`, `df_meter_segment_open`, `df_blocking_waits_active`,
  `df_players_connected`, `df_process_resident_bytes`, `df_db_commit_ms`, `df_db_pool_in_use`,
  `df_edge_connections`, `df_edge_route_lookups_total{outcome}`, `df_worker_slots_used`,
  `df_worker_slots_total`, `df_save_lag_ms`, `df_failover_checks_total{outcome}`.
- Domain owners emit through the same registry (D20): B finance, O2 storage, C cost with the price
  version label, W4 text and voice latency. A domain module registers its series at composition;
  no second exporter.
- `df-telemetry` wiring: each process exports OTLP logs and traces to the host's ingestor
  (df-telemetry, SQLite segments and spool under `/var/lib/dungeonflux/telemetry`, bounds from
  service-operations); runtimes keep a bounded local spool (proposed 256 MiB) for ingestor outages
  and report loss explicitly. The audit sink (`df_observe::install_audit_sink`) feeds the same path.

## 13. Answers to S2 (round-0 "From O1")

1. Binaries: `df-server` (modes above) and `df-runner`; config `--config /etc/dungeonflux/<role>.toml`
   with `[instance]`, `[listen]`, `[database]` (`tls.mode = "verify-full"`, `tls.ca_file`,
   `password_file`, pool size, statement timeout), `[objects]` (O2), `[identity]`/`[payments]` (B),
   `[providers]` (dispatcher only), `[runtime]` (lease, presence, waits, drain, seats), `[telemetry]`.
2. Endpoints and series: section 12. Runtimes push to the host ingestor; Prometheus scrapes the
   ingestor and the control plane, dispatcher and runner, never a runtime (D23).
3. `--healthcheck`: section 12.
4. Drain: section 6.5; `SIGTERM` and `POST /internal/drain` equivalent; edge closes tunnels after
   `drain.edge_seconds` (proposed 20); orchestrator stop timeout 100 s.
5. Writable paths: `/tmp` tmpfs 512 MiB; `/var/cache/dungeonflux` bounded by O2 (2 GiB LRU,
   disposable); `/var/lib/dungeonflux/telemetry` per host, persistent, service-operations bounds;
   runtime spool 256 MiB. Runtimes move media bytes only through the media gateway, so they need no
   object-store path or credentials. No other durable local path in hosted mode (9.3).
6. Runner interface and proxy policy: section 7.
7. Migration runner: `df-server migrate status|plan|apply` with `df_migrator` credentials; ledger
   `df_meta.schema_migrations(version, name, sha256, phase expand|contract, minimum_binary,
   maximum_binary, applied_at, applied_by)`; `pg_advisory_lock`; one transaction per migration;
   applied versions skipped; changed checksum refused; readiness fails outside the binary's range.
8. TLS: section 11.
9. Fake providers: `[providers] mode = "fake"` wires the in-process fakes into the dispatcher and the
   runtime; `live` refused in staging unless `allow_live = true`. Which capabilities have a faithful
   fake is a Round 3 audit; one without stays off in fake mode and says so at startup.
10. Feature flags: `df_runtime.feature_flags(scope, category, state, reason, changed_by,
    changed_at)`; categories `narration_voice`, `transcription`, `scene_stills`, `portraits`,
    `music`, `cinematics`, `splat_worlds`, `prefetch`; read on hint and every 5 s into
    `TableGate`; a stopped category admits no new work and keeps the session. Distinct from the
    customer generation policy (plan 4.4).

## 14. Migration path

1. Local play stays the default at every step (`local-dev` on).
2. `df-server` compiles with configuration, TLS, migrations, health and metrics, without the game.
3. Runtime tables, lifecycle and directory are exercised with fakes and the scratch PostgreSQL
   harness, still without the game.
4. The seam (8.2) lands after W4's stages 0 and 1 (D16); the hosted runtime plays a fake session
   on loopback.
5. Runner, edge and dispatcher connect it; OPS-01 and J10 run on one machine.
6. Save points, side documents (with W3/N), compaction, restore and branch; then the importer; then
   SAVE-01..05 (D19 order).
7. Capacity measured; slot formula configured from it.

## 15. Order of work (rounds after critic review)

| Round | Delivers | Proving tests |
|---|---|---|
| 1 | `df-server` member; typed config; subcommands; health and metrics registry; `postgres_tls`; migration registry and `migrate`; the subsystem-architecture row (below); journal-before-send latency measurement against O2's writer on staging-shaped storage (ruling (c)) | config refusals; TLS accept/reject; literal-loopback check; migration checksum and order; healthcheck exit codes; measured p50/p99 added latency reported to the owner |
| 2 | `df-session::lifecycle` (holds, presence `watch`, wait board, rolling segments, credit function); 0015, 0016, 0018, 0019; definer functions; placement, claim, renew, release; `fence_current`; failover check and `service_state`; sweeper (orphans and abandoned tables); row policies | TIME-01..03; fence tests including the lost superseding claim; no deadlock between renewals and commits; catalog walk; cross-campaign and superseded-token denial; orphan and abandoned sweeps; credit overlap refused |
| 3 | the seam (D16, after W4); `hosted_scope`; one bounded pool per runtime; edge player and display paths; batch-start membership and the `event_seq` feed consumer; `local-dev` and the environment enforcement (8.4); `PinMismatchPolicy` | AUTH-01, AUTH-02, J07; production surface; configuration dump; dependency-graph scan; feed lag above 5 s pauses private views and new streams; a campaign-level revoking row forces re-authorization; seating refuses a member without `holds_seat` and beyond `seat_count` |
| 4 | `df-runner` (cleared-environment launch); drain, transfer, idle release; edge recheck; local lease exit; the egress dispatcher (send sequence, handle persistence, staging retry, late results) | OPS-01; J10; racing runners; stale upstream closed; JOB-01/02; unconfirmed entry records `VerifiedUnsent`; a confirmation after the timeout sends nothing (latch); `VerifiedUnsent` from another instance is refused; fence lost between confirm and egress sends nothing; re-fetch through a request row makes no new provider call; a handle ciphertext moved to another row fails to open; another instance cannot move an attempt it did not send; a finished re-fetch request is never claimed again and is reset by a new request |
| 5 | save writer (own fenced transaction, full asset list to O2); side-document port in the commit transaction; compaction; compatibility; restore with reconciliation kept; branch | SAVE-02..05; a refused save point never fails a decision; side documents written only on change; stale pre-restore results rejected |
| 6 | importer (after W3/N/C adopt); SAVE-01; capacity measurement | SAVE-01; measured p95 memory and CPU |

**Subsystem-architecture change (D15, Round 1; the file is currently locked by B):** the
`df-server` row's dependency list gains `df-tools` (as the game-runtime library, `default-features
= false`), its ownership text reads "composition binary: control plane, campaign runtime, egress
dispatcher, migrations, health and metrics", and a new `df-runner` row: "trusted per-host runner:
placement, leases, launch through a launcher port; depends on df-types, df-observe,
df-persistence".

## 16. Questions

**F (D27), alignment:** F is authoritative for every finance table and function. O1's needs are
the eight items of 10.2, the admission endpoint shape of 10.1 (SAN check, `enter_service()` first,
reconnect gate, idle limit, `EXECUTE` on `fence_current`), reconstruction as an endpoint call
ending in `record_money_state` (6.3), and `meter_funded_until` read at segment open and roll
(5.2). Answered by F's note: price version pinned by `open_session_funding` (F 18.8); funds state
`session_funding_state` (F 18.8, lock-free, `NOTIFY df_finance_funding` as hint), folded into
`TableGate`; start eligibility `start_readiness` (F 18.8), read in O1's `Start`.

**B:** In-memory pending sign-ins are fine for one control-plane instance only; acceptable for the
pilot?

**O2:** (0) Q-O1-2: agreed, `AssetReference.key` carries the `AssetId`. Requests: your writer
drains on every `NOTIFY` with batching (10.3); the staging endpoint answers `AlreadyStaged` for a
repeated write of a staged attempt (10.4); your sweeper requests re-fetch through
`df_runtime.refetch_requests` (ruling (a)); the save path follows D25 (O1 rows, then media);
`RuntimeSpec` has no object-store scope (7). (1) The save writer calls `record_save_references` with the full list in its own fenced
transaction and `drop_save_references` when compaction drops a save point (your section 14, D24);
your policies on tables granted to the runtime use the scope convention of 4.4. (2) The media
gateway's runtime endpoint (D23): request shapes for reading and staging bytes by catalog handle
under the placement identity. (3) `Q-O1-4`: I will provide the
audience-safe `permitted_excerpts(campaign, member)` from the projection layer in Round 5.
(4) `end_room(campaign, epoch)` is called with the full fence on release.

**S1 (agreed, taken in from your request.md round 0d):** `df_runtime.open_session(campaign) ->
SessionId` (your name; it creates the `df_runtime.table_sessions` row in phase `Lobby`, idempotent,
at most one non-ended session per campaign; O1 owns the row afterwards, no foreign key from your
tables); seating by `holds_seat` and `seat_count` under the `maximum_seats` ceiling (6.4);
`member_snapshot` for ticket minting (6.4); `POST /display/renew` extending the same ticket, with
O1 building the client (6.4); the feed consumer as a gate with no durable cursor, keyset polling
with a 15 s overlap, `LISTEN` as a hint, revoking as re-check and campaign-level re-authorization,
and `publication_batch_maximum_ms` (6.4); unfenced `session_ended` and `revoke_display_tickets` on
every session end (6.5); local-only Reclaim and room code (8.2).

**Requests to S1:** (0) Your fenced writes call `enter_service()` first, then
`df_runtime.fence_current(tenant, campaign, recovery_epoch, claim_seq, placement_id, 'game')`
(six arguments; your request cites five), then lock `campaigns.campaign` (D29). Use the 4.4 scope
convention (`leased_table_sessions()` for `display_ticket`). (1) Your `start(session, roster)` calls
my `Start` with F's funding evidence. (2) `SaveDirectory::latest` reads `df_runtime.save_points`.

**Owner (proposed defaults, configuration):** blocking-wait grace 10 s gated and 0 s outage with the
blocked interval free; presence window 30 s; lease 30 s with 10 s renewal; idle release 15 minutes;
away auto-end 12 hours; segment roll 60 s; the company absorbs the uncertain tail after the last
proven progress (at most about 25 s per incident); paid work pauses only while F's automatic
reconstruction runs after a crash (proposed p95 under 5 s, 30 s deadline) and freezes only if it
fails (ruling (b));
the latency journal-before-send adds to every paid call (budget p99 25 ms, measured in Round 1);
and whether the selected DigitalOcean tier gives a synchronous standby (account eligibility, 6.3).

**S2 (requests):** no image declares `ENV` (8.4); no statement-mode connection pooler anywhere
(transaction-scoped advisory locks and `SET LOCAL`, 6.1); session or transaction mode only; the control plane's internal mTLS
listener for dispatchers (10.1, ruling (b)); certificate issuance with the URI SANs of section 2;
the deployment paragraph of `service-operations.md`.

**Requests (not edits):** wait guards at gated sites (W3, N, C, W6); `TableGate` read at paid and
background generation sites; lifecycle and save fields rendered by the clients (W5); side-document
adoption (W3, N); cost log to PostgreSQL (C); the 10.2 needs, the reconstruction endpoint and `record_money_state`
in place of `clear_paid_work_freeze` (F);
the batched journal drain (O2); configuration fields replacing environment reads at
the files in 8.4.

## 17. Risks

- The seam sits in the busiest files and waits on W4; a late split delays Rounds 3-6.
- The scope function's cost per statement is unmeasured.
- Pool size per runtime (proposed 4) times slots against the managed plan's connection limit;
  transaction-mode pooling needs unnamed statements or session mode. Measured in Round 6.
- Capacity is unknown until measured; no campaign count is claimed (plan 12.3).
- A failover that loses committed writes is detected (6.3) and halts service until the recovery
  epoch is raised; the lost game commits are the restore path's declared RPO, not a runtime
  guarantee. Detection relies on every process's connection breaking at a primary change and on
  the journal mirror of claims and dispatches.
