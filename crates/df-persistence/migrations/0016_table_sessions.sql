-- Table sessions and meter segments (planning/hosted-runtime-research.md 5.2, 6.2; O1).
-- The segment tables and the lease renewal that rolls them. Transitions, credits and feature
-- flags arrive with the lifecycle task. Times are database clock readings only.

CREATE TABLE df_runtime.table_sessions (
    tenant_id bytea NOT NULL CHECK (octet_length(tenant_id) = 16),
    campaign_id bytea NOT NULL CHECK (octet_length(campaign_id) = 16),
    session_id bytea NOT NULL CHECK (octet_length(session_id) = 16),
    phase text NOT NULL CHECK (phase IN ('scheduled', 'lobby', 'preparing', 'active', 'paused',
        'settling', 'ended')),
    created_at timestamptz NOT NULL,
    ended_at timestamptz,
    PRIMARY KEY (tenant_id, session_id),
    CHECK ((phase = 'ended') = (ended_at IS NOT NULL))
);
CREATE INDEX table_sessions_campaign ON df_runtime.table_sessions (tenant_id, campaign_id);
CREATE UNIQUE INDEX table_sessions_one_open ON df_runtime.table_sessions (tenant_id, campaign_id)
    WHERE phase <> 'ended';

CREATE TABLE df_runtime.meter_segments (
    segment_id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    tenant_id bytea NOT NULL,
    session_id bytea NOT NULL,
    opened_at timestamptz NOT NULL,
    closed_at timestamptz,
    close_reason text CHECK (close_reason IN ('roll', 'owner_lost', 'release', 'owner_exit')),
    FOREIGN KEY (tenant_id, session_id) REFERENCES df_runtime.table_sessions,
    CHECK (closed_at IS NULL OR closed_at >= opened_at),
    CHECK ((closed_at IS NULL) = (close_reason IS NULL))
);
CREATE UNIQUE INDEX meter_segments_one_open ON df_runtime.meter_segments (tenant_id, session_id)
    WHERE closed_at IS NULL;

-- Closes the open segment of every session of the campaign at the last proven progress, never
-- before the segment opened, so the tail of a crash or stall is never priced (5.2).
CREATE FUNCTION df_runtime.close_open_segments(
    p_tenant_id bytea, p_campaign_id bytea, p_progress_at timestamptz, p_reason text) RETURNS void
LANGUAGE sql VOLATILE
SET search_path = pg_catalog, df_runtime, pg_temp
AS $$
    UPDATE df_runtime.meter_segments s
       SET closed_at = GREATEST(s.opened_at, COALESCE(p_progress_at, s.opened_at)),
           close_reason = p_reason
      FROM df_runtime.table_sessions t
     WHERE t.tenant_id = p_tenant_id AND t.campaign_id = p_campaign_id
       AND s.tenant_id = t.tenant_id AND s.session_id = t.session_id
       AND s.closed_at IS NULL
$$;

-- The time through which the table's funding covers metered time, or NULL when none does.
-- TODO(F): replace the body with a call to finance.meter_funded_until, F's lock-free read.
-- Until then no segment is ever reopened by a roll, so the meter stays free, never over-charged.
CREATE FUNCTION df_runtime.meter_funded_until(
    p_tenant_id bytea, p_campaign_id bytea, p_session_id bytea) RETURNS timestamptz
LANGUAGE sql STABLE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, pg_temp
AS $$ SELECT NULL::timestamptz $$;

-- The lease renewal (6.2), by the lifecycle task. p_progressed moves last_progress_at; p_roll
-- also closes each open segment at GREATEST(opened_at, last_progress_at) and opens the next
-- one when funding covers another interval. The roll runs in a sub-block, so an error in it
-- loses only the roll (out_roll_failed, one NOTICE) and never the lease renewal.
CREATE FUNCTION df_runtime.renew(
    p_tenant_id bytea, p_campaign_id bytea, p_recovery_epoch bigint, p_claim_seq bigint,
    p_placement_id bytea, p_lease_seconds integer, p_progressed boolean,
    p_roll boolean, p_segment_roll_seconds integer)
RETURNS TABLE (out_lease_until timestamptz, out_last_progress_at timestamptz,
               out_rolled boolean, out_roll_failed boolean, out_funding_required boolean)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, df_meta, pg_temp
AS $$
DECLARE
    v_renewed df_runtime.campaign_owners%ROWTYPE;
    v_fence bytea;
    v_updated bigint;
    v_open record;
    v_roll_at timestamptz;
    v_funded_until timestamptz;
BEGIN
    PERFORM df_runtime.require_service();
    PERFORM df_runtime.require_seconds(p_lease_seconds, 'lease');
    IF p_roll THEN
        PERFORM df_runtime.require_seconds(p_segment_roll_seconds, 'segment roll');
    END IF;
    IF NOT df_runtime.caller_holds_placement(p_placement_id) THEN
        RETURN;
    END IF;
    UPDATE df_runtime.campaign_owners o
       SET lease_until = clock_timestamp() + make_interval(secs => p_lease_seconds),
           last_progress_at = CASE WHEN p_progressed THEN clock_timestamp()
                                   ELSE o.last_progress_at END
     WHERE o.tenant_id = p_tenant_id AND o.campaign_id = p_campaign_id
       AND o.recovery_epoch = p_recovery_epoch AND o.claim_seq = p_claim_seq
       AND o.placement_id = p_placement_id AND o.state IN ('serving', 'draining')
       AND o.lease_until > clock_timestamp() AND df_meta.is_serving()
    RETURNING * INTO v_renewed;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    UPDATE df_runtime.placement_secrets SET lease_until = v_renewed.lease_until
     WHERE placement_id = p_placement_id AND is_current;
    GET DIAGNOSTICS v_updated = ROW_COUNT;
    IF v_updated <> 1 THEN
        RAISE EXCEPTION 'placement secret not current for a serving owner' USING ERRCODE = 'DF004';
    END IF;
    v_fence := df_runtime.fence_digest(p_recovery_epoch, p_claim_seq, p_placement_id);
    PERFORM df_runtime.project_owner(
        p_tenant_id, p_campaign_id, v_fence, v_fence, v_renewed.lease_until);
    out_lease_until := v_renewed.lease_until;
    out_last_progress_at := v_renewed.last_progress_at;
    out_rolled := false;
    out_roll_failed := false;
    out_funding_required := false;
    IF p_roll THEN
        BEGIN
            FOR v_open IN
                SELECT s.segment_id, s.session_id, s.opened_at
                  FROM df_runtime.table_sessions t
                  JOIN df_runtime.meter_segments s
                    ON s.tenant_id = t.tenant_id AND s.session_id = t.session_id
                 WHERE t.tenant_id = p_tenant_id AND t.campaign_id = p_campaign_id
                   AND s.closed_at IS NULL
                   FOR UPDATE OF s
            LOOP
                v_roll_at := GREATEST(v_open.opened_at, v_renewed.last_progress_at);
                UPDATE df_runtime.meter_segments
                   SET closed_at = v_roll_at, close_reason = 'roll'
                 WHERE segment_id = v_open.segment_id;
                v_funded_until := df_runtime.meter_funded_until(
                    p_tenant_id, p_campaign_id, v_open.session_id);
                IF v_funded_until IS NOT NULL
                   AND v_funded_until >= v_roll_at + make_interval(secs => p_segment_roll_seconds) THEN
                    INSERT INTO df_runtime.meter_segments (tenant_id, session_id, opened_at)
                    VALUES (p_tenant_id, v_open.session_id, v_roll_at);
                    out_rolled := true;
                ELSE
                    out_funding_required := true;
                END IF;
            END LOOP;
        EXCEPTION WHEN OTHERS THEN
            RAISE NOTICE 'meter roll failed: %', SQLSTATE;
            out_rolled := false;
            out_funding_required := false;
            out_roll_failed := true;
        END;
    END IF;
    RETURN NEXT;
END $$;

-- The table sessions of the campaign the caller's placement token leases (4.4); the scope
-- row security uses for every table keyed by a table session.
CREATE FUNCTION df_runtime.leased_table_sessions()
RETURNS TABLE (tenant_id bytea, session_id bytea)
LANGUAGE sql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, pg_temp
AS $$
    SELECT t.tenant_id, t.session_id
      FROM df_runtime.leased_campaign() l
      JOIN df_runtime.table_sessions t
        ON t.tenant_id = l.tenant_id AND t.campaign_id = l.campaign_id
$$;
