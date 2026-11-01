ALTER TABLE imagery.image_dispatch_usage ADD COLUMN IF NOT EXISTS raw_output_sha256 bytea GENERATED ALWAYS AS (sha256(raw_output)) STORED;
ALTER TABLE imagery.image_dispatch_attempt ADD COLUMN IF NOT EXISTS cost_unsent_ack boolean NOT NULL DEFAULT false;
-- Submit rejection is a technical observation; every old financial liability remains outstanding.
ALTER TABLE imagery.image_dispatch_attempt DROP CONSTRAINT IF EXISTS image_dispatch_attempt_dispatch_state_check;
ALTER TABLE imagery.image_dispatch_attempt ADD CONSTRAINT image_dispatch_attempt_dispatch_state_check
 CHECK(dispatch_state IN ('prepared','sent','succeeded','failed','unknown','released','declined'));
ALTER TABLE imagery.image_execution_stage ADD COLUMN IF NOT EXISTS retry_dispatch_id bytea
 REFERENCES imagery.image_dispatch_attempt(dispatch_id) DEFERRABLE INITIALLY DEFERRED;
CREATE TABLE IF NOT EXISTS imagery.image_decline_observation (
 dispatch_id bytea NOT NULL REFERENCES imagery.image_dispatch_attempt(dispatch_id),
 observation_version bigint NOT NULL CHECK(observation_version>0),
 owner_scope bytea NOT NULL,job_id bytea NOT NULL,stage_id bytea NOT NULL,
 usage_revision integer NOT NULL CHECK(usage_revision>0),
 raw_sha256 bytea NOT NULL CHECK(octet_length(raw_sha256)=32),
 raw_bytes bigint NOT NULL CHECK(raw_bytes>0),
 provider text NOT NULL,model text NOT NULL,cause text NOT NULL,detail text NOT NULL,
 metadata text NOT NULL CHECK(octet_length(metadata)<=1048576),
 retry_not_before_ms bigint NOT NULL CHECK(retry_not_before_ms>0),
 max_attempts bigint NOT NULL CHECK(max_attempts>0),
 retry_deadline_ms bigint NOT NULL CHECK(retry_deadline_ms>0),
 config_snapshot_revision bigint NOT NULL CHECK(config_snapshot_revision>0),
 observed_at_ms bigint NOT NULL CHECK(observed_at_ms>0),
 PRIMARY KEY(dispatch_id,observation_version),
 FOREIGN KEY(job_id,stage_id,dispatch_id) REFERENCES imagery.image_dispatch_attempt(job_id,stage_id,dispatch_id),
 FOREIGN KEY(job_id,owner_scope) REFERENCES imagery.image_job(job_id,owner_scope)
);
CREATE TABLE IF NOT EXISTS imagery.image_decline_checkpoint (
 dispatch_id bytea PRIMARY KEY,observation_version bigint NOT NULL,usage_revision integer NOT NULL,
 FOREIGN KEY(dispatch_id,observation_version) REFERENCES imagery.image_decline_observation(dispatch_id,observation_version)
);
CREATE OR REPLACE FUNCTION imagery.forbid_decline_observation_mutation() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog AS $$ BEGIN RAISE EXCEPTION 'immutable decline observation' USING ERRCODE='23514'; END $$;
DROP TRIGGER IF EXISTS image_decline_observation_immutable ON imagery.image_decline_observation;
CREATE TRIGGER image_decline_observation_immutable BEFORE UPDATE OR DELETE ON imagery.image_decline_observation FOR EACH ROW EXECUTE FUNCTION imagery.forbid_decline_observation_mutation();
DROP TRIGGER IF EXISTS image_decline_checkpoint_immutable ON imagery.image_decline_checkpoint;
CREATE TRIGGER image_decline_checkpoint_immutable BEFORE UPDATE OR DELETE ON imagery.image_decline_checkpoint FOR EACH ROW EXECUTE FUNCTION imagery.forbid_decline_observation_mutation();
DO $$ BEGIN
 IF EXISTS(SELECT 1 FROM pg_roles WHERE rolname='df_imagery_publication') THEN
  GRANT SELECT ON imagery.image_decline_observation,imagery.image_decline_checkpoint TO df_imagery_publication;
 END IF;
 IF EXISTS(SELECT 1 FROM pg_roles WHERE rolname='df_imagery_admission') THEN
  GRANT SELECT ON imagery.image_decline_observation,imagery.image_decline_checkpoint TO df_imagery_admission;
 END IF;
END $$;

-- Preserve exact seal authority while closing a cancelled queued retry; old dispatch money is untouched.
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
        IF EXISTS(SELECT 1 FROM imagery.image_execution_stage q WHERE q.job_id=wanted_job AND q.stage_id=wanted_tail_stage AND q.retry_dispatch_id=wanted_tail_dispatch AND ((q.stage_state='planned' AND NOT q.capacity_reserved) OR(q.stage_state='admitted' AND q.capacity_reserved))) THEN
            SELECT * INTO s FROM imagery.image_execution_stage WHERE job_id=wanted_job AND stage_id=wanted_tail_stage FOR UPDATE;
            IF s.capacity_reserved THEN
                UPDATE imagery.image_execution_capacity SET in_flight=in_flight-1,updated_at_ms=wanted_now_ms WHERE capacity_scope=s.capacity_scope AND in_flight>0;
            ELSE
                UPDATE imagery.image_execution_capacity SET pending_jobs=pending_jobs-1,pending_bytes=pending_bytes-octet_length(j.intent_bytes)-octet_length(j.config_bytes),updated_at_ms=wanted_now_ms
                    WHERE capacity_scope=s.capacity_scope AND pending_jobs>0 AND pending_bytes>=octet_length(j.intent_bytes)+octet_length(j.config_bytes);
            END IF;
            GET DIAGNOSTICS changed=ROW_COUNT;
            IF changed<>1 THEN RAISE EXCEPTION 'terminal declined retry reserve missing' USING ERRCODE='23514';END IF;
            UPDATE imagery.image_execution_stage SET stage_state='released',capacity_reserved=false,retry_dispatch_id=NULL,lease_until_ms=0,queued_notice=NULL,claim_generation=claim_generation+1,updated_at_ms=wanted_now_ms WHERE job_id=wanted_job AND stage_id=wanted_tail_stage;
        END IF;
    ELSE RETURN QUERY SELECT 'invalid'::text,j.revision;RETURN;END IF;
    UPDATE imagery.image_job SET state='retired',revision=seal.sealed_job_revision,updated_at_ms=wanted_now_ms WHERE job_id=wanted_job;
    RETURN QUERY SELECT 'closed'::text,seal.sealed_job_revision::bigint;
END $$;
REVOKE ALL ON FUNCTION imagery.close_terminal_output(bytea,bytea,text,bytea,bytea,text,bytea,bytea,bytea,bigint,bigint) FROM PUBLIC;
