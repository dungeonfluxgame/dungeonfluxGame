-- Explicit selected standalone analytical results. No media, quota, dispatch or money fabrication.
-- Runtime qualification happens before transactions; stored source/dependency metadata is immutable.
-- The optional publication pair is genuinely all-or-none; CHECK must reject SQL NULL truth values.
ALTER TABLE imagery.image_job DROP CONSTRAINT image_job_planned_identity_ids;
ALTER TABLE imagery.image_job ADD CONSTRAINT image_job_planned_identity_ids CHECK (
    (planned_publication_asset IS NULL AND planned_publication_attempt IS NULL)
    OR (planned_publication_asset IS NOT NULL AND planned_publication_attempt IS NOT NULL
        AND octet_length(planned_publication_asset)=16 AND octet_length(planned_publication_attempt)=16
        AND planned_publication_asset<>decode('00000000000000000000000000000000','hex')
        AND planned_publication_attempt<>decode('00000000000000000000000000000000','hex'))
);
ALTER TABLE imagery.image_execution_stage
    DROP CONSTRAINT image_execution_stage_stage_kind_check;
ALTER TABLE imagery.image_execution_stage ADD CONSTRAINT image_execution_stage_stage_kind_check
    CHECK(stage_kind IN ('classification','generation','edit','extraction','inspection','correction',
        'local_transform','photo_inspection','head_locations','segmentation','staging_identity','staging_facing'));
ALTER TABLE imagery.image_decision_observation
    DROP CONSTRAINT image_decision_observation_decision_kind_check;
ALTER TABLE imagery.image_decision_observation ADD CONSTRAINT image_decision_observation_decision_kind_check
    CHECK(decision_kind IN ('classification','inspection','photo_inspection','head_locations',
        'segmentation','staging_identity','staging_facing'));

-- One actual deterministic result per stage. No supplier attempt or raw-usage surrogate exists.
CREATE TABLE imagery.image_local_analysis (
    job_id bytea NOT NULL,
    stage_id bytea NOT NULL,
    owner_scope bytea NOT NULL CHECK(octet_length(owner_scope) IN (18,34)),
    decision_kind text NOT NULL CHECK(decision_kind IN ('photo_inspection','head_locations',
        'segmentation','staging_identity','staging_facing')),
    checkpoint_version bigint NOT NULL CHECK(checkpoint_version BETWEEN 1 AND 4294967295),
    schema_revision bigint NOT NULL CHECK(schema_revision BETWEEN 1 AND 4294967295),
    validator_revision bigint NOT NULL CHECK(validator_revision BETWEEN 1 AND 4294967295),
    input_revision bigint NOT NULL CHECK(input_revision BETWEEN 1 AND 4294967295),
    input_digest bytea NOT NULL CHECK(octet_length(input_digest)=32),
    config_revision bigint NOT NULL CHECK(config_revision>0),
    config_digest bytea NOT NULL CHECK(octet_length(config_digest)=32),
    parent_dispatch bytea CHECK(parent_dispatch IS NULL OR octet_length(parent_dispatch)=16),
    intent_digest bytea NOT NULL CHECK(octet_length(intent_digest)=32),
    source_digest bytea NOT NULL CHECK(octet_length(source_digest)=32),
    result_revision bigint NOT NULL CHECK(result_revision BETWEEN 1 AND 4294967295),
    result bytea NOT NULL CHECK(octet_length(result)>0),
    result_sha256 bytea NOT NULL CHECK(octet_length(result_sha256)=32),
    dependencies jsonb NOT NULL CHECK(jsonb_typeof(dependencies)='array'),
    claim_token bytea NOT NULL CHECK(octet_length(claim_token)=16),
    claim_generation bigint NOT NULL CHECK(claim_generation>0),
    recorded_at_ms bigint NOT NULL CHECK(recorded_at_ms>0),
    PRIMARY KEY(job_id,stage_id),
    FOREIGN KEY(job_id,owner_scope) REFERENCES imagery.image_job(job_id,owner_scope),
    FOREIGN KEY(job_id,stage_id) REFERENCES imagery.image_execution_stage(job_id,stage_id)
);

-- One explicitly selected full chain per job. A dependency references this selection, never newest stage.
CREATE TABLE imagery.image_analysis_checkpoint (
    job_id bytea PRIMARY KEY,
    stage_id bytea NOT NULL,
    owner_scope bytea NOT NULL CHECK(octet_length(owner_scope) IN (18,34)),
    decision_kind text NOT NULL CHECK(decision_kind IN ('photo_inspection','head_locations',
        'segmentation','staging_identity','staging_facing')),
    checkpoint_version bigint NOT NULL CHECK(checkpoint_version BETWEEN 1 AND 4294967295),
    schema_revision bigint NOT NULL CHECK(schema_revision BETWEEN 1 AND 4294967295),
    validator_revision bigint NOT NULL CHECK(validator_revision BETWEEN 1 AND 4294967295),
    result_revision bigint NOT NULL CHECK(result_revision BETWEEN 1 AND 4294967295),
    result bytea NOT NULL CHECK(octet_length(result)>0),
    result_sha256 bytea NOT NULL CHECK(octet_length(result_sha256)=32),
    intent_digest bytea NOT NULL CHECK(octet_length(intent_digest)=32),
    config_digest bytea NOT NULL CHECK(octet_length(config_digest)=32),
    source_digest bytea NOT NULL CHECK(octet_length(source_digest)=32),
    stages jsonb NOT NULL CHECK(jsonb_typeof(stages)='array' AND jsonb_array_length(stages)>0),
    dependencies jsonb NOT NULL CHECK(jsonb_typeof(dependencies)='array'),
    selected_at_ms bigint NOT NULL CHECK(selected_at_ms>0),
    FOREIGN KEY(job_id,owner_scope) REFERENCES imagery.image_job(job_id,owner_scope),
    FOREIGN KEY(job_id,stage_id) REFERENCES imagery.image_execution_stage(job_id,stage_id)
);
REVOKE ALL ON imagery.image_local_analysis,imagery.image_analysis_checkpoint FROM PUBLIC;
DO $analysis_roles$ BEGIN
 IF EXISTS(SELECT 1 FROM pg_roles WHERE rolname='df_imagery_output') THEN
  GRANT SELECT,INSERT ON imagery.image_local_analysis,imagery.image_analysis_checkpoint TO df_imagery_output;
 END IF;
 IF EXISTS(SELECT 1 FROM pg_roles WHERE rolname='df_imagery_admission') THEN
  -- Finance admission needs only the final-execution seal, never private analytical result bytes.
  GRANT SELECT(owner_scope,job_id) ON imagery.image_analysis_checkpoint TO df_imagery_admission;
 END IF;
 IF EXISTS(SELECT 1 FROM pg_roles WHERE rolname='df_imagery_publication') THEN
  GRANT SELECT ON imagery.image_local_analysis,imagery.image_analysis_checkpoint TO df_imagery_publication;
 END IF;
END $analysis_roles$;

-- Upgrade the existing same-ledger binding; standalone analysis has no planned media pair.
DO $analysis_finance$ BEGIN
 IF EXISTS(SELECT 1 FROM pg_attribute WHERE attrelid=to_regclass('imagery.image_finance_admission_binding')
    AND attname='execution_stage_id' AND NOT attisdropped) THEN
ALTER TABLE imagery.image_finance_admission_binding DROP CONSTRAINT stage_media_presence;
ALTER TABLE imagery.image_finance_admission_binding DROP CONSTRAINT stage_binding_shape;
  ALTER TABLE imagery.image_finance_admission_binding ADD CONSTRAINT stage_media_presence CHECK((
   (publication_attempt IS NULL AND stage_id IS NULL AND asset_id IS NULL AND stage_kind IN ('classification','photo_inspection','head_locations','segmentation','staging_identity','staging_facing'))
   OR (publication_attempt IS NOT NULL AND stage_id IS NOT NULL AND asset_id IS NOT NULL)) IS TRUE);
  ALTER TABLE imagery.image_finance_admission_binding ADD CONSTRAINT stage_binding_shape CHECK((
   (execution_stage_id IS NULL AND dispatch_id IS NULL AND parent_dispatch IS NULL AND stage_kind IS NULL
    AND stage_call IS NULL AND route_key IS NULL AND cost_route IS NULL AND planned_asset IS NULL AND planned_publication IS NULL)
   OR (octet_length(execution_stage_id)=16 AND execution_stage_id IS NOT NULL
    AND octet_length(dispatch_id)=16 AND dispatch_id IS NOT NULL
    AND (parent_dispatch IS NULL OR octet_length(parent_dispatch)=16)
    AND stage_kind IN ('classification','generation','edit','inspection','correction','extraction','photo_inspection','head_locations','segmentation','staging_identity','staging_facing')
    AND jsonb_typeof(stage_call)='object' AND stage_call IS NOT NULL
    AND octet_length(route_key)=32 AND route_key IS NOT NULL AND cost_route IS NOT NULL AND jsonb_typeof(cost_route)='object'
    AND ((stage_kind IN ('photo_inspection','head_locations','segmentation','staging_identity','staging_facing') AND planned_asset IS NULL AND planned_publication IS NULL)
     OR (stage_kind NOT IN ('photo_inspection','head_locations','segmentation','staging_identity','staging_facing')
      AND octet_length(planned_asset)=16 AND planned_asset IS NOT NULL
      AND octet_length(planned_publication)=16 AND planned_publication IS NOT NULL)))) IS TRUE);

 END IF;
 IF to_regprocedure('finance.draft_scope_matches(jsonb,jsonb)') IS NOT NULL THEN
  EXECUTE $analysis_definition$
CREATE OR REPLACE FUNCTION finance.draft_scope_matches(grant_scope jsonb,binding jsonb) RETURNS boolean
LANGUAGE plpgsql IMMUTABLE SET search_path=pg_catalog,finance,pg_temp AS $$
DECLARE approved jsonb; selected jsonb:=binding->'stage_route'; call jsonb:=binding->'call';
 actual jsonb:=binding->'scope'; maximum jsonb; units jsonb; n integer;
BEGIN
 IF grant_scope->>'scope_kind' IS DISTINCT FROM 'job' THEN
  RETURN grant_scope=actual AND NOT binding ? 'call';
 END IF;
 IF selected IS NULL OR call IS NULL
  OR grant_scope-ARRAY['scope_kind','routes','config_revision'] IS DISTINCT FROM
     actual-ARRAY['provider','model','route','price_version','maximum_units']
  OR call->'job' IS DISTINCT FROM actual->'job'
  OR call->'stage' IS DISTINCT FROM binding->'stage'
  OR call->'asset' IS DISTINCT FROM binding->'asset'
  OR call->'publication' IS DISTINCT FROM binding->'publication'
  OR call->'kind' IS DISTINCT FROM selected->'kind'
  OR call->'config_revision' IS DISTINCT FROM grant_scope->'config_revision'
  OR call->'config_digest' IS DISTINCT FROM grant_scope->'config_digest'
  OR selected->'provider' IS DISTINCT FROM actual->'provider'
  OR selected->'model' IS DISTINCT FROM actual->'model'
  OR selected->'route' IS DISTINCT FROM actual->'route'
  OR selected->'price_version' IS DISTINCT FROM actual->'price_version'
  OR selected->'maximum_units' IS DISTINCT FROM actual->'maximum_units'
  OR jsonb_array_length(grant_scope->'routes') NOT BETWEEN 1 AND 16 THEN RETURN false; END IF;
 IF call->>'kind' IN ('photo_inspection','head_locations','segmentation','staging_identity','staging_facing') THEN
  IF call->'asset' IS DISTINCT FROM 'null'::jsonb OR call->'publication' IS DISTINCT FROM 'null'::jsonb THEN RETURN false; END IF;
 ELSE
  IF jsonb_typeof(call->'asset') IS DISTINCT FROM 'array' OR jsonb_typeof(call->'publication') IS DISTINCT FROM 'array' THEN RETURN false; END IF;
  IF jsonb_array_length(call->'asset')<>16 OR jsonb_array_length(call->'publication')<>16 THEN RETURN false; END IF;
 END IF;
 FOR approved IN SELECT value FROM jsonb_array_elements(grant_scope->'routes') LOOP
  IF approved-'maximum_units' IS DISTINCT FROM selected-'maximum_units' THEN CONTINUE; END IF;
  IF jsonb_array_length(approved->'maximum_units') IS DISTINCT FROM jsonb_array_length(selected->'maximum_units')
   OR jsonb_array_length(selected->'maximum_units') NOT BETWEEN 1 AND 32 THEN RETURN false; END IF;
  n:=0;
  FOR maximum IN SELECT value FROM jsonb_array_elements(approved->'maximum_units') LOOP
   units:=selected->'maximum_units'->n;
   IF maximum->0 IS DISTINCT FROM units->0 OR (units->>1)::numeric<=0
    OR (units->>1)::numeric>(maximum->>1)::numeric THEN RETURN false; END IF;
   n:=n+1;
  END LOOP;
  RETURN true;
 END LOOP;
 RETURN false;
END $$;
$analysis_definition$;
 END IF;
END $analysis_finance$;

-- Upgrade only this existing canonical function guard; no money equation is copied or repriced.
DO $analysis_release$ DECLARE definition text; old_guard text; release_guard text; new_guard text; BEGIN
 IF to_regprocedure('finance.draft_image_command(bytea,jsonb)') IS NOT NULL THEN
  old_guard:=$old_guard$IF action IN ('prepare','sent') AND q.draft_binding ? 'call' AND q.draft_binding IS DISTINCT FROM p->'binding' THEN$old_guard$;
  release_guard:=$release_guard$IF action IN ('prepare','sent','release') AND q.draft_binding ? 'call' AND q.draft_binding IS DISTINCT FROM p->'binding' THEN$release_guard$;
  new_guard:=$new_guard$IF action IN ('prepare','sent','release','unknown') AND q.draft_binding ? 'call' AND q.draft_binding IS DISTINCT FROM p->'binding' THEN$new_guard$;
  SELECT pg_get_functiondef('finance.draft_image_command(bytea,jsonb)'::regprocedure) INTO definition;
  IF position(old_guard IN definition)>0 THEN
   EXECUTE replace(definition,old_guard,new_guard);
  ELSIF position(release_guard IN definition)>0 THEN
   EXECUTE replace(definition,release_guard,new_guard);
  ELSIF position(new_guard IN definition)=0 THEN
   RAISE EXCEPTION 'canonical draft stage protocol guard unavailable' USING ERRCODE='23514';
  END IF;
 END IF;
END $analysis_release$;
