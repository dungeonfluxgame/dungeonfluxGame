# Story memory: vector search research and owner decision

Date: 2026-10-17. Status: research and owner decision record. Design only: no
implementation, no provider calls, no commit. Production use stays gated by G05
(physical schema, indexes, limits), G08 (provider cost qualification), G10
(episode schema, replay, rights) and G11 (workload bounds, retention, relevance
calibration).

## Outcome

The narrative engine must remember everything relevant to keep the story
consistent. Before it decides or narrates, it recalls pinned facts, related
events and per-entity dossiers from its own history. The owner chose OpenAI
`text-embedding-3-small` with pgvector (HNSW) and PostgreSQL full-text search in
the game's own PostgreSQL 18.6. This document records the research behind that
choice, how it relates to the two existing memory policies, and the conflicts that
must be resolved before implementation. It does not claim a running system.

## Owner decision

Recorded from the owner's question and instructions.

1. The question: "cant we do openai text embeddings and do vector search in
   postgres?" The research supports it at this scale (see Options).
2. The requirement: the DM must "remember everything and or everything pertaining
   to" the story.
3. The chosen design:
   - Embeddings: OpenAI `text-embedding-3-small`.
   - Store: pgvector with an HNSW index, plus PostgreSQL full-text search, in the
     game's own PostgreSQL 18.6. No second database.
   - pgvector is built from source because the bundled EDB 18.6 build does not
     include it. `development/pgvector.sh` already does this: it pins pgvector
     0.8.7 and the EDB 18.6 x64 archive by SHA256, builds with nmake under MSVC,
     and installs `vector.dll` and the SQL files into the bundled tree. It does not
     create the extension.
   - Every table line, ruling, roll, narration, canon fact, grievance, promise and
     scene change is stored as an append-only event.
   - Recall before generation returns pinned facts, related events and per-entity
     dossiers.
   - Jev judges contradictions. A contradicted event is marked superseded and kept.
     Nothing is deleted.

Tracker entries: `planning/issue-triage.md` WIP-11 (priority 1, "DM total recall")
and WIP-12 (pgvector build, "proven on a scratch cluster, never the durable one").
WIP-11 says wiring into the storyteller, NPC reply and freeform prompts waits for
WIP-03, WIP-05 and WIP-07. WIP-12 says the pgvector pin is in
`development/postgres.sh`. The pin is actually in `development/pgvector.sh`;
`postgres.sh` exists. The tracker should be corrected.

## Current state in the repo

Decisions are persisted in several places. None is searchable by meaning.

- Local canon: `crates/df-tools/src/gameplay/local_lore.rs`. `LocalFact` (lines
  206-211) is kept per scene, capped at 12 facts per scene and 32 scenes (26-29).
  Saved as `local_lore.json`, schema `df-npc-local-lore/1` (31), atomic write
  (389-409). Written from a reply's `local_fact` proposal, checked by `check_text`,
  which still uses `CLAIM_WORDS`, `RULE_WORDS` and `RUMOUR_OPENERS` (34-42,
  622-627). WIP-03 is removing those lists.
- Social ledger: `social.rs`. Grievances capped at 8 and 200 characters (57-66),
  saved as `df-npc-social/1` (1651-1672). `grievance_lines` and
  `permanent_grievances` (1100-1140) feed text into NPC prompts.
- Scene-change journal: `journey/living_scene.rs`, `scene_change_journal`
  (1402-1430) replays `AcceptedDecision` entries from the checkpoint, stored in
  PostgreSQL through `df-persistence/src/decision_adapter.rs`.
- Storyteller: `storyteller.rs`. No memory across calls. The FACTS block is the
  only source of truth (INSTRUCTION rule 1, lines 89-111). `build_prompt`
  (1532-1568). Prompt cap 32 KiB (73). `PROMPT_VERSION` "storyteller-4" (64).
- Freeform adjudication: `freeform_action.rs` records checks per hero (around
  1384-1457). Its persistence path was not fully traced.

Consequence: local canon is small, so the real gap is cross-scene recall. Canon is
keyed by scene and grievances by hero and NPC. Nothing can be found by meaning
across them, for example "who owns the warehouse" or "what did Marek promise".

## Relation to existing policies

Two documents govern this area: `planning/g11-d03-native-memory-retrieval-policy.md`
("Bounded memory candidates and native retrieval ownership") and
`planning/scene-memory-representation.md` (2026-10-16, design only).

### Where the owner decision agrees

- PostgreSQL is the one durable store. Both forbid a memory database or a second
  schema authority (G11-D03 "Do not add a memory database"; scene-memory "No new
  memory database").
- Summaries and semantic indexes are derived, versioned and replaceable. They
  cannot replace committed facts, replay inputs or provenance (G11-D03, lines
  89-94).
- Retrieval is bounded and reauthorized before publication. A similarity score or
  cached result is never a disclosure grant (G11-D03, lines 37-68).
- A missing or stale index is a visible coverage gap, answered by a bounded
  canonical query (G11-D03, lines 57-60).
- Ordering is deterministic with stable tie breaks. A contradictory belief stays
  attributed; a summary cannot rewrite a fact (G11-D03, lines 70-77).
- Wall-clock time never advances game time (scene-memory, line 82).

### Conflicts to resolve before implementation

Against G11-D03:

1. Deferred index. G11-D03 line 239: "Always use an external/vector index:
   deferred. No embedding/index choice is justified until representative workload
   evidence shows benefit." The owner decision adopts pgvector now. pgvector is
   in-database rather than external, but the sentence covers any vector index.
   Resolution: record the owner decision as an amendment backed by a workload
   measurement under G11, or build the event store and full-text path first and
   keep the vector index behind that gate.
2. Provider calls on the retrieval path. G11-D03 lines 59-60: "No index means no
   embedding/provider call on the retrieval path." Its acceptance (lines 268-270)
   requires replay with zero provider calls. Embedding the query at recall time is
   a provider call on that path. Resolution: the query embedding runs as an admitted
   job, and its vector or digest is recorded in the recall record, so replay reads
   the recording. Embedding each event at write time is also a provider call. It
   must run as an admitted job after commit, never inside the commit path.
3. Supersession by a model. G11-D03 lines 72-74: model confidence cannot supersede
   committed truth. The owner's "Jev judges contradictions; superseded, never
   deleted" would let a model mark an event superseded. Resolution: Jev's output is
   an attributed judgement. Supersession is a status change on belief-level events,
   committed through the session, with the original kept. Jev never rewrites a
   rules-committed fact; a disagreement with one becomes a flagged contradiction for
   review. This needs the owner's confirmation.
4. Second event store. G11-D03 line 84 and scene-memory lines 7 and 125-126: the
   checkpoint and decision journal are the canonical history. An append-only
   `story_memory` table is a second event log unless it is a projection of that
   journal. Table lines and dice rolls may not be journaled today (not verified).
   Resolution: `story_memory` is a derived projection of the committed decision
   journal. Anything not journaled (table lines, rolls, narration) is either added to
   the canonical journal through the G10 episode schema, or stored as a labelled
   non-canonical record with an explicit coverage gap.
5. Bounds versus "everything". G11-D03 lines 52-56: reject over-limit budgets and
   never widen caller limits. "Remember everything" could mean unbounded storage.
   Resolution: storage may grow under a retention policy, but recall is always
   bounded and reports coverage when it truncates. The owner's wording reads as
   "everything relevant within bounds, with coverage stated." Retention and decay
   are G11 gates.
6. Full-text search and keyword rules. The owner bans keyword tables. PostgreSQL
   full-text search matches terms. Resolution: use it only as candidate generation,
   fused with vector ranks by reciprocal rank fusion (rank-based, no term lists).
   It must never filter meaning or authorization. The owner should confirm this
   reading.
7. Audience. G11-D03 lines 37-41 and 66-68, and scene-memory line 121: authorize
   for the NPC speaker and the intended listener, and filter before ranking. A global
   recall that feeds the storyteller across scenes can leak another audience's
   private facts. Resolution: the recall query carries observer and audience, and
   private events are filtered before ranking.

Against scene-memory-representation.md:

8. Semantic search is out of the first slice. Line 138: "Keep automatic new-entity
   discovery, belief inference, semantic search and image regeneration outside this
   slice." The owner decision is a semantic search feature. Resolution: story memory
   is a separate slice that follows the scene-memory first slice, or the first slice
   is amended. WIP-11's wait on WIP-03, WIP-05 and WIP-07 fits that order.
9. Canonical history. Scene-memory lines 7 and 125 name the checkpoint and decision
   journal as canonical. Same as conflict 4.
10. Dossiers. The first slice (line 138) calls for a "no-dossier NPC projection" and
    says the first NPC consumer uses canonical facts. The owner asks for per-entity
    dossiers. Resolution: dossiers are derived views that cite source event IDs. They
    are not authority, and NPC decisions do not use them until the witness adapter is
    admitted (lines 139-140).
11. Entity identity. Scene-memory lines 77-78: entity keys must be trusted
    `EntityId` bindings, not names or labels. The current snapshot uses hero-name
    IDs. Story-memory entity tags depend on this binding. Resolution: events carry
    the bound `EntityId` and a binding revision. Until the binding is frozen, the
    event store does not key on display names.
12. Numeric bounds. Scene-memory line 127 proposes 16 plate records, 64 events and
    64 KiB per query, and 32 entities and 64 KiB per observation. These are candidate
    figures only. Story-memory recall limits must be frozen with them under G11, not
    chosen separately.

## Options and prices

Prices as found on 2026-10-17. Dates are the page dates where the page shows one.
Aggregator and undated figures are marked.

| Option | Price | Free allowance | Date and source | Notes |
| --- | --- | --- | --- | --- |
| OpenAI text-embedding-3-small | $0.02 per 1M input tokens | none listed | Page undated, fetched 2026-10-17: https://developers.openai.com/api/docs/pricing | Chosen. Existing key. Default 1536 dimensions. |
| OpenAI text-embedding-3-large | $0.13 per 1M input tokens | none listed | Same page | Not needed at this scale. |
| OpenAI vector stores | $0.10 per GB per day | 1 GB | Same page | Not used. pgvector keeps vectors in the game database. |
| Voyage voyage-4-lite / voyage-4 / voyage-4-large | $0.02 / $0.06 / $0.12 per 1M | 200M tokens each | Page dated 2026-09-29: https://docs.voyageai.com/docs/pricing | Alternative. voyage-3.5 and voyage-3.5-lite are $0.06 and $0.02, with no free tokens listed. |
| Google gemini-embedding-2 | $0.20 per 1M input (paid) | Free tier exists | Page updated 2026-10-16: https://ai.google.dev/gemini-api/docs/pricing | gemini-embedding-001 is not listed on that page. An older Google post gives $0.15 per 1M: https://developers.googleblog.com/gemini-embedding-available-gemini-api/ . Sources conflict. |
| Cohere embed-v4 | about $0.12 per 1M input | not stated | Aggregator only: https://www.getmaxim.ai/bifrost/llm-cost-calculator/provider/cohere/model/embed-v4.0 | Cohere's own page was not reached. |
| Mistral mistral-embed | about $0.10 per 1M input | not stated | Aggregator only: https://pricepertoken.com/embedding/model/mistral-mistral-embed | Not verified. |
| Local fastembed-rs (default BAAI/bge-small-en-v1.5) | $0 per call | n/a | README: https://github.com/Anush008/fastembed-rs | Needs ONNX Runtime (ort) on Windows ARM64. Not proven. |
| pgvector in the game's Postgres | $0 software | n/a | Version 0.8.7 pinned in `development/pgvector.sh` | Cost is database storage and CPU already in use. |
| Pinecone Database On-Demand | $0.33 per GB-month storage; writes $4-4.50 per M; reads $16-18 per M | Starter: 2 GB, 2M writes, 1M reads per month | Page undated: https://www.pinecone.io/pricing/ | Minimums: Builder $20, Standard $50. Not chosen. |
| Qdrant Cloud | usage-based, hourly; no start price shown | Free: 1 GB RAM, 4 GB disk, one cluster | Page undated: https://qdrant.tech/pricing/ | Not chosen. |
| Upstash Vector | $0.40 per 100K requests; $0.25 per GB | 10K daily requests, 1 GB | Page undated: https://upstash.com/pricing/vector | No monthly fee. Not chosen. |
| Cloudflare Vectorize | $0.05 per 100M stored dimensions after 10M; $0.01 per M queried dimensions after 50M | Free 5M / 30M; Paid 10M / 50M | Page dated 2026-04-21: https://developers.cloudflare.com/vectorize/platform/pricing/ | Not chosen. |
| Chroma Cloud | $0.33 per GiB storage; $2.50 per GiB written; $0.0075 per TiB queried | $5 credits | Page undated: https://www.trychroma.com/pricing | Team plan $250 per month base. Not chosen. |
| Supabase | Pro from $25 per month | 500 MB per project | Page undated: https://supabase.com/pricing | pgvector availability not confirmed from the page. Not chosen. |
| Turbopuffer | Third-party only: about $1 per PB queried; writes up to $2 per GB; Launch minimum $16 or $64 (sources conflict) | none confirmed | Aggregator, July-August 2026: https://www.usagepricing.com/blueprint/activity/turbopuffer-2026-07-14-launch-minimum-cut | Official page unreadable. Not chosen. |

Cost under the chosen design (estimate, not measured):

- Events per session: 2,000-5,000 if every table line, roll, narration and fact is
  stored, at about 150 tokens each. That is 0.3M-0.75M tokens, or $0.006-$0.015 per
  session at $0.02 per 1M.
- Re-embedding after a model change: the same amount, once per run.
- Queries: about 5 per turn, 100 turns, about 100 tokens each, so about 50k tokens,
  or about $0.001.
- Storage: a 1536-dimension float4 vector is about 6 KiB. 5,000 events is about
  30 MB before HNSW overhead, which was not measured. pgvector's documented limit for
  the `vector` type index is 2,000 dimensions (from the pgvector project; not
  re-checked here).
- Jev judgements are not priced here, and they will dominate. Batch them per turn.

## Recommended design under the owner decision

Write path (append-only):

- The canonical commit comes first. Anything committed goes through the existing
  decision journal.
- The story-memory event is a projection written after commit by an admitted job,
  idempotent on (run, source revision, kind, content digest).
- Event fields: id, run_id, epoch, sequence and fact ordinal, kind (table_line,
  ruling, roll, narration, canon_fact, grievance, promise, scene_change), subject
  entity IDs (bound `EntityId`), audience scope, logical time, source reference,
  canonical sentence text, status (active or superseded), superseded_by, supersedes,
  embedding model, dimensions, prompt version and content digest.
- Embedding is a hub resource keyed by (model, dimensions, prompt version, text
  digest), so identical text is embedded once. Spend goes through the hub ledger:
  reserve before the call, refund if it is not sent, reconcile after.
- If the embedding call fails, the event still writes, with embedding pending. The
  full-text path still works, and the vector is backfilled later. Play is never
  blocked.

Recall path (before generation):

- Inputs: observer (speaker and listener), purpose, scene, entities present, the
  hero's line, a deadline, and hard item and byte caps.
- Filter by audience and access generation before ranking.
- Pinned: active canon facts, open promises and grievances for the entities present.
  These come from canonical state, not search. Candidate cap 8.
- Related: vector top-k and full-text top-k, fused by reciprocal rank fusion. Candidate
  cap 6.
- Dossiers: per entity, a derived summary of active events, each with its event ID,
  rebuilt when events change. Candidate cap 2. Never authoritative.
- Relevance: Jev judges whether the related events bear on the current turn when
  Jev is available. Without Jev the result is neutral: ranked candidates, marked
  unjudged, and audited. No keyword fallback.
- Output: a bounded batch with coverage and truncation, plus the query vector ID.
  The recall record stores candidate IDs and scores so that replay reads them and
  does not query again.

Contradictions (write time, admitted job):

- For a new active event E, candidates are the top active events sharing a subject,
  plus the nearest neighbours by vector and by full text.
- Jev returns a closed label: contradicts, extends, unrelated or unknown.
- On "contradicts" for a belief-level event, the session commits status superseded
  with superseded_by = E. Both rows remain. Recall shows a superseded event only with
  its supersession note.
- Rules-committed facts are not superseded by Jev. A contradiction with one is
  flagged for review.
- If Jev is unavailable, nothing is superseded. Both stay active, and a contradiction
  gap is recorded.

Dev CLI and hub:

- `dev memory search <query>` is read-only. It shows rank, score, status, audience
  and source. Add `dev memory events --entity` and `dev memory superseded` the same
  way. The owner's rule (issue-triage) is that every system has logs in the dev CLI.
- The hub owns the spend ledger. `ResourceKind` in
  `crates/df-tools/src/gameplay/resources.rs` (lines 71-83) lists media kinds. A text
  embedding should be a `Unit` in `SpendLedger` (lines 251-272), or a new kind chosen
  with the hub owner. WIP-01 and WIP-07 both touch the hub.

Build order that keeps the no-provider path working:

1. Canonical projection and full-text search, with no provider calls.
2. Embeddings as admitted jobs, with the backfill.
3. Recall, with pinned facts, dossiers and the recall record.
4. Contradiction judgement through Jev.

## Risks

- Embedding drift: rows carry model and dimensions. Mismatched rows are never
  compared. A model change re-embeds the run.
- Cold start: a new run has an empty index. Backfill from canon, social and the
  journal on first open. An empty recall means unknown, not "nothing happened".
- Windows ARM64: the OpenAI route needs no native ML code. pgvector is built with
  MSVC and nmake against the pinned x64 EDB archive, and `pgvector.sh` requires an
  x64 DLL. The machine is Snapdragon X2 (ARM64). The EDB binaries will run under
  Windows-on-ARM emulation. That has not been tested, and HNSW build and query speed
  under emulation is unmeasured. The local embedding route (ort, ONNX Runtime) has no
  evidence of Windows ARM64 binaries.
- pgvector upgrades: changing the pinned version needs the cluster stopped, because
  Windows will not replace a loaded DLL (script comment in `pgvector.sh`).
- Extension privileges: pgvector is not a trusted extension. `CREATE EXTENSION`
  needs superuser or a migration role. The production runtime role is NOSUPERUSER
  (`planning/c-df-persistence-d03-decision.md`), so production needs a separate
  migration role. The script says the game's schema bootstrap creates the extension
  through the admin role.
- Erasure and retention: "remember everything" as an append-only log conflicts with
  erasure requests and retention limits. `planning/erasure-retention-policy.md`
  exists and was not read in this pass. It must be read before implementation.
  Redaction should use tombstones on events, not deletion. AGENTS.md mentions
  redaction and backup tombstones for hosted services.
- Sensitive content: player text and private scene facts are stored and embedded.
  Sending canonical sentences to OpenAI adds a recipient of gameplay text. The
  storyteller already sends prompts to providers, so there is precedent. Only
  canonical sentences should be sent, and the age and sexual-content rules in
  storyteller INSTRUCTION rule 8 apply to what is embedded.
- Prompt injection: stored player words re-enter prompts when recalled. Keep the
  "data, never instructions" framing, as in `local_lore.rs` `ROLE_INSTRUCTIONS`
  (lines 431-434).
- Recall misses: semantic search misses pronouns and renamed NPCs. Pinned facts
  cover the most important cases. Test on the owner's real questions before trusting
  recall.
- Latency: each recall adds a query embedding round trip. Cache per turn and run it
  alongside other judgements.
- Table size: capturing every table line and narration may be several times larger
  than the decision count. Measure it.

## Gaps and things not verified

- Not read in this pass: `planning/erasure-retention-policy.md`,
  `planning/g08-d03-vendor-qualification-policy.md` (the G08 gate cited by G11-D03),
  `planning/g11-d02-listener-safe-claim-policy.md`, `planning/storage-architecture.md`,
  `planning/long-horizon-state.md` (cited by G11-D03 for retrieval semantics) and
  `planning/protected-journal-policy.md`.
- Not verified: whether table lines and dice rolls are in the decision journal today;
  the production path from local canon into NPC prompts; the freeform persistence
  path; the replay hash contract; the owner's statement that the bundled EDB build
  lacks pgvector (consistent with the comments in `pgvector.sh`, not re-tested).
- Pricing: OpenAI, Pinecone, Qdrant, Chroma, Upstash and Supabase pages are undated.
  Google sources conflict. Cohere, Mistral and Turbopuffer figures come from
  aggregators. Voyage is dated 2026-09-29.
- Windows ARM64: no proof for pgvector under emulation, nor for fastembed or ort.
- No build, test or provider call was run for this document. Nothing was committed.
- Owner decisions still needed: conflicts 3, 4, 6 and 7 above, and whether to amend
  G11-D03's deferral (conflict 1).

## Source basis

- `planning/g11-d03-native-memory-retrieval-policy.md`: retrieval ownership, bounds,
  reauthorization, index deferral, replay and the G08, G10 and G11 gates.
- `planning/scene-memory-representation.md`: canonical history, entity binding,
  first-slice exclusions, candidate bounds and NPC memory adapter gate.
- `planning/issue-triage.md` lines 1446 and 1461-1462: owner decisions and WIP-11 and
  WIP-12.
- `development/pgvector.sh`: pgvector 0.8.7 and EDB 18.6 pins, MSVC build, extension
  creation ownership.
- `crates/df-tools/src/gameplay/local_lore.rs`, `social.rs`, `storyteller.rs`,
  `journey/living_scene.rs`, `resources.rs`: current persistence and prompt paths, as
  cited above.
- `AGENTS.md`: PostgreSQL as the durable store; redaction and backup tombstones;
  artifacts directory rules.
- Pricing sources as linked in the table, fetched 2026-10-17.
