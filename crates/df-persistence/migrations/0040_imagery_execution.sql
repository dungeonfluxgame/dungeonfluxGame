-- Durable image execution journal (tasks IMG-EXECUTION-JOURNAL-20261028-A1/A2).
-- Schema imagery in the one hosted database. Times are milliseconds since the
-- Unix epoch from the service clock. Ids are 16 bytes; digests are 32.
--
-- What it owns: one logical stage row per job stage (the claim triple:
-- token, generation, lease deadline), one dispatch row per send attempt
-- (send/outcome/recovery state), the current/refetch cursor, output
-- checkpoints with validation proof, versioned supplier resume checkpoints,
-- durable capacity bounds with active and pending reserves, and the job-level
-- planned publication identity allocated before classification, plus the
-- actively applied execution gate per scope (distinct from every job's
-- immutable config snapshot).
-- What it never does: no money, no provider payload, no prompt text, no
-- finance/catalog writes, no 0039 admission binding early insert (the
-- allocation FK needs the actual asset row and frozen recipe), no change to
-- existing 0039 first-output admission behavior.
-- Lock order inside these transactions: image_job FOR UPDATE, then the
-- capacity and gate rows, then stage, dispatch, cursor and checkpoint rows.
-- Finance, fence and quota/catalog locks always precede imagery locks;
-- withdraw takes the job before interests and never touches media or
-- finance rows.

CREATE SCHEMA IF NOT EXISTS imagery;

-- Immutable snapshot digests are computed once; callers cannot assert their own metadata.
ALTER TABLE imagery.image_job ADD COLUMN IF NOT EXISTS intent_digest bytea
    GENERATED ALWAYS AS(pg_catalog.sha256(intent_bytes)) STORED;
ALTER TABLE imagery.image_job ADD COLUMN IF NOT EXISTS config_digest bytea
    GENERATED ALWAYS AS(pg_catalog.sha256(config_bytes)) STORED;

-- Job-level planned publication identity, allocated before classification so
-- classification and later stages link to the retained identity without
-- claiming an asset row. Both columns stay NULL until the first enqueue sets
-- them atomically; a later enqueue naming another pair is a conflict.
ALTER TABLE imagery.image_job
    ADD COLUMN IF NOT EXISTS planned_publication_asset bytea;
ALTER TABLE imagery.image_job
    ADD COLUMN IF NOT EXISTS planned_publication_attempt bytea;
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'image_job_planned_identity_ids'
    ) THEN
        ALTER TABLE imagery.image_job ADD CONSTRAINT image_job_planned_identity_ids CHECK (
            (planned_publication_asset IS NULL AND planned_publication_attempt IS NULL)
            OR (planned_publication_asset <> decode('00000000000000000000000000000000', 'hex')
                AND octet_length(planned_publication_asset) = 16
                AND planned_publication_attempt <> decode('00000000000000000000000000000000', 'hex')
                AND octet_length(planned_publication_attempt) = 16)
        );
    END IF;
END
$$;

-- One logical stage: immutable owner/job/stage/kind plus the input/config
-- basis, and the mutable claim triple (token, generation, lease deadline).
-- Retries never add a second stage row.
CREATE TABLE IF NOT EXISTS imagery.image_execution_stage (
    job_id bytea NOT NULL,
    stage_id bytea NOT NULL CHECK (octet_length(stage_id) = 16 AND stage_id <> decode('00000000000000000000000000000000', 'hex')),
    owner_scope bytea NOT NULL CHECK (octet_length(owner_scope) IN (18, 34)),
    stage_kind text NOT NULL CHECK (stage_kind IN
        ('classification','generation','edit','extraction','inspection',
         'correction','local_transform')),
    stage_state text NOT NULL CHECK (stage_state IN
        ('planned','admitted','released')),
    claim_token bytea NOT NULL CHECK (octet_length(claim_token) = 16 AND claim_token <> decode('00000000000000000000000000000000', 'hex')),
    claim_generation bigint NOT NULL CHECK (claim_generation >= 0),
    claim_config_revision bigint NOT NULL DEFAULT 0 CHECK (claim_config_revision >= 0),
    lease_until_ms bigint NOT NULL CHECK (lease_until_ms >= 0),
    queued_not_before_ms bigint NOT NULL DEFAULT 0 CHECK(queued_not_before_ms>=0),
    input_revision bigint NOT NULL CHECK (input_revision > 0 AND input_revision <= 4294967295),
    input_digest bytea NOT NULL CHECK (octet_length(input_digest) = 32),
    config_revision bigint NOT NULL CHECK (config_revision > 0),
    config_digest bytea NOT NULL CHECK (octet_length(config_digest) = 32),
    capacity_scope text NOT NULL CHECK (char_length(capacity_scope) BETWEEN 1 AND 128),
    capacity_reserved boolean NOT NULL DEFAULT false,
    local_accounting boolean NOT NULL DEFAULT false,
    created_at_ms bigint NOT NULL CHECK (created_at_ms >= 0),
    updated_at_ms bigint NOT NULL CHECK (updated_at_ms >= created_at_ms),
    PRIMARY KEY (job_id, stage_id),
    CONSTRAINT image_execution_stage_job_fkey FOREIGN KEY (job_id, owner_scope)
        REFERENCES imagery.image_job (job_id, owner_scope)
        DEFERRABLE INITIALLY DEFERRED
);

-- One dispatch attempt per send: immutable lineage (sequence, finance tuple,
-- parent, book call id, claim snapshot) plus the mutable send/outcome state.
-- Local-only work carries no finance tuple at all.
CREATE TABLE IF NOT EXISTS imagery.image_dispatch_attempt (
    dispatch_id bytea NOT NULL CHECK (octet_length(dispatch_id) = 16 AND dispatch_id <> decode('00000000000000000000000000000000', 'hex')),
    job_id bytea NOT NULL,
    stage_id bytea NOT NULL,
    dispatch_seq integer NOT NULL CHECK (dispatch_seq >= 0),
    dispatch_state text NOT NULL CHECK (dispatch_state IN
        ('prepared','sent','succeeded','failed','unknown','released')),
    failure_cause text CHECK (failure_cause IS NULL OR failure_cause IN
        ('refused','invalid_output','transport_lost','rate_limited','quota_exhausted','deadline_exceeded','provider')),
    settlement text NOT NULL DEFAULT 'held' CHECK (settlement IN
        ('held','settled','unusable_settled','unknown_outstanding','released')),
    finance_attempt bytea CHECK (finance_attempt IS NULL OR (octet_length(finance_attempt) = 16 AND finance_attempt <> decode('00000000000000000000000000000000', 'hex'))),
    finance_operation bytea CHECK (finance_operation IS NULL OR (octet_length(finance_operation) = 16 AND finance_operation <> decode('00000000000000000000000000000000', 'hex'))),
    finance_effect bytea CHECK (finance_effect IS NULL OR (octet_length(finance_effect) = 16 AND finance_effect <> decode('00000000000000000000000000000000', 'hex'))),
    supplier_job_id text CHECK (supplier_job_id IS NULL OR char_length(supplier_job_id) BETWEEN 1 AND 256),
    usage_revision integer NOT NULL DEFAULT 0 CHECK (usage_revision >= 0),
    book_call_id text NOT NULL CHECK (char_length(book_call_id) BETWEEN 1 AND 80),
    lineage_parent bytea CHECK (lineage_parent IS NULL OR (octet_length(lineage_parent) = 16 AND lineage_parent <> decode('00000000000000000000000000000000', 'hex'))),
    claim_token bytea NOT NULL CHECK (octet_length(claim_token) = 16),
    claim_generation bigint NOT NULL CHECK (claim_generation >= 0),
    claim_lease_until_ms bigint NOT NULL DEFAULT 0 CHECK(claim_lease_until_ms>=0),
    local_accounting boolean NOT NULL DEFAULT false,
    created_at_ms bigint NOT NULL CHECK (created_at_ms >= 0),
    updated_at_ms bigint NOT NULL CHECK (updated_at_ms >= created_at_ms),
    CONSTRAINT image_dispatch_attempt_pkey PRIMARY KEY (dispatch_id),
    CONSTRAINT image_dispatch_attempt_stage_fkey FOREIGN KEY (job_id, stage_id)
        REFERENCES imagery.image_execution_stage (job_id, stage_id)
        DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT image_dispatch_attempt_seq_unique UNIQUE (job_id, stage_id, dispatch_seq),
    CONSTRAINT image_dispatch_attempt_finance_op_unique UNIQUE (finance_operation),
    CONSTRAINT image_dispatch_attempt_finance_all_or_none CHECK (
        (finance_attempt IS NULL) = (finance_operation IS NULL)
        AND (finance_operation IS NULL) = (finance_effect IS NULL)),
    CONSTRAINT image_dispatch_attempt_local_has_none CHECK (
        (local_accounting = false)
        OR ((finance_attempt IS NULL) AND (finance_operation IS NULL)
            AND (finance_effect IS NULL)))
);

ALTER TABLE imagery.image_dispatch_attempt ADD COLUMN IF NOT EXISTS cost_unsent_ack boolean NOT NULL DEFAULT false;

ALTER TABLE imagery.image_dispatch_attempt ADD COLUMN IF NOT EXISTS claim_lease_until_ms bigint NOT NULL DEFAULT 0
    CHECK(claim_lease_until_ms>=0);

-- The (job, stage, dispatch) identity behind the composite cursor and
-- lineage foreign keys below, so a cursor or parent can never name a
-- dispatch of another stage.
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'image_dispatch_attempt_stage_identity_unique'
    ) THEN
        ALTER TABLE imagery.image_dispatch_attempt
            ADD CONSTRAINT image_dispatch_attempt_stage_identity_unique
            UNIQUE (job_id, stage_id, dispatch_id);
    END IF;
END
$$;

-- Lineage stays inside its own stage: a retry's parent is the previous
-- current dispatch of the same job and stage, never a foreign row.
ALTER TABLE imagery.image_dispatch_attempt
    DROP CONSTRAINT IF EXISTS image_dispatch_attempt_parent_fkey;
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'image_dispatch_attempt_parent_stage_fkey'
    ) THEN
        ALTER TABLE imagery.image_dispatch_attempt
            ADD CONSTRAINT image_dispatch_attempt_parent_stage_fkey
            FOREIGN KEY (job_id, stage_id, lineage_parent)
            REFERENCES imagery.image_dispatch_attempt (job_id, stage_id, dispatch_id)
            DEFERRABLE INITIALLY DEFERRED;
    END IF;
END
$$;

-- Which dispatch a refetch or late result belongs to: the current output
-- dispatch plus the persisted refetch selection. A late result for any
-- other dispatch is stale and refused. Both references are composite over
-- the stage, so a cursor can never point at another stage's dispatch.
CREATE TABLE IF NOT EXISTS imagery.image_stage_cursor (
    job_id bytea NOT NULL,
    stage_id bytea NOT NULL,
    current_dispatch bytea NOT NULL CHECK (octet_length(current_dispatch) = 16),
    refetch_dispatch bytea CHECK (refetch_dispatch IS NULL OR octet_length(refetch_dispatch) = 16),
    updated_at_ms bigint NOT NULL CHECK (updated_at_ms >= 0),
    CONSTRAINT image_stage_cursor_pkey PRIMARY KEY (job_id, stage_id),
    CONSTRAINT image_stage_cursor_stage_fkey FOREIGN KEY (job_id, stage_id)
        REFERENCES imagery.image_execution_stage (job_id, stage_id)
        DEFERRABLE INITIALLY DEFERRED
);
ALTER TABLE imagery.image_stage_cursor
    DROP CONSTRAINT IF EXISTS image_stage_cursor_current_fkey;
ALTER TABLE imagery.image_stage_cursor
    DROP CONSTRAINT IF EXISTS image_stage_cursor_refetch_fkey;
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'image_stage_cursor_current_stage_fkey'
    ) THEN
        ALTER TABLE imagery.image_stage_cursor
            ADD CONSTRAINT image_stage_cursor_current_stage_fkey
            FOREIGN KEY (job_id, stage_id, current_dispatch)
            REFERENCES imagery.image_dispatch_attempt (job_id, stage_id, dispatch_id)
            DEFERRABLE INITIALLY DEFERRED;
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'image_stage_cursor_refetch_stage_fkey'
    ) THEN
        ALTER TABLE imagery.image_stage_cursor
            ADD CONSTRAINT image_stage_cursor_refetch_stage_fkey
            FOREIGN KEY (job_id, stage_id, refetch_dispatch)
            REFERENCES imagery.image_dispatch_attempt (job_id, stage_id, dispatch_id)
            DEFERRABLE INITIALLY DEFERRED;
    END IF;
END
$$;

-- Output checkpoint per dispatch: the staged object locator plus the
-- validation proof publication needs. The storage key alone proves nothing:
-- the immutable object version, exact byte length, content digest,
-- validated metadata (mime, dimensions, alpha) and usage revision must all
-- match the staged bytes. The first committed write wins; a conflicting
-- replay is refused by the store, never overwritten.
CREATE TABLE IF NOT EXISTS imagery.image_dispatch_checkpoint (
    dispatch_id bytea NOT NULL,
    storage_key text NOT NULL CHECK (char_length(storage_key) BETWEEN 1 AND 512),
    object_version text NOT NULL CHECK (octet_length(object_version) BETWEEN 1 AND 1024),
    byte_len bigint NOT NULL CHECK (byte_len > 0),
    sha256 bytea NOT NULL CHECK (octet_length(sha256) = 32),
    recipe_digest bytea NOT NULL CHECK (octet_length(recipe_digest) = 32),
    mime text NOT NULL CHECK (char_length(mime) BETWEEN 1 AND 128),
    width_px integer CHECK (width_px IS NULL OR width_px > 0),
    height_px integer CHECK (height_px IS NULL OR height_px > 0),
    alpha text NOT NULL CHECK (alpha IN ('opaque', 'transparent')),
    usage_revision integer NOT NULL CHECK (usage_revision >= 0),
    book_call_id text NOT NULL CHECK (char_length(book_call_id) BETWEEN 1 AND 80),
    recorded_at_ms bigint NOT NULL CHECK (recorded_at_ms >= 0),
    CONSTRAINT image_dispatch_checkpoint_pkey PRIMARY KEY (dispatch_id),
    CONSTRAINT image_dispatch_checkpoint_dispatch_fkey FOREIGN KEY (dispatch_id)
        REFERENCES imagery.image_dispatch_attempt (dispatch_id)
        DEFERRABLE INITIALLY DEFERRED
);
ALTER TABLE imagery.image_dispatch_checkpoint
    ADD COLUMN IF NOT EXISTS object_version text NOT NULL DEFAULT '1';
ALTER TABLE imagery.image_dispatch_checkpoint
    ADD COLUMN IF NOT EXISTS byte_len bigint NOT NULL DEFAULT 1
    CHECK (byte_len > 0);
ALTER TABLE imagery.image_dispatch_checkpoint
    ADD COLUMN IF NOT EXISTS alpha text NOT NULL DEFAULT 'opaque'
    CHECK (alpha IN ('opaque', 'transparent'));

-- Versioned supplier resume checkpoints per dispatch: the bounded opaque
-- bytes plus origin, model and stage references a fal/BFL worker needs to
-- resume or reconcile without resending. The supplier correlation id alone
-- cannot resume work, so it is never the only retained supplier fact.
CREATE TABLE IF NOT EXISTS imagery.image_supplier_checkpoint (
    dispatch_id bytea NOT NULL,
    checkpoint_version integer NOT NULL CHECK (checkpoint_version > 0),
    supplier_origin text NOT NULL CHECK (char_length(supplier_origin) BETWEEN 1 AND 64),
    supplier_model text NOT NULL CHECK (char_length(supplier_model) BETWEEN 1 AND 128),
    checkpoint_bytes bytea NOT NULL CHECK (octet_length(checkpoint_bytes) BETWEEN 1 AND 1048576),
    recorded_at_ms bigint NOT NULL CHECK (recorded_at_ms >= 0),
    CONSTRAINT image_supplier_checkpoint_pkey PRIMARY KEY (dispatch_id, checkpoint_version),
    CONSTRAINT image_supplier_checkpoint_dispatch_fkey FOREIGN KEY (dispatch_id)
        REFERENCES imagery.image_dispatch_attempt (dispatch_id)
        DEFERRABLE INITIALLY DEFERRED
);

-- Durable queue and concurrency bounds. Pending jobs and their retained
-- intent/config bytes are tracked separately from active concurrency:
-- enqueue reserves pending, claim moves pending to active, and terminal
-- outcomes plus stage release retire active. No process counter is
-- durability proof: only these rows count. Lowering a bound stops new work;
-- it never invalidates existing liability, so no bound check constrains the
-- stored counters here: the statements enforce limits under row lock.
CREATE TABLE IF NOT EXISTS imagery.image_execution_capacity (
    capacity_scope text NOT NULL CHECK (char_length(capacity_scope) BETWEEN 1 AND 128),
    bound bigint NOT NULL CHECK (bound > 0),
    in_flight bigint NOT NULL DEFAULT 0 CHECK (in_flight >= 0),
    pending_jobs bigint NOT NULL DEFAULT 0 CHECK (pending_jobs >= 0),
    pending_bytes bigint NOT NULL DEFAULT 0 CHECK (pending_bytes >= 0),
    pending_jobs_bound bigint NOT NULL DEFAULT 256 CHECK (pending_jobs_bound > 0),
    pending_bytes_bound bigint NOT NULL DEFAULT 1073741824 CHECK (pending_bytes_bound > 0),
    updated_at_ms bigint NOT NULL CHECK (updated_at_ms >= 0),
    CONSTRAINT image_execution_capacity_pkey PRIMARY KEY (capacity_scope)
);
ALTER TABLE imagery.image_execution_capacity
    DROP CONSTRAINT IF EXISTS image_execution_capacity_within_bound;
ALTER TABLE imagery.image_execution_capacity
    ADD COLUMN IF NOT EXISTS pending_jobs bigint NOT NULL DEFAULT 0
    CHECK (pending_jobs >= 0);
ALTER TABLE imagery.image_execution_capacity
    ADD COLUMN IF NOT EXISTS pending_bytes bigint NOT NULL DEFAULT 0
    CHECK (pending_bytes >= 0);
ALTER TABLE imagery.image_execution_capacity
    ADD COLUMN IF NOT EXISTS pending_jobs_bound bigint NOT NULL DEFAULT 256
    CHECK (pending_jobs_bound > 0);
ALTER TABLE imagery.image_execution_capacity
    ADD COLUMN IF NOT EXISTS pending_bytes_bound bigint NOT NULL DEFAULT 1073741824
    CHECK (pending_bytes_bound > 0);

-- Actively applied execution gate per scope, distinct from every job's
-- immutable config snapshot. Claims name the applied revision they hold; a
-- committed disable refuses claims, prepares and sends while already-sent
-- work keeps its liability and reconciliation continues.
CREATE TABLE IF NOT EXISTS imagery.image_execution_gate (
    gate_scope text NOT NULL CHECK (char_length(gate_scope) BETWEEN 1 AND 128),
    applied_revision bigint NOT NULL CHECK (applied_revision > 0),
    enabled boolean NOT NULL,
    updated_at_ms bigint NOT NULL CHECK (updated_at_ms >= 0),
    CONSTRAINT image_execution_gate_pkey PRIMARY KEY (gate_scope)
);

-- Rekey guards: identity, kind, basis, capacity and lineage columns are
-- immutable once written. Outcome columns stay writable so send, supplier,
-- settlement and cursor updates never need a delete.
CREATE OR REPLACE FUNCTION imagery.forbid_execution_stage_rekey() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.job_id IS DISTINCT FROM NEW.job_id
        OR OLD.stage_id IS DISTINCT FROM NEW.stage_id
        OR OLD.owner_scope IS DISTINCT FROM NEW.owner_scope
        OR OLD.stage_kind IS DISTINCT FROM NEW.stage_kind
        OR OLD.input_revision IS DISTINCT FROM NEW.input_revision
        OR OLD.input_digest IS DISTINCT FROM NEW.input_digest
        OR OLD.config_revision IS DISTINCT FROM NEW.config_revision
        OR OLD.config_digest IS DISTINCT FROM NEW.config_digest
        OR OLD.capacity_scope IS DISTINCT FROM NEW.capacity_scope
        OR OLD.local_accounting IS DISTINCT FROM NEW.local_accounting
        OR OLD.parent_stage_id IS DISTINCT FROM NEW.parent_stage_id
        OR OLD.parent_dispatch IS DISTINCT FROM NEW.parent_dispatch
        OR OLD.created_at_ms IS DISTINCT FROM NEW.created_at_ms
    THEN
        RAISE EXCEPTION 'imagery execution stage identity is immutable';
    END IF;
    RETURN NEW;
END
$$;

CREATE OR REPLACE FUNCTION imagery.forbid_execution_dispatch_rekey() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.dispatch_id IS DISTINCT FROM NEW.dispatch_id
        OR OLD.job_id IS DISTINCT FROM NEW.job_id
        OR OLD.stage_id IS DISTINCT FROM NEW.stage_id
        OR OLD.dispatch_seq IS DISTINCT FROM NEW.dispatch_seq
        OR OLD.finance_attempt IS DISTINCT FROM NEW.finance_attempt
        OR OLD.finance_operation IS DISTINCT FROM NEW.finance_operation
        OR OLD.finance_effect IS DISTINCT FROM NEW.finance_effect
        OR OLD.lineage_parent IS DISTINCT FROM NEW.lineage_parent
        OR OLD.book_call_id IS DISTINCT FROM NEW.book_call_id
        OR OLD.claim_token IS DISTINCT FROM NEW.claim_token
        OR OLD.claim_generation IS DISTINCT FROM NEW.claim_generation
        OR OLD.local_accounting IS DISTINCT FROM NEW.local_accounting
        OR OLD.created_at_ms IS DISTINCT FROM NEW.created_at_ms
    OR OLD.claim_lease_until_ms IS DISTINCT FROM NEW.claim_lease_until_ms
    THEN
        RAISE EXCEPTION 'imagery dispatch lineage is immutable';
    END IF;
    RETURN NEW;
END
$$;

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_trigger WHERE tgname = 'image_execution_stage_rekey_guard') THEN
        CREATE TRIGGER image_execution_stage_rekey_guard
            BEFORE UPDATE ON imagery.image_execution_stage
            FOR EACH ROW EXECUTE FUNCTION imagery.forbid_execution_stage_rekey();
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_trigger WHERE tgname = 'image_dispatch_attempt_rekey_guard') THEN
        CREATE TRIGGER image_dispatch_attempt_rekey_guard
            BEFORE UPDATE ON imagery.image_dispatch_attempt
            FOR EACH ROW EXECUTE FUNCTION imagery.forbid_execution_dispatch_rekey();
    END IF;
END
$$;

CREATE INDEX IF NOT EXISTS image_execution_stage_owner_state
    ON imagery.image_execution_stage (owner_scope, stage_state);
CREATE INDEX IF NOT EXISTS image_execution_stage_scope_created
    ON imagery.image_execution_stage (capacity_scope, stage_state, created_at_ms);
CREATE INDEX IF NOT EXISTS image_dispatch_attempt_stage_state
    ON imagery.image_dispatch_attempt (job_id, stage_id, dispatch_state);
CREATE INDEX IF NOT EXISTS image_dispatch_checkpoint_book
    ON imagery.image_dispatch_checkpoint (book_call_id);

REVOKE ALL ON imagery.image_execution_stage FROM PUBLIC;
REVOKE ALL ON imagery.image_dispatch_attempt FROM PUBLIC;
REVOKE ALL ON imagery.image_stage_cursor FROM PUBLIC;
REVOKE ALL ON imagery.image_dispatch_checkpoint FROM PUBLIC;
REVOKE ALL ON imagery.image_supplier_checkpoint FROM PUBLIC;
REVOKE ALL ON imagery.image_execution_capacity FROM PUBLIC;
REVOKE ALL ON imagery.image_execution_gate FROM PUBLIC;

ALTER TABLE imagery.image_execution_stage
    ADD COLUMN IF NOT EXISTS claim_config_revision bigint NOT NULL DEFAULT 0
    CHECK (claim_config_revision >= 0);

ALTER TABLE imagery.image_dispatch_checkpoint
    DROP CONSTRAINT IF EXISTS image_dispatch_checkpoint_object_version_check;
ALTER TABLE imagery.image_dispatch_checkpoint
    ALTER COLUMN object_version DROP DEFAULT,
    ALTER COLUMN object_version TYPE text USING object_version::text;
ALTER TABLE imagery.image_dispatch_checkpoint
    ADD CONSTRAINT image_dispatch_checkpoint_object_version_check
    CHECK (octet_length(object_version) BETWEEN 1 AND 1024);
ALTER TABLE imagery.image_job ADD COLUMN IF NOT EXISTS planned_output_stage bytea;

ALTER TABLE imagery.image_execution_stage
    ADD COLUMN IF NOT EXISTS claim_lease_ms bigint NOT NULL DEFAULT 0
    CHECK (claim_lease_ms >= 0);

-- Recovery leases cannot become normal claims or authorize dispatch. Acknowledgment
-- is independent of supplier outcome and is retained to make lost acknowledgments replayable.
CREATE TABLE IF NOT EXISTS imagery.image_dispatch_recovery (
    dispatch_id bytea PRIMARY KEY,
    job_id bytea NOT NULL,
    stage_id bytea NOT NULL,
    claim_token bytea NOT NULL CHECK (octet_length(claim_token) = 16),
    claim_generation bigint NOT NULL CHECK (claim_generation > 0),
    lease_ms bigint NOT NULL CHECK (lease_ms > 0),
    lease_until_ms bigint NOT NULL CHECK (lease_until_ms >= 0),
    recovery_state text NOT NULL CHECK (recovery_state IN ('claimed', 'retry', 'completed')),
    next_at_ms bigint NOT NULL CHECK (next_at_ms >= 0),
    updated_at_ms bigint NOT NULL CHECK (updated_at_ms > 0),
    FOREIGN KEY (job_id, stage_id, dispatch_id)
        REFERENCES imagery.image_dispatch_attempt(job_id, stage_id, dispatch_id)
        DEFERRABLE INITIALLY DEFERRED
);
CREATE INDEX IF NOT EXISTS image_dispatch_recovery_due
    ON imagery.image_dispatch_recovery(next_at_ms, dispatch_id)
    WHERE recovery_state <> 'completed';

-- Host policy has a configurable default; the SQL binary ceiling is a separate technical limit.
ALTER TABLE imagery.image_execution_capacity
    ADD COLUMN IF NOT EXISTS decision_payload_bytes_bound bigint NOT NULL DEFAULT 262144
    CHECK (decision_payload_bytes_bound BETWEEN 1 AND 1073741820);
ALTER TABLE imagery.image_execution_stage ADD COLUMN IF NOT EXISTS parent_stage_id bytea;
ALTER TABLE imagery.image_execution_stage ADD COLUMN IF NOT EXISTS parent_dispatch bytea;
DO $$ BEGIN
    IF NOT EXISTS(SELECT 1 FROM pg_constraint WHERE conname='image_execution_stage_parent_identity') THEN
        ALTER TABLE imagery.image_execution_stage ADD CONSTRAINT image_execution_stage_parent_identity
            CHECK ((parent_dispatch IS NULL AND parent_stage_id IS NULL)
                OR (parent_dispatch IS NOT NULL AND parent_stage_id IS NOT NULL AND parent_stage_id<>stage_id));
        ALTER TABLE imagery.image_execution_stage ADD CONSTRAINT image_execution_stage_parent_fkey
            FOREIGN KEY(job_id,parent_stage_id,parent_dispatch)
            REFERENCES imagery.image_dispatch_attempt(job_id,stage_id,dispatch_id)
            DEFERRABLE INITIALLY DEFERRED;
    END IF;
END $$;
CREATE UNIQUE INDEX IF NOT EXISTS image_execution_stage_child_identity
    ON imagery.image_execution_stage(job_id,parent_dispatch,stage_kind)
    WHERE parent_dispatch IS NOT NULL;

-- Private observed decisions are source evidence, never catalog assets or semantic approval.
CREATE TABLE IF NOT EXISTS imagery.image_decision_observation (
    dispatch_id bytea NOT NULL,
    observation_version bigint NOT NULL CHECK(observation_version BETWEEN 1 AND 4294967295),
    owner_scope bytea NOT NULL,
    job_id bytea NOT NULL,
    stage_id bytea NOT NULL,
    decision_kind text NOT NULL CHECK(decision_kind IN ('classification','inspection')),
    schema_revision bigint NOT NULL CHECK(schema_revision BETWEEN 1 AND 4294967295),
    validator_revision bigint NOT NULL CHECK(validator_revision BETWEEN 1 AND 4294967295),
    input_revision bigint NOT NULL CHECK(input_revision BETWEEN 1 AND 4294967295),
    input_digest bytea NOT NULL CHECK(octet_length(input_digest)=32),
    config_revision bigint NOT NULL CHECK(config_revision>0),
    config_digest bytea NOT NULL CHECK(octet_length(config_digest)=32),
    parent_dispatch bytea,
    payload bytea NOT NULL CHECK(octet_length(payload) BETWEEN 1 AND 1073741820),
    payload_sha256 bytea NOT NULL CHECK(octet_length(payload_sha256)=32),
    usage_revision integer NOT NULL CHECK(usage_revision>0),
    observed_at_ms bigint NOT NULL CHECK(observed_at_ms>0),
    PRIMARY KEY(dispatch_id,observation_version),
    FOREIGN KEY(job_id,owner_scope) REFERENCES imagery.image_job(job_id,owner_scope),
    FOREIGN KEY(job_id,stage_id,dispatch_id)
        REFERENCES imagery.image_dispatch_attempt(job_id,stage_id,dispatch_id)
);
CREATE TABLE IF NOT EXISTS imagery.image_decision_checkpoint (
    dispatch_id bytea PRIMARY KEY,
    observation_version bigint NOT NULL,
    usage_revision integer NOT NULL CHECK(usage_revision>0),
    recorded_at_ms bigint NOT NULL CHECK(recorded_at_ms>0),
    FOREIGN KEY(dispatch_id,observation_version)
        REFERENCES imagery.image_decision_observation(dispatch_id,observation_version)
);
CREATE OR REPLACE FUNCTION imagery.forbid_decision_mutation() RETURNS trigger
LANGUAGE plpgsql AS $$ BEGIN
    RAISE EXCEPTION 'imagery observed decision evidence is immutable';
END $$;
DROP TRIGGER IF EXISTS image_decision_observation_immutable ON imagery.image_decision_observation;
CREATE TRIGGER image_decision_observation_immutable BEFORE UPDATE OR DELETE ON imagery.image_decision_observation
    FOR EACH ROW EXECUTE FUNCTION imagery.forbid_decision_mutation();
DROP TRIGGER IF EXISTS image_decision_checkpoint_immutable ON imagery.image_decision_checkpoint;
CREATE TRIGGER image_decision_checkpoint_immutable BEFORE UPDATE OR DELETE ON imagery.image_decision_checkpoint
    FOR EACH ROW EXECUTE FUNCTION imagery.forbid_decision_mutation();
REVOKE ALL ON imagery.image_decision_observation,imagery.image_decision_checkpoint FROM PUBLIC;

-- Intermediate candidate receipts bind real raw usage bytes; no image object or duplicate blob.
CREATE TABLE IF NOT EXISTS imagery.image_candidate_observation (
    dispatch_id bytea NOT NULL,
    observation_version bigint NOT NULL CHECK(observation_version BETWEEN 1 AND 4294967295),
    owner_scope bytea NOT NULL,job_id bytea NOT NULL,stage_id bytea NOT NULL,
    candidate_kind text NOT NULL CHECK(candidate_kind IN ('generation','edit','correction','extraction')),
    input_revision bigint NOT NULL CHECK(input_revision BETWEEN 1 AND 4294967295),
    input_digest bytea NOT NULL CHECK(octet_length(input_digest)=32),
    config_revision bigint NOT NULL CHECK(config_revision>0),
    config_digest bytea NOT NULL CHECK(octet_length(config_digest)=32),parent_dispatch bytea,
    mime text NOT NULL CHECK(mime IN ('image/png','image/jpeg','image/webp')),
    width_px integer NOT NULL CHECK(width_px>0),height_px integer NOT NULL CHECK(height_px>0),
    alpha text NOT NULL CHECK(alpha IN ('opaque','transparent')),
    byte_len bigint NOT NULL CHECK(byte_len>0),sha256 bytea NOT NULL CHECK(octet_length(sha256)=32),
    validator_revision bigint NOT NULL CHECK(validator_revision BETWEEN 1 AND 4294967295),
    usage_revision integer NOT NULL CHECK(usage_revision>0),observed_at_ms bigint NOT NULL CHECK(observed_at_ms>0),
    PRIMARY KEY(dispatch_id,observation_version),
    FOREIGN KEY(job_id,owner_scope) REFERENCES imagery.image_job(job_id,owner_scope),
    FOREIGN KEY(job_id,stage_id,dispatch_id) REFERENCES imagery.image_dispatch_attempt(job_id,stage_id,dispatch_id)
);
CREATE TABLE IF NOT EXISTS imagery.image_candidate_checkpoint (
    dispatch_id bytea PRIMARY KEY,observation_version bigint NOT NULL,
    usage_revision integer NOT NULL CHECK(usage_revision>0),recorded_at_ms bigint NOT NULL CHECK(recorded_at_ms>0),
    FOREIGN KEY(dispatch_id,observation_version)
        REFERENCES imagery.image_candidate_observation(dispatch_id,observation_version)
);
DROP TRIGGER IF EXISTS image_candidate_observation_immutable ON imagery.image_candidate_observation;
CREATE TRIGGER image_candidate_observation_immutable BEFORE UPDATE OR DELETE ON imagery.image_candidate_observation
    FOR EACH ROW EXECUTE FUNCTION imagery.forbid_decision_mutation();
DROP TRIGGER IF EXISTS image_candidate_checkpoint_immutable ON imagery.image_candidate_checkpoint;
CREATE TRIGGER image_candidate_checkpoint_immutable BEFORE UPDATE OR DELETE ON imagery.image_candidate_checkpoint
    FOR EACH ROW EXECUTE FUNCTION imagery.forbid_decision_mutation();
REVOKE ALL ON imagery.image_candidate_observation,imagery.image_candidate_checkpoint FROM PUBLIC;

-- Exact recovery capability that issued a local child claim; not immutable content identity.
ALTER TABLE imagery.image_execution_stage ADD COLUMN IF NOT EXISTS reconciliation_dispatch bytea;
ALTER TABLE imagery.image_execution_stage ADD COLUMN IF NOT EXISTS reconciliation_token bytea;
ALTER TABLE imagery.image_execution_stage ADD COLUMN IF NOT EXISTS reconciliation_generation bigint;
DO $$ BEGIN
    IF NOT EXISTS(SELECT 1 FROM pg_constraint WHERE conname='image_execution_reconciliation_claim') THEN
        ALTER TABLE imagery.image_execution_stage ADD CONSTRAINT image_execution_reconciliation_claim CHECK (
            (reconciliation_dispatch IS NULL AND reconciliation_token IS NULL AND reconciliation_generation IS NULL)
            OR (reconciliation_dispatch IS NOT NULL AND reconciliation_token IS NOT NULL
                AND reconciliation_generation>0 AND octet_length(reconciliation_token)=16
                AND local_accounting AND stage_kind='local_transform' AND parent_dispatch IS NOT NULL));
        ALTER TABLE imagery.image_execution_stage ADD CONSTRAINT image_execution_reconciliation_parent
            FOREIGN KEY(reconciliation_dispatch) REFERENCES imagery.image_dispatch_recovery(dispatch_id);
    END IF;
END $$;

-- Scheduling is separate from content identity and immutable per accepted interest.
ALTER TABLE imagery.image_execution_capacity ADD COLUMN IF NOT EXISTS urgency_promotion_ms bigint NOT NULL DEFAULT 5000
    CHECK(urgency_promotion_ms>0);
CREATE TABLE IF NOT EXISTS imagery.image_interest_schedule (
    interest_id bytea PRIMARY KEY REFERENCES imagery.image_interest(interest_id),
    urgency smallint NOT NULL CHECK(urgency BETWEEN 0 AND 2),
    accepted_at_ms bigint NOT NULL CHECK(accepted_at_ms>0)
);
DROP TRIGGER IF EXISTS image_interest_schedule_immutable ON imagery.image_interest_schedule;
CREATE TRIGGER image_interest_schedule_immutable BEFORE UPDATE OR DELETE ON imagery.image_interest_schedule
    FOR EACH ROW EXECUTE FUNCTION imagery.forbid_decision_mutation();
REVOKE ALL ON imagery.image_interest_schedule FROM PUBLIC;


-- Private common egress gate; only the two mode-specific definer wrappers can invoke it.
CREATE OR REPLACE FUNCTION imagery.mark_execution_dispatch_sent(
    wanted_owner bytea,wanted_job bytea,wanted_stage bytea,wanted_dispatch bytea,
    wanted_claim bytea,wanted_generation bigint,wanted_now_ms bigint,
    wanted_local boolean,finance_sent_this_tx boolean
) RETURNS TABLE(verdict text,current_revision bigint)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE
    j imagery.image_job%ROWTYPE;s imagery.image_execution_stage%ROWTYPE;
    d imagery.image_dispatch_attempt%ROWTYPE;g imagery.image_execution_gate%ROWTYPE;
    r imagery.image_dispatch_recovery%ROWTYPE;current_id bytea;
    reconciles boolean:=false;original_claim boolean:=false;
BEGIN
    IF wanted_owner IS NULL OR wanted_job IS NULL OR wanted_stage IS NULL
        OR wanted_dispatch IS NULL OR wanted_claim IS NULL OR wanted_generation IS NULL
        OR wanted_now_ms IS NULL OR wanted_local IS NULL OR finance_sent_this_tx IS NULL
        OR wanted_now_ms<=0 OR wanted_generation<=0
        OR octet_length(wanted_claim)<>16 THEN
        RETURN QUERY SELECT 'invalid'::text,0::bigint;RETURN;
    END IF;
    SELECT * INTO j FROM imagery.image_job WHERE job_id=wanted_job AND owner_scope=wanted_owner FOR UPDATE;
    IF NOT FOUND THEN RETURN QUERY SELECT 'not_found'::text,0::bigint;RETURN;END IF;
    SELECT * INTO s FROM imagery.image_execution_stage WHERE job_id=wanted_job AND stage_id=wanted_stage FOR UPDATE;
    IF NOT FOUND OR s.owner_scope<>wanted_owner THEN RETURN QUERY SELECT 'not_found'::text,0::bigint;RETURN;END IF;
    SELECT * INTO d FROM imagery.image_dispatch_attempt WHERE dispatch_id=wanted_dispatch FOR UPDATE;
    IF NOT FOUND OR d.job_id<>wanted_job OR d.stage_id<>wanted_stage THEN
        RETURN QUERY SELECT 'not_found'::text,0::bigint;RETURN;
    END IF;
    IF s.local_accounting IS DISTINCT FROM wanted_local OR d.local_accounting IS DISTINCT FROM wanted_local
        OR (wanted_local AND (d.finance_attempt IS NOT NULL OR d.finance_operation IS NOT NULL OR d.finance_effect IS NOT NULL))
        OR (NOT wanted_local AND (d.finance_attempt IS NULL OR d.finance_operation IS NULL OR d.finance_effect IS NULL)) THEN
        RETURN QUERY SELECT 'basis'::text,0::bigint;RETURN;
    END IF;
    SELECT c.current_dispatch INTO current_id FROM imagery.image_stage_cursor c
        WHERE c.job_id=wanted_job AND c.stage_id=wanted_stage FOR UPDATE;
    IF current_id IS DISTINCT FROM wanted_dispatch THEN RETURN QUERY SELECT 'superseded'::text,0::bigint;RETURN;END IF;
    original_claim:=d.claim_token=wanted_claim AND d.claim_generation=wanted_generation;
    IF original_claim AND d.dispatch_state IN ('sent','succeeded') THEN
        RETURN QUERY SELECT 'already_sent'::text,0::bigint;RETURN;
    END IF;
    IF s.stage_state<>'admitted' OR s.claim_token<>wanted_claim OR s.claim_generation<>wanted_generation
        OR s.lease_until_ms<=wanted_now_ms THEN RETURN QUERY SELECT 'lost_claim'::text,0::bigint;RETURN;END IF;
    IF wanted_local AND s.reconciliation_dispatch IS NOT NULL THEN
        SELECT * INTO r FROM imagery.image_dispatch_recovery WHERE dispatch_id=s.reconciliation_dispatch FOR UPDATE;
        IF FOUND AND r.job_id=wanted_job AND r.claim_token=s.reconciliation_token
            AND r.claim_generation=s.reconciliation_generation AND r.recovery_state='claimed'
            AND r.lease_until_ms>wanted_now_ms AND s.lease_until_ms<=r.lease_until_ms THEN
            SELECT EXISTS(SELECT 1 FROM imagery.image_dispatch_attempt candidate
                JOIN imagery.image_stage_cursor cc ON cc.current_dispatch=candidate.dispatch_id
                JOIN imagery.image_candidate_checkpoint cp ON cp.dispatch_id=candidate.dispatch_id
                JOIN imagery.image_candidate_observation co ON co.dispatch_id=cp.dispatch_id AND co.observation_version=cp.observation_version
                JOIN imagery.image_dispatch_attempt actor ON actor.dispatch_id=r.dispatch_id
                JOIN imagery.image_stage_cursor ac ON ac.current_dispatch=actor.dispatch_id
                JOIN imagery.image_execution_stage ast ON ast.job_id=actor.job_id AND ast.stage_id=actor.stage_id
                WHERE candidate.dispatch_id=s.parent_dispatch AND candidate.job_id=wanted_job
                    AND candidate.dispatch_state='succeeded' AND co.owner_scope=wanted_owner
                    AND actor.job_id=wanted_job AND actor.dispatch_state='succeeded'
                    AND (actor.dispatch_id=candidate.dispatch_id OR (ast.stage_kind='inspection'
                        AND ast.parent_dispatch=candidate.dispatch_id
                        AND EXISTS(SELECT 1 FROM imagery.image_decision_checkpoint dc WHERE dc.dispatch_id=actor.dispatch_id)))) INTO reconciles;
        END IF;
    END IF;
    IF NOT original_claim AND NOT reconciles THEN RETURN QUERY SELECT 'lost_claim'::text,0::bigint;RETURN;END IF;
    IF NOT reconciles THEN
        IF j.state='retired' THEN RETURN QUERY SELECT 'retired'::text,0::bigint;RETURN;END IF;
        IF j.absolute_deadline_ms<=wanted_now_ms THEN RETURN QUERY SELECT 'expired'::text,0::bigint;RETURN;END IF;
        IF NOT EXISTS(SELECT 1 FROM imagery.image_interest i WHERE i.job_id=wanted_job AND i.state='attached') THEN
            RETURN QUERY SELECT 'demand_lapsed'::text,0::bigint;RETURN;
        END IF;
        SELECT * INTO g FROM imagery.image_execution_gate WHERE gate_scope=s.capacity_scope FOR SHARE;
        IF NOT FOUND THEN RETURN QUERY SELECT 'fenced'::text,0::bigint;RETURN;END IF;
        IF NOT g.enabled THEN RETURN QUERY SELECT 'disabled'::text,g.applied_revision;RETURN;END IF;
        IF g.applied_revision<>s.claim_config_revision THEN RETURN QUERY SELECT 'fenced'::text,g.applied_revision;RETURN;END IF;
    END IF;
    IF d.dispatch_state IN ('sent','succeeded') THEN RETURN QUERY SELECT 'already_sent'::text,0::bigint;RETURN;END IF;
    IF d.dispatch_state<>'prepared' THEN RETURN QUERY SELECT 'superseded'::text,0::bigint;RETURN;END IF;
    IF NOT wanted_local AND NOT finance_sent_this_tx THEN
        RETURN QUERY SELECT 'basis'::text,0::bigint;RETURN;
    END IF;
    IF to_regclass('imagery.image_no_delivery') IS NOT NULL THEN
        IF EXISTS(SELECT 1 FROM imagery.image_no_delivery WHERE owner_scope=wanted_owner AND job_id=wanted_job) THEN
            RETURN QUERY SELECT 'sealed'::text,0::bigint;RETURN;
        END IF;
    END IF;
    IF to_regclass('imagery.image_job_result') IS NOT NULL THEN
        IF EXISTS(SELECT 1 FROM imagery.image_job_result WHERE owner_scope=wanted_owner AND job_id=wanted_job) THEN
            RETURN QUERY SELECT 'sealed'::text,0::bigint;RETURN;
        END IF;
    END IF;
    -- A real selected standalone result is a successful terminal seal, separate from NoDelivery.
    IF to_regclass('imagery.image_analysis_checkpoint') IS NOT NULL THEN
        IF EXISTS(SELECT 1 FROM imagery.image_analysis_checkpoint WHERE owner_scope=wanted_owner AND job_id=wanted_job) THEN
            RETURN QUERY SELECT 'sealed'::text,0::bigint;RETURN;
        END IF;
    END IF;
    UPDATE imagery.image_dispatch_attempt SET dispatch_state='sent',updated_at_ms=wanted_now_ms WHERE dispatch_id=wanted_dispatch;
    UPDATE imagery.image_job SET revision=revision+1,updated_at_ms=wanted_now_ms WHERE job_id=wanted_job;
    RETURN QUERY SELECT 'new'::text,0::bigint;
END $$;
REVOKE ALL ON FUNCTION imagery.mark_execution_dispatch_sent(bytea,bytea,bytea,bytea,bytea,bigint,bigint,boolean,boolean) FROM PUBLIC;

CREATE OR REPLACE FUNCTION imagery.mark_local_dispatch_sent(
    wanted_owner bytea,wanted_job bytea,wanted_stage bytea,wanted_dispatch bytea,
    wanted_claim bytea,wanted_generation bigint,wanted_now_ms bigint
) RETURNS TABLE(verdict text,current_revision bigint)
LANGUAGE sql SECURITY DEFINER SET search_path=pg_catalog AS $$
    SELECT * FROM imagery.mark_execution_dispatch_sent($1,$2,$3,$4,$5,$6,$7,true,false)
$$;
REVOKE ALL ON FUNCTION imagery.mark_local_dispatch_sent(bytea,bytea,bytea,bytea,bytea,bigint,bigint) FROM PUBLIC;

-- Finance Sent is already written in this transaction; source/grant locks belong to its caller.
CREATE OR REPLACE FUNCTION imagery.mark_hosted_dispatch_sent(
    wanted_owner bytea,wanted_job bytea,wanted_stage bytea,wanted_dispatch bytea,
    wanted_claim bytea,wanted_generation bigint,wanted_now_ms bigint,
    wanted_attempt bytea,wanted_operation bytea,wanted_effect bytea
) RETURNS TABLE(verdict text,current_revision bigint)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE sent_here boolean;
BEGIN
    IF wanted_attempt IS NULL OR wanted_operation IS NULL OR wanted_effect IS NULL THEN
        RETURN QUERY SELECT 'invalid'::text,0::bigint;RETURN;
    END IF;
    -- Financial locks precede imagery locks, including a direct restricted-role call.
    SELECT a.xmin=pg_current_xact_id()::xid INTO sent_here
    FROM imagery.image_finance_admission_binding b
    JOIN finance.provider_attempt a ON a.attempt_id=b.finance_attempt AND a.effect_id=b.effect_id
        AND a.reservation_id=b.reservation_id
    JOIN finance.reservation r ON r.reservation_id=a.reservation_id AND r.operation_id=b.operation_id
    JOIN imagery.image_dispatch_attempt d ON d.dispatch_id=b.dispatch_id
    WHERE b.owner_scope=wanted_owner AND b.job_id=wanted_job AND b.execution_stage_id=wanted_stage
        AND b.dispatch_id=wanted_dispatch AND b.finance_attempt=wanted_attempt
        AND b.operation_id=wanted_operation AND b.effect_id=wanted_effect
        AND d.job_id=b.job_id AND d.stage_id=b.execution_stage_id
        AND d.finance_attempt=b.finance_attempt AND d.finance_operation=b.operation_id AND d.finance_effect=b.effect_id
        AND b.stage_call IS NOT NULL AND b.cost_route IS NOT NULL AND NOT d.local_accounting
        AND a.dispatch_state='sent' FOR SHARE OF a,r;
    IF NOT FOUND THEN RETURN QUERY SELECT 'basis'::text,0::bigint;RETURN;END IF;
    RETURN QUERY SELECT * FROM imagery.mark_execution_dispatch_sent(
        wanted_owner,wanted_job,wanted_stage,wanted_dispatch,wanted_claim,wanted_generation,wanted_now_ms,false,sent_here);
END $$;
REVOKE ALL ON FUNCTION imagery.mark_hosted_dispatch_sent(bytea,bytea,bytea,bytea,bytea,bigint,bigint,bytea,bytea,bytea) FROM PUBLIC;

CREATE TABLE IF NOT EXISTS imagery.image_local_inspection (
    job_id bytea NOT NULL,stage_id bytea NOT NULL,owner_scope bytea NOT NULL,
    input_revision bigint NOT NULL CHECK(input_revision>0),input_digest bytea NOT NULL CHECK(octet_length(input_digest)=32),
    config_revision bigint NOT NULL CHECK(config_revision>0),config_digest bytea NOT NULL CHECK(octet_length(config_digest)=32),
    parent_dispatch bytea NOT NULL,candidate_stage_id bytea NOT NULL,candidate_dispatch_id bytea NOT NULL,
    candidate_observation_version bigint NOT NULL CHECK(candidate_observation_version>0),
    candidate_usage_revision integer NOT NULL CHECK(candidate_usage_revision>0),
    source_digest bytea NOT NULL CHECK(octet_length(source_digest)=32),
    reason text NOT NULL CHECK(reason IN ('preparation','disabled')),observed_at_ms bigint NOT NULL CHECK(observed_at_ms>0),
    schema_revision bigint NOT NULL CHECK(schema_revision=1),validator_revision bigint NOT NULL CHECK(validator_revision=1),
    claim_token bytea NOT NULL CHECK(octet_length(claim_token)=16),claim_generation bigint NOT NULL CHECK(claim_generation>0),
    intent_snapshot_revision bigint NOT NULL CHECK(intent_snapshot_revision>0),intent_digest bytea NOT NULL CHECK(octet_length(intent_digest)=32),
    config_snapshot_revision bigint NOT NULL CHECK(config_snapshot_revision>0),
    PRIMARY KEY(job_id,stage_id),
    FOREIGN KEY(job_id,owner_scope) REFERENCES imagery.image_job(job_id,owner_scope),
    FOREIGN KEY(job_id,stage_id) REFERENCES imagery.image_execution_stage(job_id,stage_id),
    FOREIGN KEY(job_id,candidate_stage_id,candidate_dispatch_id) REFERENCES imagery.image_dispatch_attempt(job_id,stage_id,dispatch_id),
    FOREIGN KEY(candidate_dispatch_id,candidate_observation_version) REFERENCES imagery.image_candidate_observation(dispatch_id,observation_version)
);
DROP TRIGGER IF EXISTS image_local_inspection_immutable ON imagery.image_local_inspection;
CREATE TRIGGER image_local_inspection_immutable BEFORE UPDATE OR DELETE ON imagery.image_local_inspection
    FOR EACH ROW EXECUTE FUNCTION imagery.forbid_decision_mutation();
REVOKE ALL ON imagery.image_local_inspection FROM PUBLIC;

-- Consume an immutable output seal; this function cannot create a seal or release supplier money.
CREATE OR REPLACE FUNCTION imagery.close_terminal_output(
    wanted_owner bytea,wanted_job bytea,wanted_actor_kind text,wanted_actor_stage bytea,wanted_actor_dispatch bytea,
    wanted_tail_kind text,wanted_tail_stage bytea,wanted_tail_dispatch bytea,wanted_candidate bytea,
    wanted_expected bigint,wanted_now_ms bigint
) RETURNS TABLE(verdict text,job_revision bigint)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE j imagery.image_job%ROWTYPE;s imagery.image_execution_stage%ROWTYPE;seal record;changed bigint;paid boolean:=false;
BEGIN
    IF wanted_owner IS NULL OR wanted_job IS NULL OR wanted_expected IS NULL OR wanted_expected<=0
        OR wanted_now_ms IS NULL OR wanted_now_ms<=0 THEN RETURN QUERY SELECT 'invalid'::text,0::bigint;RETURN;END IF;
    IF to_regclass('imagery.image_no_delivery') IS NULL THEN RETURN QUERY SELECT 'not_found'::text,0::bigint;RETURN;END IF;
    SELECT * INTO j FROM imagery.image_job WHERE job_id=wanted_job AND owner_scope=wanted_owner FOR UPDATE;
    IF NOT FOUND THEN RETURN QUERY SELECT 'not_found'::text,0::bigint;RETURN;END IF;
    SELECT * INTO seal FROM imagery.image_no_delivery WHERE job_id=wanted_job AND owner_scope=wanted_owner;
    IF NOT FOUND THEN RETURN QUERY SELECT 'not_found'::text,j.revision;RETURN;END IF;
    IF seal.actor_kind IS DISTINCT FROM wanted_actor_kind OR seal.actor_stage_id IS DISTINCT FROM wanted_actor_stage
        OR seal.actor_dispatch_id IS DISTINCT FROM wanted_actor_dispatch OR seal.tail_kind IS DISTINCT FROM wanted_tail_kind
        OR seal.tail_stage_id IS DISTINCT FROM wanted_tail_stage OR seal.tail_dispatch_id IS DISTINCT FROM wanted_tail_dispatch
        OR seal.candidate_dispatch_id IS DISTINCT FROM wanted_candidate OR seal.expected_job_revision<>wanted_expected
        OR seal.sealed_job_revision<>wanted_expected+1 THEN RETURN QUERY SELECT 'basis'::text,j.revision;RETURN;END IF;
    IF j.state='retired' AND j.revision>=seal.sealed_job_revision THEN RETURN QUERY SELECT 'existing'::text,j.revision;RETURN;END IF;
    IF j.revision<>wanted_expected THEN RETURN QUERY SELECT 'stale'::text,j.revision;RETURN;END IF;
    IF wanted_tail_kind IN ('unsent_stage','unclaimed_stage','deferred_stage','released_stage') THEN
        SELECT * INTO s FROM imagery.image_execution_stage WHERE job_id=wanted_job AND stage_id=wanted_tail_stage FOR UPDATE;
        IF NOT FOUND OR s.owner_scope<>wanted_owner THEN RETURN QUERY SELECT 'not_found'::text,j.revision;RETURN;END IF;
        IF wanted_tail_kind<>'unsent_stage' THEN
            IF NOT (j.state='retired' OR j.absolute_deadline_ms<=wanted_now_ms OR NOT EXISTS(SELECT 1 FROM imagery.image_interest i WHERE i.job_id=wanted_job AND i.state='attached')) THEN
                RETURN QUERY SELECT 'basis'::text,j.revision;RETURN;
            END IF;
            IF to_regclass('imagery.image_job_result') IS NOT NULL THEN
                IF EXISTS(SELECT 1 FROM imagery.image_job_result WHERE owner_scope=wanted_owner AND job_id=wanted_job) THEN RETURN QUERY SELECT 'basis'::text,j.revision;RETURN;END IF;
            END IF;
            IF to_regclass('imagery.image_analysis_checkpoint') IS NOT NULL THEN
                IF EXISTS(SELECT 1 FROM imagery.image_analysis_checkpoint WHERE owner_scope=wanted_owner AND job_id=wanted_job) THEN RETURN QUERY SELECT 'basis'::text,j.revision;RETURN;END IF;
            END IF;
            IF wanted_tail_kind='unclaimed_stage' AND NOT (s.stage_state='planned' AND s.claim_generation=0 AND NOT s.capacity_reserved AND s.claim_token=s.stage_id AND s.claim_config_revision=0 AND s.claim_lease_ms=0 AND s.lease_until_ms=0) THEN RETURN QUERY SELECT 'basis'::text,j.revision;RETURN;END IF;
            IF wanted_tail_kind='deferred_stage' AND NOT (s.stage_state='planned' AND s.claim_generation>0 AND NOT s.capacity_reserved AND s.claim_token=s.stage_id AND s.claim_config_revision=0 AND s.claim_lease_ms=0 AND s.lease_until_ms=0 AND s.queued_not_before_ms>0) THEN RETURN QUERY SELECT 'basis'::text,j.revision;RETURN;END IF;
            IF wanted_tail_kind='released_stage' AND NOT (s.stage_state='released' AND NOT s.capacity_reserved AND s.lease_until_ms=0 AND seal.allocation_kind='existing' AND s.parent_dispatch IS NOT DISTINCT FROM wanted_candidate AND wanted_candidate IS NOT NULL AND EXISTS(SELECT 1 FROM imagery.image_candidate_checkpoint p JOIN imagery.image_stage_cursor c ON c.current_dispatch=p.dispatch_id WHERE p.dispatch_id=wanted_candidate AND c.job_id=wanted_job)) THEN RETURN QUERY SELECT 'basis'::text,j.revision;RETURN;END IF;
            IF to_regclass('imagery.image_dispatch_usage') IS NOT NULL THEN
                IF EXISTS(SELECT 1 FROM imagery.image_dispatch_usage WHERE job_id=wanted_job AND stage_id=wanted_tail_stage) THEN RETURN QUERY SELECT 'paid'::text,j.revision;RETURN;END IF;
            END IF;
        END IF;
        IF EXISTS(SELECT 1 FROM imagery.image_dispatch_attempt WHERE job_id=wanted_job AND stage_id=wanted_tail_stage) THEN
            RETURN QUERY SELECT 'paid'::text,j.revision;RETURN;
        END IF;
        IF to_regclass('imagery.image_finance_admission_binding') IS NOT NULL THEN
            IF EXISTS(SELECT 1 FROM pg_attribute WHERE attrelid='imagery.image_finance_admission_binding'::regclass AND attname='execution_stage_id' AND NOT attisdropped) THEN
                SELECT EXISTS(SELECT 1 FROM imagery.image_finance_admission_binding WHERE job_id=wanted_job
                    AND coalesce(execution_stage_id,stage_id)=wanted_tail_stage) INTO paid;
            ELSE
                SELECT EXISTS(SELECT 1 FROM imagery.image_finance_admission_binding WHERE job_id=wanted_job AND stage_id=wanted_tail_stage) INTO paid;
            END IF;
        END IF;
        IF paid THEN RETURN QUERY SELECT 'paid'::text,j.revision;RETURN;END IF;
        IF s.capacity_reserved THEN
            UPDATE imagery.image_execution_capacity SET in_flight=in_flight-1,updated_at_ms=wanted_now_ms
                WHERE capacity_scope=s.capacity_scope AND in_flight>0;
            GET DIAGNOSTICS changed=ROW_COUNT;
            IF changed<>1 THEN RAISE EXCEPTION 'terminal active reserve missing' USING ERRCODE='23514';END IF;
        ELSIF s.stage_state='planned' THEN
            UPDATE imagery.image_execution_capacity SET pending_jobs=pending_jobs-1,
                pending_bytes=pending_bytes-octet_length(j.intent_bytes)-octet_length(j.config_bytes),updated_at_ms=wanted_now_ms
                WHERE capacity_scope=s.capacity_scope AND pending_jobs>0
                    AND pending_bytes>=octet_length(j.intent_bytes)+octet_length(j.config_bytes);
            GET DIAGNOSTICS changed=ROW_COUNT;
            IF changed<>1 THEN RAISE EXCEPTION 'terminal pending reserve missing' USING ERRCODE='23514';END IF;
        END IF;
        UPDATE imagery.image_execution_stage SET stage_state='released',capacity_reserved=false,lease_until_ms=0,queued_notice=NULL,
            claim_generation=claim_generation+1,updated_at_ms=wanted_now_ms WHERE job_id=wanted_job AND stage_id=wanted_tail_stage
            AND (stage_state<>'released' OR capacity_reserved OR lease_until_ms<>0);
    ELSIF wanted_tail_kind='dispatch' THEN
        IF EXISTS(SELECT 1 FROM imagery.image_dispatch_attempt d WHERE d.job_id=wanted_job AND d.stage_id=wanted_tail_stage
            AND d.dispatch_id=wanted_tail_dispatch AND d.dispatch_state='prepared') THEN
            SELECT * INTO s FROM imagery.image_execution_stage WHERE job_id=wanted_job AND stage_id=wanted_tail_stage FOR UPDATE;
            IF s.capacity_reserved THEN
                UPDATE imagery.image_execution_capacity SET in_flight=in_flight-1,updated_at_ms=wanted_now_ms
                    WHERE capacity_scope=s.capacity_scope AND in_flight>0;
                GET DIAGNOSTICS changed=ROW_COUNT;
                IF changed<>1 THEN RAISE EXCEPTION 'terminal prepared reserve missing' USING ERRCODE='23514';END IF;
            END IF;
            UPDATE imagery.image_dispatch_attempt SET dispatch_state='released',failure_cause=NULL,
                cost_unsent_ack=cost_unsent_ack OR (s.stage_kind='local_transform' AND local_accounting AND finance_attempt IS NULL),
                settlement=CASE WHEN local_accounting AND finance_attempt IS NULL THEN 'released' ELSE settlement END,
                updated_at_ms=wanted_now_ms WHERE dispatch_id=wanted_tail_dispatch;
            UPDATE imagery.image_execution_stage SET stage_state='released',capacity_reserved=false,lease_until_ms=0,queued_notice=NULL,
                claim_generation=claim_generation+1,updated_at_ms=wanted_now_ms WHERE job_id=wanted_job AND stage_id=wanted_tail_stage;
        END IF;
    ELSE RETURN QUERY SELECT 'invalid'::text,j.revision;RETURN;END IF;
    UPDATE imagery.image_job SET state='retired',revision=seal.sealed_job_revision,updated_at_ms=wanted_now_ms WHERE job_id=wanted_job;
    RETURN QUERY SELECT 'closed'::text,seal.sealed_job_revision::bigint;
END $$;
REVOKE ALL ON FUNCTION imagery.close_terminal_output(bytea,bytea,text,bytea,bytea,text,bytea,bytea,bytea,bigint,bigint) FROM PUBLIC;

-- Read-only bridge predicate: no financial identifiers or values are disclosed to output workers.
CREATE OR REPLACE FUNCTION imagery.stage_has_finance(wanted_owner bytea,wanted_job bytea,wanted_stage bytea)
RETURNS boolean LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE answer boolean;
BEGIN
    IF NOT EXISTS(SELECT 1 FROM imagery.image_execution_stage s JOIN imagery.image_job j ON j.job_id=s.job_id
        WHERE s.owner_scope=wanted_owner AND j.owner_scope=wanted_owner AND s.job_id=wanted_job AND s.stage_id=wanted_stage) THEN
        RETURN NULL;
    END IF;
    IF to_regclass('imagery.image_finance_admission_binding') IS NULL THEN RETURN false; END IF;
    IF EXISTS(SELECT 1 FROM pg_attribute WHERE attrelid='imagery.image_finance_admission_binding'::regclass
        AND attname='execution_stage_id' AND NOT attisdropped) THEN
        SELECT EXISTS(SELECT 1 FROM imagery.image_finance_admission_binding
            WHERE job_id=wanted_job AND coalesce(execution_stage_id,stage_id)=wanted_stage) INTO answer;
    ELSE
        SELECT EXISTS(SELECT 1 FROM imagery.image_finance_admission_binding
            WHERE job_id=wanted_job AND stage_id=wanted_stage) INTO answer;
    END IF;
    RETURN answer;
END $$;
REVOKE ALL ON FUNCTION imagery.stage_has_finance(bytea,bytea,bytea) FROM PUBLIC;

ALTER TABLE imagery.image_execution_stage ADD COLUMN IF NOT EXISTS queued_notice text
    CHECK(queued_notice IS NULL OR queued_notice IN ('cooldown','probing'));
