# Server concurrency and latency research

Status: research and proposals, 2026-10-24. No source was changed. Requirements come from
[Coding style](coding-style.md) section 7, [Runtime reliability](runtime-reliability.md) and
[Optimization](optimization.md); everything below is a proposal until the owner picks stages.

Owner question: "start planning on multithreading the server for lower latencies, or at least
async; research this and its benefits."

## 1. Answer in short

1. The server is already multi-threaded and async where it matters. The game runs on a tokio
   multi-thread runtime with one worker per core (18 on this machine) and provider work is async
   (`run_restart.rs:41`). Adding threads to the runtime will not lower latency.
2. The one-thread owner is not the bottleneck today. Since the HS54 view fix it spends 0.3% of
   wall time reducing calls (p50 3-4 ms, p99 under 210 ms). It is a tail-risk: on the earlier
   incident its calls took 12 s at p95.
3. Player-visible latency is sequential model calls. A hero line costs about 8.4 s at p50 and
   30 s at p95 from the dev `/say` path; a travel line to a new place cost 35-59 s. Postgres
   commits are 10 ms at p50. Parallelising and trimming the model chain is where the seconds are.
4. Recommended order: instrument first (stage 0), then move work off the reply's critical path
   (stages 1-3), then shrink the owner's tail (stage 4). Per-room actors and a lock-based state are
   not worth it for latency.

## 2. What exists (verified in code)

### 2.1 The owner thread

- One OS thread named `df-real-gameplay-owner`, 16 MiB stack, drains an inbox with
  `run_with_owner_wake` (`crates/df-tools/src/gameplay.rs:2572-2586`).
- The inbox is a `std::sync::Mutex` queue plus `Condvar`, FIFO, bounded to 256 items and 2 MiB
  (`crates/df-session/src/inbox.rs:7-8`, admission `:157-195`, drain loop `:232-323`, the single
  `reducer.reduce` call at `:312`). Queue wait is not recorded anywhere.
- 24 `Call` variants (`actor.rs:371-521`) all reduce on it: View, Submit, Dialogue, PlaceMove,
  NpcEvent, Join, Presence, Presented, TelemetryIdentity, Dev and more. `Reducer::reduce`
  (`actor.rs:5492`) runs `heartbeat_if_due` and `run_committed_intents` before every
  non-dialogue call (`actor.rs:5705-5708`); that time is inside neither the
  `gameplay.local_demo_call` span (begins `actor.rs:5885`) nor any per-call figure except the
  `CALL caller_gone/refused ms=` lines.
- Every authenticated call does a synchronous Postgres query on the owner:
  `authenticate_with_principal` does `runtime.block_on(query_opt("SELECT ... FROM
  df_local_demo.grants ..."))` with a 2 s timeout (`crates/df-persistence/src/local_demo_scope.rs:320-339`).
  A call that only authenticates (`TelemetryIdentity`) logs 0-2 ms, so one round trip is about 1 ms,
  a third of the 3-4 ms p50.
- The owner also blocks on model calls in one place: `resume_fight_reactions` builds a nested
  current-thread runtime and `block_on`s it with an 8 s timeout (`actor.rs:1870-1915`; called
  from `actor.rs:2010`, `:3478`, `:4369` and `dialogue.rs:9724`, all on fight close). It is rare
  but it is exactly the "owner thread never blocks on a provider" rule (style 7) broken.

### 2.2 Views

- `Actor::view` (`actor.rs:5286-5375`) is not a pure projection. Besides
  `wire::journey_view` it mutates: `session.reaction.remember` (`:5302`), `social.sync_room` and
  `living_scene::publish` (`:5305-5316`), `notebook.observe_scene` (`:5332`),
  `notebook.sync_task_reward` (`:5350`), `tempo.decorate(&mut ..)` (`:5368`) and `seat_activity`
  (`:5373`, in memory, DB write at most every 5 s per REV-21).
- Fan-out: every watch stream (the TV and each phone) wakes on every checkpoint change, table
  line, layout change, art change and every 5 s, then submits one `Call::View`
  (`gameplay.rs:745-760`, `room.rs:16-60`, retrying every 10 ms while the inbox is full). With
  seven watchers each revision causes about seven full projections on one thread.
- The expensive derivations are memoised in `thread_local!` caches on the owner thread:
  `PHASES` (8 entries) and `RECREATED` (4), compared by deep `GameState` equality
  (`journey.rs:244-288`, `:4950-4971`). Other non-test `thread_local!` state exists
  (`beat_script_runtime.rs:139,717,884,1822`, `fight_foresight.rs:295`, `living_scene.rs:1838`,
  `tactical.rs:310`, `place.rs:1813`, `refusal_detail.rs:20`). A projection moved to another
  thread would start cold.
- Readers already have a snapshot path: the owner does `updates.send_replace(checkpoint.clone())`
  (`actor.rs:777`, `:2200`, `:2269`, `:2287`, `:2567`) and async code reads the
  `watch::Receiver<Checkpoint>`. But `Checkpoint` is a plain deep-cloned struct (`df-model
  checkpoint.rs:769-775`, `GameState` is vectors of records, no `Arc`) and the async side calls
  `updates.borrow().clone()` at 141 sites in `df-tools/src`, four or more per hero line
  (`dialogue.rs:4983,5050,5080,8489`). Per tokio docs a `borrow()` holds a read lock on the sender
  while alive; clone-under-borrow is short but unmetered.

### 2.3 Provider work

- Async tokio tasks with permit pools in `imagery.rs:539-542` (presence, general, background,
  head permits). Image, voice and music work does not touch the owner.
- Hero line pipeline, in order (all on tokio workers except step 6):
  1. `submit_line` (`dialogue.rs:4977`): `classify_input_listed` (`:5409`), then
     `resolve_addressee_with_party` (`:5529-5533`, plus an owner call `party_heroes`), then
     `judge_scene_interaction` (`:5639`), then `judge_listed_service_request` (`:5677`), plus
     `hero_line_form` and the merchant trade judgement. The line tidy-up (`line_cleanup`) is already
     started speculatively (`:5556`).
  2. Travel lines branch to `travel_by_line` (`:5436`) and place generation.
  3. The hero line is committed (`dialogue_call` -> `Call::Dialogue`, `dialogue.rs:4219`).
  4. `complete_dialogue_response` (`dialogue.rs:8441`): awaits the cleanup, publishes the typed
     line (`:8486`), then `converse_report` (`:8629`).
  5. `reply_in_public_report` (`npc_reply.rs:2478-2657`): approach and focus already run with
     `join` (`:2489`); then `prepare_fact_reply` (target -> decision -> reason -> proposal/backfill,
     `fact_backfill.rs:322-540`), `judge_person_influence` (`:2533`), `model_public` (`:2547`),
     three guards (`:2575-2580`), `public_reply_event` (`:2587`), and only then the line is pushed to
     the table log (`:2619`).
  6. NPC event commit goes back to the owner (`Call::NpcEvent`, `gameplay.rs:187-229`).
- The dev `/say` path measures both phases together (`dev_control/actions.rs:1508-1559`,
  `STORY exchange total_ms`).
- Serialisation: `prepare_fact_reply_inner` takes `self.fact_decision.lock().await`
  (`fact_backfill.rs:344`), a single `tokio::sync::Mutex<()>` inside `NpcVoice`
  (`npc_reply.rs:2076`, `:2297`), held across Jev calls, up to three model proposals and the fact
  commit. It is one lock for the whole server, so six heroes talking to six different NPCs queue
  behind each other during fact preparation. This is deliberate (REF-069: one writer for new
  local canon; comment at `fact_backfill.rs:319-321`) but its scope is wider than the invariant.

### 2.4 Persistence and runtime

Postgres through `df-persistence`; fenced, journaled commits that the owner waits for; checkpoints
are immutable values. The restart supervisor uses a current-thread runtime on Windows
(`runtime.rs:55-72`); the game runs in `serve_run` on a multi-thread runtime with 16 MiB worker
stacks (`run_restart.rs:41-45`). Machine: Snapdragon X2 Elite Extreme, 18 cores.

## 3. Measurements

Source: the eight newest launcher logs in `artifacts/tmp/play/launcher-logs/`
(`df_gameplay_demo_play_20261024181230` to `...213227`, 7-48 minutes each) plus the incident log
`...171809_a52c`. Limits: lines carry no timestamps, so stages are tied together by log order only
and parallel heroes blur it; the numbers are the dev-driven play sessions (up to six scripted
heroes, `/dev/say`), not real phones; sample sizes are small (81 inputs, 88 exchanges). The scripts
that produced the tables were throwaway Python over these logs and are not kept.

### 3.1 Owner thread

`OUTCOME operation=gameplay.local_demo_call ms=` (reduce time after the budget check):

| Log | calls | p50 | p95 | p99 | max | busy | uptime | busy share |
|---|---|---|---|---|---|---|---|---|
| a52c (incident, before HS54) | 398 | 524 ms | 12 380 ms | n/a | 43 360 ms | n/a | 1 499 s | n/a |
| 190243 | 1 143 | 4 | 20 | 166 | 424 | 8 s | 2 874 s | 0.3% |
| 213227 | 887 | 4 | 10 | 203 | 396 | 8 s | 2 861 s | 0.3% |

- In the incident log 122 watchers' `View` calls were abandoned after 1.7-15.2 s (median 10 s)
  (`CALL caller_gone call=View`, `actor.rs:5479-5487`) and 62 reduces took over 10 s. The six-hero
  view fix removed this; none of the later logs has a reduce over 424 ms.
- Queue wait is not measured, so the table understates what a caller experiences when a slow call
  is ahead of it. Heartbeat and committed-intent checks (`ms=0` at every sample) are outside the
  span.
- Commits: `SAVE commit` p50 10 ms, p95 29 ms, max 86 ms (124 commits); lookups p50 2 ms. The
  owner-side `commit_owner_candidate` p50 18 ms, p95 37 ms.
- Conclusion: CPU on the owner is no longer the constraint; what remains is the tail (a view
  or a fight-close taking seconds delays everything behind it) and about 1 ms of synchronous
  Postgres per call.

### 3.2 Hero lines

| Metric | n | p50 | p75 | p90 | p95 | max |
|---|---|---|---|---|---|---|
| `STORY input` (route + commit the hero's own line) | 81 | 1.08 s | 2.18 s | 4.6 s | 29.2 s | 60.0 s |
| `STORY exchange` (line + NPC reply, dev path) | 88 | 8.4 s | 14.2 s | 28.3 s | 30.4 s | 60.0 s |
| `NPC target` Jev resolver | 69 | 115 ms | | | 258 ms | 303 ms |
| Voice synthesis (cache miss) | 24 | 356 ms | | | 667 ms | 889 ms |

The hero's own line therefore reaches the table in about a second; the NPC reply arrives about
7 s later. The outliers at 29-60 s are all travel lines (below), not talk.

Text-model calls (`TEXT_MODEL ready purpose= elapsed_ms=`, 8 logs):

| purpose | n | p50 | p95 | notes |
|---|---|---|---|---|
| `npc_reply` | 75 | 1.4 s | 3.2 s | one per reply, two when a reply is rejected |
| `npc_fact_backfill` | 58 | 4.7 s | 10.4 s | under the global fact lock |
| `npc_fact_questions` | 26 | 2.2 s | 3.8 s | |
| `line_cleanup` | 59 | 1.3 s | 2.3 s | speculative, overlaps routing |
| `merchant_item_details` | 10 | 2.2 s | 3.6 s | |
| `place_creation` | 79 | 11.4 s | 17.6 s | max 21 s |

A traced exchange (log 190243, first exchange): 0.84 s routing and commit, then reply phase 7.8 s
made of a 1.4 s fact backfill and a 4.0 s `npc_reply` in sequence (5.4 s of model), with the
remaining ~2.4 s spent in short Jev calls (decision, reason, event classification), guards and
publication that carry no timing. A second traced exchange: two 2.1-2.2 s `npc_reply` calls because
the first reply was rejected (`NPC_FACT status=reply_rejected`), 7.9 s total. So about two thirds
of a reply is sequential model time, one third is unattributed.

Jev reliability affects the chain: 13 `APPROACH source=unavailable reason=error` and 10
`NPC_EVENT status=unavailable reason=provider_failure` lines in the same logs (about one in seven
and one in nine exchanges). A failed Jev call spends up to its 4 s deadline
(`approach_model.rs:15-19`) and the log line carries no elapsed time.

### 3.3 Travel to a generated place

The 59 s input (log 190243, 20e41d...): `place_creation` ran four times sequentially after
the Jev route: 16.2 s rejected (`place_name`), 14.8 s rejected (`place_name`), 16.6 s rejected
(`people_json`), 10.4 s accepted; the commit itself was `SAVE commit 11 ms`, `commit_owner_candidate
16 ms`. The other 35 s input is the same shape. Travel was 4 of 81 inputs but sets the p95/p99.
Look-ahead exists (`FREE_ROAM look_ahead ... outcome=ready`) but these were misses.

## 4. Options

Ground rules from the owner and the plan: one durable writer per game, deterministic SRD rules,
journal replay and idempotent commits ([Runtime reliability](runtime-reliability.md)); the owner
thread never blocks on I/O or a provider and views use computed state; no detached
`tokio::spawn`, every task owned and cancelled, every call has a deadline, queues bounded, stale
completions rejected, one flight per key (style 7).

### (a) Single writer, read-only work off the owner

What: keep one owner for writes; serve reads from an immutable `Arc<Checkpoint>` snapshot plus the
already-shared stores (`TableLog`, `SocialLedger`, `Notebook`, all `Arc` with their own locks).

- Benefit: removes the 7x view fan-out and the Presence/Presented/TelemetryIdentity traffic from the
  owner's queue; owner reduce p99 and queue-wait p99 stop depending on watcher count. Moves auth
  off the owner if combined with a grant mirror.
- Cost: medium to large. `view` first has to be split into (1) effects that must happen once per
  revision on the owner (reaction memory, `sync_room`, `publish`, `observe_scene`, tempo) and
  (2) a pure `project(snapshot, role, member)`. The memo caches become per-thread (cold) unless
  rekeyed by revision into a shared bounded cache. `Checkpoint` should become `Arc<Checkpoint>`
  (cheap clone; the 141 `borrow().clone()` sites compile unchanged) before this is worth doing.
- Risk: a view built from an older snapshot than the one the player just committed (read-your-write).
  The existing revision check (`observed_revision`) already exists; the pure projection must carry the
  basis it was built from and the stream must not publish a view older than the last one sent.
  Notebook and social mutation moved to a per-revision step must stay idempotent.
- Correctness: single-writer invariant intact; journal replay unaffected because projections write
  nothing durable. Fits the style rule "cheap views", since the owner stops doing the view.

### (b) Parallel and shorter provider chains within one hero line

What: join independent judgements, take non-essential work off the critical path, drop
unnecessary regenerations.

- Candidates verified in code: (1) `public_reply_event` runs before the reply is pushed
  (`npc_reply.rs:2587` vs `:2619`) yet only needs the final text; `source_line_sequence` is patched in
  afterwards (`:2649-2652`). It can run after publication and commit asynchronously. (2) The router
  awaits `classify_input_listed`, `resolve_addressee_with_party`, `judge_scene_interaction`,
  `judge_listed_service_request`, `hero_line_form` one after another although their inputs are the
  typed text and the scene. They can be issued together; unused results are dropped (Jev calls are
  cheap and the ledger never blocks spend). (3) `guard_*` stages: those that only check can run
  together; those that rewrite stay ordered. (4) The 4 s Jev deadline: a failed approach or event call
  costs up to 4 s; a hedge or smaller deadline for non-gating classifications bounds the tail.
  (5) Travel: place generation retries after validation rejects (`place_name`, `people_json`);
  shaping the schema, repairing rather than regenerating, or running the retry speculatively cuts the
  35-59 s cases.
- Streaming reply text is not recommended: the guards (name, fact, price bounds,
  `publication_current`) need the complete reply before anyone sees it. Voice is 0.36 s at p50, so
  streaming it gains little; a thinking mark already exists (`reply_pending`).
- Benefit: the largest of all options; it attacks the 5-6 s of sequential model time and the 2 s of
  tail.
- Cost: small to medium per item, no architecture change.
- Risk: more concurrent Jev spend; results that disagree must keep the current precedence (the owner
  still revalidates every proposal, so a wrong judgement is refused there, as today). Tasks must live
  in a `JoinSet` or be plain `join!` futures (no detached spawn).
- Correctness: unchanged rules authority; judgements remain proposals the owner repeats.

### (b2) Narrow the fact-preparation lock

`fact_decision` is global (`npc_reply.rs:2076`). Replace it with a flight per key (NPC id, or NPC id
plus fact subject), as the style guide already prescribes ("a second request for a key joins the first
flight"). The invariant to keep is that two replies cannot invent contradicting canon for the same
subject; across different NPCs the store's atomic append is the arbiter. Open decision for the owner:
whether canon that two NPCs could both invent about a shared place needs a place-level key.
Benefit: removes cross-hero queueing for the 4.7 s p50 / 10.4 s p95 backfill. Risk: contradictory
canon between NPCs if the key is too narrow; a longer review of REF-069.

### (c) Per-room or per-session actors

What: one owner thread and one `Actor` per game instead of per process.

- Benefit for latency: none today. A launcher game is one process and one room (the logs show a
  single `room=`); a second owner would sit idle. It matters for hosted capacity ("hosted
  remote/mixed-room play is required core") and for isolating one room's slow call from others.
- Cost: large. `Actor` is a single struct holding issuer, tempo, notebook, social, session; there are
  94 process-wide statics and `thread_local!` items in `df-tools` (non-test, grep count) that assume one
  game; the pack (`pack::active()`), audit sink and `refusal_detail` are global.
- Recommendation: defer until the hosted-service plan needs several rooms per process, then give
  each room its own inbox and owner thread (Ryhl 2021-02-13: an actor is a task plus a handle,
  bounded channel, `oneshot` replies; the existing inbox is that pattern).

### (d) Fully concurrent state with locks or STM

Not recommended: it removes the single-writer invariant that journal replay, idempotent operation
ids and fenced commits rely on, and the cost it would remove is under 1% of wall time.

### (e) Tokio runtime tuning, `spawn_blocking`, backpressure

- Runtime: already multi-thread, 18 workers, 16 MiB stacks. Nothing to gain by adding threads.
- `spawn_blocking` suits bounded blocking I/O; its pool is very large (about 500 threads per Ryhl
  2020-12-21), so CPU-heavy work such as the pure projection of stage (a) belongs on a small bounded
  pool (rayon or a semaphore, tokio docs). Not the DB commit, which is 10 ms.
- Async code should reach an await within about 10-100 microseconds (Ryhl, 2020-12-21); measure
  `wire::journey_view`, `scene_with_party` and the 141 deep checkpoint clones on workers.
- Backpressure: the inbox is bounded (256 items, 2 MiB) and `actor_view` already waits out a full
  queue within the reply deadline. Add: a counter of full-queue refusals, queue-wait metrics, and a
  coalescing rule for watch wakes (the `select!` in `gameplay.rs:745` already coalesces watch
  changes, but each wake still submits one `View`; one in-flight view per stream is enough).
- The owner's synchronous DB round trip per call (`local_demo_scope.rs:324`): cache the grant in
  the owner (credential -> role, principal, expiry) and invalidate on issue/revoke/expiry, all of
  which the issuer already performs. Needs an owner decision because the code comment requires the principal
  to come from a live grant (`local_demo_scope.rs:341`).
- `resume_fight_reactions` blocking the owner for up to 8 s: run it as an owned async task started
  from the service after the fight close commits, as `FightAftermath` already does for the
  preparation half (`Call::FightAftermath`, `actor.rs:431-437`); the owner only commits results.

## 5. Staged plan

Order is by latency gained per risk. Targets are measured on the same harness as section 3 (six
heroes, dev `/say`, same pack), compared against the section 3 baseline.

### Stage 0: instrumentation (prerequisite, no behaviour change)

- Per-stage timing on every hero line through the shared audit path, one correlated record per
  line id: route stage names (`classify`, `addressee`, `scene_interaction`, `service`, `hero_form`),
  owner queue wait and reduce time per call name, `fact_lock_wait_ms`, `fact_decision`, `reason`,
  `proposal` attempts, `model_public`, each guard, `event_classify`, `publish`, `voice_ready`.
  Failed Jev and provider calls log elapsed time and deadline hit.
- Owner side: stamp `enqueued_at` in the inbox (`inbox.rs` `QueuedInput`), log queue wait and reduce
  per call name for successes (today only refusals and abandoned calls are named), include the
  heartbeat/committed-intent time, and record view fan-out per revision.
- A `dev` CLI summary (owner rule: every system in the dev CLI) printing p50/p95 per stage and per
  call name, and a log timestamp on every line (the logs have none today).
- Files: `inbox.rs`, `actor.rs` (`reduce`, `audit_reply`), `dialogue.rs`, `npc_reply.rs`,
  `fact_backfill.rs`, `approach_model.rs`, `dev_control/`.
- Tests first: controlled-clock test that a faked line's stage times sum to its total within 5%;
  inbox test that queue wait is enqueue-to-dequeue.
- Done when: a 20-line run produces no "unattributed" time above 10% of the exchange.

### Stage 1: take event classification off the reply's critical path

- Change: publish the guarded reply, then run `public_reply_event` and commit via `Call::NpcEvent`
  as an owned follow-up (`npc_reply.rs:2578-2656`, `dialogue.rs:8941`). No change to what the owner
  accepts.
- Target: reply publication p50 down by one Jev call (about 0.25 s typical) and the p95 by the Jev
  failure tail (up to 4 s on the one in nine exchanges where it failed). Success metric: time from
  `complete_dialogue_response` start to the NPC line in the table log, p50 and p95.
- Risk: low. The event result is already optional ("the spoken reply remains valid even when Jev is
  unavailable", `dialogue.rs:8634-8635`). Test: reply published before a deliberately slow event fake
  returns; event still committed once; stale reply (`publication_current` false) commits nothing.

### Stage 2: join the router's independent judgements

- Change: issue `classify_input_listed`, `hero_line_form`, `resolve_addressee_with_party` and the
  merchant trade judgement together at the top of `submit_line_inner`, then branch as today with
  their results; start `judge_scene_interaction` and `judge_listed_service_request` together once the
  intent is known.
- Target: `STORY input` p50 from 1.08 s to under 0.6 s and p90 from 4.6 s to under 2.5 s.
- Risk: low to medium (spend, and ordering of audit lines). Test with fake Jev endpoints that record
  call overlap; all existing routing tests unchanged in outcome.

### Stage 3: per-key fact flights and a travel generation fix

- 3a: replace the global `fact_decision` mutex by a flight per NPC (and subject) keyed as in the
  style guide; keep the atomic append. Target: with six heroes in one scene, `fact_lock_wait_ms` p95
  under 200 ms and `STORY exchange` p90 from 28 s to under 15 s. Test: two heroes ask different NPCs
  concurrently (overlap observed), two ask the same NPC about the same unrecorded detail (one
  proposal, both replies consistent). Needs the open decision in (b2).
- 3b: cut place-generation retries (`place_name`, `people_json` rejections): schema or repair
  instead of regenerate, and a second speculative attempt rather than a serial one. Target: travel
  to a new place p95 from 35-59 s to under 20 s; rejected generations from 3 of 4 to under 1 of 4
  (measure first with stage 0; sample here is 2 travels).
- Files: `npc_reply.rs`, `fact_backfill.rs`, `place_creation.rs`, `place_move.rs`.

### Stage 4: shrink the owner's tail

- 4a: `Arc<Checkpoint>` in the `watch` channel (`actor.rs:743,777`), keeping the call sites.
- 4b: one in-flight `View` per stream; per-revision effects (`sync_room`, `publish`,
  `observe_scene`) run once when the revision changes, and the pure projection is served from the
  snapshot on a bounded worker pool, with the memo caches keyed by revision and shared.
- 4c: grant mirror (owner decision needed) and moving `resume_fight_reactions` into an owned async
  task.
- Targets: owner queue wait p99 under 50 ms and reduce p99 under 50 ms with seven watchers and a
  hero line in flight (today reduce p99 95-203 ms, queue wait unmeasured); no reduce over 1 s in a
  fight-close scenario; view latency p95 unchanged or lower. Do not claim a latency win for the
  hero reply from this stage; it is resilience.
- Risk: medium to high (view staleness, memo semantics). Gate: replay and revision-conflict tests
  must pass with projections on another thread, plus a stress test of 7 watchers x 200 revisions
  checking monotonic revision delivery.
- Not planned: per-room actors (c) and locking (d).

## 6. Open decisions for the owner

1. Stage 3a key: per NPC, or per place for shared canon (REF-069 scope).
2. Stage 4c: may the owner cache grants with explicit invalidation, or must every call hit Postgres?
3. Whether a faster reply model or lower reasoning effort for `npc_reply` is acceptable (a quality
   trade-off, outside this document).

## 7. Sources

- Alice Ryhl, "Actors with Tokio", 2021-02-13: <https://ryhl.io/blog/actors-with-tokio/> (actor =
  task + handle, bounded channels, `oneshot` replies, cycles of bounded channels can deadlock).
- Alice Ryhl, "Async: What is blocking?", 2020-12-21: <https://ryhl.io/blog/async-what-is-blocking/>
  (10-100 microseconds between awaits; `spawn_blocking` for blocking I/O, rayon for CPU work,
  dedicated thread for long-lived loops).
- Tokio API docs, `task::spawn_blocking`, fetched 2026-10-24:
  <https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html> (large default pool, started
  blocking tasks cannot be aborted, cap CPU-bound parallelism).
- Tokio API docs, `sync::watch` and `watch::Receiver::borrow`, fetched 2026-10-24:
  <https://docs.rs/tokio/latest/tokio/sync/watch/index.html>,
  <https://docs.rs/tokio/latest/tokio/sync/watch/struct.Receiver.html> (last value only; an
  outstanding `borrow` holds a read lock on the value).
- Repo evidence: `artifacts/tmp/play/launcher-logs/df_gameplay_demo_play_2026102*.log`; code paths
  cited by file and line above.
