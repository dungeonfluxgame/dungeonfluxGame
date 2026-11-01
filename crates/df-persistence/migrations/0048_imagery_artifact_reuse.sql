-- One catalog election and an owned unsent dependency; no second Ready cache or finance ledger.
ALTER TABLE imagery.image_publication_allocation
    ADD COLUMN IF NOT EXISTS semantic_recipe_revision bigint,
    ADD COLUMN IF NOT EXISTS semantic_recipe_bytes bytea;
DO $$ BEGIN
    IF NOT EXISTS(SELECT 1 FROM pg_constraint WHERE conname='image_allocation_semantic_recipe') THEN
        ALTER TABLE imagery.image_publication_allocation ADD CONSTRAINT image_allocation_semantic_recipe CHECK (
            (semantic_recipe_revision IS NULL AND semantic_recipe_bytes IS NULL)
            OR (semantic_recipe_revision IS NOT NULL AND semantic_recipe_revision BETWEEN 1 AND 4294967295
                AND semantic_recipe_bytes IS NOT NULL
                AND octet_length(semantic_recipe_bytes) BETWEEN 1 AND 1048576)
        );
    END IF;
END $$;

CREATE TABLE IF NOT EXISTS imagery.image_artifact_dependency (
    owner_scope bytea NOT NULL CHECK(octet_length(owner_scope)=34),
    job_id bytea NOT NULL,
    stage_id bytea NOT NULL,
    claim_generation bigint NOT NULL CHECK(claim_generation>=1),
    semantic_recipe_revision bigint NOT NULL CHECK(semantic_recipe_revision IS NOT NULL AND semantic_recipe_revision BETWEEN 1 AND 4294967295),
    semantic_recipe_bytes bytea NOT NULL CHECK(octet_length(semantic_recipe_bytes) BETWEEN 1 AND 1048576),
    semantic_key bytea NOT NULL CHECK(octet_length(semantic_key)=32),
    asset_id bytea NOT NULL CHECK(octet_length(asset_id)=16 AND asset_id<>decode(repeat('00',16),'hex')),
    publication_attempt bytea NOT NULL CHECK(octet_length(publication_attempt)=16 AND publication_attempt<>decode(repeat('00',16),'hex')),
    reference_state text NOT NULL CHECK(reference_state IN ('waiting','resolved','released')),
    accepted_at_ms bigint NOT NULL CHECK(accepted_at_ms>=0),
    next_check_at_ms bigint NOT NULL CHECK(next_check_at_ms>=accepted_at_ms),
    resolved_at_ms bigint,
    resolved_job_revision bigint,
    PRIMARY KEY(owner_scope,job_id),
    FOREIGN KEY(job_id,owner_scope) REFERENCES imagery.image_job(job_id,owner_scope),
    FOREIGN KEY(job_id,stage_id) REFERENCES imagery.image_execution_stage(job_id,stage_id),
    CHECK((reference_state='resolved' AND resolved_at_ms IS NOT NULL AND resolved_job_revision IS NOT NULL
        AND resolved_at_ms>=accepted_at_ms AND resolved_job_revision>0)
        OR (reference_state IN ('waiting','released') AND resolved_at_ms IS NULL AND resolved_job_revision IS NULL))
);

-- Catalog deletion remains possible. Every reader/writer revalidates asset/attempt under catalog locks.
CREATE INDEX IF NOT EXISTS image_artifact_dependency_due
    ON imagery.image_artifact_dependency(next_check_at_ms,job_id) WHERE reference_state='waiting';

CREATE OR REPLACE FUNCTION imagery.guard_artifact_dependency() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'artifact dependency history is retained' USING ERRCODE='23514';
    END IF;
    IF OLD.reference_state='resolved'
        OR NEW.owner_scope<>OLD.owner_scope OR NEW.job_id<>OLD.job_id OR NEW.stage_id<>OLD.stage_id
        OR NEW.semantic_recipe_revision<>OLD.semantic_recipe_revision
        OR NEW.semantic_recipe_bytes<>OLD.semantic_recipe_bytes OR NEW.semantic_key<>OLD.semantic_key
        OR NEW.accepted_at_ms<>OLD.accepted_at_ms OR NEW.claim_generation<OLD.claim_generation
        OR (OLD.reference_state='released' AND NEW.reference_state='waiting'
            AND NEW.claim_generation<=OLD.claim_generation)
        OR (OLD.reference_state='released' AND NEW.reference_state='resolved') THEN
        RAISE EXCEPTION 'artifact dependency transition refused' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END $$;
DROP TRIGGER IF EXISTS image_artifact_dependency_guard ON imagery.image_artifact_dependency;
CREATE TRIGGER image_artifact_dependency_guard BEFORE UPDATE OR DELETE ON imagery.image_artifact_dependency
    FOR EACH ROW EXECUTE FUNCTION imagery.guard_artifact_dependency();
REVOKE ALL ON FUNCTION imagery.guard_artifact_dependency() FROM PUBLIC;
REVOKE ALL ON imagery.image_artifact_dependency FROM PUBLIC;
GRANT SELECT ON imagery.image_artifact_dependency TO df_imagery_publication,df_imagery_output;

GRANT EXECUTE ON FUNCTION imagery.stage_has_finance(bytea,bytea,bytea) TO df_imagery_publication;
