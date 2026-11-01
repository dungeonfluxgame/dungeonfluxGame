-- Atomic imagery publication, after media 0010 and imagery 0038/0039/0040.
-- Results and their exact proof bindings are immutable. No finance state is written.
ALTER TABLE media.asset DROP CONSTRAINT IF EXISTS asset_reject_reason_check;
ALTER TABLE media.asset ADD CONSTRAINT asset_reject_reason_check CHECK(reject_reason IN (
    'validation_failed','screen_rejected','abandoned','not_dispatched','supplier_failed',
    'unknown_outcome_failed','output_lost','over_maximum_bytes','review_expired',
    'screen_unavailable','staged_expired','staged_lost','quota_exceeded','demand_ended','source_denied','preflight_rejected','supplier_declined'));
CREATE TABLE IF NOT EXISTS imagery.image_final_selection (
    job_id bytea PRIMARY KEY,
    owner_scope bytea NOT NULL,
    allocation_stage_id bytea NOT NULL,
    final_stage_id bytea NOT NULL,
    final_dispatch_id bytea NOT NULL,
    asset_id bytea NOT NULL,
    publication_attempt bytea NOT NULL,
    plan_recipe_digest bytea NOT NULL CHECK (octet_length(plan_recipe_digest)=32),
    resolved_recipe_revision bigint NOT NULL CHECK (resolved_recipe_revision BETWEEN 1 AND 4294967295),
    resolved_recipe_bytes bytea NOT NULL CHECK (octet_length(resolved_recipe_bytes) BETWEEN 1 AND 1048576),
    resolved_recipe_digest bytea NOT NULL CHECK (octet_length(resolved_recipe_digest)=32),
    candidate_stage_id bytea,
    candidate_dispatch_id bytea,
    candidate_binding bytea,
    decision_binding text NOT NULL DEFAULT '[]' CHECK (octet_length(decision_binding) BETWEEN 2 AND 1048576),
    local_inspection_binding text NOT NULL DEFAULT '[]' CHECK (octet_length(local_inspection_binding) BETWEEN 2 AND 1048576),
    source_binding bytea NOT NULL,
    selected_at_ms bigint NOT NULL CHECK (selected_at_ms>=0),
    FOREIGN KEY (job_id,owner_scope) REFERENCES imagery.image_job(job_id,owner_scope),
    FOREIGN KEY (job_id,final_stage_id,final_dispatch_id)
        REFERENCES imagery.image_dispatch_attempt(job_id,stage_id,dispatch_id),
    FOREIGN KEY (job_id,candidate_stage_id,candidate_dispatch_id)
        REFERENCES imagery.image_dispatch_attempt(job_id,stage_id,dispatch_id),
    FOREIGN KEY (publication_attempt,job_id,allocation_stage_id,asset_id,owner_scope)
        REFERENCES imagery.image_publication_allocation(publication_attempt,job_id,stage_id,asset_id,owner_scope),
    CHECK ((candidate_stage_id IS NULL)=(candidate_dispatch_id IS NULL)
        AND (candidate_dispatch_id IS NULL)=(candidate_binding IS NULL))
);
CREATE TABLE IF NOT EXISTS imagery.image_job_result (
    job_id bytea PRIMARY KEY,
    owner_scope bytea NOT NULL,
    stage_id bytea NOT NULL,
    allocation_stage_id bytea NOT NULL,
    dispatch_id bytea NOT NULL,
    asset_id bytea NOT NULL,
    publication_attempt bytea NOT NULL,
    recipe_digest bytea NOT NULL CHECK (octet_length(recipe_digest) = 32),
    binding_bytes bytea NOT NULL CHECK (octet_length(binding_bytes) BETWEEN 1 AND 16384),
    job_revision bigint NOT NULL CHECK (job_revision > 0),
    published_at_ms bigint NOT NULL CHECK (published_at_ms >= 0),
    private_reconciliation boolean NOT NULL,
    FOREIGN KEY (job_id, owner_scope) REFERENCES imagery.image_job(job_id, owner_scope),
    FOREIGN KEY (job_id, stage_id, dispatch_id)
        REFERENCES imagery.image_dispatch_attempt(job_id, stage_id, dispatch_id),
    FOREIGN KEY (publication_attempt, job_id, allocation_stage_id, asset_id, owner_scope)
        REFERENCES imagery.image_publication_allocation(publication_attempt, job_id, stage_id, asset_id, owner_scope),
    UNIQUE (asset_id, publication_attempt)
);
CREATE TABLE IF NOT EXISTS imagery.image_result_variant (
    job_id bytea NOT NULL REFERENCES imagery.image_job_result(job_id),
    variant text NOT NULL CHECK (variant IN ('master', 'display')),
    write_id bytea NOT NULL CHECK (octet_length(write_id) = 16),
    object_version text NOT NULL CHECK (octet_length(object_version) BETWEEN 1 AND 1024),
    byte_len bigint NOT NULL CHECK (byte_len > 0),
    sha256 bytea NOT NULL CHECK (octet_length(sha256) = 32),
    mime text NOT NULL CHECK (mime IN ('image/png', 'image/jpeg', 'image/webp')),
    width_px integer NOT NULL CHECK (width_px > 0),
    height_px integer NOT NULL CHECK (height_px > 0),
    alpha text NOT NULL CHECK (alpha IN ('opaque', 'transparent')),
    PRIMARY KEY (job_id, variant)
);
CREATE TABLE IF NOT EXISTS imagery.image_no_delivery (
    job_id bytea PRIMARY KEY,
    owner_scope bytea NOT NULL,
    actor_kind text NOT NULL CHECK(actor_kind IN ('dispatch','unsent_stage','unclaimed_stage','deferred_stage','released_stage')),
    actor_stage_id bytea NOT NULL,
    actor_dispatch_id bytea,
    actor_usage_revision integer CHECK(actor_usage_revision>=0),
    tail_kind text NOT NULL CHECK(tail_kind IN ('dispatch','unsent_stage','unclaimed_stage','deferred_stage','released_stage')),
    tail_stage_id bytea NOT NULL,
    tail_dispatch_id bytea,
    tail_usage_revision integer CHECK(tail_usage_revision>=0),
    candidate_stage_id bytea,
    candidate_dispatch_id bytea,
    allocation_kind text NOT NULL CHECK(allocation_kind IN ('no_allocation','existing')),
    allocation_stage_id bytea,
    asset_id bytea,
    publication_attempt bytea,
    expected_job_revision bigint NOT NULL CHECK(expected_job_revision>0),
    sealed_job_revision bigint NOT NULL CHECK(sealed_job_revision=expected_job_revision+1),
    reason text NOT NULL CHECK(reason IN ('inspection_rejected','required_inspection_unjudged',
        'creator_needs_revision','creator_uncertain','retired','expired',
        'required_inspection_preparation','required_inspection_disabled','supplier_no_output','source_denied','missing_credential','unsupported_profile','supplier_declined')),
    no_output_raw_sha256 bytea CHECK(no_output_raw_sha256 IS NULL OR octet_length(no_output_raw_sha256)=32),
    legacy_output_write_id bytea,
    legacy_output_version text,
    legacy_output_sha256 bytea,
    legacy_output_recipe_digest bytea,
    evidence_binding bytea NOT NULL CHECK(octet_length(evidence_binding) BETWEEN 1 AND 1048576),
    decision_binding text NOT NULL CHECK(octet_length(decision_binding) BETWEEN 2 AND 1048576),
    candidates_binding text NOT NULL CHECK(octet_length(candidates_binding) BETWEEN 2 AND 1048576),
    local_inspection_binding text NOT NULL CHECK(octet_length(local_inspection_binding) BETWEEN 2 AND 1048576),
    sealed_at_ms bigint NOT NULL CHECK(sealed_at_ms>=0),
    FOREIGN KEY(job_id,owner_scope) REFERENCES imagery.image_job(job_id,owner_scope),
    FOREIGN KEY(job_id,actor_stage_id) REFERENCES imagery.image_execution_stage(job_id,stage_id),
    FOREIGN KEY(job_id,tail_stage_id) REFERENCES imagery.image_execution_stage(job_id,stage_id),
    FOREIGN KEY(job_id,actor_stage_id,actor_dispatch_id) REFERENCES imagery.image_dispatch_attempt(job_id,stage_id,dispatch_id),
    FOREIGN KEY(job_id,tail_stage_id,tail_dispatch_id) REFERENCES imagery.image_dispatch_attempt(job_id,stage_id,dispatch_id),
    FOREIGN KEY(job_id,candidate_stage_id,candidate_dispatch_id) REFERENCES imagery.image_dispatch_attempt(job_id,stage_id,dispatch_id),
    FOREIGN KEY(publication_attempt,job_id,allocation_stage_id,asset_id,owner_scope)
        REFERENCES imagery.image_publication_allocation(publication_attempt,job_id,stage_id,asset_id,owner_scope),
    CHECK((actor_kind='dispatch' AND actor_dispatch_id IS NOT NULL AND actor_usage_revision IS NOT NULL)
        OR (actor_kind IN ('unsent_stage','unclaimed_stage','deferred_stage','released_stage') AND actor_dispatch_id IS NULL AND actor_usage_revision IS NULL)),
    CHECK((tail_kind='dispatch' AND tail_dispatch_id IS NOT NULL AND tail_usage_revision IS NOT NULL)
        OR (tail_kind IN ('unsent_stage','unclaimed_stage','deferred_stage','released_stage') AND tail_dispatch_id IS NULL AND tail_usage_revision IS NULL)),
    CHECK((candidate_stage_id IS NULL)=(candidate_dispatch_id IS NULL)),
    CHECK(reason='supplier_no_output' OR no_output_raw_sha256 IS NULL),
    CHECK((legacy_output_write_id IS NULL AND legacy_output_version IS NULL AND legacy_output_sha256 IS NULL AND legacy_output_recipe_digest IS NULL)
        OR (reason='source_denied' AND candidate_dispatch_id IS NULL AND actor_kind='dispatch' AND tail_kind='dispatch'
            AND legacy_output_write_id IS NOT NULL AND octet_length(legacy_output_write_id)=16
            AND legacy_output_version IS NOT NULL AND octet_length(legacy_output_version) BETWEEN 1 AND 1024
            AND legacy_output_sha256 IS NOT NULL AND octet_length(legacy_output_sha256)=32
            AND legacy_output_recipe_digest IS NOT NULL AND octet_length(legacy_output_recipe_digest)=32)),
    CHECK((allocation_kind='existing' AND allocation_stage_id IS NOT NULL AND asset_id IS NOT NULL AND publication_attempt IS NOT NULL)
        OR (allocation_kind='no_allocation' AND allocation_stage_id IS NULL AND asset_id IS NULL AND publication_attempt IS NULL))
);
CREATE OR REPLACE FUNCTION imagery.forbid_result_mutation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'imagery publication results are immutable';
END $$;
DROP TRIGGER IF EXISTS image_job_result_immutable ON imagery.image_job_result;
CREATE TRIGGER image_job_result_immutable BEFORE UPDATE OR DELETE ON imagery.image_job_result
    FOR EACH ROW EXECUTE FUNCTION imagery.forbid_result_mutation();
DROP TRIGGER IF EXISTS image_result_variant_immutable ON imagery.image_result_variant;
CREATE TRIGGER image_result_variant_immutable BEFORE UPDATE OR DELETE ON imagery.image_result_variant
    FOR EACH ROW EXECUTE FUNCTION imagery.forbid_result_mutation();
DROP TRIGGER IF EXISTS image_final_selection_immutable ON imagery.image_final_selection;
CREATE TRIGGER image_final_selection_immutable BEFORE UPDATE OR DELETE ON imagery.image_final_selection
    FOR EACH ROW EXECUTE FUNCTION imagery.forbid_result_mutation();
DROP TRIGGER IF EXISTS image_no_delivery_immutable ON imagery.image_no_delivery;
CREATE TRIGGER image_no_delivery_immutable BEFORE UPDATE OR DELETE ON imagery.image_no_delivery
    FOR EACH ROW EXECUTE FUNCTION imagery.forbid_result_mutation();
REVOKE ALL ON imagery.image_final_selection FROM PUBLIC;
REVOKE ALL ON imagery.image_no_delivery FROM PUBLIC;
REVOKE ALL ON imagery.image_job_result, imagery.image_result_variant FROM PUBLIC;
CREATE INDEX IF NOT EXISTS image_job_result_recipe ON imagery.image_job_result(owner_scope, recipe_digest);

-- The publication service has no finance membership and owns no tables.
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'df_imagery_publication') THEN
        CREATE ROLE df_imagery_publication NOLOGIN;
    END IF;
    ALTER ROLE df_imagery_publication NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
END $$;
GRANT USAGE ON SCHEMA media, imagery TO df_imagery_publication;
GRANT SELECT, INSERT, UPDATE ON media.quota_usage TO df_imagery_publication;
GRANT SELECT ON media.asset, media.asset_variant,media.tombstone TO df_imagery_publication;
GRANT UPDATE (state, step, step_owner, step_lease_until_ms, ready_at_ms, updated_at_ms,
    reject_reason,
    winning_writes, mime, byte_length, sha256, width_px, height_px,
    audience_kind, audience_account, audience_campaign, audience_members)
    ON media.asset TO df_imagery_publication;
GRANT SELECT, DELETE ON media.quota_reservation TO df_imagery_publication;
GRANT INSERT ON media.settlement_outbox TO df_imagery_publication;
GRANT USAGE ON SEQUENCE media.settlement_outbox_outbox_seq_seq TO df_imagery_publication;
GRANT SELECT ON imagery.image_job, imagery.image_execution_stage,
    imagery.image_dispatch_attempt, imagery.image_stage_cursor,
    imagery.image_dispatch_checkpoint, imagery.image_publication_allocation, imagery.image_interest
    TO df_imagery_publication;
GRANT UPDATE (revision, updated_at_ms) ON imagery.image_job TO df_imagery_publication;
-- PostgreSQL needs UPDATE on one column for each FOR UPDATE lock.
GRANT UPDATE (updated_at_ms) ON imagery.image_execution_stage,
    imagery.image_dispatch_attempt, imagery.image_stage_cursor TO df_imagery_publication;
GRANT SELECT, INSERT ON imagery.image_job_result, imagery.image_result_variant
    TO df_imagery_publication;
GRANT SELECT ON imagery.image_final_selection TO df_imagery_publication;
GRANT SELECT ON imagery.image_no_delivery TO df_imagery_publication,df_imagery_admission;
DO $$ BEGIN
    IF EXISTS(SELECT 1 FROM pg_roles WHERE rolname='df_imagery_usage') THEN
        GRANT SELECT ON imagery.image_no_delivery TO df_imagery_usage;
    END IF;
END $$;
GRANT SELECT ON imagery.image_candidate_observation,imagery.image_candidate_checkpoint TO df_imagery_publication;
GRANT SELECT ON imagery.image_decision_observation,imagery.image_decision_checkpoint TO df_imagery_publication;
GRANT SELECT ON imagery.image_local_inspection TO df_imagery_publication;
-- Canonical settlement reads immutable delivered contribution evidence without publication writes.
GRANT SELECT ON imagery.image_job_result,imagery.image_final_selection,
    imagery.image_candidate_observation,imagery.image_candidate_checkpoint,
    imagery.image_decision_checkpoint,imagery.image_dispatch_checkpoint TO df_imagery_admission;
GRANT SELECT (owner_scope,job_id,stage_id,dispatch_id,decision_kind,observation_version,
    schema_revision,validator_revision,input_revision,input_digest,config_revision,config_digest,
    parent_dispatch,usage_revision,observed_at_ms,payload_sha256)
    ON imagery.image_decision_observation TO df_imagery_admission;
-- ON CONFLICT(attempt) needs SELECT on the arbiter column even with DO NOTHING.
GRANT SELECT (attempt) ON media.settlement_outbox TO df_imagery_publication;

-- Durable Local mode gates the only output-service allocation writer.
CREATE OR REPLACE FUNCTION imagery.allocate_local_output(
    wanted_owner bytea, wanted_job bytea, wanted_stage bytea,
    wanted_asset bytea, wanted_attempt bytea, now_ms bigint
) RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    job imagery.image_job%ROWTYPE;
    stage imagery.image_execution_stage%ROWTYPE;
    asset media.asset%ROWTYPE;
    existing imagery.image_publication_allocation%ROWTYPE;
BEGIN
    SELECT * INTO job FROM imagery.image_job WHERE job_id=wanted_job AND owner_scope=wanted_owner FOR UPDATE;
    IF NOT FOUND THEN RAISE EXCEPTION 'local output job missing' USING ERRCODE='23514'; END IF;
    SELECT * INTO stage FROM imagery.image_execution_stage WHERE job_id=wanted_job AND stage_id=wanted_stage;
    IF NOT FOUND OR NOT stage.local_accounting OR stage.owner_scope<>wanted_owner
        OR job.planned_publication_asset IS DISTINCT FROM wanted_asset
        OR job.planned_publication_attempt IS DISTINCT FROM wanted_attempt
        OR job.planned_output_stage IS DISTINCT FROM wanted_stage
        OR job.resolved_recipe_key IS NULL OR job.recipe_bytes IS NULL THEN
        RAISE EXCEPTION 'local output identity refused' USING ERRCODE='23514';
    END IF;
    SELECT * INTO asset FROM media.asset WHERE asset_id=wanted_asset;
    IF NOT FOUND OR asset.attempt<>wanted_attempt OR asset.kind<>'image'
        OR asset.provenance<>'generated' OR asset.request_key IS DISTINCT FROM job.resolved_recipe_key
        OR ((octet_length(wanted_owner)=34 AND get_byte(wanted_owner,1)=0
                AND asset.campaign_id=substring(wanted_owner FROM 3 FOR 16))
            OR (octet_length(wanted_owner)=18 AND get_byte(wanted_owner,1)=1
                AND asset.campaign_id IS NULL AND asset.origin_account=substring(wanted_owner FROM 3 FOR 16)
                AND asset.charge_scope_kind='account' AND asset.charge_scope_id=asset.origin_account
                AND asset.audience_kind='owner_private' AND asset.audience_account=asset.origin_account)) IS NOT TRUE THEN
        RAISE EXCEPTION 'local output asset refused' USING ERRCODE='23514';
    END IF;
    SELECT * INTO existing FROM imagery.image_publication_allocation WHERE publication_attempt=wanted_attempt;
    IF FOUND THEN
        IF existing.asset_id<>wanted_asset OR existing.job_id<>wanted_job OR existing.stage_id<>wanted_stage
            OR existing.owner_scope<>wanted_owner OR existing.original_recipe_key<>job.resolved_recipe_key
            OR existing.original_recipe_revision<>job.recipe_revision OR existing.original_recipe_bytes<>job.recipe_bytes
            OR existing.original_config_revision<>job.config_revision OR existing.original_config_bytes<>job.config_bytes THEN
            RAISE EXCEPTION 'local output replay refused' USING ERRCODE='23514';
        END IF;
        RETURN false;
    END IF;
    IF job.state<>'recipe_frozen' OR now_ms<0 OR job.absolute_deadline_ms<=now_ms
        OR NOT EXISTS(SELECT 1 FROM imagery.image_interest WHERE job_id=wanted_job AND state='attached') THEN
        RAISE EXCEPTION 'local output demand ended' USING ERRCODE='23514';
    END IF;
    INSERT INTO imagery.image_publication_allocation(publication_attempt,asset_id,job_id,stage_id,owner_scope,
        original_recipe_key,original_recipe_revision,original_recipe_bytes,original_config_revision,original_config_bytes,created_at_ms)
        VALUES(wanted_attempt,wanted_asset,wanted_job,wanted_stage,wanted_owner,job.resolved_recipe_key,
            job.recipe_revision,job.recipe_bytes,job.config_revision,job.config_bytes,now_ms);
    RETURN true;
END $$;
REVOKE ALL ON FUNCTION imagery.allocate_local_output(bytea,bytea,bytea,bytea,bytea,bigint) FROM PUBLIC;
DO $$
BEGIN
    IF NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname='df_imagery_output') THEN CREATE ROLE df_imagery_output NOLOGIN; END IF;
    ALTER ROLE df_imagery_output NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
END $$;
GRANT df_imagery_publication TO df_imagery_output;
DO $$ BEGIN
    IF to_regclass('media.schema_migration') IS NOT NULL THEN
        GRANT SELECT ON media.schema_migration TO df_imagery_output;
    END IF;
END $$;
GRANT INSERT,DELETE ON media.asset TO df_imagery_output;
GRANT UPDATE(staged_key,staged_version,staged_byte_length,staged_sha256,staged_at_ms) ON media.asset TO df_imagery_output;
GRANT INSERT ON media.quota_reservation TO df_imagery_output;
GRANT INSERT ON media.asset_variant TO df_imagery_output;
GRANT UPDATE(state,object_version,byte_length,sha256) ON media.asset_variant TO df_imagery_output;
GRANT INSERT ON imagery.image_final_selection TO df_imagery_output;
GRANT INSERT ON imagery.image_no_delivery TO df_imagery_output;
GRANT EXECUTE ON FUNCTION imagery.close_terminal_output(bytea,bytea,text,bytea,bytea,text,bytea,bytea,bytea,bigint,bigint) TO df_imagery_output;
GRANT EXECUTE ON FUNCTION imagery.stage_has_finance(bytea,bytea,bytea) TO df_imagery_output;
GRANT EXECUTE ON FUNCTION imagery.allocate_local_output(bytea,bytea,bytea,bytea,bytea,bigint) TO df_imagery_output;
GRANT SELECT ON imagery.image_dispatch_recovery TO df_imagery_publication;
GRANT EXECUTE ON FUNCTION imagery.mark_local_dispatch_sent(bytea,bytea,bytea,bytea,bytea,bigint,bigint) TO df_imagery_output;
GRANT UPDATE(updated_at_ms) ON imagery.image_dispatch_recovery TO df_imagery_publication;

-- The output role may mirror record-only outcomes but must never settle hosted liability.
CREATE OR REPLACE FUNCTION imagery.mirror_local_outcome(
    wanted_owner bytea, wanted_job bytea, wanted_dispatch bytea,
    usable boolean, now_ms bigint
) RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    job imagery.image_job%ROWTYPE;
    stage imagery.image_execution_stage%ROWTYPE;
    dispatch imagery.image_dispatch_attempt%ROWTYPE;
    current_dispatch bytea;
    wanted_settlement text;
BEGIN
    IF current_setting('transaction_isolation') <> 'read committed' OR now_ms IS NULL OR now_ms < 0 THEN
        RAISE EXCEPTION 'local outcome transaction refused' USING ERRCODE='23514';
    END IF;
    SELECT * INTO job FROM imagery.image_job WHERE job_id=wanted_job AND owner_scope=wanted_owner FOR UPDATE;
    IF NOT FOUND THEN RAISE EXCEPTION 'local outcome job missing' USING ERRCODE='23514'; END IF;
    SELECT s.* INTO stage FROM imagery.image_execution_stage s
        JOIN imagery.image_dispatch_attempt d USING(job_id,stage_id)
        WHERE d.dispatch_id=wanted_dispatch AND d.job_id=wanted_job FOR UPDATE OF s;
    IF NOT FOUND OR NOT stage.local_accounting OR stage.owner_scope<>wanted_owner
        OR stage.stage_kind IN ('classification','inspection','photo_inspection','head_locations',
            'segmentation','staging_identity','staging_facing') THEN
        RAISE EXCEPTION 'local outcome stage refused' USING ERRCODE='23514';
    END IF;
    SELECT * INTO dispatch FROM imagery.image_dispatch_attempt
        WHERE dispatch_id=wanted_dispatch AND job_id=wanted_job AND stage_id=stage.stage_id FOR UPDATE;
    IF NOT FOUND OR NOT dispatch.local_accounting OR dispatch.finance_attempt IS NOT NULL
        OR dispatch.finance_operation IS NOT NULL OR dispatch.finance_effect IS NOT NULL THEN
        RAISE EXCEPTION 'local outcome dispatch refused' USING ERRCODE='23514';
    END IF;
    SELECT c.current_dispatch INTO current_dispatch FROM imagery.image_stage_cursor c
        WHERE c.job_id=wanted_job AND c.stage_id=stage.stage_id FOR UPDATE;
    IF NOT FOUND OR current_dispatch<>wanted_dispatch OR now_ms<dispatch.updated_at_ms
        OR now_ms<job.updated_at_ms THEN
        RAISE EXCEPTION 'local outcome lineage refused' USING ERRCODE='23514';
    END IF;
    wanted_settlement := CASE
        WHEN usable IS TRUE AND dispatch.dispatch_state='succeeded' THEN 'settled'
        WHEN usable IS FALSE AND dispatch.dispatch_state='failed' THEN 'unusable_settled'
        WHEN usable IS NULL AND dispatch.dispatch_state='unknown' THEN 'unknown_outstanding'
        ELSE NULL END;
    IF wanted_settlement IS NULL THEN
        RAISE EXCEPTION 'local outcome state refused' USING ERRCODE='23514';
    END IF;
    IF dispatch.settlement=wanted_settlement THEN RETURN false; END IF;
    -- Match mirror_settlement_in: only Held/UnknownOutstanding advance; replay is read-only.
    IF dispatch.settlement NOT IN ('held','unknown_outstanding') THEN
        RAISE EXCEPTION 'local outcome replay refused' USING ERRCODE='23514';
    END IF;
    UPDATE imagery.image_dispatch_attempt SET settlement=wanted_settlement,updated_at_ms=now_ms
        WHERE dispatch_id=wanted_dispatch;
    UPDATE imagery.image_job SET revision=revision+1,updated_at_ms=now_ms WHERE job_id=wanted_job;
    RETURN true;
END $$;
REVOKE ALL ON FUNCTION imagery.mirror_local_outcome(bytea,bytea,bytea,boolean,bigint) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION imagery.mirror_local_outcome(bytea,bytea,bytea,boolean,bigint) TO df_imagery_output;

-- No usability argument or finance authority: derive Local execution projection from exact evidence.
CREATE OR REPLACE FUNCTION imagery.mirror_local_decision_outcome(
    wanted_owner bytea,wanted_job bytea,wanted_stage bytea,wanted_dispatch bytea,
    wanted_usage integer,wanted_payload_sha bytea,wanted_raw_sha bytea,now_ms bigint
) RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    job imagery.image_job%ROWTYPE;
    stage imagery.image_execution_stage%ROWTYPE;
    dispatch imagery.image_dispatch_attempt%ROWTYPE;
    current_dispatch bytea;
    retained record;
    payload jsonb;
    wanted_settlement text;
BEGIN
    IF current_setting('transaction_isolation')<>'read committed' OR now_ms IS NULL OR now_ms<0
        OR wanted_usage IS NULL OR wanted_usage<=0
        OR octet_length(wanted_payload_sha) IS DISTINCT FROM 32
        OR octet_length(wanted_raw_sha) IS DISTINCT FROM 32 THEN
        RAISE EXCEPTION 'local decision identity refused' USING ERRCODE='23514';
    END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended(encode(wanted_dispatch,'hex'),44));
    SELECT * INTO job FROM imagery.image_job WHERE owner_scope=wanted_owner AND job_id=wanted_job FOR UPDATE;
    IF NOT FOUND THEN RAISE EXCEPTION 'local decision job missing' USING ERRCODE='23514'; END IF;
    SELECT * INTO stage FROM imagery.image_execution_stage WHERE owner_scope=wanted_owner
        AND job_id=wanted_job AND stage_id=wanted_stage FOR UPDATE;
    IF NOT FOUND OR NOT stage.local_accounting OR stage.stage_kind NOT IN ('classification','inspection',
        'photo_inspection','head_locations','segmentation','staging_identity','staging_facing') THEN
        RAISE EXCEPTION 'local decision stage refused' USING ERRCODE='23514';
    END IF;
    SELECT * INTO dispatch FROM imagery.image_dispatch_attempt WHERE job_id=wanted_job
        AND stage_id=wanted_stage AND dispatch_id=wanted_dispatch FOR UPDATE;
    IF NOT FOUND OR NOT dispatch.local_accounting OR dispatch.finance_attempt IS NOT NULL
        OR dispatch.finance_operation IS NOT NULL OR dispatch.finance_effect IS NOT NULL
        OR dispatch.dispatch_state<>'succeeded' OR dispatch.usage_revision<>wanted_usage THEN
        RAISE EXCEPTION 'local decision no-finance scope refused' USING ERRCODE='23514';
    END IF;
    SELECT c.current_dispatch INTO current_dispatch FROM imagery.image_stage_cursor c
        WHERE c.job_id=wanted_job AND c.stage_id=wanted_stage FOR UPDATE;
    IF NOT FOUND OR current_dispatch<>wanted_dispatch OR now_ms<job.updated_at_ms OR now_ms<dispatch.updated_at_ms THEN
        RAISE EXCEPTION 'local decision cursor refused' USING ERRCODE='23514';
    END IF;
    SELECT o.decision_kind,o.payload,u.evidence INTO retained
        FROM imagery.image_decision_checkpoint p
        JOIN imagery.image_decision_observation o USING(dispatch_id,observation_version)
        JOIN imagery.image_dispatch_usage u ON u.dispatch_id=o.dispatch_id AND u.revision=o.usage_revision
        WHERE o.owner_scope=wanted_owner AND o.job_id=wanted_job AND o.stage_id=wanted_stage
            AND o.dispatch_id=wanted_dispatch AND o.usage_revision=wanted_usage
            AND p.usage_revision=o.usage_revision AND o.payload_sha256=wanted_payload_sha
            AND u.owner_scope=o.owner_scope AND u.job_id=o.job_id AND u.stage_id=o.stage_id
            AND u.raw_output_sha256=wanted_raw_sha;
    IF NOT FOUND THEN RAISE EXCEPTION 'local decision receipt refused' USING ERRCODE='23514'; END IF;
    IF retained.evidence='unknown' THEN
        wanted_settlement:='unknown_outstanding';
    ELSIF retained.decision_kind IN ('photo_inspection','head_locations','segmentation','staging_identity','staging_facing') THEN
        IF octet_length(retained.payload)<2 OR get_byte(retained.payload,0)<>1
            OR get_byte(retained.payload,1) NOT IN (0,1) THEN
            RAISE EXCEPTION 'local analysis decision header refused' USING ERRCODE='23514';
        END IF;
        wanted_settlement:=CASE get_byte(retained.payload,1) WHEN 1 THEN 'settled' ELSE 'unusable_settled' END;
    ELSE
        BEGIN
            payload:=convert_from(retained.payload,'UTF8')::jsonb;
        EXCEPTION WHEN OTHERS THEN
            RAISE EXCEPTION 'local decision payload refused' USING ERRCODE='23514';
        END;
        IF (jsonb_typeof(payload)='object' AND payload->>'schema_revision'='1') IS NOT TRUE THEN
            RAISE EXCEPTION 'local decision schema refused' USING ERRCODE='23514';
        END IF;
        wanted_settlement:=CASE
            WHEN retained.decision_kind='classification'
                AND payload->>'route' IN ('general','bfl','revise','uncertain') THEN 'settled'
            WHEN retained.decision_kind='inspection' AND payload->>'verdict' IN ('pass','failed') THEN 'settled'
            WHEN retained.decision_kind='inspection' AND payload->>'verdict'='unjudged' THEN 'unusable_settled'
            ELSE NULL END;
    END IF;
    IF wanted_settlement IS NULL THEN RAISE EXCEPTION 'local decision projection refused' USING ERRCODE='23514'; END IF;
    IF dispatch.settlement=wanted_settlement THEN RETURN false; END IF;
    IF dispatch.settlement NOT IN ('held','unknown_outstanding') THEN
        RAISE EXCEPTION 'local decision terminal conflict' USING ERRCODE='23514';
    END IF;
    UPDATE imagery.image_dispatch_attempt SET settlement=wanted_settlement,updated_at_ms=now_ms WHERE dispatch_id=wanted_dispatch;
    UPDATE imagery.image_job SET revision=revision+1,updated_at_ms=now_ms WHERE job_id=wanted_job;
    RETURN true;
END $$;
REVOKE ALL ON FUNCTION imagery.mirror_local_decision_outcome(bytea,bytea,bytea,bytea,integer,bytea,bytea,bigint) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION imagery.mirror_local_decision_outcome(bytea,bytea,bytea,bytea,integer,bytea,bytea,bigint) TO df_imagery_output;

CREATE OR REPLACE FUNCTION imagery.mirror_local_transport_outcome(
    wanted_owner bytea,wanted_job bytea,wanted_stage bytea,wanted_dispatch bytea,now_ms bigint
) RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE
    job imagery.image_job%ROWTYPE;
    stage imagery.image_execution_stage%ROWTYPE;
    dispatch imagery.image_dispatch_attempt%ROWTYPE;
    retained record;
    wanted_settlement text;
BEGIN
    IF current_setting('transaction_isolation')<>'read committed' OR now_ms IS NULL OR now_ms<0 THEN
        RAISE EXCEPTION 'local transport transaction refused' USING ERRCODE='23514';
    END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended(encode(wanted_dispatch,'hex'),44));
    SELECT * INTO job FROM imagery.image_job WHERE owner_scope=wanted_owner AND job_id=wanted_job FOR UPDATE;
    IF NOT FOUND THEN RAISE EXCEPTION 'local transport job missing' USING ERRCODE='23514'; END IF;
    SELECT * INTO stage FROM imagery.image_execution_stage WHERE owner_scope=wanted_owner AND job_id=wanted_job AND stage_id=wanted_stage FOR UPDATE;
    IF NOT FOUND OR NOT stage.local_accounting THEN RAISE EXCEPTION 'local transport stage refused' USING ERRCODE='23514'; END IF;
    SELECT * INTO dispatch FROM imagery.image_dispatch_attempt WHERE job_id=wanted_job AND stage_id=wanted_stage AND dispatch_id=wanted_dispatch FOR UPDATE;
    IF NOT FOUND OR NOT dispatch.local_accounting OR dispatch.finance_attempt IS NOT NULL
        OR dispatch.finance_operation IS NOT NULL OR dispatch.finance_effect IS NOT NULL
        OR dispatch.dispatch_state NOT IN ('failed','unknown','declined') OR now_ms<job.updated_at_ms OR now_ms<dispatch.updated_at_ms THEN
        RAISE EXCEPTION 'local transport no-finance scope refused' USING ERRCODE='23514';
    END IF;
    IF NOT EXISTS(SELECT 1 FROM imagery.image_stage_cursor WHERE job_id=wanted_job AND stage_id=wanted_stage AND current_dispatch=wanted_dispatch FOR UPDATE) THEN
        RAISE EXCEPTION 'local transport cursor refused' USING ERRCODE='23514';
    END IF;
    IF dispatch.dispatch_state='declined' THEN
        IF dispatch.supplier_job_id IS NOT NULL
            OR EXISTS(SELECT 1 FROM imagery.image_supplier_checkpoint WHERE dispatch_id=wanted_dispatch)
            OR NOT EXISTS(SELECT 1 FROM imagery.image_decline_checkpoint c
                JOIN imagery.image_decline_observation o USING(dispatch_id,observation_version)
                JOIN imagery.image_dispatch_usage u ON u.owner_scope=o.owner_scope AND u.job_id=o.job_id AND u.stage_id=o.stage_id AND u.dispatch_id=o.dispatch_id AND u.revision=o.usage_revision
                WHERE c.dispatch_id=wanted_dispatch AND c.usage_revision=dispatch.usage_revision AND o.usage_revision=dispatch.usage_revision
                    AND o.owner_scope=wanted_owner AND o.job_id=wanted_job AND o.stage_id=wanted_stage
                    AND u.evidence='unknown' AND NOT u.usable AND u.units=u.maxima
                    AND u.raw_output_sha256=o.raw_sha256 AND octet_length(u.raw_output)::bigint=o.raw_bytes
                    AND u.provider=o.provider AND u.model=o.model AND u.cause=o.cause AND u.detail=o.detail
                    AND u.observed_at_ms=o.observed_at_ms AND o.config_snapshot_revision=job.config_revision) THEN
            RAISE EXCEPTION 'local transport decline proof refused' USING ERRCODE='23514';
        END IF;
        wanted_settlement:='unknown_outstanding';
    ELSIF dispatch.dispatch_state='unknown' THEN
        SELECT evidence,usable,raw_output_sha256 INTO retained FROM imagery.image_dispatch_usage
            WHERE owner_scope=wanted_owner AND job_id=wanted_job AND stage_id=wanted_stage
                AND dispatch_id=wanted_dispatch ORDER BY revision DESC LIMIT 1;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'local transport unknown evidence refused' USING ERRCODE='23514';
        END IF;
        IF retained.evidence<>'unknown' THEN
            IF dispatch.settlement NOT IN ('held','unknown_outstanding') THEN
                RAISE EXCEPTION 'local transport pending conflict' USING ERRCODE='23514';
            END IF;
            RETURN false;
        END IF;
        wanted_settlement:='unknown_outstanding';
    ELSE
        SELECT evidence,usable,raw_output_sha256 INTO retained FROM imagery.image_dispatch_usage
            WHERE owner_scope=wanted_owner AND job_id=wanted_job AND stage_id=wanted_stage
                AND dispatch_id=wanted_dispatch AND revision=dispatch.usage_revision;
        IF NOT FOUND THEN RAISE EXCEPTION 'local transport raw revision refused' USING ERRCODE='23514'; END IF;
        IF retained.evidence<>'unknown' AND NOT retained.usable AND EXISTS(
        SELECT 1 FROM imagery.image_no_delivery s WHERE s.owner_scope=wanted_owner AND s.job_id=wanted_job
            AND s.reason='supplier_no_output' AND s.actor_kind='dispatch' AND s.tail_kind='dispatch'
            AND s.actor_stage_id=wanted_stage AND s.tail_stage_id=wanted_stage
            AND s.actor_dispatch_id=wanted_dispatch AND s.tail_dispatch_id=wanted_dispatch
            AND s.actor_usage_revision=dispatch.usage_revision AND s.tail_usage_revision=dispatch.usage_revision
            AND s.no_output_raw_sha256 IS NOT DISTINCT FROM retained.raw_output_sha256
        ) THEN wanted_settlement:='unusable_settled';
        ELSE RAISE EXCEPTION 'local transport no-output proof refused' USING ERRCODE='23514'; END IF;
    END IF;
    IF dispatch.settlement=wanted_settlement THEN RETURN false; END IF;
    IF dispatch.settlement NOT IN ('held','unknown_outstanding') THEN
        RAISE EXCEPTION 'local transport terminal conflict' USING ERRCODE='23514';
    END IF;
    UPDATE imagery.image_dispatch_attempt SET settlement=wanted_settlement,updated_at_ms=now_ms WHERE dispatch_id=wanted_dispatch;
    UPDATE imagery.image_job SET revision=revision+1,updated_at_ms=now_ms WHERE job_id=wanted_job;
    RETURN true;
END $$;
REVOKE ALL ON FUNCTION imagery.mirror_local_transport_outcome(bytea,bytea,bytea,bytea,bigint) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION imagery.mirror_local_transport_outcome(bytea,bytea,bytea,bytea,bigint) TO df_imagery_output;

CREATE OR REPLACE FUNCTION imagery.mirror_local_analysis_failure(
    wanted_owner bytea,wanted_job bytea,wanted_stage bytea,wanted_dispatch bytea,
    wanted_usage integer,wanted_raw_sha bytea,now_ms bigint
) RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE j imagery.image_job%ROWTYPE;s imagery.image_execution_stage%ROWTYPE;d imagery.image_dispatch_attempt%ROWTYPE;
BEGIN
    IF current_setting('transaction_isolation')<>'read committed' OR now_ms IS NULL OR now_ms<0
        OR wanted_usage IS NULL OR wanted_usage<=0
        OR (wanted_raw_sha IS NOT NULL AND octet_length(wanted_raw_sha)<>32) THEN
        RAISE EXCEPTION 'local analysis failure identity refused' USING ERRCODE='23514';
    END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended(encode(wanted_dispatch,'hex'),44));
    SELECT * INTO j FROM imagery.image_job WHERE owner_scope=wanted_owner AND job_id=wanted_job FOR UPDATE;
    IF NOT FOUND OR j.planned_publication_asset IS NOT NULL OR j.planned_publication_attempt IS NOT NULL THEN
        RAISE EXCEPTION 'local analysis media allocation refused' USING ERRCODE='23514';
    END IF;
    SELECT * INTO s FROM imagery.image_execution_stage WHERE owner_scope=wanted_owner AND job_id=wanted_job AND stage_id=wanted_stage FOR UPDATE;
    IF NOT FOUND OR NOT s.local_accounting OR s.stage_kind NOT IN ('photo_inspection','head_locations','segmentation','staging_identity','staging_facing') THEN
        RAISE EXCEPTION 'local analysis failure stage refused' USING ERRCODE='23514';
    END IF;
    SELECT * INTO d FROM imagery.image_dispatch_attempt WHERE job_id=wanted_job AND stage_id=wanted_stage AND dispatch_id=wanted_dispatch FOR UPDATE;
    IF NOT FOUND OR NOT d.local_accounting OR d.finance_attempt IS NOT NULL OR d.finance_operation IS NOT NULL OR d.finance_effect IS NOT NULL
        OR d.dispatch_state<>'failed' OR d.usage_revision<>wanted_usage OR now_ms<j.updated_at_ms OR now_ms<d.updated_at_ms THEN
        RAISE EXCEPTION 'local analysis failure no-finance scope refused' USING ERRCODE='23514';
    END IF;
    IF NOT EXISTS(SELECT 1 FROM imagery.image_stage_cursor WHERE job_id=wanted_job AND stage_id=wanted_stage AND current_dispatch=wanted_dispatch FOR UPDATE)
        OR NOT EXISTS(SELECT 1 FROM imagery.image_dispatch_usage u WHERE u.owner_scope=wanted_owner AND u.job_id=wanted_job
            AND u.stage_id=wanted_stage AND u.dispatch_id=wanted_dispatch AND u.revision=wanted_usage AND u.evidence<>'unknown' AND NOT u.usable
            AND u.raw_output_sha256 IS NOT DISTINCT FROM wanted_raw_sha
            AND NOT EXISTS(SELECT 1 FROM imagery.image_dispatch_usage later WHERE later.dispatch_id=u.dispatch_id AND later.revision>u.revision)) THEN
        RAISE EXCEPTION 'local analysis failure raw proof refused' USING ERRCODE='23514';
    END IF;
    IF d.settlement='unusable_settled' THEN RETURN false;END IF;
    IF d.settlement NOT IN ('held','unknown_outstanding') THEN RAISE EXCEPTION 'local analysis failure terminal conflict' USING ERRCODE='23514';END IF;
    UPDATE imagery.image_dispatch_attempt SET settlement='unusable_settled',updated_at_ms=now_ms WHERE dispatch_id=wanted_dispatch;
    UPDATE imagery.image_job SET revision=revision+1,updated_at_ms=now_ms WHERE job_id=wanted_job;
    RETURN true;
END $$;
REVOKE ALL ON FUNCTION imagery.mirror_local_analysis_failure(bytea,bytea,bytea,bytea,integer,bytea,bigint) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION imagery.mirror_local_analysis_failure(bytea,bytea,bytea,bytea,integer,bytea,bigint) TO df_imagery_output;

-- Candidate delivery is derived from immutable output evidence, never a caller usability value.
CREATE OR REPLACE FUNCTION imagery.mirror_local_candidate_outcome(
    wanted_owner bytea,wanted_job bytea,wanted_stage bytea,wanted_dispatch bytea,
    wanted_usage integer,now_ms bigint
) RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE
    j imagery.image_job%ROWTYPE;s imagery.image_execution_stage%ROWTYPE;d imagery.image_dispatch_attempt%ROWTYPE;
    raw_evidence text;roots bytea[];is_undelivered boolean;is_private boolean;private_published_at_ms bigint;
    contains_candidate boolean;contains_correction boolean;has_candidate boolean;wanted_settlement text;
BEGIN
    IF current_setting('transaction_isolation')<>'read committed' OR now_ms IS NULL OR now_ms<0
        OR wanted_usage IS NULL OR wanted_usage<=0 THEN
        RAISE EXCEPTION 'local candidate transaction refused' USING ERRCODE='23514';
    END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended(encode(wanted_dispatch,'hex'),44));
    SELECT * INTO j FROM imagery.image_job WHERE owner_scope=wanted_owner AND job_id=wanted_job FOR UPDATE;
    IF NOT FOUND THEN RAISE EXCEPTION 'local candidate job missing' USING ERRCODE='23514';END IF;
    SELECT * INTO s FROM imagery.image_execution_stage WHERE owner_scope=wanted_owner AND job_id=wanted_job AND stage_id=wanted_stage FOR UPDATE;
    IF NOT FOUND OR NOT s.local_accounting OR s.stage_kind NOT IN ('generation','edit','correction','extraction') THEN
        RAISE EXCEPTION 'local candidate stage refused' USING ERRCODE='23514';END IF;
    SELECT * INTO d FROM imagery.image_dispatch_attempt WHERE job_id=wanted_job AND stage_id=wanted_stage AND dispatch_id=wanted_dispatch FOR UPDATE;
    IF NOT FOUND OR NOT d.local_accounting OR d.finance_attempt IS NOT NULL OR d.finance_operation IS NOT NULL OR d.finance_effect IS NOT NULL
        OR d.dispatch_state<>'succeeded' OR d.usage_revision<>wanted_usage OR now_ms<j.updated_at_ms OR now_ms<d.updated_at_ms THEN
        RAISE EXCEPTION 'local candidate no-finance scope refused' USING ERRCODE='23514';END IF;
    IF NOT EXISTS(SELECT 1 FROM imagery.image_stage_cursor WHERE job_id=wanted_job AND stage_id=wanted_stage AND current_dispatch=wanted_dispatch FOR UPDATE) THEN
        RAISE EXCEPTION 'local candidate current dispatch refused' USING ERRCODE='23514';END IF;
    SELECT EXISTS(SELECT 1 FROM imagery.image_candidate_checkpoint c JOIN imagery.image_candidate_observation o USING(dispatch_id,observation_version)
            WHERE c.dispatch_id=wanted_dispatch AND c.usage_revision=wanted_usage AND o.usage_revision=wanted_usage
                AND o.owner_scope=wanted_owner AND o.job_id=wanted_job AND o.stage_id=wanted_stage AND o.candidate_kind=s.stage_kind
                AND o.input_revision=s.input_revision AND o.input_digest=s.input_digest AND o.config_revision=s.config_revision
                AND o.config_digest=s.config_digest AND o.parent_dispatch IS NOT DISTINCT FROM s.parent_dispatch) INTO has_candidate;
    IF NOT has_candidate AND EXISTS(SELECT 1 FROM imagery.image_candidate_checkpoint WHERE dispatch_id=wanted_dispatch) THEN
        RAISE EXCEPTION 'local candidate receipt conflict' USING ERRCODE='23514';END IF;
    SELECT evidence INTO raw_evidence FROM imagery.image_dispatch_usage WHERE owner_scope=wanted_owner AND job_id=wanted_job
        AND stage_id=wanted_stage AND dispatch_id=wanted_dispatch AND revision=wanted_usage;
    IF NOT FOUND THEN RAISE EXCEPTION 'local candidate raw scope refused' USING ERRCODE='23514';END IF;
    SELECT ARRAY[n.actor_dispatch_id,n.tail_dispatch_id,n.candidate_dispatch_id] INTO roots FROM imagery.image_no_delivery n
        WHERE n.owner_scope=wanted_owner AND n.job_id=wanted_job;
    is_undelivered:=FOUND;
    IF is_undelivered THEN
        IF NOT has_candidate AND NOT EXISTS(
            SELECT 1 FROM imagery.image_no_delivery n
            JOIN imagery.image_dispatch_checkpoint p ON p.dispatch_id=n.actor_dispatch_id
                AND p.usage_revision=n.actor_usage_revision AND p.book_call_id=d.book_call_id
            WHERE n.owner_scope=wanted_owner AND n.job_id=wanted_job AND n.reason='source_denied'
                AND n.actor_kind='dispatch' AND n.tail_kind='dispatch'
                AND n.actor_stage_id=wanted_stage AND n.actor_dispatch_id=wanted_dispatch
                AND n.tail_stage_id=n.actor_stage_id AND n.tail_dispatch_id=n.actor_dispatch_id
                AND n.candidate_dispatch_id IS NULL AND n.actor_usage_revision=wanted_usage
                AND n.legacy_output_write_id IS NOT NULL AND p.object_version=n.legacy_output_version
                AND p.sha256=n.legacy_output_sha256 AND p.recipe_digest=n.legacy_output_recipe_digest
                AND NOT EXISTS(SELECT 1 FROM imagery.image_final_selection f WHERE f.job_id=n.job_id)
        ) THEN RAISE EXCEPTION 'local candidate sealed receipt refused' USING ERRCODE='23514';END IF;
        IF EXISTS(SELECT 1 FROM imagery.image_job_result WHERE owner_scope=wanted_owner AND job_id=wanted_job) THEN
            RAISE EXCEPTION 'local candidate conflicting output refused' USING ERRCODE='23514';END IF;
        is_private:=false;
    ELSIF has_candidate THEN
        SELECT ARRAY[f.candidate_dispatch_id],r.private_reconciliation,r.published_at_ms INTO roots,is_private,private_published_at_ms
            FROM imagery.image_job_result r JOIN imagery.image_final_selection f ON f.owner_scope=r.owner_scope AND f.job_id=r.job_id
                AND f.final_stage_id=r.stage_id AND f.final_dispatch_id=r.dispatch_id AND f.allocation_stage_id=r.allocation_stage_id
                AND f.asset_id=r.asset_id AND f.publication_attempt=r.publication_attempt AND f.resolved_recipe_digest=r.recipe_digest
            JOIN media.asset a ON a.asset_id=r.asset_id AND a.attempt=r.publication_attempt
            WHERE r.owner_scope=wanted_owner AND r.job_id=wanted_job AND a.state='ready' AND a.kind='image' AND a.provenance='generated';
        IF NOT FOUND OR (is_private AND j.state<>'retired' AND j.absolute_deadline_ms>private_published_at_ms) THEN
            RAISE EXCEPTION 'local candidate output evidence refused' USING ERRCODE='23514';END IF;
    ELSE
        SELECT ARRAY[r.dispatch_id],r.private_reconciliation,r.published_at_ms INTO roots,is_private,private_published_at_ms
            FROM imagery.image_job_result r JOIN media.asset a ON a.asset_id=r.asset_id AND a.attempt=r.publication_attempt
            JOIN imagery.image_dispatch_checkpoint p ON p.dispatch_id=r.dispatch_id AND p.usage_revision=wanted_usage AND p.recipe_digest=r.recipe_digest
            WHERE r.owner_scope=wanted_owner AND r.job_id=wanted_job AND r.stage_id=wanted_stage AND r.dispatch_id=wanted_dispatch
                AND a.state='ready' AND a.kind='image' AND a.provenance='generated'
                AND NOT EXISTS(SELECT 1 FROM imagery.image_final_selection f WHERE f.job_id=r.job_id);
        IF NOT FOUND OR (is_private AND j.state<>'retired' AND j.absolute_deadline_ms>private_published_at_ms) THEN
            RAISE EXCEPTION 'local legacy output evidence refused' USING ERRCODE='23514';END IF;
    END IF;
    WITH RECURSIVE lineage AS (
        SELECT p.dispatch_id,p.stage_id,e.parent_dispatch,e.stage_kind,1 AS depth,ARRAY[p.dispatch_id] AS seen
            FROM imagery.image_dispatch_attempt p JOIN imagery.image_execution_stage e USING(job_id,stage_id)
            WHERE p.job_id=wanted_job AND e.owner_scope=wanted_owner AND p.dispatch_id=ANY(roots)
        UNION ALL
        SELECT p.dispatch_id,p.stage_id,e.parent_dispatch,e.stage_kind,l.depth+1,l.seen||p.dispatch_id
            FROM lineage l JOIN imagery.image_dispatch_attempt p ON p.dispatch_id=l.parent_dispatch AND p.job_id=wanted_job
            JOIN imagery.image_execution_stage e ON e.job_id=p.job_id AND e.stage_id=p.stage_id
            WHERE e.owner_scope=wanted_owner AND l.depth<7 AND NOT p.dispatch_id=ANY(l.seen)
    ) SELECT coalesce(bool_or(dispatch_id=wanted_dispatch AND stage_id=wanted_stage),false),
        coalesce(bool_or(stage_kind='correction'),false) INTO contains_candidate,contains_correction FROM lineage;
    IF NOT contains_candidate THEN RAISE EXCEPTION 'local candidate output lineage refused' USING ERRCODE='23514';END IF;
    wanted_settlement:=CASE WHEN raw_evidence='unknown' THEN 'unknown_outstanding'
        WHEN is_undelivered OR is_private OR (s.stage_kind IN ('generation','edit') AND contains_correction) THEN 'unusable_settled'
        ELSE 'settled' END;
    IF d.settlement=wanted_settlement THEN RETURN false;END IF;
    IF d.settlement NOT IN ('held','unknown_outstanding') THEN RAISE EXCEPTION 'local candidate terminal conflict' USING ERRCODE='23514';END IF;
    UPDATE imagery.image_dispatch_attempt SET settlement=wanted_settlement,updated_at_ms=now_ms WHERE dispatch_id=wanted_dispatch;
    UPDATE imagery.image_job SET revision=revision+1,updated_at_ms=now_ms WHERE job_id=wanted_job;
    RETURN true;
END $$;
REVOKE ALL ON FUNCTION imagery.mirror_local_candidate_outcome(bytea,bytea,bytea,bytea,integer,bigint) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION imagery.mirror_local_candidate_outcome(bytea,bytea,bytea,bytea,integer,bigint) TO df_imagery_output;
