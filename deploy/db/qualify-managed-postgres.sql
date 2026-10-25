-- Qualification of two assumptions that O1's migrations make about MANAGED PostgreSQL (round 3d). STATUS: UNVERIFIED, never run against a
-- cluster. Run as the cluster administrator (doadmin) against the STAGING cluster; everything happens in one transaction that is rolled back:
--   psql -v ON_ERROR_STOP=1 -f deploy/db/qualify-managed-postgres.sql      (through deploy/db/run-admin-sql.sh for a managed cluster)
-- It prints Q01 ok and Q02 ok, or raises with the reason. The checklist is deploy/qualification/staging-checks.csv.
\set ON_ERROR_STOP on
BEGIN;

-- Q03 (information): the server version, and whether the vector extension is available.
SELECT current_setting('server_version') AS server_version,
       current_setting('server_version_num')::int >= 170000 AS is_17_or_later,
       EXISTS (SELECT 1 FROM pg_available_extensions WHERE name = 'vector') AS pgvector_available;

-- Q01: migration 0018 does  GRANT pg_signal_backend TO df_runtime_definer  (the definer may terminate the backends that hold the service lock).
CREATE ROLE df_qual_definer NOLOGIN;
GRANT pg_signal_backend TO df_qual_definer;
DO $q01$
BEGIN
  IF NOT pg_has_role('df_qual_definer', 'pg_signal_backend', 'member') THEN
    RAISE EXCEPTION 'Q01 FAILED: pg_signal_backend was not granted to the definer role';
  END IF;
  RAISE NOTICE 'Q01 ok: pg_signal_backend can be granted to a NOLOGIN definer role by %', current_user;
END
$q01$;

-- Q02: migration 0018 grants the definer to the migrator (current_user) so that ownership can be transferred, and REVOKES it again at the end.
-- Here the migrator is the administrator itself, as it would be on a managed cluster.
GRANT df_qual_definer TO current_user;
DO $q02a$
BEGIN
  IF NOT pg_has_role(current_user, 'df_qual_definer', 'member') THEN
    RAISE EXCEPTION 'Q02 FAILED: the migrator did not become a member of the definer role';
  END IF;
END
$q02a$;
REVOKE df_qual_definer FROM current_user;
DO $q02b$
BEGIN
  IF pg_has_role(current_user, 'df_qual_definer', 'member') THEN
    RAISE EXCEPTION 'Q02 FAILED: the migrator is STILL a member of the definer role after REVOKE (it would keep standing access to every definer-owned object)';
  END IF;
  RAISE NOTICE 'Q02 ok: the temporary membership of % in the definer role is revoked', current_user;
END
$q02b$;

ROLLBACK;
\echo Q01 and Q02 passed; nothing was changed (rolled back).
