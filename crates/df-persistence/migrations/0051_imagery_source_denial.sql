-- Source denial closes media delivery only; supplier and financial outcomes remain independent.
ALTER TABLE imagery.image_no_delivery ADD COLUMN IF NOT EXISTS legacy_output_write_id bytea,
    ADD COLUMN IF NOT EXISTS legacy_output_version text,
    ADD COLUMN IF NOT EXISTS legacy_output_sha256 bytea,
    ADD COLUMN IF NOT EXISTS legacy_output_recipe_digest bytea;
ALTER TABLE imagery.image_no_delivery DROP CONSTRAINT IF EXISTS image_no_delivery_legacy_output;
ALTER TABLE imagery.image_no_delivery ADD CONSTRAINT image_no_delivery_legacy_output CHECK(
    (legacy_output_write_id IS NULL AND legacy_output_version IS NULL AND legacy_output_sha256 IS NULL AND legacy_output_recipe_digest IS NULL)
    OR (reason='source_denied' AND candidate_dispatch_id IS NULL AND actor_kind='dispatch' AND tail_kind='dispatch'
        AND legacy_output_write_id IS NOT NULL AND octet_length(legacy_output_write_id)=16
        AND legacy_output_version IS NOT NULL AND octet_length(legacy_output_version) BETWEEN 1 AND 1024
        AND legacy_output_sha256 IS NOT NULL AND octet_length(legacy_output_sha256)=32
        AND legacy_output_recipe_digest IS NOT NULL AND octet_length(legacy_output_recipe_digest)=32));
ALTER TABLE media.asset DROP CONSTRAINT IF EXISTS asset_reject_reason_check;
ALTER TABLE media.asset ADD CONSTRAINT asset_reject_reason_check CHECK(reject_reason IN (
    'validation_failed','screen_rejected','abandoned','not_dispatched','supplier_failed',
    'unknown_outcome_failed','output_lost','over_maximum_bytes','review_expired',
    'screen_unavailable','staged_expired','staged_lost','quota_exceeded','demand_ended','source_denied','preflight_rejected','supplier_declined'));
ALTER TABLE imagery.image_no_delivery DROP CONSTRAINT IF EXISTS image_no_delivery_reason_check;
ALTER TABLE imagery.image_no_delivery ADD CONSTRAINT image_no_delivery_reason_check CHECK(reason IN (
    'inspection_rejected','required_inspection_unjudged','creator_needs_revision','creator_uncertain',
    'retired','expired','required_inspection_preparation','required_inspection_disabled',
    'supplier_no_output','source_denied','missing_credential','unsupported_profile','supplier_declined'));

-- Refresh narrow Local-only projections for installations that already applied 0042.
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
