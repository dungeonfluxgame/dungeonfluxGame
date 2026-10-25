-- Runtime roles, ownership, grants and row security (planning/hosted-runtime-research.md 4.3, 4.4).
-- Roles are NOLOGIN here: the deployment (S2) adds LOGIN and a password per environment and
-- sets role connection limits from its pool sizes. No role is a superuser, bypasses row
-- security or owns a table it uses at runtime; the directory tables and functions belong to
-- the NOLOGIN definer role.

DO $$
DECLARE
    v_role text;
BEGIN
    FOREACH v_role IN ARRAY ARRAY['df_runtime_definer', 'df_control', 'df_runner',
        'df_campaign_runtime', 'df_egress', 'df_story_maintenance', 'df_finance',
        'df_finance_replay', 'df_importer']
    LOOP
        IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = v_role) THEN
            EXECUTE format('CREATE ROLE %I NOLOGIN', v_role);
        END IF;
        EXECUTE format('ALTER ROLE %I NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS',
            v_role);
        -- The definer owns every directory object and bypasses nothing: it never logs in.
        IF v_role = 'df_runtime_definer' THEN
            EXECUTE format('ALTER ROLE %I NOLOGIN', v_role);
        END IF;
    END LOOP;
    -- Needed for the ownership transfers below when the migrator is not a superuser; revoked
    -- again at the end of this file, so the migrator holds no standing data access.
    IF NOT pg_has_role(current_user, 'df_runtime_definer', 'member') THEN
        EXECUTE format('GRANT df_runtime_definer TO %I', current_user);
        PERFORM set_config('df.granted_definer_membership', 'yes', true);
    END IF;
    EXECUTE format('REVOKE TEMP ON DATABASE %I FROM PUBLIC', current_database());
    EXECUTE format('REVOKE TEMP ON DATABASE %I FROM df_control, df_runner, df_campaign_runtime, '
        'df_egress, df_story_maintenance, df_finance, df_finance_replay, df_importer, '
        'df_runtime_definer', current_database());
    EXECUTE format('GRANT CONNECT ON DATABASE %I TO df_control, df_runner, df_campaign_runtime, '
        'df_egress, df_story_maintenance, df_finance, df_finance_replay, df_importer', current_database());
    -- The definer may terminate backends that hold the service lock when it halts or restores
    -- the service (acquire_exclusive_service_lock); no other role holds the privilege.
    EXECUTE 'GRANT pg_signal_backend TO df_runtime_definer';
END $$;

-- Every role that takes the shared service lock lets go of an idle transaction in 5 seconds,
-- so one stuck transaction cannot hold a halt, and all fenced work behind it, for long (6.1).
ALTER ROLE df_campaign_runtime SET statement_timeout = '30s';
ALTER ROLE df_campaign_runtime SET idle_in_transaction_session_timeout = '5s';
ALTER ROLE df_control SET statement_timeout = '30s';
ALTER ROLE df_control SET idle_in_transaction_session_timeout = '5s';
ALTER ROLE df_runner SET statement_timeout = '10s';
ALTER ROLE df_runner SET idle_in_transaction_session_timeout = '5s';
ALTER ROLE df_egress SET statement_timeout = '30s';
ALTER ROLE df_egress SET idle_in_transaction_session_timeout = '5s';
ALTER ROLE df_finance SET idle_in_transaction_session_timeout = '5s';
ALTER ROLE df_finance_replay SET idle_in_transaction_session_timeout = '5s';
ALTER ROLE df_control CONNECTION LIMIT 24;

-- Row security. The directory tables carry one policy for the definer role, because FORCE
-- applies to table owners; placement_secrets has none and is granted to nobody, so the scope
-- functions that read it never meet a policy that calls back into itself (4.4 item 1).
DO $$
DECLARE
    v_table text;
BEGIN
    FOREACH v_table IN ARRAY ARRAY['df_meta.service_state', 'df_runtime.workers',
        'df_runtime.campaign_owners', 'df_runtime.placements',
        'df_runtime.campaign_game_sessions', 'df_runtime.placement_requests',
        'df_runtime.dispatcher_instances', 'df_runtime.refetch_requests',
        'df_runtime.table_sessions', 'df_runtime.meter_segments', 'df_game.sessions',
        'df_game.checkpoints', 'df_game.operations', 'df_game.facts', 'df_game.intents',
        'df_game.retired_namespaces']
    LOOP
        EXECUTE format('ALTER TABLE %s ENABLE ROW LEVEL SECURITY', v_table);
        EXECUTE format('ALTER TABLE %s FORCE ROW LEVEL SECURITY', v_table);
        EXECUTE format('CREATE POLICY definer_all ON %s FOR ALL TO df_runtime_definer '
            'USING (true) WITH CHECK (true)', v_table);
    END LOOP;
END $$;

CREATE POLICY control_read ON df_runtime.campaign_owners FOR SELECT TO df_control USING (true);
CREATE POLICY control_read ON df_runtime.placements FOR SELECT TO df_control USING (true);
CREATE POLICY control_read ON df_runtime.table_sessions FOR SELECT TO df_control USING (true);
CREATE POLICY control_read ON df_runtime.meter_segments FOR SELECT TO df_control USING (true);
CREATE POLICY finance_read ON df_runtime.table_sessions FOR SELECT TO df_finance USING (true);
CREATE POLICY finance_read ON df_runtime.meter_segments FOR SELECT TO df_finance USING (true);

-- A runtime reads only the table sessions of the campaign its placement token leases.
CREATE POLICY runtime_scope ON df_runtime.table_sessions FOR SELECT TO df_campaign_runtime
    USING ((tenant_id, session_id) IN
        (SELECT l.tenant_id, l.session_id FROM df_runtime.leased_table_sessions() l));
CREATE POLICY runtime_scope ON df_runtime.meter_segments FOR SELECT TO df_campaign_runtime
    USING ((tenant_id, session_id) IN
        (SELECT l.tenant_id, l.session_id FROM df_runtime.leased_table_sessions() l));

-- Re-fetch requests (10.4): the control plane asks and resets, a dispatcher claims and
-- finishes what it holds.
CREATE POLICY control_read ON df_runtime.refetch_requests FOR SELECT TO df_control USING (true);
CREATE POLICY control_request ON df_runtime.refetch_requests FOR INSERT TO df_control
    WITH CHECK (true);
CREATE POLICY control_reset ON df_runtime.refetch_requests FOR UPDATE TO df_control
    USING (outcome IS NOT NULL) WITH CHECK (outcome IS NULL);
CREATE POLICY dispatcher_read ON df_runtime.refetch_requests FOR SELECT TO df_egress
    USING (true);
CREATE POLICY dispatcher_claim ON df_runtime.refetch_requests FOR UPDATE TO df_egress
    USING (outcome IS NULL AND (claimed_until IS NULL OR claimed_until < clock_timestamp()
        OR claimed_by_instance = (SELECT df_runtime.current_dispatcher())))
    WITH CHECK (claimed_by_instance = (SELECT df_runtime.current_dispatcher()));
CREATE POLICY dispatcher_prune ON df_runtime.refetch_requests FOR DELETE TO df_egress
    USING (outcome IS NOT NULL AND finished_at < clock_timestamp() - make_interval(days => 7));

-- The game tables: a runtime sees and writes only the game sessions registered to the campaign
-- its placement token leases (4.4 item 3).
DO $$
DECLARE
    v_table text;
BEGIN
    FOREACH v_table IN ARRAY ARRAY['sessions', 'checkpoints', 'operations', 'facts', 'intents',
        'retired_namespaces']
    LOOP
        EXECUTE format('CREATE POLICY runtime_scope ON df_game.%I FOR ALL TO df_campaign_runtime '
            'USING ((tenant_id, session_id) IN '
            '(SELECT l.tenant_id, l.session_id FROM df_runtime.leased_game_sessions() l)) '
            'WITH CHECK ((tenant_id, session_id) IN '
            '(SELECT l.tenant_id, l.session_id FROM df_runtime.leased_game_sessions() l))',
            v_table);
    END LOOP;
END $$;

-- Ownership: the definer owns the directory schemas, tables, views, functions and types, and
-- nothing in them is executable by PUBLIC. Later migrations that add objects to df_meta or
-- df_runtime end with SELECT df_meta.adopt_objects() (run by a superuser or a member of the
-- definer role), so a function created later cannot keep its creator as owner and run past row
-- security. A new function from the migrator is also born without PUBLIC execution, through the
-- default privileges below.
ALTER DEFAULT PRIVILEGES REVOKE EXECUTE ON FUNCTIONS FROM PUBLIC;

CREATE FUNCTION df_meta.adopt_objects() RETURNS void
LANGUAGE plpgsql VOLATILE
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    v_object record;
BEGIN
    ALTER SCHEMA df_meta OWNER TO df_runtime_definer;
    ALTER SCHEMA df_runtime OWNER TO df_runtime_definer;
    FOR v_object IN
        SELECT CASE c.relkind WHEN 'v' THEN 'VIEW' WHEN 'm' THEN 'MATERIALIZED VIEW'
                              WHEN 'f' THEN 'FOREIGN TABLE' ELSE 'TABLE' END AS kind,
               format('%I.%I', n.nspname, c.relname) AS name
          FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname IN ('df_meta', 'df_runtime') AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
           AND pg_get_userbyid(c.relowner) <> 'df_runtime_definer'
    LOOP
        EXECUTE format('ALTER %s %s OWNER TO df_runtime_definer', v_object.kind, v_object.name);
    END LOOP;
    FOR v_object IN
        SELECT p.oid::regprocedure::text AS name
          FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
         WHERE n.nspname IN ('df_meta', 'df_runtime')
           AND pg_get_userbyid(p.proowner) <> 'df_runtime_definer'
    LOOP
        EXECUTE format('ALTER FUNCTION %s OWNER TO df_runtime_definer', v_object.name);
    END LOOP;
    FOR v_object IN
        SELECT format('%I.%I', n.nspname, t.typname) AS name
          FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace
         WHERE n.nspname IN ('df_meta', 'df_runtime') AND t.typtype IN ('e', 'd')
           AND pg_get_userbyid(t.typowner) <> 'df_runtime_definer'
    LOOP
        EXECUTE format('ALTER TYPE %s OWNER TO df_runtime_definer', v_object.name);
    END LOOP;
    REVOKE ALL ON ALL FUNCTIONS IN SCHEMA df_meta, df_runtime FROM PUBLIC;
END $$;

SELECT df_meta.adopt_objects();

-- Privileges. Nothing is executable by PUBLIC; each function is granted to exactly the roles
-- that call it. The helpers (project_owner, release_owner, close_open_segments, ...) are
-- reachable only through the definer functions above them.
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA df_meta, df_runtime FROM PUBLIC;
GRANT USAGE ON SCHEMA df_game TO df_runtime_definer, df_campaign_runtime;
GRANT SELECT, INSERT, UPDATE ON df_game.sessions TO df_runtime_definer;
GRANT USAGE ON SCHEMA df_runtime TO df_control, df_runner, df_campaign_runtime, df_egress,
    df_finance, df_finance_replay;

GRANT EXECUTE ON FUNCTION df_runtime.enter_service() TO df_campaign_runtime, df_control,
    df_runner, df_egress, df_finance;
GRANT EXECUTE ON FUNCTION df_runtime.fence_current(bytea, bytea, bigint, bigint, bytea,
    df_runtime.fence_path) TO df_campaign_runtime, df_control, df_runner, df_egress, df_finance;
GRANT EXECUTE ON FUNCTION df_runtime.request_placement(bytea, bytea) TO df_control;
GRANT EXECUTE ON FUNCTION df_runtime.worker_heartbeat(bytea, text, integer) TO df_runner;
GRANT EXECUTE ON FUNCTION df_runtime.place(bytea, bytea, bytea, bytea, bytea, integer)
    TO df_runner;
GRANT EXECUTE ON FUNCTION df_runtime.release_after_exit(bytea, bytea, bigint, bigint, bytea)
    TO df_runner;
GRANT EXECUTE ON FUNCTION df_runtime.claim(bytea, bytea, bytea, text, integer, bytea)
    TO df_campaign_runtime;
GRANT EXECUTE ON FUNCTION df_runtime.renew(bytea, bytea, bigint, bigint, bytea, integer,
    boolean, boolean, integer) TO df_campaign_runtime;
GRANT EXECUTE ON FUNCTION df_runtime.release_clean(bytea, bytea, bigint, bigint, bytea)
    TO df_campaign_runtime;
GRANT EXECUTE ON FUNCTION df_runtime.record_reconstruction_deadline(bytea, bytea, bytea)
    TO df_campaign_runtime;
GRANT EXECUTE ON FUNCTION df_runtime.register_game_session(bytea) TO df_campaign_runtime;
GRANT EXECUTE ON FUNCTION df_runtime.leased_campaign(), df_runtime.leased_game_sessions(),
    df_runtime.leased_table_sessions() TO df_campaign_runtime;
GRANT EXECUTE ON FUNCTION df_runtime.record_money_state(bytea, bytea, bytea, text)
    TO df_finance;
GRANT EXECUTE ON FUNCTION df_runtime.register_dispatcher(bytea, bytea, bytea) TO df_egress;
GRANT EXECUTE ON FUNCTION df_runtime.current_dispatcher() TO df_egress;
GRANT EXECUTE ON FUNCTION df_runtime.dispatcher_heartbeat() TO df_egress;
GRANT EXECUTE ON FUNCTION df_runtime.end_dispatcher() TO df_egress;

GRANT SELECT ON df_runtime.campaign_owners, df_runtime.placements, df_runtime.table_sessions,
    df_runtime.meter_segments TO df_control;
GRANT SELECT ON df_runtime.table_sessions, df_runtime.meter_segments TO df_finance;
GRANT SELECT ON df_runtime.table_sessions, df_runtime.meter_segments TO df_campaign_runtime;
GRANT SELECT, INSERT, UPDATE ON df_runtime.refetch_requests TO df_control;
GRANT SELECT, UPDATE, DELETE ON df_runtime.refetch_requests TO df_egress;

-- The runtime never writes its own fence or lease: those columns change only through claim,
-- renew and release.
GRANT SELECT, INSERT ON df_game.sessions TO df_campaign_runtime;
GRANT UPDATE (recovery_epoch, in_epoch_sequence) ON df_game.sessions TO df_campaign_runtime;
GRANT SELECT, INSERT, UPDATE ON df_game.checkpoints, df_game.operations TO df_campaign_runtime;
GRANT SELECT, INSERT ON df_game.facts, df_game.intents TO df_campaign_runtime;
GRANT SELECT, INSERT, DELETE ON df_game.retired_namespaces TO df_campaign_runtime;

-- The serving state: a lock-free read for the finance and control roles, and the one function
-- that ends a restore, executable only by F's replay role (begin_restore is the operator's).
GRANT EXECUTE ON FUNCTION df_runtime.service_state() TO df_control, df_finance,
    df_finance_replay;
GRANT EXECUTE ON FUNCTION df_runtime.restore_complete(bigint) TO df_finance_replay;

-- The control plane's remedies for a stuck or hostile hold of the service lock and its upkeep
-- of import jobs; every service process checks at startup that prepared transactions are off.
GRANT EXECUTE ON FUNCTION df_runtime.break_service_lock(integer, integer),
    df_runtime.halt_service(integer, integer), df_runtime.purge_import_jobs(integer, integer)
    TO df_control;
GRANT EXECUTE ON FUNCTION df_runtime.require_no_prepared_transactions() TO df_campaign_runtime,
    df_control, df_runner, df_egress, df_finance, df_finance_replay;

-- The offline importer (decision 19, rulings 2026-10-25): a dedicated role scoped to ONE campaign
-- by an import job token (df.import_token, set per transaction like the placement token). It
-- reads and inserts the rows of that campaign's game sessions only; it creates a session through
-- import_game_session, never directly, so it cannot write owner_fence or lease_until; it cannot
-- update or delete and reads no directory table. The migrator and a backup role get no row access
-- here: the backup role waits on the owner's BYPASSRLS decision.
GRANT USAGE ON SCHEMA df_game, df_runtime TO df_importer;
GRANT SELECT ON df_game.sessions TO df_importer;
GRANT SELECT, INSERT ON df_game.checkpoints, df_game.operations, df_game.facts,
    df_game.intents, df_game.retired_namespaces TO df_importer;
GRANT EXECUTE ON FUNCTION df_runtime.importing_campaign(),
    df_runtime.importing_game_sessions() TO df_importer;
GRANT EXECUTE ON FUNCTION df_runtime.import_game_session(bytea, bytea, numeric, numeric)
    TO df_importer;
DO $$
DECLARE
    v_table text;
BEGIN
    FOREACH v_table IN ARRAY ARRAY['sessions', 'checkpoints', 'operations', 'facts', 'intents',
        'retired_namespaces']
    LOOP
        EXECUTE format('CREATE POLICY importer_read ON df_game.%I FOR SELECT TO df_importer '
            'USING ((tenant_id, session_id) IN '
            '(SELECT i.tenant_id, i.session_id FROM df_runtime.importing_game_sessions() i))',
            v_table);
        IF v_table <> 'sessions' THEN
            EXECUTE format('CREATE POLICY importer_insert ON df_game.%I FOR INSERT '
                'TO df_importer WITH CHECK ((tenant_id, session_id) IN '
                '(SELECT i.tenant_id, i.session_id FROM df_runtime.importing_game_sessions() i))',
                v_table);
        END IF;
    END LOOP;
END $$;

-- The migrator needed definer membership only to transfer ownership above.
DO $$
BEGIN
    IF current_setting('df.granted_definer_membership', true) = 'yes' THEN
        EXECUTE format('REVOKE df_runtime_definer FROM %I', current_user);
    END IF;
END $$;
