-- Account closure support and the roles around it (planning/account-service-research.md,
-- planning/campaigns-and-table-research.md 8.6, 8.7; launch decisions 29 and 31). Applied by the
-- store with search_path set to schema `accounts`.
--
-- `df_accounts` is the accounts service's runtime role and `df_campaigns` the campaign service's.
-- Both are created NOLOGIN here when absent; the deployment (S2) adds LOGIN and a password per
-- environment. PostgreSQL 17 or later is required (transaction_timeout).

-- Roles are cluster-wide, so concurrent migrations (one per database) can collide on them:
-- every role statement runs only when it would change something, and is retried a few times.
DO $$
DECLARE
    v_role text;
    v_attempt integer;
BEGIN
    FOREACH v_role IN ARRAY ARRAY['df_accounts', 'df_campaigns']
    LOOP
        FOR v_attempt IN 1..5 LOOP
            BEGIN
                IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = v_role) THEN
                    EXECUTE format('CREATE ROLE %I NOLOGIN', v_role);
                END IF;
                IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = v_role AND (rolsuper
                        OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls)) THEN
                    EXECUTE format('ALTER ROLE %I NOSUPERUSER NOCREATEDB NOCREATEROLE '
                        'NOREPLICATION NOBYPASSRLS', v_role);
                END IF;
                EXIT;
            EXCEPTION WHEN OTHERS THEN
                IF v_attempt = 5 THEN
                    RAISE;
                END IF;
                PERFORM pg_sleep(0.1 * v_attempt);
            END;
        END LOOP;
    END LOOP;

    -- The close transaction calls the campaign service's event-bumping methods. A transaction
    -- that outlives the event feed's overlap could commit an event a reader has already read
    -- past, so the bound is the role's default (a per-transaction SET LOCAL does not shorten a
    -- running timer), and the service refuses to start unless it holds (TimeoutPolicy in
    -- account_closure.rs).
    FOR v_attempt IN 1..5 LOOP
        BEGIN
            IF NOT EXISTS (
                SELECT 1 FROM pg_db_role_setting s JOIN pg_roles r ON r.oid = s.setrole
                WHERE r.rolname = 'df_accounts' AND s.setdatabase = 0
                  AND 'transaction_timeout=10s' = ANY (s.setconfig)
            ) THEN
                ALTER ROLE df_accounts SET transaction_timeout = '10s';
            END IF;
            EXIT;
        EXCEPTION WHEN OTHERS THEN
            IF v_attempt = 5 THEN
                RAISE;
            END IF;
            PERFORM pg_sleep(0.1 * v_attempt);
        END;
    END LOOP;
END $$;

-- Privileges are granted table by table, exactly what the store's statements need, and there is
-- no default-privilege grant: a table added later gives the runtime role nothing until its
-- migration says so. The schema_migration table is read-only for the runtime role.
GRANT USAGE ON SCHEMA accounts TO df_accounts;
GRANT SELECT, INSERT, UPDATE ON account TO df_accounts;
GRANT SELECT, INSERT, UPDATE, DELETE ON password_credential TO df_accounts;
GRANT SELECT, INSERT, DELETE ON identity_link TO df_accounts;
GRANT SELECT, INSERT, UPDATE, DELETE ON web_session TO df_accounts;
GRANT SELECT, INSERT, UPDATE, DELETE ON email_change TO df_accounts;
GRANT SELECT, INSERT, UPDATE, DELETE ON account_token TO df_accounts;
GRANT SELECT, INSERT, UPDATE, DELETE ON operation_receipt TO df_accounts;
-- Append-only: the trigger refuses UPDATE; only the retention pass deletes.
GRANT SELECT, INSERT, DELETE ON account_event TO df_accounts;
GRANT USAGE, SELECT ON SEQUENCE account_event_event_id_seq TO df_accounts;
GRANT SELECT ON schema_migration TO df_accounts;

-- Production knows one identity provider. The test provider stays out of the production schema:
-- tests that use it relax this constraint in their own scratch database.
ALTER TABLE identity_link DROP CONSTRAINT identity_link_provider_check;
ALTER TABLE identity_link ADD CONSTRAINT identity_link_provider_check CHECK (provider = 'workos');

-- A read-only view of an account's status, for the campaign service's single-statement
-- membership read. It locks nothing and exposes nothing else.
CREATE VIEW account_status AS
    SELECT account_id, status FROM account;

-- `FOR SHARE` needs UPDATE privilege on the row's table, which the campaign service's role must
-- not have. The definer takes the share lock for it, so a transfer acceptance either commits
-- before a concurrent closure or sees `closing` (lock order: accounts, then campaigns).
CREATE FUNCTION lock_account_status(p_account_id bytea) RETURNS text
    LANGUAGE sql VOLATILE SECURITY DEFINER
    SET search_path = pg_catalog, accounts, pg_temp
AS $$
    SELECT status FROM accounts.account WHERE account_id = p_account_id FOR SHARE
$$;

REVOKE ALL ON FUNCTION lock_account_status(bytea) FROM PUBLIC;
GRANT USAGE ON SCHEMA accounts TO df_campaigns;
GRANT EXECUTE ON FUNCTION lock_account_status(bytea) TO df_campaigns;
GRANT SELECT ON account_status TO df_campaigns;
