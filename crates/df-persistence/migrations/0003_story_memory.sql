-- STORY-MEM-01: the append-only story memory of one run (core tables, no vector column).
-- Applied on every start inside one transaction; every statement is idempotent. The vector
-- column, the embedding bookkeeping and the HNSW index live in 0004 so that a server without
-- the pgvector extension still remembers everything and searches it by full text.
CREATE SCHEMA IF NOT EXISTS df_story;
REVOKE ALL ON SCHEMA df_story FROM PUBLIC;

CREATE TABLE IF NOT EXISTS df_story.story_memory (
    event_id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    run_id text NOT NULL CHECK (char_length(run_id) BETWEEN 1 AND 128),
    -- One natural key per source event: a replay or a restart presents the same key again.
    natural_key text NOT NULL CHECK (char_length(natural_key) BETWEEN 1 AND 256),
    -- Provenance: which capture path produced the event, and the source's own position.
    source text NOT NULL CHECK (char_length(source) BETWEEN 1 AND 48),
    source_epoch text CHECK (source_epoch IS NULL OR char_length(source_epoch) <= 64),
    source_seq bigint CHECK (source_seq IS NULL OR source_seq >= 0),
    -- Logical game time when the source knows it; created_at is diagnostic wall time.
    game_revision bigint CHECK (game_revision IS NULL OR game_revision >= 0),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    kind text NOT NULL CHECK (kind IN (
        'hero_speech', 'npc_speech', 'narration', 'action_ruling', 'check_roll',
        'combat_outcome', 'scene_change', 'canon_fact', 'grievance_attitude',
        'promise_thread', 'transaction')),
    visibility text NOT NULL CHECK (visibility IN ('public', 'party', 'private')),
    -- A member-private clue names the one hero who may be told it.
    private_to text CHECK (private_to IS NULL OR char_length(private_to) BETWEEN 1 AND 128),
    CHECK ((visibility = 'private') = (private_to IS NOT NULL)),
    body text NOT NULL CHECK (char_length(body) BETWEEN 1 AND 2000),
    body_digest bytea NOT NULL CHECK (octet_length(body_digest) = 32),
    scene text CHECK (scene IS NULL OR char_length(scene) <= 128),
    npc_ids text[] NOT NULL DEFAULT '{}' CHECK (cardinality(npc_ids) <= 16),
    hero_ids text[] NOT NULL DEFAULT '{}' CHECK (cardinality(hero_ids) <= 16),
    place_ids text[] NOT NULL DEFAULT '{}' CHECK (cardinality(place_ids) <= 16),
    item_ids text[] NOT NULL DEFAULT '{}' CHECK (cardinality(item_ids) <= 16),
    entity_ids text[] GENERATED ALWAYS AS (npc_ids || hero_ids || place_ids || item_ids) STORED,
    -- True when a canonical source (canon file, social ledger, decision journal) can write the
    -- event again; the run index rebuild drops only these unless told otherwise.
    rebuildable boolean NOT NULL DEFAULT false,
    status text NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'superseded')),
    supersedes_id bigint REFERENCES df_story.story_memory (event_id) ON DELETE SET NULL,
    superseded_by bigint REFERENCES df_story.story_memory (event_id) ON DELETE SET NULL,
    retired_reason text CHECK (retired_reason IS NULL OR char_length(retired_reason) <= 64),
    body_tsv tsvector GENERATED ALWAYS AS (to_tsvector('english', body)) STORED,
    UNIQUE (run_id, natural_key)
);

CREATE INDEX IF NOT EXISTS story_memory_run_recent
    ON df_story.story_memory (run_id, event_id DESC);
CREATE INDEX IF NOT EXISTS story_memory_run_kind
    ON df_story.story_memory (run_id, kind, status);
CREATE INDEX IF NOT EXISTS story_memory_run_source
    ON df_story.story_memory (run_id, source, source_epoch, source_seq);
CREATE INDEX IF NOT EXISTS story_memory_entities
    ON df_story.story_memory USING gin (entity_ids);
CREATE INDEX IF NOT EXISTS story_memory_fulltext
    ON df_story.story_memory USING gin (body_tsv);

-- Append-only: an event's words, kind, audience and entities never change. Only its status
-- (active to superseded, one way), its supersession links (set once) and its embedding do.
-- Deleting or redacting is refused except inside an explicit maintenance transaction
-- (df_story.maintenance = on): a run index rebuild or an erasure.
CREATE OR REPLACE FUNCTION df_story.story_memory_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF current_setting('df_story.maintenance', true) IS NOT DISTINCT FROM 'on' THEN
        RETURN COALESCE(NEW, OLD);
    END IF;
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'story memory is append-only';
    END IF;
    IF NEW.event_id IS DISTINCT FROM OLD.event_id
       OR NEW.run_id IS DISTINCT FROM OLD.run_id
       OR NEW.natural_key IS DISTINCT FROM OLD.natural_key
       OR NEW.source IS DISTINCT FROM OLD.source
       OR NEW.source_epoch IS DISTINCT FROM OLD.source_epoch
       OR NEW.source_seq IS DISTINCT FROM OLD.source_seq
       OR NEW.game_revision IS DISTINCT FROM OLD.game_revision
       OR NEW.created_at IS DISTINCT FROM OLD.created_at
       OR NEW.kind IS DISTINCT FROM OLD.kind
       OR NEW.visibility IS DISTINCT FROM OLD.visibility
       OR NEW.private_to IS DISTINCT FROM OLD.private_to
       OR NEW.body IS DISTINCT FROM OLD.body
       OR NEW.body_digest IS DISTINCT FROM OLD.body_digest
       OR NEW.scene IS DISTINCT FROM OLD.scene
       OR NEW.npc_ids IS DISTINCT FROM OLD.npc_ids
       OR NEW.hero_ids IS DISTINCT FROM OLD.hero_ids
       OR NEW.place_ids IS DISTINCT FROM OLD.place_ids
       OR NEW.item_ids IS DISTINCT FROM OLD.item_ids
       OR NEW.rebuildable IS DISTINCT FROM OLD.rebuildable THEN
        RAISE EXCEPTION 'story memory events are immutable';
    END IF;
    IF OLD.status = 'superseded' AND NEW.status <> 'superseded' THEN
        RAISE EXCEPTION 'a superseded story memory event stays superseded';
    END IF;
    IF OLD.supersedes_id IS NOT NULL AND NEW.supersedes_id IS DISTINCT FROM OLD.supersedes_id THEN
        RAISE EXCEPTION 'story memory supersession links are set once';
    END IF;
    IF OLD.superseded_by IS NOT NULL AND NEW.superseded_by IS DISTINCT FROM OLD.superseded_by THEN
        RAISE EXCEPTION 'story memory supersession links are set once';
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS story_memory_append_only ON df_story.story_memory;
CREATE TRIGGER story_memory_append_only
    BEFORE UPDATE OR DELETE ON df_story.story_memory
    FOR EACH ROW EXECUTE FUNCTION df_story.story_memory_guard();

-- Which events fed each recall, so a replay can reproduce the prompt's facts exactly.
CREATE TABLE IF NOT EXISTS df_story.memory_recalls (
    recall_id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    run_id text NOT NULL CHECK (char_length(run_id) BETWEEN 1 AND 128),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    purpose text NOT NULL CHECK (char_length(purpose) <= 48),
    scene text,
    speaker text,
    addressee text,
    audience text NOT NULL CHECK (char_length(audience) <= 160),
    query_text text NOT NULL CHECK (char_length(query_text) <= 600),
    mode text NOT NULL CHECK (char_length(mode) <= 32),
    embedding_model text,
    -- SHA-256 of the query vector that was used (hex): a replay reads this record and the
    -- recorded ids and makes no provider call.
    query_vector_digest text CHECK (query_vector_digest IS NULL OR char_length(query_vector_digest) = 64),
    pinned_ids bigint[] NOT NULL DEFAULT '{}',
    related_ids bigint[] NOT NULL DEFAULT '{}',
    scores double precision[] NOT NULL DEFAULT '{}',
    pack_text text NOT NULL CHECK (char_length(pack_text) <= 8000)
);
CREATE INDEX IF NOT EXISTS memory_recalls_run
    ON df_story.memory_recalls (run_id, recall_id DESC);

-- The rolling summary of everything known about one entity: derived, never authority. It is
-- keyed by the hash of the entity's event set, so a changed set is a stale summary.
CREATE TABLE IF NOT EXISTS df_story.entity_dossiers (
    run_id text NOT NULL CHECK (char_length(run_id) BETWEEN 1 AND 128),
    entity_id text NOT NULL CHECK (char_length(entity_id) BETWEEN 1 AND 128),
    audience text NOT NULL CHECK (audience IN ('public', 'party')),
    event_set_hash text NOT NULL CHECK (char_length(event_set_hash) = 64),
    covered_ids bigint[] NOT NULL DEFAULT '{}',
    summary text NOT NULL CHECK (char_length(summary) BETWEEN 1 AND 4000),
    writer text NOT NULL CHECK (char_length(writer) <= 64),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (run_id, entity_id, audience)
);
