-- Hosted runtime directory (planning/hosted-runtime-research.md 4, 6.1, 6.2, 10.2, 10.4; O1).
-- Applied by the migration runner as df_migrator, in one transaction, after 0001 and 0003.
-- Tables and function bodies only: 0018 creates the roles, transfers ownership to the NOLOGIN
-- definer role, forces row security and grants execution. A fenced function never waits on the
-- service halt (require_service uses a try-lock) and never reads pg_locks.

CREATE SCHEMA IF NOT EXISTS df_meta;
CREATE SCHEMA IF NOT EXISTS df_runtime;
REVOKE ALL ON SCHEMA df_meta FROM PUBLIC;
REVOKE ALL ON SCHEMA df_runtime FROM PUBLIC;

-- The cluster-wide serving switch. The halt (6.3) takes the service lock exclusively, then
-- flips serving; recovery_epoch is raised only by the restore procedure, which leaves the state
-- 'restoring' (every directory function and the fence refuse) until F's replay completes.
CREATE TABLE df_meta.service_state (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    serving text NOT NULL CHECK (serving IN ('serving', 'restoring', 'halted_regression')),
    recovery_epoch bigint NOT NULL CHECK (recovery_epoch >= 1),
    checked_at timestamptz NOT NULL,
    -- The two-int advisory lock key of the service lock. Random, readable by no service role,
    -- and replaced by every restore: a runtime that learns it (pg_locks shows held advisory
    -- keys) loses it at the next restore, and the halt terminates what holds the lock anyway.
    lock_key_high integer NOT NULL CHECK (lock_key_high > 0),
    lock_key_low integer NOT NULL CHECK (lock_key_low > 0)
);
INSERT INTO df_meta.service_state
    (serving, recovery_epoch, checked_at, lock_key_high, lock_key_low)
VALUES ('serving', 1, clock_timestamp(), 1 + floor(random() * 2147483646)::integer,
        1 + floor(random() * 2147483646)::integer);

CREATE TABLE df_runtime.workers (
    worker_id bytea PRIMARY KEY CHECK (octet_length(worker_id) = 16),
    name text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 64),
    slots_total integer NOT NULL CHECK (slots_total >= 0),
    started_at timestamptz NOT NULL,
    last_heartbeat_at timestamptz NOT NULL
);

CREATE TABLE df_runtime.campaign_owners (
    tenant_id bytea NOT NULL CHECK (octet_length(tenant_id) = 16),
    campaign_id bytea NOT NULL CHECK (octet_length(campaign_id) = 16),
    recovery_epoch bigint NOT NULL CHECK (recovery_epoch >= 1),
    claim_seq bigint NOT NULL CHECK (claim_seq >= 0),
    state text NOT NULL CHECK (state IN ('released', 'placing', 'serving', 'draining')),
    placement_id bytea CHECK (placement_id IS NULL OR octet_length(placement_id) = 16),
    worker_id bytea REFERENCES df_runtime.workers,
    placement_lease_until timestamptz,
    runtime_endpoint text CHECK (char_length(runtime_endpoint) <= 256),
    lease_until timestamptz NOT NULL,
    last_progress_at timestamptz,
    released_cleanly boolean NOT NULL DEFAULT true,
    money_state text NOT NULL DEFAULT 'reconciled'
        CHECK (money_state IN ('reconciled', 'reconstructing', 'frozen')),
    reconstruction_id bytea CHECK (reconstruction_id IS NULL OR octet_length(reconstruction_id) = 16),
    PRIMARY KEY (tenant_id, campaign_id),
    CHECK ((state = 'placing') = (placement_lease_until IS NOT NULL)),
    CHECK (state = 'released' OR placement_id IS NOT NULL)
);

CREATE TABLE df_runtime.placements (
    placement_id bytea PRIMARY KEY CHECK (octet_length(placement_id) = 16),
    tenant_id bytea NOT NULL CHECK (octet_length(tenant_id) = 16),
    campaign_id bytea NOT NULL CHECK (octet_length(campaign_id) = 16),
    worker_id bytea NOT NULL REFERENCES df_runtime.workers,
    created_at timestamptz NOT NULL,
    ended_at timestamptz,
    end_reason text CHECK (end_reason IN ('superseded', 'released', 'exited')),
    CHECK ((ended_at IS NULL) = (end_reason IS NULL))
);
CREATE UNIQUE INDEX placements_one_live ON df_runtime.placements (tenant_id, campaign_id)
    WHERE ended_at IS NULL;

-- The lease state a placement token is checked against. No row security and no grant to any
-- login role: only the definer functions read it (4.4 item 1). token_digest is the SHA-256 of
-- the 32 raw token bytes.
CREATE TABLE df_runtime.placement_secrets (
    placement_id bytea PRIMARY KEY REFERENCES df_runtime.placements,
    tenant_id bytea NOT NULL CHECK (octet_length(tenant_id) = 16),
    campaign_id bytea NOT NULL CHECK (octet_length(campaign_id) = 16),
    token_digest bytea NOT NULL UNIQUE CHECK (octet_length(token_digest) = 32),
    recovery_epoch bigint NOT NULL CHECK (recovery_epoch >= 1),
    claim_seq bigint NOT NULL CHECK (claim_seq >= 0),
    is_current boolean NOT NULL,
    lease_until timestamptz NOT NULL
);
CREATE UNIQUE INDEX placement_secrets_one_current
    ON df_runtime.placement_secrets (tenant_id, campaign_id) WHERE is_current;

-- Which df_game sessions belong to a campaign; written where hosted sessions are created.
CREATE TABLE df_runtime.campaign_game_sessions (
    tenant_id bytea NOT NULL CHECK (octet_length(tenant_id) = 16),
    campaign_id bytea NOT NULL CHECK (octet_length(campaign_id) = 16),
    game_session_id bytea NOT NULL CHECK (octet_length(game_session_id) = 16),
    PRIMARY KEY (tenant_id, game_session_id)
);
CREATE INDEX campaign_game_sessions_campaign
    ON df_runtime.campaign_game_sessions (tenant_id, campaign_id);

-- A player or host asked for a table that has no live owner; runners take these (7).
CREATE TABLE df_runtime.placement_requests (
    tenant_id bytea NOT NULL CHECK (octet_length(tenant_id) = 16),
    campaign_id bytea NOT NULL CHECK (octet_length(campaign_id) = 16),
    requested_at timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, campaign_id)
);

-- One row per egress dispatcher process start (10.2). token_digest is the SHA-256 of the
-- 32 raw bytes of the token the process sets as df.dispatcher_token.
CREATE TABLE df_runtime.dispatcher_instances (
    instance_id bytea PRIMARY KEY CHECK (octet_length(instance_id) = 16),
    worker_id bytea NOT NULL REFERENCES df_runtime.workers,
    token_digest bytea NOT NULL UNIQUE CHECK (octet_length(token_digest) = 32),
    started_at timestamptz NOT NULL,
    last_seen_at timestamptz NOT NULL,
    ended_at timestamptz
);

-- One row per offline import of a local campaign (decision 19): the importer presents the job's
-- token as df.import_token, exactly as a runtime presents its placement token, and sees and
-- writes only that campaign. Like placement_secrets: no row security, granted to nobody; only
-- the definer functions read it. The operator opens and closes jobs.
CREATE TABLE df_runtime.import_jobs (
    import_token_digest bytea PRIMARY KEY CHECK (octet_length(import_token_digest) = 32),
    tenant_id bytea NOT NULL CHECK (octet_length(tenant_id) = 16),
    campaign_id bytea NOT NULL CHECK (octet_length(campaign_id) = 16),
    created_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL CHECK (expires_at > created_at)
);

-- Re-fetch of provider output the object store never received (10.4, ruling (a)).
CREATE TABLE df_runtime.refetch_requests (
    attempt_id bytea PRIMARY KEY CHECK (octet_length(attempt_id) = 16),
    requested_at timestamptz NOT NULL,
    request_count integer NOT NULL CHECK (request_count >= 1),
    attempts integer NOT NULL CHECK (attempts >= 0),
    claimed_by_instance bytea CHECK (octet_length(claimed_by_instance) = 16),
    claimed_until timestamptz,
    outcome text CHECK (outcome IN ('staged', 'already_staged', 'handle_expired',
        'attempt_not_current', 'failed', 'exhausted')),
    finished_at timestamptz,
    CHECK ((outcome IS NULL) = (finished_at IS NULL)),
    CHECK ((claimed_by_instance IS NULL) = (claimed_until IS NULL))
);

CREATE TYPE df_runtime.fence_path AS ENUM ('game', 'paid');

-- Constants and small helpers.

-- Takes the service lock exclusively for the rest of the transaction (the halt and the restore,
-- 6.1): each attempt waits p_lock_timeout_ms; after p_attempts failed attempts it terminates the
-- backends that hold the lock, since a service lock held past that is a stuck or hostile
-- session (any session can lift its own statement and idle timeouts), then tries again.
-- Terminating needs pg_signal_backend (0018). Raises when the lock is still not free.
CREATE FUNCTION df_meta.acquire_exclusive_service_lock(
    p_lock_timeout_ms integer, p_attempts integer) RETURNS void
LANGUAGE plpgsql VOLATILE
SET search_path = pg_catalog, df_meta, pg_temp
AS $$
DECLARE
    v_high integer;
    v_low integer;
    v_caller_timeout text := current_setting('lock_timeout');
    v_attempt integer := 0;
    v_terminated boolean := false;
BEGIN
    IF p_lock_timeout_ms NOT BETWEEN 1 AND 60000 OR p_attempts NOT BETWEEN 1 AND 20 THEN
        RAISE EXCEPTION 'lock timeout or attempts out of range' USING ERRCODE = '22023';
    END IF;
    SELECT s.lock_key_high, s.lock_key_low INTO v_high, v_low FROM df_meta.service_state s;
    LOOP
        v_attempt := v_attempt + 1;
        PERFORM set_config('lock_timeout', p_lock_timeout_ms::text || 'ms', true);
        BEGIN
            PERFORM pg_advisory_xact_lock(v_high, v_low);
            PERFORM set_config('lock_timeout', v_caller_timeout, true);
            RETURN;
        EXCEPTION WHEN lock_not_available THEN
            IF v_attempt >= p_attempts THEN
                IF v_terminated THEN
                    RAISE EXCEPTION 'service lock still held after terminating its holders'
                        USING ERRCODE = 'DF002';
                END IF;
                PERFORM pg_terminate_backend(l.pid)
                   FROM pg_locks l
                  WHERE l.locktype = 'advisory' AND l.granted
                    AND l.classid = v_high::oid AND l.objid = v_low::oid AND l.objsubid = 2
                    AND l.pid <> pg_backend_pid();
                v_terminated := true;
                v_attempt := 0;
            END IF;
        END;
    END LOOP;
END $$;

-- Replaces the service lock key with a new random one. Called while the lock is held
-- exclusively, so no fenced transaction straddles the change.
CREATE FUNCTION df_meta.rotate_service_lock_key() RETURNS void
LANGUAGE sql VOLATILE
SET search_path = pg_catalog, df_meta, pg_temp
AS $$
    UPDATE df_meta.service_state
       SET lock_key_high = 1 + floor(random() * 2147483646)::integer,
           lock_key_low = 1 + floor(random() * 2147483646)::integer
$$;

-- The control plane's remedy for a hostile or stuck hold of the service lock: fenced calls
-- keep raising DF002 while the state says 'serving'. Takes the lock exclusively (terminating
-- the holders after the retries) and replaces the key; the epoch and the state are untouched.
-- Residual: a compromised runtime can read the new key from pg_locks and hold it again, so it
-- can keep causing DF002 until its credentials are cut off. Granted to df_control only; call
-- it as the first statement of its own transaction (it cannot upgrade a shared hold).
CREATE FUNCTION df_runtime.break_service_lock(
    p_lock_timeout_ms integer DEFAULT 2000, p_attempts integer DEFAULT 3) RETURNS void
LANGUAGE plpgsql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, df_meta, pg_temp
AS $$
BEGIN
    PERFORM df_meta.acquire_exclusive_service_lock(p_lock_timeout_ms, p_attempts);
    PERFORM df_meta.rotate_service_lock_key();
END $$;

-- The halt (6.3): breaks the lock as above and sets halted_regression, so every directory
-- function and the fence refuse until the restore procedure raises the epoch. The epoch is
-- not touched. False when the service was not 'serving' (a restore in progress stays so).
CREATE FUNCTION df_runtime.halt_service(
    p_lock_timeout_ms integer DEFAULT 2000, p_attempts integer DEFAULT 3) RETURNS boolean
LANGUAGE plpgsql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, df_meta, pg_temp
AS $$
DECLARE
    v_updated bigint;
BEGIN
    PERFORM df_meta.acquire_exclusive_service_lock(p_lock_timeout_ms, p_attempts);
    PERFORM df_meta.rotate_service_lock_key();
    UPDATE df_meta.service_state
       SET serving = 'halted_regression', checked_at = clock_timestamp()
     WHERE serving = 'serving';
    GET DIAGNOSTICS v_updated = ROW_COUNT;
    RETURN v_updated = 1;
END $$;

-- A prepared transaction keeps its advisory locks after its session ends and cannot be broken
-- by terminating a backend, so the service refuses to run where they are possible.
CREATE FUNCTION df_meta.refuse_prepared_transactions(p_maximum integer, p_pending bigint)
RETURNS void
LANGUAGE plpgsql IMMUTABLE
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF p_maximum <> 0 OR p_pending <> 0 THEN
        RAISE EXCEPTION 'prepared transactions must be disabled (max_prepared_transactions = %, pending %)',
            p_maximum, p_pending USING ERRCODE = 'DF006';
    END IF;
END $$;

-- Every service process calls this at startup (and this migration calls it once).
CREATE FUNCTION df_runtime.require_no_prepared_transactions() RETURNS void
LANGUAGE sql STABLE SECURITY DEFINER
SET search_path = pg_catalog, df_meta, pg_temp
AS $$
    SELECT df_meta.refuse_prepared_transactions(
        current_setting('max_prepared_transactions')::integer,
        (SELECT count(*) FROM pg_prepared_xacts))
$$;

CREATE FUNCTION df_runtime.token_digest(p_token text) RETURNS bytea
LANGUAGE sql IMMUTABLE STRICT PARALLEL SAFE
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT CASE WHEN p_token ~ '^[0-9a-f]{64}$' THEN sha256(decode(p_token, 'hex')) END
$$;

-- First 16 bytes of SHA-256 over recovery_epoch || claim_seq || placement_id, big-endian: the
-- value df_game.sessions.owner_fence holds while this fence is current.
CREATE FUNCTION df_runtime.fence_digest(
    p_recovery_epoch bigint, p_claim_seq bigint, p_placement_id bytea) RETURNS bytea
LANGUAGE sql IMMUTABLE STRICT PARALLEL SAFE
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT substring(
        sha256(int8send(p_recovery_epoch) || int8send(p_claim_seq) || p_placement_id)
        FROM 1 FOR 16)
$$;

CREATE FUNCTION df_runtime.require_seconds(p_seconds integer, p_label text) RETURNS void
LANGUAGE plpgsql IMMUTABLE
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF p_seconds IS NULL OR p_seconds NOT BETWEEN 1 AND 3600 THEN
        RAISE EXCEPTION '% must be between 1 and 3600 seconds', p_label USING ERRCODE = '22023';
    END IF;
END $$;

CREATE FUNCTION df_meta.is_serving() RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER
SET search_path = pg_catalog, df_meta, pg_temp
AS $$ SELECT COALESCE((SELECT serving = 'serving' FROM df_meta.service_state), false) $$;

CREATE FUNCTION df_runtime.current_recovery_epoch() RETURNS bigint
LANGUAGE sql STABLE SECURITY DEFINER
SET search_path = pg_catalog, df_meta, pg_temp
AS $$ SELECT recovery_epoch FROM df_meta.service_state $$;

-- Lock-free read of the serving state and recovery epoch, for the finance service and the
-- control plane (no service lock, no row lock).
CREATE FUNCTION df_runtime.service_state()
RETURNS TABLE (out_serving text, out_recovery_epoch bigint)
LANGUAGE sql STABLE SECURITY DEFINER
SET search_path = pg_catalog, df_meta, pg_temp
AS $$ SELECT serving, recovery_epoch FROM df_meta.service_state $$;

-- The operator's restore step (6.3): takes the service lock exclusively (terminating a stuck
-- holder, see acquire_exclusive_service_lock), raises the recovery epoch strictly, replaces the
-- service lock key and leaves the service 'restoring', so every directory function and the
-- fence refuse until the finance replay has completed. Every owner below the new epoch loses
-- its lease at once, is marked as not released cleanly, and its projected game-session lease
-- is expired too: a restore can bring back an unexpired lease at an old epoch, and nothing
-- else would force that owner to reconstruct. False when the epoch is not greater than the
-- current one. Granted to no service role.
CREATE FUNCTION df_runtime.begin_restore(
    p_recovery_epoch bigint, p_lock_timeout_ms integer DEFAULT 2000,
    p_attempts integer DEFAULT 3) RETURNS boolean
LANGUAGE plpgsql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, df_meta, pg_temp
AS $$
DECLARE
    v_updated bigint;
BEGIN
    PERFORM df_meta.acquire_exclusive_service_lock(p_lock_timeout_ms, p_attempts);
    UPDATE df_meta.service_state
       SET serving = 'restoring', recovery_epoch = p_recovery_epoch,
           checked_at = clock_timestamp(),
           lock_key_high = 1 + floor(random() * 2147483646)::integer,
           lock_key_low = 1 + floor(random() * 2147483646)::integer
     WHERE recovery_epoch < p_recovery_epoch;
    GET DIAGNOSTICS v_updated = ROW_COUNT;
    IF v_updated <> 1 THEN
        RETURN false;
    END IF;
    UPDATE df_game.sessions g
       SET lease_until = LEAST(g.lease_until, clock_timestamp())
      FROM df_runtime.campaign_game_sessions m
      JOIN df_runtime.campaign_owners o
        ON o.tenant_id = m.tenant_id AND o.campaign_id = m.campaign_id
     WHERE o.recovery_epoch < p_recovery_epoch
       AND g.tenant_id = m.tenant_id AND g.session_id = m.game_session_id;
    UPDATE df_runtime.placement_secrets s
       SET lease_until = LEAST(s.lease_until, clock_timestamp())
      FROM df_runtime.campaign_owners o
     WHERE o.tenant_id = s.tenant_id AND o.campaign_id = s.campaign_id
       AND o.recovery_epoch < p_recovery_epoch;
    UPDATE df_runtime.campaign_owners o
       SET lease_until = LEAST(o.lease_until, clock_timestamp()),
           placement_lease_until = CASE WHEN o.placement_lease_until IS NULL THEN NULL
                                        ELSE LEAST(o.placement_lease_until, clock_timestamp()) END,
           released_cleanly = false
     WHERE o.recovery_epoch < p_recovery_epoch;
    RETURN true;
END $$;

-- F's replay moves 'restoring' to 'serving' once its post-replay check passed, in the same
-- transaction that marks the replay complete. Takes no service lock: the replay may already
-- hold the shared one. False unless the service is restoring at exactly this epoch.
CREATE FUNCTION df_runtime.restore_complete(p_recovery_epoch bigint) RETURNS boolean
LANGUAGE plpgsql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_meta, pg_temp
AS $$
DECLARE
    v_updated bigint;
BEGIN
    UPDATE df_meta.service_state
       SET serving = 'serving', checked_at = clock_timestamp()
     WHERE serving = 'restoring' AND recovery_epoch = p_recovery_epoch;
    GET DIAGNOSTICS v_updated = ROW_COUNT;
    RETURN v_updated = 1;
END $$;

-- The first statement of every transaction that uses the fence or the directory, before any
-- row lock (6.1): waits at most the caller's lock_timeout for a halt that holds the lock.
CREATE FUNCTION df_runtime.enter_service() RETURNS void
LANGUAGE sql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, df_meta, pg_temp
AS $$ SELECT pg_advisory_xact_lock_shared(s.lock_key_high, s.lock_key_low)
        FROM df_meta.service_state s $$;

-- Called first by every fenced function and never waits: an already held shared lock returns
-- at once; otherwise the lock is taken only when no halt holds or awaits it (DF002).
CREATE FUNCTION df_runtime.require_service() RETURNS void
LANGUAGE plpgsql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, df_meta, pg_temp
AS $$
DECLARE
    v_high integer;
    v_low integer;
BEGIN
    SELECT s.lock_key_high, s.lock_key_low INTO v_high, v_low FROM df_meta.service_state s;
    IF NOT pg_try_advisory_xact_lock_shared(v_high, v_low) THEN
        RAISE EXCEPTION 'service halting' USING ERRCODE = 'DF002';
    END IF;
END $$;

-- True when the token in df.placement_token is the digest recorded for this placement.
CREATE FUNCTION df_runtime.caller_holds_placement(p_placement_id bytea) RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, pg_temp
AS $$
    SELECT EXISTS (
        SELECT 1 FROM df_runtime.placement_secrets s
         WHERE s.placement_id = p_placement_id
           AND s.token_digest = df_runtime.token_digest(
               current_setting('df.placement_token', true)))
$$;

-- The three-part fence (6.1). 'game' covers game-side writes; 'paid' also needs a reconciled
-- money state. Never raises on a stale fence: false means refuse.
CREATE FUNCTION df_runtime.fence_current(
    p_tenant_id bytea, p_campaign_id bytea,
    p_recovery_epoch bigint, p_claim_seq bigint, p_placement_id bytea,
    p_path df_runtime.fence_path) RETURNS boolean
LANGUAGE plpgsql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, df_meta, pg_temp
AS $$
BEGIN
    PERFORM df_runtime.require_service();
    RETURN EXISTS (
        SELECT 1 FROM df_meta.service_state s, df_runtime.campaign_owners o
         WHERE s.serving = 'serving'
           AND o.tenant_id = p_tenant_id AND o.campaign_id = p_campaign_id
           AND o.recovery_epoch = p_recovery_epoch AND o.claim_seq = p_claim_seq
           AND o.placement_id = p_placement_id
           AND o.state IN ('serving', 'draining') AND o.lease_until > clock_timestamp()
           AND (p_path = 'game' OR o.money_state = 'reconciled')
         FOR SHARE OF o);
END $$;

-- The scope of the caller's placement token (4.4): the campaign it currently leases, or no row.
-- A placing placement leases for the claim itself.
CREATE FUNCTION df_runtime.leased_campaign()
RETURNS TABLE (tenant_id bytea, campaign_id bytea)
LANGUAGE sql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, pg_temp
AS $$
    SELECT s.tenant_id, s.campaign_id
      FROM df_runtime.placement_secrets s
     WHERE s.is_current AND s.lease_until > clock_timestamp()
       AND s.token_digest = df_runtime.token_digest(
           current_setting('df.placement_token', true))
$$;

CREATE FUNCTION df_runtime.leased_game_sessions()
RETURNS TABLE (tenant_id bytea, session_id bytea)
LANGUAGE sql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, pg_temp
AS $$
    SELECT m.tenant_id, m.game_session_id
      FROM df_runtime.leased_campaign() l
      JOIN df_runtime.campaign_game_sessions m
        ON m.tenant_id = l.tenant_id AND m.campaign_id = l.campaign_id
$$;

-- Registers a game session of the leased campaign so row security admits its rows (4.4).
-- False when the id already belongs to another campaign.
CREATE FUNCTION df_runtime.register_game_session(p_game_session_id bytea) RETURNS boolean
LANGUAGE plpgsql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, pg_temp
AS $$
DECLARE
    v_tenant bytea;
    v_campaign bytea;
BEGIN
    SELECT l.tenant_id, l.campaign_id INTO v_tenant, v_campaign FROM df_runtime.leased_campaign() l;
    IF v_tenant IS NULL THEN
        RETURN false;
    END IF;
    INSERT INTO df_runtime.campaign_game_sessions (tenant_id, campaign_id, game_session_id)
    VALUES (v_tenant, v_campaign, p_game_session_id)
    ON CONFLICT DO NOTHING;
    RETURN EXISTS (
        SELECT 1 FROM df_runtime.campaign_game_sessions m
         WHERE m.tenant_id = v_tenant AND m.game_session_id = p_game_session_id
           AND m.campaign_id = v_campaign);
END $$;

-- Whether the campaign has a live owner: placing within its placing lease, or serving or
-- draining within its lease. An import never overlaps one (decision 19).
CREATE FUNCTION df_runtime.owner_is_live(p_tenant_id bytea, p_campaign_id bytea) RETURNS boolean
LANGUAGE sql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, pg_temp
AS $$
    SELECT EXISTS (
        SELECT 1 FROM df_runtime.campaign_owners o
         WHERE o.tenant_id = p_tenant_id AND o.campaign_id = p_campaign_id
           AND ((o.state = 'placing' AND o.placement_lease_until > clock_timestamp())
                OR (o.state IN ('serving', 'draining') AND o.lease_until > clock_timestamp())))
$$;

-- Import jobs. Operator-only (granted to no service role): open a job for one campaign with the
-- digest of its token, and close it when the import is done. A job opens only on a campaign
-- with no live owner; it takes the owner row lock that `place` takes, so a placement and a
-- job cannot both succeed, and `place` refuses while a job is unexpired.
CREATE FUNCTION df_runtime.open_import(
    p_tenant_id bytea, p_campaign_id bytea, p_import_token_digest bytea,
    p_valid_seconds integer) RETURNS void
LANGUAGE plpgsql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, pg_temp
AS $$
BEGIN
    PERFORM df_runtime.require_seconds(p_valid_seconds, 'import job');
    INSERT INTO df_runtime.campaign_owners
        (tenant_id, campaign_id, recovery_epoch, claim_seq, state, lease_until)
    VALUES (p_tenant_id, p_campaign_id, df_runtime.current_recovery_epoch(), 0, 'released',
            '-infinity')
    ON CONFLICT DO NOTHING;
    PERFORM 1 FROM df_runtime.campaign_owners o
     WHERE o.tenant_id = p_tenant_id AND o.campaign_id = p_campaign_id FOR UPDATE;
    IF df_runtime.owner_is_live(p_tenant_id, p_campaign_id) THEN
        RAISE EXCEPTION 'campaign has a live owner; no import into it' USING ERRCODE = 'DF007';
    END IF;
    INSERT INTO df_runtime.import_jobs
        (import_token_digest, tenant_id, campaign_id, created_at, expires_at)
    VALUES (p_import_token_digest, p_tenant_id, p_campaign_id, clock_timestamp(),
            clock_timestamp() + make_interval(secs => p_valid_seconds));
END $$;

CREATE FUNCTION df_runtime.close_import(p_import_token_digest bytea) RETURNS boolean
LANGUAGE sql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, pg_temp
AS $$
    WITH closed AS (
        DELETE FROM df_runtime.import_jobs WHERE import_token_digest = p_import_token_digest
        RETURNING 1)
    SELECT EXISTS (SELECT 1 FROM closed)
$$;

-- Retention: jobs that were never closed are removed once they expired more than
-- p_retention_seconds ago (the control-plane sweeper passes its configured retention).
CREATE FUNCTION df_runtime.purge_import_jobs(
    p_retention_seconds integer DEFAULT 604800, p_limit integer DEFAULT 1000)
RETURNS bigint
LANGUAGE sql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, pg_temp
AS $$
    WITH batch AS (
        SELECT ctid FROM df_runtime.import_jobs
         WHERE expires_at < clock_timestamp()
                            - make_interval(secs => GREATEST(p_retention_seconds, 0))
         ORDER BY expires_at, ctid
         LIMIT LEAST(GREATEST(p_limit, 1), 1000)
         FOR UPDATE SKIP LOCKED),
    purged AS (
        DELETE FROM df_runtime.import_jobs AS jobs
         USING batch WHERE jobs.ctid = batch.ctid
        RETURNING 1)
    SELECT count(*) FROM purged
$$;

-- The campaign the caller's import token names while its job is open and the campaign has no
-- live owner, or no row.
CREATE FUNCTION df_runtime.importing_campaign()
RETURNS TABLE (tenant_id bytea, campaign_id bytea)
LANGUAGE sql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, pg_temp
AS $$
    SELECT j.tenant_id, j.campaign_id
      FROM df_runtime.import_jobs j
     WHERE j.expires_at > clock_timestamp()
       AND j.import_token_digest = df_runtime.token_digest(
           current_setting('df.import_token', true))
       AND NOT df_runtime.owner_is_live(j.tenant_id, j.campaign_id)
$$;

CREATE FUNCTION df_runtime.importing_game_sessions()
RETURNS TABLE (tenant_id bytea, session_id bytea)
LANGUAGE sql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, pg_temp
AS $$
    SELECT m.tenant_id, m.game_session_id
      FROM df_runtime.importing_campaign() i
      JOIN df_runtime.campaign_game_sessions m
        ON m.tenant_id = i.tenant_id AND m.campaign_id = i.campaign_id
$$;

-- The importer's way to create a game session: it maps the session to the job's campaign and
-- writes the row with an expired lease and a placeholder fence, so an import never writes a
-- live owner fence; the first claim of the campaign takes the session over. False when the
-- caller has no open job, or the session id already belongs to another campaign.
CREATE FUNCTION df_runtime.import_game_session(
    p_game_session_id bytea, p_run_id bytea, p_recovery_epoch numeric,
    p_in_epoch_sequence numeric) RETURNS boolean
LANGUAGE plpgsql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, pg_temp
AS $$
DECLARE
    v_tenant bytea;
    v_campaign bytea;
BEGIN
    SELECT i.tenant_id, i.campaign_id INTO v_tenant, v_campaign
      FROM df_runtime.importing_campaign() i;
    IF v_tenant IS NULL THEN
        RETURN false;
    END IF;
    INSERT INTO df_runtime.campaign_game_sessions (tenant_id, campaign_id, game_session_id)
    VALUES (v_tenant, v_campaign, p_game_session_id)
    ON CONFLICT DO NOTHING;
    IF NOT EXISTS (SELECT 1 FROM df_runtime.campaign_game_sessions m
                    WHERE m.tenant_id = v_tenant AND m.game_session_id = p_game_session_id
                      AND m.campaign_id = v_campaign) THEN
        RETURN false;
    END IF;
    INSERT INTO df_game.sessions
        (tenant_id, session_id, run_id, recovery_epoch, in_epoch_sequence, owner_fence,
         lease_until)
    VALUES (v_tenant, p_game_session_id, p_run_id, p_recovery_epoch, p_in_epoch_sequence,
            substring(sha256(p_game_session_id || p_run_id) FROM 1 FOR 16), '-infinity');
    RETURN true;
END $$;

-- Rewrites the df_game.sessions projection of every session of the campaign (6.2). A
-- projection that updates fewer rows than the mapping lists raises DF004, so a silent no-op
-- cannot pass. p_current_fence is NULL only for a claim, which takes the sessions over.
CREATE FUNCTION df_runtime.project_owner(
    p_tenant_id bytea, p_campaign_id bytea, p_current_fence bytea, p_new_fence bytea,
    p_lease_until timestamptz) RETURNS void
LANGUAGE plpgsql VOLATILE
SET search_path = pg_catalog, df_runtime, df_meta, pg_temp
AS $$
DECLARE
    v_expected bigint;
    v_updated bigint;
BEGIN
    -- Only mapped sessions that have a game row count: a runtime that registered a session and
    -- failed before inserting its row leaves a mapping that must not wedge claims and renewals.
    SELECT count(*) INTO v_expected
      FROM df_runtime.campaign_game_sessions m
      JOIN df_game.sessions g ON g.tenant_id = m.tenant_id AND g.session_id = m.game_session_id
     WHERE m.tenant_id = p_tenant_id AND m.campaign_id = p_campaign_id;
    UPDATE df_game.sessions g
       SET owner_fence = p_new_fence, lease_until = p_lease_until
      FROM df_runtime.campaign_game_sessions m
     WHERE m.tenant_id = p_tenant_id AND m.campaign_id = p_campaign_id
       AND g.tenant_id = m.tenant_id AND g.session_id = m.game_session_id
       AND (p_current_fence IS NULL OR g.owner_fence = p_current_fence);
    GET DIAGNOSTICS v_updated = ROW_COUNT;
    IF v_updated <> v_expected THEN
        RAISE EXCEPTION 'owner projection updated % of % sessions', v_updated, v_expected
            USING ERRCODE = 'DF004';
    END IF;
END $$;

-- The edge's placement request (6.2 "Row creation"): the owner row if absent, then the
-- request row.
CREATE FUNCTION df_runtime.request_placement(p_tenant_id bytea, p_campaign_id bytea) RETURNS void
LANGUAGE plpgsql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, df_meta, pg_temp
AS $$
BEGIN
    PERFORM df_runtime.require_service();
    INSERT INTO df_runtime.campaign_owners
        (tenant_id, campaign_id, recovery_epoch, claim_seq, state, lease_until)
    VALUES (p_tenant_id, p_campaign_id, df_runtime.current_recovery_epoch(), 0, 'released',
            '-infinity')
    ON CONFLICT DO NOTHING;
    INSERT INTO df_runtime.placement_requests (tenant_id, campaign_id, requested_at)
    VALUES (p_tenant_id, p_campaign_id, clock_timestamp())
    ON CONFLICT DO NOTHING;
END $$;

CREATE FUNCTION df_runtime.worker_heartbeat(
    p_worker_id bytea, p_name text, p_slots_total integer) RETURNS void
LANGUAGE sql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, pg_temp
AS $$
    INSERT INTO df_runtime.workers (worker_id, name, slots_total, started_at, last_heartbeat_at)
    VALUES (p_worker_id, p_name, p_slots_total, clock_timestamp(), clock_timestamp())
    ON CONFLICT (worker_id) DO UPDATE
       SET name = EXCLUDED.name, slots_total = EXCLUDED.slots_total,
           last_heartbeat_at = EXCLUDED.last_heartbeat_at
$$;

-- Dispatcher identity (10.2). The token is set per transaction as df.dispatcher_token.

CREATE FUNCTION df_runtime.register_dispatcher(
    p_worker_id bytea, p_instance_id bytea, p_token_digest bytea) RETURNS void
LANGUAGE sql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, pg_temp
AS $$
    INSERT INTO df_runtime.dispatcher_instances
        (instance_id, worker_id, token_digest, started_at, last_seen_at)
    VALUES (p_instance_id, p_worker_id, p_token_digest, clock_timestamp(), clock_timestamp())
$$;

-- The calling instance, while its heartbeat is at most 30 seconds old; else NULL.
CREATE FUNCTION df_runtime.current_dispatcher() RETURNS bytea
LANGUAGE sql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, pg_temp
AS $$
    SELECT d.instance_id
      FROM df_runtime.dispatcher_instances d
     WHERE d.token_digest = df_runtime.token_digest(current_setting('df.dispatcher_token', true))
       AND d.ended_at IS NULL
       AND d.last_seen_at > clock_timestamp() - interval '30 seconds'
$$;

CREATE FUNCTION df_runtime.dispatcher_heartbeat() RETURNS boolean
LANGUAGE sql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, pg_temp
AS $$
    WITH touched AS (
        UPDATE df_runtime.dispatcher_instances d
           SET last_seen_at = clock_timestamp()
         WHERE d.token_digest = df_runtime.token_digest(
                   current_setting('df.dispatcher_token', true))
           AND d.ended_at IS NULL
           AND d.last_seen_at > clock_timestamp() - interval '30 seconds'
        RETURNING 1)
    SELECT EXISTS (SELECT 1 FROM touched)
$$;

CREATE FUNCTION df_runtime.end_dispatcher() RETURNS boolean
LANGUAGE sql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, pg_temp
AS $$
    WITH ended AS (
        UPDATE df_runtime.dispatcher_instances d
           SET ended_at = clock_timestamp()
         WHERE d.token_digest = df_runtime.token_digest(
                   current_setting('df.dispatcher_token', true))
           AND d.ended_at IS NULL
        RETURNING 1)
    SELECT EXISTS (SELECT 1 FROM ended)
$$;

-- Placement, claim, renew and release (6.2). Each begins with require_service(); the caller
-- takes enter_service() first in its transaction.

CREATE FUNCTION df_runtime.place(
    p_tenant_id bytea, p_campaign_id bytea, p_placement_id bytea, p_worker_id bytea,
    p_token_digest bytea, p_placing_lease_seconds integer)
RETURNS TABLE (out_recovery_epoch bigint, out_claim_seq bigint)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, df_meta, pg_temp
AS $$
DECLARE
    v_owner df_runtime.campaign_owners%ROWTYPE;
BEGIN
    PERFORM df_runtime.require_service();
    PERFORM df_runtime.require_seconds(p_placing_lease_seconds, 'placing lease');
    SELECT * INTO v_owner FROM df_runtime.campaign_owners o
     WHERE o.tenant_id = p_tenant_id AND o.campaign_id = p_campaign_id FOR UPDATE;
    IF NOT FOUND OR NOT df_meta.is_serving() THEN
        RETURN;
    END IF;
    IF NOT (v_owner.state = 'released'
            OR (v_owner.state = 'placing' AND v_owner.placement_lease_until <= clock_timestamp())
            OR (v_owner.state IN ('serving', 'draining')
                AND v_owner.lease_until <= clock_timestamp())) THEN
        RETURN;
    END IF;
    -- An open import job (decision 19) keeps the campaign out of placement until it closes or
    -- expires; open_import takes this same row lock, so the two cannot both succeed.
    IF EXISTS (SELECT 1 FROM df_runtime.import_jobs j
                WHERE j.tenant_id = p_tenant_id AND j.campaign_id = p_campaign_id
                  AND j.expires_at > clock_timestamp()) THEN
        RETURN;
    END IF;
    PERFORM df_runtime.close_open_segments(
        p_tenant_id, p_campaign_id, v_owner.last_progress_at, 'owner_lost');
    UPDATE df_runtime.campaign_owners o
       SET state = 'placing', placement_id = p_placement_id, worker_id = p_worker_id,
           placement_lease_until = clock_timestamp()
               + make_interval(secs => p_placing_lease_seconds),
           released_cleanly = (o.state = 'released' AND o.released_cleanly)
     WHERE o.tenant_id = p_tenant_id AND o.campaign_id = p_campaign_id;
    UPDATE df_runtime.placements
       SET ended_at = clock_timestamp(), end_reason = 'superseded'
     WHERE tenant_id = p_tenant_id AND campaign_id = p_campaign_id AND ended_at IS NULL;
    UPDATE df_runtime.placement_secrets SET is_current = false
     WHERE tenant_id = p_tenant_id AND campaign_id = p_campaign_id AND is_current;
    INSERT INTO df_runtime.placements (placement_id, tenant_id, campaign_id, worker_id, created_at)
    VALUES (p_placement_id, p_tenant_id, p_campaign_id, p_worker_id, clock_timestamp());
    INSERT INTO df_runtime.placement_secrets
        (placement_id, tenant_id, campaign_id, token_digest, recovery_epoch, claim_seq,
         is_current, lease_until)
    VALUES (p_placement_id, p_tenant_id, p_campaign_id, p_token_digest, v_owner.recovery_epoch,
            v_owner.claim_seq, true,
            clock_timestamp() + make_interval(secs => p_placing_lease_seconds));
    DELETE FROM df_runtime.placement_requests
     WHERE tenant_id = p_tenant_id AND campaign_id = p_campaign_id;
    out_recovery_epoch := v_owner.recovery_epoch;
    out_claim_seq := v_owner.claim_seq;
    RETURN NEXT;
END $$;

CREATE FUNCTION df_runtime.claim(
    p_tenant_id bytea, p_campaign_id bytea, p_placement_id bytea,
    p_runtime_endpoint text, p_lease_seconds integer, p_new_reconstruction_id bytea)
RETURNS TABLE (out_recovery_epoch bigint, out_claim_seq bigint, out_lease_until timestamptz,
               out_money_state text, out_reconstruction_id bytea)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, df_meta, pg_temp
AS $$
DECLARE
    v_owner df_runtime.campaign_owners%ROWTYPE;
    v_claimed df_runtime.campaign_owners%ROWTYPE;
    v_current_epoch bigint;
    v_needs_reconstruction boolean;
BEGIN
    PERFORM df_runtime.require_service();
    PERFORM df_runtime.require_seconds(p_lease_seconds, 'lease');
    IF p_new_reconstruction_id IS NULL OR octet_length(p_new_reconstruction_id) <> 16 THEN
        RAISE EXCEPTION 'reconstruction id must be 16 bytes' USING ERRCODE = '22023';
    END IF;
    IF NOT df_runtime.caller_holds_placement(p_placement_id) THEN
        RETURN;
    END IF;
    SELECT * INTO v_owner FROM df_runtime.campaign_owners o
     WHERE o.tenant_id = p_tenant_id AND o.campaign_id = p_campaign_id FOR UPDATE;
    IF NOT FOUND OR NOT df_meta.is_serving()
       OR v_owner.state <> 'placing' OR v_owner.placement_id IS DISTINCT FROM p_placement_id
       OR v_owner.placement_lease_until <= clock_timestamp() THEN
        RETURN;
    END IF;
    v_current_epoch := df_runtime.current_recovery_epoch();
    v_needs_reconstruction := v_owner.money_state IN ('reconstructing', 'frozen')
        OR v_owner.recovery_epoch < v_current_epoch OR NOT v_owner.released_cleanly;
    PERFORM df_runtime.close_open_segments(
        p_tenant_id, p_campaign_id, v_owner.last_progress_at, 'owner_lost');
    UPDATE df_runtime.campaign_owners o
       SET claim_seq = o.claim_seq + 1,
           money_state = CASE WHEN v_needs_reconstruction THEN 'reconstructing'
                              ELSE o.money_state END,
           reconstruction_id = CASE WHEN v_needs_reconstruction THEN p_new_reconstruction_id
                                    ELSE o.reconstruction_id END,
           recovery_epoch = v_current_epoch, state = 'serving',
           runtime_endpoint = p_runtime_endpoint, placement_lease_until = NULL,
           lease_until = clock_timestamp() + make_interval(secs => p_lease_seconds),
           last_progress_at = clock_timestamp(), released_cleanly = false
     WHERE o.tenant_id = p_tenant_id AND o.campaign_id = p_campaign_id
    RETURNING * INTO v_claimed;
    UPDATE df_runtime.placement_secrets
       SET recovery_epoch = v_claimed.recovery_epoch, claim_seq = v_claimed.claim_seq,
           lease_until = v_claimed.lease_until
     WHERE placement_id = p_placement_id AND is_current;
    PERFORM df_runtime.project_owner(
        p_tenant_id, p_campaign_id, NULL,
        df_runtime.fence_digest(v_claimed.recovery_epoch, v_claimed.claim_seq, p_placement_id),
        v_claimed.lease_until);
    out_recovery_epoch := v_claimed.recovery_epoch;
    out_claim_seq := v_claimed.claim_seq;
    out_lease_until := v_claimed.lease_until;
    out_money_state := v_claimed.money_state;
    out_reconstruction_id := v_claimed.reconstruction_id;
    RETURN NEXT;
END $$;

-- Stage 1 of a release: only the three-part fence of a serving, draining or (runner only)
-- placing owner passes; the caller decides cleanliness. Closes the open segment at the last
-- proven progress and expires the projected lease.
CREATE FUNCTION df_runtime.release_owner(
    p_tenant_id bytea, p_campaign_id bytea, p_recovery_epoch bigint, p_claim_seq bigint,
    p_placement_id bytea, p_cleanly boolean, p_may_release_placing boolean) RETURNS boolean
LANGUAGE plpgsql VOLATILE
SET search_path = pg_catalog, df_runtime, df_meta, pg_temp
AS $$
DECLARE
    v_owner df_runtime.campaign_owners%ROWTYPE;
BEGIN
    PERFORM df_runtime.require_service();
    SELECT * INTO v_owner FROM df_runtime.campaign_owners o
     WHERE o.tenant_id = p_tenant_id AND o.campaign_id = p_campaign_id
       AND o.recovery_epoch = p_recovery_epoch AND o.claim_seq = p_claim_seq
       AND o.placement_id = p_placement_id FOR UPDATE;
    IF NOT FOUND OR NOT (v_owner.state IN ('serving', 'draining')
                         OR (p_may_release_placing AND v_owner.state = 'placing')) THEN
        RETURN false;
    END IF;
    PERFORM df_runtime.close_open_segments(
        p_tenant_id, p_campaign_id, v_owner.last_progress_at,
        CASE WHEN p_cleanly THEN 'release' ELSE 'owner_exit' END);
    UPDATE df_runtime.campaign_owners o
       SET state = 'released', placement_id = NULL, worker_id = NULL,
           placement_lease_until = NULL, runtime_endpoint = NULL,
           lease_until = LEAST(o.lease_until, clock_timestamp()),
           released_cleanly = CASE WHEN o.state = 'placing' THEN o.released_cleanly
                                   ELSE p_cleanly END
     WHERE o.tenant_id = p_tenant_id AND o.campaign_id = p_campaign_id;
    UPDATE df_runtime.placements
       SET ended_at = clock_timestamp(),
           end_reason = CASE WHEN p_cleanly THEN 'released' ELSE 'exited' END
     WHERE placement_id = p_placement_id AND ended_at IS NULL;
    UPDATE df_runtime.placement_secrets SET is_current = false
     WHERE placement_id = p_placement_id AND is_current;
    IF v_owner.state <> 'placing' THEN
        PERFORM df_runtime.project_owner(
            p_tenant_id, p_campaign_id,
            df_runtime.fence_digest(p_recovery_epoch, p_claim_seq, p_placement_id),
            df_runtime.fence_digest(p_recovery_epoch, p_claim_seq, p_placement_id),
            clock_timestamp());
    END IF;
    RETURN true;
END $$;

-- The runtime itself, after a drain or an idle release (6.5).
CREATE FUNCTION df_runtime.release_clean(
    p_tenant_id bytea, p_campaign_id bytea, p_recovery_epoch bigint, p_claim_seq bigint,
    p_placement_id bytea) RETURNS boolean
LANGUAGE plpgsql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, df_meta, pg_temp
AS $$
BEGIN
    IF NOT df_runtime.caller_holds_placement(p_placement_id) THEN
        RETURN false;
    END IF;
    RETURN df_runtime.release_owner(
        p_tenant_id, p_campaign_id, p_recovery_epoch, p_claim_seq, p_placement_id, true, false);
END $$;

-- The runner, after it observed the child exit without a clean release.
CREATE FUNCTION df_runtime.release_after_exit(
    p_tenant_id bytea, p_campaign_id bytea, p_recovery_epoch bigint, p_claim_seq bigint,
    p_placement_id bytea) RETURNS boolean
LANGUAGE sql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, df_meta, pg_temp
AS $$
    SELECT df_runtime.release_owner(
        p_tenant_id, p_campaign_id, p_recovery_epoch, p_claim_seq, p_placement_id, false, true)
$$;

-- Money reconstruction outcome (6.3). Only F's finance service role may call it. A stale
-- reconstruction id, or an outcome after a success, changes nothing.
CREATE FUNCTION df_runtime.record_money_state(
    p_tenant_id bytea, p_campaign_id bytea, p_reconstruction_id bytea, p_outcome text)
RETURNS boolean
LANGUAGE plpgsql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, df_meta, pg_temp
AS $$
DECLARE
    v_updated bigint;
BEGIN
    IF p_outcome IS NULL OR p_outcome NOT IN ('reconstructed', 'failed') THEN
        RAISE EXCEPTION 'unknown reconstruction outcome' USING ERRCODE = '22023';
    END IF;
    UPDATE df_runtime.campaign_owners o
       SET money_state = CASE WHEN p_outcome = 'reconstructed' THEN 'reconciled' ELSE 'frozen' END
     WHERE o.tenant_id = p_tenant_id AND o.campaign_id = p_campaign_id
       AND o.reconstruction_id = p_reconstruction_id
       AND (o.money_state = 'reconstructing'
            OR (o.money_state = 'frozen' AND p_outcome = 'reconstructed'));
    GET DIAGNOSTICS v_updated = ROW_COUNT;
    RETURN v_updated = 1;
END $$;

-- The runtime's only way to change money_state: reconstructing to frozen for the campaign its
-- current placement token names, never to reconciled.
CREATE FUNCTION df_runtime.record_reconstruction_deadline(
    p_tenant_id bytea, p_campaign_id bytea, p_reconstruction_id bytea) RETURNS boolean
LANGUAGE plpgsql VOLATILE SECURITY DEFINER
SET search_path = pg_catalog, df_runtime, df_meta, pg_temp
AS $$
DECLARE
    v_updated bigint;
BEGIN
    UPDATE df_runtime.campaign_owners o
       SET money_state = 'frozen'
     WHERE o.tenant_id = p_tenant_id AND o.campaign_id = p_campaign_id
       AND o.reconstruction_id = p_reconstruction_id AND o.money_state = 'reconstructing'
       AND df_runtime.caller_holds_placement(o.placement_id);
    GET DIAGNOSTICS v_updated = ROW_COUNT;
    RETURN v_updated = 1;
END $$;

-- Prepared transactions would keep advisory locks past their session; refuse to be installed
-- where they are possible.
SELECT df_runtime.require_no_prepared_transactions();
