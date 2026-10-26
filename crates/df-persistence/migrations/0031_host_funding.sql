-- Host consent is an intention until Start atomically opens and allocates through commit_event.
-- Customer prices are the published preset ceilings, never the platform rate or supplier tariff.
CREATE TABLE finance.host_session_consent (
    session_id bytea NOT NULL CHECK (octet_length(session_id) = 16),
    campaign_id bytea NOT NULL CHECK (octet_length(campaign_id) = 16),
    account_id bytea NOT NULL CHECK (octet_length(account_id) = 16),
    operation_id bytea NOT NULL CHECK (octet_length(operation_id) = 16),
    price_version_id bytea NOT NULL REFERENCES finance.price_version,
    hourly_micro_dollars bigint NOT NULL CHECK (hourly_micro_dollars > 0),
    cap_micro_dollars bigint NOT NULL CHECK (cap_micro_dollars >= hourly_micro_dollars),
    terms_version integer NOT NULL CHECK (terms_version = 1),
    consented_at_ms bigint NOT NULL,
    PRIMARY KEY (session_id, account_id, operation_id),
    UNIQUE (account_id, operation_id)
);
CREATE TRIGGER host_session_consent_append_only BEFORE UPDATE OR DELETE
    ON finance.host_session_consent FOR EACH ROW EXECUTE FUNCTION finance.refuse_change();

CREATE TABLE finance.host_session_close (
    session_id bytea PRIMARY KEY CHECK (octet_length(session_id) = 16),
    closed_at_ms bigint NOT NULL
);
CREATE TRIGGER host_session_close_append_only BEFORE UPDATE OR DELETE
    ON finance.host_session_close FOR EACH ROW EXECUTE FUNCTION finance.refuse_change();

CREATE FUNCTION finance.lock_host_session(p_session bytea) RETURNS void
LANGUAGE sql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, finance, pg_temp AS $$
    SELECT pg_advisory_xact_lock(hashtextextended(encode(p_session, 'hex'), 30))
$$;

-- One payer/session flight, then the established account -> finance -> runtime -> campaign order.
-- The caller retains this transaction until the posting API commits or rolls back every change.
CREATE FUNCTION finance.lock_host_funding(p_session bytea, p_account bytea) RETURNS void
LANGUAGE plpgsql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, finance, pg_temp AS $$
BEGIN
    PERFORM finance.lock_host_session(p_session);
    PERFORM 1 FROM accounts.account WHERE account_id = p_account FOR SHARE;
    PERFORM finance.lock_flow_allocate(jsonb_build_object('payer', encode(p_account, 'hex'),
                                                        'session', encode(p_session, 'hex')));
    PERFORM 1 FROM df_runtime.table_sessions WHERE session_id = p_session FOR SHARE;
    PERFORM 1 FROM campaigns.campaign c JOIN df_runtime.table_sessions s
        ON s.campaign_id = c.campaign_id AND s.tenant_id = c.tenant_id
        WHERE s.session_id = p_session FOR SHARE OF c;
    PERFORM 1 FROM campaigns.campaign_member m JOIN df_runtime.table_sessions s
        ON s.campaign_id = m.campaign_id WHERE s.session_id = p_session
        AND m.role = 'host' AND m.status = 'active' FOR SHARE OF m;
END $$;

CREATE FUNCTION finance.host_funding_host(p_campaign bytea) RETURNS bytea
LANGUAGE sql STABLE SECURITY DEFINER
SET search_path = pg_catalog, finance, pg_temp AS $$
    SELECT account_id FROM campaigns.campaign_member WHERE campaign_id = p_campaign
        AND role = 'host' AND status = 'active'
$$;

CREATE FUNCTION finance.host_funding_context(p_session bytea, p_account bytea, p_now bigint)
RETURNS TABLE (campaign_id bytea, tenant_id bytea, phase text, is_active boolean,
    payer_active boolean, price_version_id bytea, hourly_micro_dollars bigint,
    available_micro_dollars bigint)
LANGUAGE sql STABLE SECURITY DEFINER
SET search_path = pg_catalog, finance, pg_temp AS $$
    SELECT c.campaign_id, c.tenant_id, s.phase,
           c.status = 'active' AND b.status = 'active', coalesce(p.standing = 'active', true),
           v.price_version_id,
           CASE WHEN c.spend_preset = 'custom' THEN c.custom_hourly_micro_dollars
                ELSE (v.presets #>> ARRAY[c.spend_preset, 'hourly_ceiling_micro_dollars'])::bigint END,
           coalesce((SELECT sum(a.natural_micro_dollars)::bigint
              FROM finance.account_balance a JOIN finance.account_key k USING (key_id)
              JOIN finance.funding_lot l ON l.lot_id = k.lot_id
              WHERE k.payer_account_id = p_account AND k.code IN (2110,2210)
                AND l.state = 'active' AND l.currency = 'USD'
                AND (l.expires_at_ms IS NULL OR l.expires_at_ms > p_now)), 0)
      FROM df_runtime.table_sessions s
      JOIN campaigns.campaign c ON c.campaign_id = s.campaign_id AND c.tenant_id = s.tenant_id
      JOIN campaigns.campaign_member m ON m.campaign_id = c.campaign_id
           AND m.account_id = p_account AND m.role = 'host' AND m.status = 'active'
      JOIN accounts.account b ON b.account_id = p_account
      LEFT JOIN finance.payer p ON p.account_id = p_account
      LEFT JOIN LATERAL (SELECT * FROM finance.price_version
          WHERE status = 'published' AND currency = 'USD' AND effective_from_ms <= p_now
          ORDER BY effective_from_ms DESC, price_version_id DESC LIMIT 1) v ON true
     WHERE s.session_id = p_session AND NOT EXISTS (SELECT 1 FROM finance.host_session_close x WHERE x.session_id = p_session)
$$;

CREATE FUNCTION finance.record_host_consent(p_session bytea, p_account bytea, p_operation bytea,
    p_version bytea, p_cap bigint, p_terms integer, p_now bigint) RETURNS void
LANGUAGE plpgsql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE v record; old finance.host_session_consent%ROWTYPE;
BEGIN
    PERFORM finance.lock_host_funding(p_session, p_account);
    SELECT * INTO v FROM finance.host_funding_context(p_session, p_account, p_now);
    IF NOT FOUND OR NOT v.is_active OR NOT v.payer_active OR v.phase <> 'lobby'
       OR v.price_version_id IS DISTINCT FROM p_version OR v.hourly_micro_dollars IS NULL
       OR v.hourly_micro_dollars <= 0 OR p_cap < v.hourly_micro_dollars OR p_terms <> 1 THEN
        RAISE EXCEPTION 'host consent refused' USING ERRCODE = 'FH001';
    END IF;
    INSERT INTO finance.host_session_consent VALUES (p_session, v.campaign_id, p_account,
        p_operation, p_version, v.hourly_micro_dollars, p_cap, p_terms, p_now)
        ON CONFLICT (account_id, operation_id) DO NOTHING;
    SELECT * INTO STRICT old FROM finance.host_session_consent
        WHERE account_id = p_account AND operation_id = p_operation;
    IF old.session_id <> p_session OR old.price_version_id <> p_version OR old.cap_micro_dollars <> p_cap
       OR old.hourly_micro_dollars <> v.hourly_micro_dollars OR old.terms_version <> p_terms THEN
        RAISE EXCEPTION 'consent already differs' USING ERRCODE = 'FH002';
    END IF;
END $$;

-- Closure metadata is journaled through the same posting boundary as the allocation release.
INSERT INTO finance.change_kind (kind,table_name,op,pk_cols,phase,unjournaled,columns,updatable_cols,journal_cols) VALUES
    ('host_close_insert','host_session_close','insert','{session_id}',1,false,
        '{session_id,closed_at_ms}','{}','{session_id,closed_at_ms}'),
    ('host_session_close_update','session_funding','update','{session_id}',2,false,'{}',
        '{status,settling_at_ms,closed_at_ms}','{session_id,status,settling_at_ms,closed_at_ms}'),
    ('host_allocation_close_update','allocation','update','{allocation_id}',2,false,'{}',
        '{status}','{allocation_id,status}');
INSERT INTO finance.event_kind VALUES ('host_funding_closed',false,true,
    '{host_close_insert,host_session_close_update,host_allocation_close_update}','{}');
INSERT INTO finance.event_update_cols VALUES
    ('host_funding_closed','host_session_close_update','{status,settling_at_ms,closed_at_ms}'),
    ('host_funding_closed','host_allocation_close_update','{status}');

CREATE FUNCTION finance.apply_host_close_insert(p_row jsonb) RETURNS void
LANGUAGE sql VOLATILE SET search_path = pg_catalog, finance, pg_temp AS $$
    INSERT INTO finance.host_session_close SELECT r.*
        FROM jsonb_populate_record(NULL::finance.host_session_close,p_row) r
$$;
CREATE FUNCTION finance.apply_host_session_close_update(p_row jsonb,p_expected bigint) RETURNS bigint
LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE n bigint;
BEGIN
    IF p_row->>'status' NOT IN ('settling','closed') THEN
        RAISE EXCEPTION 'host closure cannot reopen funding' USING ERRCODE='FN003';
    END IF;
    UPDATE finance.session_funding s SET status=r.status, settling_at_ms=r.settling_at_ms,
        closed_at_ms=r.closed_at_ms, revision=s.revision+1
        FROM jsonb_populate_record(NULL::finance.session_funding,p_row) r
        WHERE s.session_id=r.session_id AND s.revision=p_expected AND s.status<>'closed';
    GET DIAGNOSTICS n = ROW_COUNT;
    RETURN n;
END $$;
CREATE FUNCTION finance.apply_host_allocation_close_update(p_row jsonb,p_expected bigint) RETURNS bigint
LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE n bigint;
BEGIN
    IF p_row->>'status' IS DISTINCT FROM 'closing' THEN
        RAISE EXCEPTION 'host closure cannot reactivate an allocation' USING ERRCODE='FN003';
    END IF;
    UPDATE finance.allocation a SET status=r.status, revision=a.revision+1
        FROM jsonb_populate_record(NULL::finance.allocation,p_row) r
        WHERE a.allocation_id=r.allocation_id AND a.revision=p_expected AND a.status IN ('active','withdrawn');
    GET DIAGNOSTICS n = ROW_COUNT;
    RETURN n;
END $$;
-- Preserve the generated dispatchers' OIDs and every existing branch, including cached callers.
DO $$
DECLARE body text;
BEGIN
    body := pg_get_functiondef('finance.apply_insert(text,jsonb)'::regprocedure);
    EXECUTE replace(body,'CASE p_kind', E'CASE p_kind\n WHEN ''host_close_insert'' THEN PERFORM finance.apply_host_close_insert(p_row);');
    body := pg_get_functiondef('finance.apply_update(text,jsonb,bigint)'::regprocedure);
    EXECUTE replace(body,'CASE p_kind', E'CASE p_kind\n WHEN ''host_session_close_update'' THEN RETURN finance.apply_host_session_close_update(p_row,p_expected);\n WHEN ''host_allocation_close_update'' THEN RETURN finance.apply_host_allocation_close_update(p_row,p_expected);');
END $$;

CREATE FUNCTION finance.close_host_funding(p_session bytea, p_now bigint) RETURNS void
LANGUAGE plpgsql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, finance, pg_temp AS $$
DECLARE changes jsonb := '[]'; s finance.session_funding%ROWTYPE; a finance.allocation%ROWTYPE;
    target text;
BEGIN
    PERFORM finance.lock_host_session(p_session);
    PERFORM finance.note_locks('session_funding',ARRAY(SELECT encode(session_id,'hex')
        FROM finance.session_funding WHERE session_id=p_session FOR UPDATE));
    PERFORM finance.note_locks('allocation',ARRAY(SELECT encode(allocation_id,'hex')
        FROM finance.allocation WHERE session_id=p_session ORDER BY allocation_id FOR UPDATE));
    SELECT * INTO s FROM finance.session_funding WHERE session_id=p_session;
    IF s.session_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM finance.host_session_consent WHERE session_id=p_session) THEN
        RAISE EXCEPTION 'not host funding' USING ERRCODE = 'FH003';
    END IF;
    target := CASE WHEN EXISTS (SELECT 1 FROM finance.reservation r
        WHERE r.session_id=p_session AND r.state='held') THEN 'settling' ELSE 'closed' END;
    IF NOT EXISTS (SELECT 1 FROM finance.host_session_close WHERE session_id=p_session) THEN
        changes := changes || jsonb_build_array(jsonb_build_object('kind','host_close_insert',
            'row',jsonb_build_object('session_id',p_session,'closed_at_ms',p_now)));
    END IF;
    FOR a IN SELECT * FROM finance.allocation WHERE session_id=p_session
        AND status IN ('active','withdrawn') LOOP
        changes := changes || jsonb_build_array(jsonb_build_object('kind','host_allocation_close_update',
            'expected_revision',a.revision,'row',jsonb_build_object('allocation_id',a.allocation_id,'status','closing')));
    END LOOP;
    IF s.session_id IS NOT NULL AND s.status <> target THEN
        changes := changes || jsonb_build_array(jsonb_build_object('kind','host_session_close_update',
            'expected_revision',s.revision,'row',jsonb_build_object('session_id',p_session,'status',target,
            'settling_at_ms',coalesce(s.settling_at_ms,p_now),
            'closed_at_ms',CASE WHEN target='closed' THEN p_now ELSE NULL END)));
    END IF;
    IF jsonb_array_length(changes)>0 THEN
        PERFORM finance.commit_event(jsonb_build_object('event_id',md5('host-close:'||encode(p_session,'hex')||target),
            'event_kind','host_funding_closed','payload_schema',1,'entries','[]'::jsonb,'changes',changes));
    END IF;
END $$;

REVOKE ALL ON finance.host_session_close FROM PUBLIC;
GRANT SELECT ON finance.host_session_close TO df_finance, df_finance_definer;
GRANT INSERT ON finance.host_session_close TO df_finance_definer;
ALTER FUNCTION finance.lock_host_session(bytea) OWNER TO df_finance_definer;
REVOKE ALL ON FUNCTION finance.lock_host_session(bytea) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.lock_host_session(bytea) TO df_finance;
REVOKE ALL ON finance.host_session_consent FROM PUBLIC;
GRANT SELECT ON finance.host_session_consent TO df_finance, df_finance_definer;
GRANT INSERT ON finance.host_session_consent TO df_finance_definer;
GRANT USAGE ON SCHEMA accounts, campaigns, df_runtime TO df_finance_definer;
GRANT SELECT ON accounts.account, campaigns.campaign, campaigns.campaign_member,
    df_runtime.table_sessions TO df_finance_definer;
GRANT UPDATE (account_id) ON accounts.account TO df_finance_definer;
GRANT UPDATE (campaign_id) ON campaigns.campaign TO df_finance_definer;
GRANT UPDATE (account_id) ON campaigns.campaign_member TO df_finance_definer;
GRANT UPDATE (session_id) ON df_runtime.table_sessions TO df_finance_definer;
GRANT UPDATE (status, settling_at_ms, closed_at_ms, revision) ON finance.session_funding
    TO df_finance_definer;
ALTER FUNCTION finance.lock_host_funding(bytea,bytea) OWNER TO df_finance_definer;
ALTER FUNCTION finance.host_funding_context(bytea,bytea,bigint) OWNER TO df_finance_definer;
ALTER FUNCTION finance.record_host_consent(bytea,bytea,bytea,bytea,bigint,integer,bigint)
    OWNER TO df_finance_definer;
ALTER FUNCTION finance.close_host_funding(bytea,bigint) OWNER TO df_finance_definer;
REVOKE ALL ON FUNCTION finance.lock_host_funding(bytea,bytea),
    finance.host_funding_context(bytea,bytea,bigint),
    finance.record_host_consent(bytea,bytea,bytea,bytea,bigint,integer,bigint),
    finance.close_host_funding(bytea,bigint) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.lock_host_funding(bytea,bytea),
    finance.host_funding_context(bytea,bytea,bigint),
    finance.record_host_consent(bytea,bytea,bytea,bytea,bigint,integer,bigint),
    finance.close_host_funding(bytea,bigint) TO df_finance;
-- JOURNEYWIRE's bounded consumer writes metadata only, never an allocation or journal row.
GRANT INSERT, UPDATE ON finance.funding_feed_position TO df_finance;
GRANT INSERT ON finance.funding_effect TO df_finance;

ALTER FUNCTION finance.host_funding_host(bytea) OWNER TO df_finance_definer;
REVOKE ALL ON FUNCTION finance.host_funding_host(bytea) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.host_funding_host(bytea) TO df_finance;

GRANT UPDATE (status, ended_at_ms, revision) ON finance.allocation TO df_finance_definer;

REVOKE ALL ON FUNCTION finance.apply_host_close_insert(jsonb),
    finance.apply_host_session_close_update(jsonb,bigint),
    finance.apply_host_allocation_close_update(jsonb,bigint) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.apply_host_close_insert(jsonb),
    finance.apply_host_session_close_update(jsonb,bigint),
    finance.apply_host_allocation_close_update(jsonb,bigint) TO df_finance_definer;

-- The replay marker is committed with the lifecycle transition, before money is released.
CREATE TABLE df_runtime.host_lobby_cancel (
    session_id bytea PRIMARY KEY CHECK (octet_length(session_id) = 16),
    account_id bytea NOT NULL CHECK (octet_length(account_id) = 16)
);
REVOKE ALL ON df_runtime.host_lobby_cancel FROM PUBLIC;
GRANT SELECT, INSERT ON df_runtime.host_lobby_cancel TO df_finance_definer;
GRANT UPDATE (phase, ended_at) ON df_runtime.table_sessions TO df_finance_definer;
CREATE FUNCTION df_runtime.cancel_host_lobby(p_session bytea, p_account bytea) RETURNS boolean
LANGUAGE plpgsql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, finance, pg_temp AS $$
DECLARE v_phase text; v_campaign bytea; v_tenant bytea;
BEGIN
    PERFORM finance.lock_host_funding(p_session, p_account);
    SELECT s.phase, s.campaign_id, s.tenant_id INTO v_phase, v_campaign, v_tenant
        FROM df_runtime.table_sessions s WHERE s.session_id = p_session FOR UPDATE;
    IF NOT FOUND OR NOT EXISTS (
        SELECT 1 FROM campaigns.campaign c JOIN campaigns.campaign_member m USING (campaign_id)
            JOIN accounts.account a ON a.account_id = m.account_id
            LEFT JOIN finance.payer p ON p.account_id = a.account_id
            WHERE c.campaign_id = v_campaign AND c.tenant_id = v_tenant AND c.status = 'active'
                AND m.account_id = p_account AND m.role = 'host' AND m.status = 'active'
                AND a.status = 'active' AND coalesce(p.standing = 'active', true)
    ) THEN RETURN false; END IF;
    IF v_phase = 'ended' THEN
        RETURN EXISTS (SELECT 1 FROM df_runtime.host_lobby_cancel
            WHERE session_id = p_session AND account_id = p_account);
    END IF;
    IF v_phase <> 'lobby' THEN RETURN false; END IF;
    INSERT INTO df_runtime.host_lobby_cancel VALUES (p_session, p_account);
    UPDATE df_runtime.table_sessions SET phase = 'ended', ended_at = clock_timestamp()
        WHERE session_id = p_session AND tenant_id = v_tenant;
    RETURN true;
END $$;
ALTER FUNCTION df_runtime.cancel_host_lobby(bytea,bytea) OWNER TO df_finance_definer;
REVOKE ALL ON FUNCTION df_runtime.cancel_host_lobby(bytea,bytea) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION df_runtime.cancel_host_lobby(bytea,bytea) TO df_finance;

GRANT USAGE ON SCHEMA df_runtime TO df_finance;

CREATE POLICY finance_host_funding_read ON df_runtime.table_sessions
    FOR SELECT TO df_finance_definer USING (true);
CREATE POLICY finance_host_lobby_end ON df_runtime.table_sessions
    FOR UPDATE TO df_finance_definer
    USING (phase IN ('lobby', 'ended')) WITH CHECK (phase = 'ended');
