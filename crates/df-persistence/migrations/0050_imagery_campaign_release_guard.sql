-- Canonical expanded image stages must use their protected stage protocol ports.
-- This predicate reveals only binding existence; ordinary finance gets no private imagery table access.
CREATE FUNCTION finance.image_stage_protocol_required(wanted_attempt bytea) RETURNS boolean
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $guard$
DECLARE binding_table regclass;
BEGIN
 binding_table:=to_regclass('imagery.image_finance_admission_binding');
 IF binding_table IS NULL OR wanted_attempt IS NULL THEN RETURN false; END IF;
 IF (SELECT count(*) FROM pg_catalog.pg_attribute WHERE attrelid=binding_table AND NOT attisdropped
      AND attname IN ('execution_stage_id','dispatch_id','stage_call'))<>3 THEN RETURN false; END IF;
 RETURN EXISTS(SELECT 1 FROM imagery.image_finance_admission_binding b
  WHERE b.finance_attempt=wanted_attempt AND b.execution_stage_id IS NOT NULL
   AND b.dispatch_id IS NOT NULL AND jsonb_typeof(b.stage_call)='object');
END $guard$;
REVOKE ALL ON FUNCTION finance.image_stage_protocol_required(bytea) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.image_stage_protocol_required(bytea) TO df_finance;

-- Resolve the complete existing held-flow dependency closure without taking row locks.
CREATE FUNCTION finance.image_stage_plan_targets(wanted_attempts bytea[],wanted_reservations bytea[],wanted_usages bytea[])
RETURNS jsonb LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $targets$
 WITH targets AS (
  SELECT a.attempt_id,a.reservation_id,a.account_id FROM finance.provider_attempt a
  WHERE a.attempt_id=ANY(wanted_attempts) OR a.reservation_id=ANY(wanted_reservations)
   OR EXISTS(SELECT 1 FROM finance.usage_event u WHERE u.attempt_id=a.attempt_id AND u.usage_event_id=ANY(wanted_usages))
 ), reservations AS (
  SELECT r.reservation_id,r.session_id FROM finance.reservation r JOIN targets t USING(reservation_id)
 ), shares AS (
  SELECT s.reservation_id,s.allocation_id,s.lot_id FROM finance.reservation_share s JOIN targets t USING(reservation_id)
 ), payers AS (
  SELECT t.account_id AS payer FROM targets t WHERE t.account_id IS NOT NULL
  UNION SELECT a.payer_account_id FROM finance.allocation a JOIN shares s USING(allocation_id)
 )
 SELECT jsonb_build_object(
  'attempts',ARRAY(SELECT encode(attempt_id,'hex') FROM targets ORDER BY attempt_id),
  'reservations',ARRAY(SELECT encode(reservation_id,'hex') FROM reservations ORDER BY reservation_id),
  'sessions',ARRAY(SELECT DISTINCT encode(session_id,'hex') FROM reservations WHERE session_id IS NOT NULL ORDER BY 1),
  'allocations',ARRAY(SELECT DISTINCT encode(allocation_id,'hex') FROM shares ORDER BY 1),
  'lots',ARRAY(SELECT DISTINCT encode(lot_id,'hex') FROM shares ORDER BY 1),
  'payers',ARRAY(SELECT encode(payer,'hex') FROM payers ORDER BY payer),
  'shares',ARRAY(SELECT encode(reservation_id,'hex')||'|'||encode(allocation_id,'hex')||'|'||encode(lot_id,'hex') FROM shares ORDER BY reservation_id,allocation_id,lot_id)
 )
$targets$;
REVOKE ALL ON FUNCTION finance.image_stage_plan_targets(bytea[],bytea[],bytea[]) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.image_stage_plan_targets(bytea[],bytea[],bytea[]) TO df_finance;

-- One rank/key ordering across every target, including crossing sessions and shared lots.
CREATE FUNCTION finance.lock_image_stage_plan_targets(wanted_attempts bytea[],wanted_reservations bytea[],wanted_usages bytea[])
RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog SET lock_timeout='2s' AS $locks$
DECLARE targets jsonb;
BEGIN
 targets:=finance.image_stage_plan_targets(wanted_attempts,wanted_reservations,wanted_usages);
 PERFORM finance.note_locks('payer',ARRAY(SELECT encode(account_id,'hex') FROM finance.payer
  WHERE account_id IN(SELECT decode(value,'hex') FROM jsonb_array_elements_text(targets->'payers')) ORDER BY account_id FOR UPDATE));
 PERFORM finance.note_locks('session_funding',ARRAY(SELECT encode(session_id,'hex') FROM finance.session_funding
  WHERE session_id IN(SELECT decode(value,'hex') FROM jsonb_array_elements_text(targets->'sessions')) ORDER BY session_id FOR UPDATE));
 PERFORM finance.note_locks('allocation',ARRAY(SELECT encode(allocation_id,'hex') FROM finance.allocation
  WHERE allocation_id IN(SELECT decode(value,'hex') FROM jsonb_array_elements_text(targets->'allocations')) ORDER BY allocation_id FOR UPDATE));
 PERFORM finance.note_locks('funding_lot',ARRAY(SELECT encode(lot_id,'hex') FROM finance.funding_lot
  WHERE lot_id IN(SELECT decode(value,'hex') FROM jsonb_array_elements_text(targets->'lots')) ORDER BY lot_id FOR UPDATE));
 PERFORM finance.note_locks('provider_attempt',ARRAY(SELECT encode(attempt_id,'hex') FROM finance.provider_attempt
  WHERE attempt_id IN(SELECT decode(value,'hex') FROM jsonb_array_elements_text(targets->'attempts')) ORDER BY attempt_id FOR UPDATE));
 -- The generic campaign protocol owns campaign reservations; account stage targets refuse below.
 PERFORM finance.note_locks('reservation',ARRAY(SELECT encode(reservation_id,'hex') FROM finance.reservation
  WHERE session_id IS NOT NULL AND reservation_id IN(SELECT decode(value,'hex') FROM jsonb_array_elements_text(targets->'reservations')) ORDER BY reservation_id FOR UPDATE));
 PERFORM finance.note_locks('reservation_share',ARRAY(SELECT encode(reservation_id,'hex')||'|'||encode(allocation_id,'hex')||'|'||encode(lot_id,'hex')
  FROM finance.reservation_share WHERE reservation_id IN(SELECT decode(value,'hex') FROM jsonb_array_elements_text(targets->'reservations'))
  ORDER BY reservation_id,allocation_id,lot_id FOR UPDATE));
 IF targets IS DISTINCT FROM finance.image_stage_plan_targets(wanted_attempts,wanted_reservations,wanted_usages) THEN
  RAISE EXCEPTION 'stage plan dependencies changed' USING ERRCODE='40001';
 END IF;
 RETURN targets;
END $locks$;
REVOKE ALL ON FUNCTION finance.lock_image_stage_plan_targets(bytea[],bytea[],bytea[]) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.lock_image_stage_plan_targets(bytea[],bytea[],bytea[]) TO df_finance;

-- Existing 0043/0046 installations retain the same action function and money equations.
DO $draft_unknown_guard$ DECLARE definition text; old_guard text; release_guard text; new_guard text; BEGIN
 IF to_regprocedure('finance.draft_image_command(bytea,jsonb)') IS NOT NULL THEN
  old_guard:=$old_guard$IF action IN ('prepare','sent') AND q.draft_binding ? 'call' AND q.draft_binding IS DISTINCT FROM p->'binding' THEN$old_guard$;
  release_guard:=$release_guard$IF action IN ('prepare','sent','release') AND q.draft_binding ? 'call' AND q.draft_binding IS DISTINCT FROM p->'binding' THEN$release_guard$;
  new_guard:=$new_guard$IF action IN ('prepare','sent','release','unknown') AND q.draft_binding ? 'call' AND q.draft_binding IS DISTINCT FROM p->'binding' THEN$new_guard$;
  SELECT pg_get_functiondef('finance.draft_image_command(bytea,jsonb)'::regprocedure) INTO definition;
  IF position(old_guard IN definition)>0 THEN EXECUTE replace(definition,old_guard,new_guard);
  ELSIF position(release_guard IN definition)>0 THEN EXECUTE replace(definition,release_guard,new_guard);
  ELSIF position(new_guard IN definition)=0 THEN
   RAISE EXCEPTION 'canonical draft stage protocol guard unavailable' USING ERRCODE='23514';
  END IF;
 END IF;
END $draft_unknown_guard$;
