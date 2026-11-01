ALTER TABLE imagery.image_dispatch_attempt ADD COLUMN IF NOT EXISTS cost_unsent_ack boolean NOT NULL DEFAULT false;
-- Execution outcome/permission upgrade for an already installed execution journal.

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

DO $$ BEGIN
    IF EXISTS(SELECT 1 FROM pg_roles WHERE rolname='df_imagery_output') THEN
        GRANT EXECUTE ON FUNCTION imagery.stage_has_finance(bytea,bytea,bytea) TO df_imagery_output;
    END IF;
    IF EXISTS(SELECT 1 FROM pg_roles WHERE rolname='df_imagery_publication') THEN
        GRANT EXECUTE ON FUNCTION imagery.stage_has_finance(bytea,bytea,bytea) TO df_imagery_publication;
    END IF;
    IF EXISTS(SELECT 1 FROM pg_roles WHERE rolname='df_imagery_admission') THEN
        GRANT EXECUTE ON FUNCTION imagery.stage_has_finance(bytea,bytea,bytea) TO df_imagery_admission;
    END IF;
END $$;

ALTER TABLE imagery.image_dispatch_attempt DROP CONSTRAINT IF EXISTS image_dispatch_attempt_failure_cause_check;
ALTER TABLE imagery.image_dispatch_attempt ADD CONSTRAINT image_dispatch_attempt_failure_cause_check
    CHECK(failure_cause IS NULL OR failure_cause IN ('refused','invalid_output','transport_lost',
        'rate_limited','quota_exhausted','deadline_exceeded','provider'));

ALTER TABLE imagery.image_execution_stage ADD COLUMN IF NOT EXISTS queued_notice text
    CHECK(queued_notice IS NULL OR queued_notice IN ('cooldown','probing'));

-- Upgrade finite NoDelivery actor checks on a database whose42 was installed before these states.
DO $terminal_kinds$ DECLARE item record; BEGIN
    IF to_regclass('imagery.image_no_delivery') IS NOT NULL THEN
        FOR item IN SELECT conname FROM pg_constraint
            WHERE conrelid='imagery.image_no_delivery'::regclass AND contype='c'
                AND (pg_get_constraintdef(oid) LIKE '%actor_kind%' OR pg_get_constraintdef(oid) LIKE '%tail_kind%')
        LOOP
            EXECUTE format('ALTER TABLE imagery.image_no_delivery DROP CONSTRAINT %I',item.conname);
        END LOOP;
        ALTER TABLE imagery.image_no_delivery ADD CONSTRAINT image_no_delivery_actor_kind_terminal_check
            CHECK(actor_kind IN ('dispatch','unsent_stage','unclaimed_stage','deferred_stage','released_stage'));
        ALTER TABLE imagery.image_no_delivery ADD CONSTRAINT image_no_delivery_tail_kind_terminal_check
            CHECK(tail_kind IN ('dispatch','unsent_stage','unclaimed_stage','deferred_stage','released_stage'));
        ALTER TABLE imagery.image_no_delivery ADD CONSTRAINT image_no_delivery_actor_dispatch_terminal_check
            CHECK((actor_kind='dispatch' AND actor_dispatch_id IS NOT NULL AND actor_usage_revision IS NOT NULL)
                OR(actor_kind IN ('unsent_stage','unclaimed_stage','deferred_stage','released_stage') AND actor_dispatch_id IS NULL AND actor_usage_revision IS NULL));
        ALTER TABLE imagery.image_no_delivery ADD CONSTRAINT image_no_delivery_tail_dispatch_terminal_check
            CHECK((tail_kind='dispatch' AND tail_dispatch_id IS NOT NULL AND tail_usage_revision IS NOT NULL)
                OR(tail_kind IN ('unsent_stage','unclaimed_stage','deferred_stage','released_stage') AND tail_dispatch_id IS NULL AND tail_usage_revision IS NULL));
    END IF;
END $terminal_kinds$;

ALTER TABLE imagery.image_dispatch_attempt ADD COLUMN IF NOT EXISTS claim_lease_until_ms bigint NOT NULL DEFAULT 0
    CHECK(claim_lease_until_ms>=0);

-- Refresh execution-only canonical guard on already applied40; no financial equation is copied.
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

-- Refresh execution-only canonical guard on already applied40; no financial equation is copied.
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

-- Refresh execution-only canonical guard on already applied40; no financial equation is copied.
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

-- CPU-only local transforms have no supplier cost call and require no invented Book tombstone.
DO $$ BEGIN
 IF to_regclass('imagery.image_no_delivery') IS NOT NULL THEN
  UPDATE imagery.image_dispatch_attempt d SET cost_unsent_ack=true FROM imagery.image_execution_stage s,imagery.image_no_delivery z
   WHERE d.job_id=s.job_id AND d.stage_id=s.stage_id AND s.stage_kind='local_transform'
    AND d.local_accounting AND d.finance_attempt IS NULL AND d.dispatch_state='released' AND d.settlement='released'
    AND d.usage_revision=0 AND z.owner_scope=s.owner_scope AND z.job_id=d.job_id AND z.tail_dispatch_id=d.dispatch_id
    AND NOT EXISTS(SELECT 1 FROM imagery.image_dispatch_usage u WHERE u.dispatch_id=d.dispatch_id);
 END IF;
END $$;
-- Restore the scoped predicate boundary on databases that installed an earlier broad analysis grant.
DO $$ BEGIN
 IF to_regclass('imagery.image_finance_admission_binding') IS NOT NULL THEN
  IF EXISTS(SELECT 1 FROM pg_roles WHERE rolname='df_imagery_output') THEN
   REVOKE SELECT ON imagery.image_finance_admission_binding FROM df_imagery_output;
  END IF;
  IF EXISTS(SELECT 1 FROM pg_roles WHERE rolname='df_imagery_publication') THEN
   REVOKE SELECT ON imagery.image_finance_admission_binding FROM df_imagery_publication;
  END IF;
 END IF;
END $$;
