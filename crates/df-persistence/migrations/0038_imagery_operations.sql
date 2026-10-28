-- Scoped imagery operations and recipe freezes (task IMG-OPERATION-STORE). Schema imagery in
-- the one hosted database. Times are milliseconds since the Unix epoch from the service clock.
-- Ids are 16 bytes; fingerprints, sharing digests and recipe keys are 32. Owner scopes are the
-- canonical versioned bytes from df-imagery (version tag, owner tag, exact campaign/run or
-- account ids), never formatted text. No table here stores money, supplier requests, credentials,
-- raw URLs or published asset bytes. No grants: only the owner and superuser reach these rows
-- until the later hosted authorization/definer binding lands.
-- Lock order: the advisory lock over (owner_scope, operation_id), then the image_job row
-- FOR UPDATE, then image_interest rows. At most one job row is ever locked per call.
-- The operation<->interest link is checked in both directions over the full receipt tuple
-- with deferred constraints so one transaction can insert the receipt and its interest in
-- either order; a commit with a dangling, cross-owner or wrong-job link fails. The reverse
-- link must match (owner_scope, operation_id, job_id, interest_id): checking only
-- (owner_scope, operation_id) would allow a second phantom interest for a committed operation
-- pointing at the same or another same-owner job.

CREATE SCHEMA IF NOT EXISTS imagery;

CREATE TABLE IF NOT EXISTS imagery.image_job (
    job_id bytea PRIMARY KEY CHECK (octet_length(job_id) = 16 AND job_id <> decode('00000000000000000000000000000000', 'hex')),
    owner_scope bytea NOT NULL CHECK (octet_length(owner_scope) IN (18, 34)),
    sharing_identity bytea NOT NULL CHECK (octet_length(sharing_identity) = 32),
    intent_revision bigint NOT NULL CHECK (intent_revision > 0 AND intent_revision <= 4294967295),
    intent_bytes bytea NOT NULL CHECK (octet_length(intent_bytes) > 0),
    config_revision bigint NOT NULL CHECK (config_revision > 0 AND config_revision <= 4294967295),
    config_bytes bytea NOT NULL CHECK (octet_length(config_bytes) > 0),
    absolute_deadline_ms bigint NOT NULL CHECK (absolute_deadline_ms > 0),
    state text NOT NULL CHECK (state IN ('planning', 'recipe_frozen', 'retired')),
    revision bigint NOT NULL CHECK (revision > 0),
    resolved_recipe_key bytea CHECK (octet_length(resolved_recipe_key) = 32),
    recipe_revision bigint CHECK (recipe_revision > 0 AND recipe_revision <= 4294967295),
    recipe_bytes bytea CHECK (octet_length(recipe_bytes) > 0),
    created_at_ms bigint NOT NULL CHECK (created_at_ms >= 0),
    updated_at_ms bigint NOT NULL CHECK (updated_at_ms >= 0),
    CHECK (
        (state = 'planning' AND resolved_recipe_key IS NULL
            AND recipe_revision IS NULL AND recipe_bytes IS NULL)
        OR (state = 'recipe_frozen' AND resolved_recipe_key IS NOT NULL
            AND recipe_revision IS NOT NULL AND recipe_bytes IS NOT NULL)
        OR (state = 'retired' AND ((resolved_recipe_key IS NULL
            AND recipe_revision IS NULL AND recipe_bytes IS NULL)
            OR (resolved_recipe_key IS NOT NULL
            AND recipe_revision IS NOT NULL AND recipe_bytes IS NOT NULL)))
    ),
    UNIQUE (job_id, owner_scope)
);

CREATE TABLE IF NOT EXISTS imagery.image_operation (
    owner_scope bytea NOT NULL CHECK (octet_length(owner_scope) IN (18, 34)),
    operation_id bytea NOT NULL CHECK (octet_length(operation_id) = 16 AND operation_id <> decode('00000000000000000000000000000000', 'hex')),
    fingerprint bytea NOT NULL CHECK (octet_length(fingerprint) = 32),
    job_id bytea NOT NULL CHECK (octet_length(job_id) = 16 AND job_id <> decode('00000000000000000000000000000000', 'hex')),
    interest_id bytea NOT NULL UNIQUE CHECK (octet_length(interest_id) = 16 AND interest_id <> decode('00000000000000000000000000000000', 'hex')),
    created_at_ms bigint NOT NULL CHECK (created_at_ms >= 0),
    PRIMARY KEY (owner_scope, operation_id),
    CONSTRAINT image_operation_receipt_unique
        UNIQUE (owner_scope, operation_id, job_id, interest_id),
    FOREIGN KEY (job_id, owner_scope)
        REFERENCES imagery.image_job (job_id, owner_scope)
        DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE IF NOT EXISTS imagery.image_interest (
    interest_id bytea PRIMARY KEY CHECK (octet_length(interest_id) = 16 AND interest_id <> decode('00000000000000000000000000000000', 'hex')),
    owner_scope bytea NOT NULL CHECK (octet_length(owner_scope) IN (18, 34)),
    job_id bytea NOT NULL CHECK (octet_length(job_id) = 16 AND job_id <> decode('00000000000000000000000000000000', 'hex')),
    operation_id bytea NOT NULL CHECK (octet_length(operation_id) = 16 AND operation_id <> decode('00000000000000000000000000000000', 'hex')),
    state text NOT NULL CHECK (state IN ('attached', 'withdrawn')),
    created_at_ms bigint NOT NULL CHECK (created_at_ms >= 0),
    withdrawn_at_ms bigint CHECK (withdrawn_at_ms IS NULL OR withdrawn_at_ms >= created_at_ms),
    FOREIGN KEY (job_id, owner_scope)
        REFERENCES imagery.image_job (job_id, owner_scope)
        DEFERRABLE INITIALLY DEFERRED,
    UNIQUE (interest_id, owner_scope, job_id, operation_id),
    CHECK (
        (state = 'attached' AND withdrawn_at_ms IS NULL)
        OR (state = 'withdrawn' AND withdrawn_at_ms IS NOT NULL)
    )
);

-- Both directions of the receipt/interest link over the full receipt tuple, deferred so
-- either row may be inserted first while a commit with a dangling, cross-owner, wrong-job
-- or phantom-extra-interest pairing still fails.
ALTER TABLE imagery.image_interest DROP CONSTRAINT IF EXISTS image_interest_owner_scope_operation_id_fkey;
ALTER TABLE imagery.image_interest DROP CONSTRAINT IF EXISTS image_interest_operation_link;
ALTER TABLE imagery.image_operation DROP CONSTRAINT IF EXISTS image_operation_interest_link;
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'image_operation_receipt_unique'
    ) THEN
        ALTER TABLE imagery.image_operation ADD CONSTRAINT image_operation_receipt_unique
            UNIQUE (owner_scope, operation_id, job_id, interest_id);
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'image_operation_interest_link'
    ) THEN
        ALTER TABLE imagery.image_operation ADD CONSTRAINT image_operation_interest_link
            FOREIGN KEY (interest_id, owner_scope, job_id, operation_id)
            REFERENCES imagery.image_interest (interest_id, owner_scope, job_id, operation_id)
            DEFERRABLE INITIALLY DEFERRED;
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'image_interest_operation_link'
    ) THEN
        ALTER TABLE imagery.image_interest ADD CONSTRAINT image_interest_operation_link
            FOREIGN KEY (owner_scope, operation_id, job_id, interest_id)
            REFERENCES imagery.image_operation (owner_scope, operation_id, job_id, interest_id)
            DEFERRABLE INITIALLY DEFERRED;
    END IF;
END
$$;

CREATE INDEX IF NOT EXISTS image_operation_job
    ON imagery.image_operation (job_id);
CREATE INDEX IF NOT EXISTS image_interest_job
    ON imagery.image_interest (job_id);
