-- Closure-only evidence locks existing canonical authority before evaluating denial.
-- 0: missing/unproven, 1: current, 2: existing locked authority refused.
CREATE FUNCTION imagery.lock_run_authority_evidence(
 p_tenant bytea,p_campaign bytea,p_hosted bytea,p_game_tenant bytea,
 p_session bytea,p_run bytea,p_fence bytea) RETURNS smallint
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp AS $$
DECLARE campaign_row record; hosted_row record; gameplay_row record;
BEGIN
 IF p_tenant IS NULL OR octet_length(p_tenant)<>16 OR p_campaign IS NULL OR octet_length(p_campaign)<>16
  OR p_hosted IS NULL OR octet_length(p_hosted)<>16 OR p_game_tenant IS NULL OR octet_length(p_game_tenant)<>16
  OR p_session IS NULL OR octet_length(p_session)<>16 OR p_run IS NULL OR octet_length(p_run)<>16
  OR p_fence IS NULL OR octet_length(p_fence)<>16 THEN RETURN 0; END IF;
 SELECT status INTO campaign_row FROM campaigns.campaign
  WHERE tenant_id=p_tenant AND campaign_id=p_campaign FOR SHARE;
 IF NOT FOUND THEN RETURN 0; END IF;
 SELECT campaign_id,phase INTO hosted_row FROM df_runtime.table_sessions
  WHERE tenant_id=p_tenant AND session_id=p_hosted FOR SHARE;
 IF NOT FOUND THEN RETURN 0; END IF;
 SELECT run_id,owner_fence,lease_until>clock_timestamp() AS live INTO gameplay_row
  FROM df_game.sessions WHERE tenant_id=p_game_tenant AND session_id=p_session FOR SHARE;
 IF NOT FOUND THEN RETURN 0; END IF;
 IF campaign_row.status IS NULL OR hosted_row.campaign_id IS NULL OR hosted_row.phase IS NULL
  OR gameplay_row.run_id IS NULL OR gameplay_row.owner_fence IS NULL OR gameplay_row.live IS NULL
 THEN RETURN 0; END IF;
 IF campaign_row.status<>'active' OR hosted_row.campaign_id<>p_campaign
  OR hosted_row.phase NOT IN ('preparing','active','paused')
  OR gameplay_row.run_id<>p_run OR gameplay_row.owner_fence<>p_fence OR NOT gameplay_row.live
 THEN RETURN 2; END IF;
 RETURN 1;
END $$;
REVOKE ALL ON FUNCTION imagery.lock_run_authority_evidence(bytea,bytea,bytea,bytea,bytea,bytea,bytea) FROM PUBLIC;
CREATE FUNCTION imagery.lock_hosted_authority_evidence(
 p_tenant bytea,p_campaign bytea,p_hosted bytea,p_member bytea,p_generation bytea,
 p_web bytea,p_credential bytea) RETURNS smallint
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,pg_temp AS $$
DECLARE seat_hint record; run_hint record; seat_row record; run_row record;
 account_row record; web_row record; member_row record; grant_row record; canonical smallint;
BEGIN
 IF p_tenant IS NULL OR octet_length(p_tenant)<>16 OR p_campaign IS NULL OR octet_length(p_campaign)<>16
  OR p_hosted IS NULL OR octet_length(p_hosted)<>16 OR p_member IS NULL OR octet_length(p_member)<>16
  OR p_generation IS NULL OR octet_length(p_generation)<>16 OR p_web IS NULL OR octet_length(p_web)<>16
  OR p_credential IS NULL OR octet_length(p_credential)<>32 THEN RETURN 0; END IF;
 -- Hints identify immutable keys. All policy fields are rechecked under ordered locks below.
 SELECT account_id,gameplay_member_id,run_id,gameplay_session_id INTO seat_hint
  FROM imagery.image_host_seat_binding WHERE tenant_id=p_tenant AND campaign_id=p_campaign
  AND hosted_session_id=p_hosted AND member_id=p_member;
 IF NOT FOUND THEN RETURN 0; END IF;
 SELECT gameplay_tenant_id,gameplay_session_id,run_id,owner_fence INTO run_hint
  FROM imagery.image_host_run_binding WHERE tenant_id=p_tenant AND campaign_id=p_campaign
  AND hosted_session_id=p_hosted;
 IF NOT FOUND THEN RETURN 0; END IF;
 -- Matches account-close, ordinary principal resolution and hosted binding writer order.
 SELECT status,credential_generation INTO account_row FROM accounts.account
  WHERE account_id=seat_hint.account_id FOR SHARE;
 IF NOT FOUND THEN RETURN 0; END IF;
 SELECT account_id,credential_generation,revoked_at_ms,
  absolute_expires_at_ms>extract(epoch from clock_timestamp())*1000 AS live INTO web_row
  FROM accounts.web_session WHERE session_id=p_web FOR SHARE;
 IF NOT FOUND THEN RETURN 0; END IF;
 canonical:=imagery.lock_run_authority_evidence(p_tenant,p_campaign,p_hosted,
  run_hint.gameplay_tenant_id,run_hint.gameplay_session_id,run_hint.run_id,run_hint.owner_fence);
 IF canonical IS NULL OR canonical NOT IN (1,2) THEN RETURN 0; END IF;
 SELECT account_id,status,holds_seat INTO member_row FROM campaigns.campaign_member
  WHERE campaign_id=p_campaign AND member_id=p_member FOR SHARE;
 IF NOT FOUND THEN RETURN 0; END IF;
 SELECT principal_id,session_id,effective_role,active,expires_at>clock_timestamp() AS live
  INTO grant_row FROM df_local_demo.grants WHERE credential=p_credential FOR SHARE;
 IF NOT FOUND THEN RETURN 0; END IF;
 SELECT gameplay_tenant_id,gameplay_session_id,run_id,owner_fence INTO run_row
  FROM imagery.image_host_run_binding WHERE tenant_id=p_tenant AND campaign_id=p_campaign
  AND hosted_session_id=p_hosted FOR SHARE;
 IF NOT FOUND THEN RETURN 0; END IF;
 SELECT account_id,gameplay_member_id,run_id,gameplay_session_id,generation,active INTO seat_row
  FROM imagery.image_host_seat_binding WHERE tenant_id=p_tenant AND campaign_id=p_campaign
  AND hosted_session_id=p_hosted AND member_id=p_member FOR SHARE;
 IF NOT FOUND THEN RETURN 0; END IF;
 IF ROW(run_row.gameplay_tenant_id,run_row.gameplay_session_id,run_row.run_id,run_row.owner_fence)
  IS DISTINCT FROM ROW(run_hint.gameplay_tenant_id,run_hint.gameplay_session_id,run_hint.run_id,run_hint.owner_fence)
  OR ROW(seat_row.account_id,seat_row.gameplay_member_id,seat_row.run_id,seat_row.gameplay_session_id)
  IS DISTINCT FROM ROW(seat_hint.account_id,seat_hint.gameplay_member_id,seat_hint.run_id,seat_hint.gameplay_session_id)
 THEN RETURN 0; END IF;
 IF seat_row.account_id IS NULL OR seat_row.gameplay_member_id IS NULL OR seat_row.run_id IS NULL
  OR seat_row.gameplay_session_id IS NULL OR seat_row.generation IS NULL OR seat_row.active IS NULL
  OR account_row.status IS NULL OR account_row.credential_generation IS NULL OR web_row.account_id IS NULL
  OR web_row.credential_generation IS NULL OR web_row.live IS NULL OR member_row.account_id IS NULL
  OR member_row.status IS NULL OR member_row.holds_seat IS NULL OR grant_row.principal_id IS NULL
  OR grant_row.session_id IS NULL OR grant_row.effective_role IS NULL OR grant_row.active IS NULL OR grant_row.live IS NULL
 THEN RETURN 0; END IF;
 IF canonical<>1 OR NOT seat_row.active OR seat_row.generation<>p_generation
  OR seat_row.run_id<>run_row.run_id OR seat_row.gameplay_session_id<>run_row.gameplay_session_id
  OR account_row.status<>'active' OR web_row.account_id<>seat_row.account_id
  OR web_row.revoked_at_ms IS NOT NULL OR NOT web_row.live
  OR web_row.credential_generation<>account_row.credential_generation
  OR member_row.account_id<>seat_row.account_id OR member_row.status<>'active' OR NOT member_row.holds_seat
  OR grant_row.principal_id<>seat_row.gameplay_member_id OR grant_row.session_id<>run_row.gameplay_session_id
  OR grant_row.effective_role<>convert_to('player','UTF8') OR NOT grant_row.active OR NOT grant_row.live
 THEN RETURN 2; END IF;
 RETURN 1;
END $$;
REVOKE ALL ON FUNCTION imagery.lock_hosted_authority_evidence(bytea,bytea,bytea,bytea,bytea,bytea,bytea) FROM PUBLIC;
DO $$ BEGIN
 IF to_regclass('campaigns.campaign') IS NULL AND to_regclass('df_runtime.table_sessions') IS NULL THEN
  RETURN;
 END IF;
 IF to_regclass('campaigns.campaign') IS NULL OR to_regclass('df_runtime.table_sessions') IS NULL
  OR to_regrole('df_runtime_definer') IS NULL OR to_regrole('df_imagery_admission') IS NULL THEN
  RAISE EXCEPTION 'imagery authority evidence hosted prerequisites are incomplete';
 END IF;
 -- Internal NOLOGIN definer only: UPDATE of one column permits FOR SHARE; no mutation API is exposed.
 GRANT USAGE ON SCHEMA accounts,df_local_demo TO df_runtime_definer;
 GRANT SELECT,UPDATE(credential_generation) ON accounts.account TO df_runtime_definer;
 GRANT SELECT,UPDATE(revoked_at_ms) ON accounts.web_session TO df_runtime_definer;
 GRANT SELECT,UPDATE(status) ON campaigns.campaign_member TO df_runtime_definer;
 GRANT SELECT,UPDATE(active) ON imagery.image_host_seat_binding TO df_runtime_definer;
 GRANT SELECT,UPDATE(owner_fence) ON imagery.image_host_run_binding TO df_runtime_definer;
 GRANT SELECT,UPDATE(active) ON df_local_demo.grants TO df_runtime_definer;
 ALTER FUNCTION imagery.lock_run_authority_evidence(bytea,bytea,bytea,bytea,bytea,bytea,bytea)
  OWNER TO df_runtime_definer;
 ALTER FUNCTION imagery.lock_hosted_authority_evidence(bytea,bytea,bytea,bytea,bytea,bytea,bytea)
  OWNER TO df_runtime_definer;
 GRANT EXECUTE ON FUNCTION imagery.lock_hosted_authority_evidence(bytea,bytea,bytea,bytea,bytea,bytea,bytea)
  TO df_imagery_admission;
 GRANT EXECUTE ON FUNCTION imagery.lock_run_authority_evidence(bytea,bytea,bytea,bytea,bytea,bytea,bytea)
  TO df_imagery_admission;
END $$;
