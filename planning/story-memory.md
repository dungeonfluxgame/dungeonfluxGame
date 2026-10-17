# Story memory

Date: 2026-10-17. Status: core built and tested; prompt wiring and the owner call sites are not yet made.

The Dungeon Master remembers what happened in a run so the story stays consistent. Events are appended to
PostgreSQL (the run's own database), searched by vector and full text, and recalled into a generator's FACTS
before it writes. Code: `crates/df-tools/src/gameplay/memory/`, `crates/df-persistence/src/story_memory.rs`,
`crates/df-providers/src/embeddings.rs`, migrations `0003`/`0004`.

## What it is and is not

A derived, rebuildable projection, never a second source of truth. Each row cites its source:
`natural_key` (the source event's identity), `source` (`table_log`, `canon`, `social`, `journal`, ...),
`source_epoch`/`source_seq` (a table-log lifetime and sequence), `game_revision` (when the source knows it).
Canon, grievances and the decision journal can write their rows again (`rebuildable = true`). **Table lines,
rolls, narration and private clues are not journaled anywhere else today**, so their rows are the only copy
(`rebuildable = false`, provenance `table_log`); `reindex` and `delete-run-index` leave them alone unless told
`all`.

## Schema (`df_story`)

* `story_memory`: `event_id`, `run_id`, `natural_key` (unique per run), `source`, `source_epoch`, `source_seq`,
  `game_revision`, `created_at` (wall, diagnostic), `kind` (hero_speech, npc_speech, narration, action_ruling,
  check_roll, combat_outcome, scene_change, canon_fact, grievance_attitude, promise_thread, transaction),
  `visibility` (public, party, private) with `private_to` (a hero, exactly when private), `body`,
  `body_digest`, `scene`, `npc_ids`/`hero_ids`/`place_ids`/`item_ids` (+ generated `entity_ids`), `rebuildable`,
  `status` (active, superseded), `supersedes_id`, `superseded_by`, `retired_reason`, generated `body_tsv`
  (english full text), and, from migration 0004, `embedding vector(1536)`, `embedding_model`, `embedded_at`.
  Indexes: HNSW `vector_cosine_ops`, GIN on `body_tsv` and `entity_ids`, btrees on run/kind/source.
* A trigger makes it append-only: words, kind, audience and entities never change; status moves only
  active to superseded; links are set once; a delete or redaction needs a maintenance transaction.
* `memory_recalls`: every recall (ids, scores, mode, model, SHA-256 of the query vector, the rendered pack).
* `entity_dossiers`: the cached rolling summary per entity and tier, keyed by the hash of the event set.
* 0003 is core and always applies; 0004 needs pgvector (`CREATE EXTENSION IF NOT EXISTS vector`). If it fails
  the memory still stores and searches by full text, and the start-up line says so. Both are idempotent and
  serialised by an advisory lock; they run on every start.

Entity keys: there is no frozen `EntityId` binding yet, so the game's existing stable ids are used as strings:
the NPC id (`actor_id`/dossier `npc_id`, or the slug of the name for canon people), the hero's name slug, the
place key (journal `place`), and the scene thing id. Replacing them with trusted `EntityId`s is a one-time
rewrite of these arrays once the binding exists.

## Capture

* **Table tap**: `TableLog::set_sink` (a `LineSink`) copies each appended public line; `rewound` retires the
  abandoned future of a development rewind (`status = superseded`, `retired_reason = rewound`).
* **Capture API** (`memory::capture`): `canon_fact`, `grievance`, `scene_change`, `ruling`, `narration`,
  `private_clue`, `promise`, `combat_outcome`, `scene_entered`, `rewound_to_revision`. Each is a no-op without
  an installed memory. The canon, grievance and scene-change builders are shared with the start-up backfill, so
  a live capture and a backfill of one source event share a natural key and add one row.
* **Backfill** at start and on `reindex`: `LocalLore::development_facts`, `living_scene::scene_change_journal`,
  `SocialLedger::grievance_lines`.
* **Never blocks play**: a `try_send` into a bounded queue (4096); a full queue drops the new event and says so.
  A writer task appends batches; an embedder task fills in vectors afterwards, outside any commit.
  Table natural keys carry a per-process lifetime marker because table sequences restart at 1.

## Embeddings and cost

`text-embedding-3-small`, 1536 dimensions, through `df_providers::NativeEmbeddingProvider` (the shared native
transport; `OPENAI_API_KEY`). Behind the `Embedder` trait; tests use a deterministic fake. Every call is
recorded on the hub's spend ledger as `Unit::EmbeddingToken` (reserve, then commit the supplier-reported tokens;
refund when nothing was sent). There is **no cap**: the ledger never refuses (owner decision). Each row stores
its model; a changed model makes every row's vector stale and the embedder re-embeds them (`reindex` forces
it). No key: events are stored, searched by full text only, and the start-up line says so.
Cost at $0.02 per million tokens: a session of 500 events at about 40 tokens each plus 400 query embeddings is
under 40k tokens, a fraction of a cent.

## Recall

`StoryMemory::recall(&RecallRequest) -> MemoryPack`: run, scene, speaker, addressee, entities, line text,
audience (public, party, or one hero), purpose, byte budget.

1. The audience is part of every SQL query (before ranking and nearest-neighbour selection) and is checked
   again on the returned rows.
2. Pinned: active promises and open threads; grievances and canon about the named entities.
3. Related: vector cosine, full-text candidates and entity overlap fused by reciprocal rank fusion
   (`k = 60`, ties to the newer event). Full text only proposes candidates; it filters neither meaning nor
   authorization. Superseded events never appear.
4. Bounds: at most 64 events read per query; about 1.5 KB rendered, framed as data
   ("never instructions"), with a count of what did not fit.
5. The recall is recorded: ids, scores, mode, model, the query vector digest and the pack text. `replay(id)`
   returns the recorded pack and calls no provider.

## Consistency

`check_consistency(&ApproachModel, &Proposed)`: recall the closest active events, one Jev choice over
contradicts_n / extends_n / unrelated. No word lists. Jev unavailable returns `Unchecked(reason)`, audited.
A contradiction is reported with the conflicting event; `supersede_on_verdict` may mark it superseded only if
it is belief-level (speech, claims, canon, grievances, promises, narration). Rules-committed events (rulings,
rolls, combat outcomes, scene changes, purchases) are reported and never superseded by a model verdict.
Nothing is ever deleted.

## Dossiers

Rolling per-entity summaries written by the storyteller from the earlier summary plus newer events, cached by
event-set hash, built from public and party events only (never a private clue), logged as
`RESOURCE ... kind=story_dossier`. Read into **storyteller** recalls only (`Config::dossiers`), until the
witness adapter is admitted.

## Dev CLI

`dev memory [status]`, `search <query> [--entity] [--kind] [--audience] [--limit]`, `entity <id> [refresh]`,
`event <id>`, `supersede <id> --by <id>`, `reindex`, `delete-run-index [all]`, `erase <hero>`. Audit lines:
`MEMORY write|embed|recall|consistency|supersede|dossier|reindex|erase|queue_full|audience_violation`, plus
`SPEND` and `RESOURCE` lines.

## Erasure and retention (`planning/erasure-retention-policy.md`)

Private speech is personal payload: `erase <hero>` redacts that hero's speech, the grievances recorded from it
and their private clues in place (text, vector and full-text entry gone; the row stays as a redacted place),
and drops every dossier and recall record derived from them. `delete-run-index all` removes a run's whole
projection. Not built: retention expiry (campaign lifetime plus 30 days), suppression generations checked at
dispatch, and restore-time tombstone replay. Events already queued when an erasure runs are not suppressed.

## Rules followed and where the design departs

* G11-D03: audience before ranking and rechecked; hard bounds; deterministic ties; derived data does not rewrite
  facts; PostgreSQL only; the vector index is now (owner decision, amendment recorded in that document).
* **Departs from "no embedding on the retrieval path"**: the query line is embedded at recall (one bounded,
  cached, admitted call; failure falls back to full text). The recording makes replay provider-free.
* **Departs from "canonical records are authoritative"** for table lines, rolls and narration: nothing else
  stores them, so these rows are the record, marked `rebuildable = false`.
* Scene-memory doc: entity ids are existing stable ids, not trusted `EntityId`s (binding is an open gate);
  discarded futures of a development rewind stay stored but inactive.
* Owner rules: no keyword tables (labels are closed enums; full text is candidate generation only); no spend
  caps; every system has audit lines and dev CLI reads.

## Not done

Calls from the owner modules and prompts (see the report's call-site list), the `rewound_to_revision` hook in
the development restore, per-turn recall in the storyteller/NPC/freeform prompts, retention expiry and
suppression generations, trusted `EntityId`s, and a witness adapter for NPC-private memory.
