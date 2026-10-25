-- Database-level role setup (plan 12.7; hosted-runtime-research.md 4.3). STATUS: UNVERIFIED. Idempotent.
-- Normally NOT run by hand: deploy/db/create-roles.sh prints this file after the role statements (setting
-- :db first), and that stream is run as the cluster administrator (doadmin in production, the superuser in
-- staging) connected to the application database, by a pinned one-shot psql container started through the
-- deploy tool (see create-roles.sh). The defaults below can be overridden by \set lines placed before it:
--   \set statement_timeout 30s   \set idle_tx_timeout 60s   \set limit_control 16   \set limit_runner 4
--   \set limit_runtime 16        \set limit_egress 8        \set limit_finance 8    \set limit_support 4
--   \set feed_txn_timeout 10s   (role-level transaction_timeout of every role that can bump event_seq)
--
-- What this file DOES: connect/create rights, closing PUBLIC, per-role limits, the pgvector extension in its own
-- schema. What it deliberately does NOT do (critic round 1, item 8):
--  * no blanket DML on any schema or future table: each owning migration grants exactly the tables its
--    service needs (O1 for df_meta/df_runtime/df_game, B for finance, S1 for campaigns, O2 for assets);
--  * no ALTER DEFAULT PRIVILEGES run by the administrator: default privileges must be set BY the migrator,
--    so they apply to objects the migrator creates; that statement is deploy/db/bootstrap-migration.sql,
--    which O1 ships as the first migration;
--  * no row-security policies (the migrations own them).
-- deploy/db/verify-privileges.sql is the check that the whole result is least-privilege.
--
-- Roles (one per service; O1 section 4.3): df_migrator (owns the schemas, used only by `migrate`),
-- df_control (control plane, edge, sweeper), df_runner, df_campaign_runtime (one shared login for all
-- runtimes; row security separates campaigns), df_egress (the dispatcher), df_accounts (B's account service),
-- df_campaigns (S1's campaign service), df_finance (B's ledger and billing; the migrations create these three NOLOGIN when absent
-- and this deployment gives them LOGIN and a password), df_support (read-only). None is superuser, BYPASSRLS or an owner.
-- Names avoid the schema names df_meta, df_runtime and df_game on purpose: a role named like a schema would
-- be put first on the default search_path ("$user", public).

\set ON_ERROR_STOP on
\if :{?statement_timeout} \else \set statement_timeout 30s \endif
\if :{?idle_tx_timeout} \else \set idle_tx_timeout 60s \endif
\if :{?limit_control} \else \set limit_control 16 \endif
\if :{?limit_runner} \else \set limit_runner 4 \endif
\if :{?limit_runtime} \else \set limit_runtime 16 \endif
\if :{?limit_egress} \else \set limit_egress 8 \endif
\if :{?limit_accounts} \else \set limit_accounts 8 \endif
\if :{?limit_campaigns} \else \set limit_campaigns 8 \endif
\if :{?limit_finance} \else \set limit_finance 8 \endif
\if :{?limit_support} \else \set limit_support 4 \endif
\if :{?feed_txn_timeout} \else \set feed_txn_timeout 10s \endif

-- Close the database to everyone, then open it to exactly the application roles.
SELECT format('REVOKE ALL ON DATABASE %I FROM PUBLIC', :'db') \gexec
SELECT format('GRANT CONNECT ON DATABASE %I TO df_migrator, df_control, df_runner, df_campaign_runtime, df_egress, df_accounts, df_campaigns, df_finance, df_support', :'db') \gexec
-- The migrator creates the schemas, so it needs CREATE on the database (not only on a schema).
SELECT format('GRANT CREATE ON DATABASE %I TO df_migrator', :'db') \gexec

-- Nobody uses the public schema.
REVOKE ALL ON SCHEMA public FROM PUBLIC;

-- Per-role resource budgets: a runaway process exhausts its own role's budget, not the cluster.
ALTER ROLE df_migrator CONNECTION LIMIT 2;
SELECT format('ALTER ROLE df_control CONNECTION LIMIT %s', :'limit_control') \gexec
SELECT format('ALTER ROLE df_runner CONNECTION LIMIT %s', :'limit_runner') \gexec
SELECT format('ALTER ROLE df_campaign_runtime CONNECTION LIMIT %s', :'limit_runtime') \gexec
SELECT format('ALTER ROLE df_egress CONNECTION LIMIT %s', :'limit_egress') \gexec
SELECT format('ALTER ROLE df_accounts CONNECTION LIMIT %s', :'limit_accounts') \gexec
SELECT format('ALTER ROLE df_campaigns CONNECTION LIMIT %s', :'limit_campaigns') \gexec
SELECT format('ALTER ROLE df_finance CONNECTION LIMIT %s', :'limit_finance') \gexec
SELECT format('ALTER ROLE df_support CONNECTION LIMIT %s', :'limit_support') \gexec
SELECT format('ALTER ROLE %I SET statement_timeout = %L', r, :'statement_timeout')
  FROM unnest(ARRAY['df_control', 'df_runner', 'df_campaign_runtime', 'df_egress', 'df_accounts', 'df_campaigns', 'df_finance', 'df_support']) AS r \gexec
SELECT format('ALTER ROLE %I SET idle_in_transaction_session_timeout = %L', r, :'idle_tx_timeout')
  FROM unnest(ARRAY['df_control', 'df_runner', 'df_campaign_runtime', 'df_egress', 'df_accounts', 'df_campaigns', 'df_finance', 'df_support']) AS r \gexec
-- Defence in depth only: a user can change default_transaction_read_only for their own session, so this does NOT
-- make df_support read-only. The guarantee is that the role is granted SELECT and nothing else, which
-- verify-privileges.sql asserts (read-only roles check).
ALTER ROLE df_support SET default_transaction_read_only = on;

-- transaction_timeout on every role that can bump a campaign's event_seq (planning/campaigns-and-table-research.md 5.1, 13.1 item 8; D19).
-- The ROLE DEFAULT is the guarantee: PostgreSQL's timer starts at a transaction's first statement and `SET LOCAL transaction_timeout`
-- does not shorten one that is already running, so the bound the feed's poll overlap relies on must be in force from the first
-- statement. The processes check their own role at startup (and refuse to start otherwise); verify-privileges.sql checks it here.
-- Default 10s = feed_transaction_timeout_seconds; override with \set feed_txn_timeout before this file. The list is every application
-- role that serves requests; df_migrator (backfills) and df_support (read-only) are excluded. Part of the price: no single transaction
-- of these roles may exceed the value, whatever statement_timeout says (a control-plane statement_timeout above it is only an outer bound).
-- transaction_timeout exists from PostgreSQL 17; refuse an older server rather than let ALTER ROLE fail half way.
DO $guard$
BEGIN
  IF current_setting('server_version_num')::int < 170000 THEN
    RAISE EXCEPTION 'PostgreSQL 17 or later is required (transaction_timeout); this server is %', current_setting('server_version');
  END IF;
END
$guard$;
SELECT format('ALTER ROLE %I SET transaction_timeout = %L', r, :'feed_txn_timeout')
  FROM unnest(ARRAY['df_control', 'df_runner', 'df_campaign_runtime', 'df_egress', 'df_accounts', 'df_campaigns', 'df_finance']) AS r \gexec

-- pgvector (migration 0004_story_memory_vector.sql) lives in its own schema so no role needs the public
-- schema. Extension objects are created by the administrator because the migrator is not a superuser.
-- The roles that read or write story memory get USAGE on the schema only; O1 and N confirm the list.
-- TODO(O1,N): confirm which roles use vector columns and set the search_path/qualification accordingly.
CREATE SCHEMA IF NOT EXISTS df_ext;
CREATE EXTENSION IF NOT EXISTS vector SCHEMA df_ext;
REVOKE ALL ON SCHEMA df_ext FROM PUBLIC;
GRANT USAGE ON SCHEMA df_ext TO df_migrator, df_control, df_campaign_runtime;

-- TODO(owner, coordinator ruling g): df_backup is NOT created or granted here yet. Whether a backup role needs
-- BYPASSRLS (only a superuser can grant it; a managed cluster may not allow it) is pending the owner's check. When
-- decided, three places change together: deploy/db/create-roles.sh (role and password), this file (grants) and
-- deploy/db/verify-privileges.sql (the attribute checks must carve out that one role).
