-- Least-privilege verification (critic round 1 items 8 and 16, round 1c item 5; plan 12.7). STATUS: UNVERIFIED
-- (never run against a live PostgreSQL; written against the system catalogs by reading).
-- Run as the cluster administrator after the roles, grants.sql and the migrations, through the same pinned
-- one-shot psql path as the roles (deploy/db/run-admin-sql.sh), or locally in the staging mirror:
--   psql -v ON_ERROR_STOP=1 -v db=dungeonflux -f verify-privileges.sql
-- Optional variables (defaults shown; TODO(O1,B,S1,O2): the real schema and table names):
--   app_roles            df_control,df_runner,df_campaign_runtime,df_egress,df_billing,df_support
--   runtime_roles        df_campaign_runtime,df_egress   (forced RLS, no views without security_invoker, no matviews)
--   readonly_roles       df_support                      (SELECT and nothing else)
--   migrator             df_migrator
--   app_schema_regex     ^df_(meta|runtime|game|story|account|finance|campaign|asset)
--   denied_schema_regex  ^df_(account|finance|identity|ticket)        schemas runtime roles must not touch at all
--   denied_table_regex   (ticket|token|credential|secret|password|session)
--                        relations in ANY app schema that runtime-class AND read-only roles must hold no privilege on
--                        (table or column level). "session" is in the regex so that a future login-session table
--                        (df_meta.login_sessions) is caught; the legitimate runtime table is listed in the exceptions.
--   denied_table_exceptions  df_game.sessions                       comma list of schema.table exempt from the regex
--   denied_column_regex  (ticket|token|credential|secret|password)  column NAMES (in any app relation) that runtime-class and
--                        read-only roles must hold no privilege on, whatever the table is called (invite_token, password_hash)
--   denied_column_exceptions  (empty)                               comma list of schema.table.column exempt from that regex
--   append_only_regex    (ledger|journal)                tables where no app role may UPDATE, DELETE or TRUNCATE
--   check_default_acl    on|off                          DEFAULT ON: the bootstrap migration (deploy/db/bootstrap-migration.sql)
--                        must have run as the migrator. Production verification always runs with it on; the staging smoke
--                        passes off only until O1 ships that migration.
-- Raises one exception listing EVERY violation, or prints "privileges ok".
--
-- TODO(owner, coordinator ruling g): df_backup is NOT created, granted or verified anywhere yet. Whether a backup role
-- needs BYPASSRLS (only a superuser can grant it) on managed PostgreSQL is pending the owner's check. When decided,
-- three places change together: deploy/db/create-roles.sh (the role and its password), deploy/db/grants.sql (its
-- grants) and this file (its attributes; it is the one role allowed rolbypassrls, so the attribute check must carve it out).

\set ON_ERROR_STOP on
\if :{?app_roles} \else \set app_roles df_control,df_runner,df_campaign_runtime,df_egress,df_billing,df_support \endif
\if :{?runtime_roles} \else \set runtime_roles df_campaign_runtime,df_egress \endif
\if :{?readonly_roles} \else \set readonly_roles df_support \endif
\if :{?migrator} \else \set migrator df_migrator \endif
\if :{?app_schema_regex} \else \set app_schema_regex '^df_(meta|runtime|game|story|account|finance|campaign|asset)' \endif
\if :{?denied_schema_regex} \else \set denied_schema_regex '^df_(account|finance|identity|ticket)' \endif
\if :{?denied_table_regex} \else \set denied_table_regex '(ticket|token|credential|secret|password|session)' \endif
\if :{?denied_table_exceptions} \else \set denied_table_exceptions 'df_game.sessions' \endif
\if :{?denied_column_regex} \else \set denied_column_regex '(ticket|token|credential|secret|password)' \endif
\if :{?denied_column_exceptions} \else \set denied_column_exceptions '' \endif
\if :{?append_only_regex} \else \set append_only_regex '(ledger|journal)' \endif
\if :{?check_default_acl} \else \set check_default_acl on \endif

SELECT set_config('df.app_roles', :'app_roles', false),
       set_config('df.runtime_roles', :'runtime_roles', false),
       set_config('df.readonly_roles', :'readonly_roles', false),
       set_config('df.migrator', :'migrator', false),
       set_config('df.app_schema_regex', :'app_schema_regex', false),
       set_config('df.denied_schema_regex', :'denied_schema_regex', false),
       set_config('df.denied_table_regex', :'denied_table_regex', false),
       set_config('df.denied_table_exceptions', :'denied_table_exceptions', false),
       set_config('df.denied_column_regex', :'denied_column_regex', false),
       set_config('df.denied_column_exceptions', :'denied_column_exceptions', false),
       set_config('df.append_only_regex', :'append_only_regex', false),
       set_config('df.check_default_acl', :'check_default_acl', false),
       set_config('df.db', :'db', false) \gset

DO $verify$
DECLARE
  roles text[] := string_to_array(current_setting('df.app_roles'), ',');
  runtime_roles text[] := string_to_array(current_setting('df.runtime_roles'), ',');
  readonly_roles text[] := string_to_array(current_setting('df.readonly_roles'), ',');
  migrator text := current_setting('df.migrator');
  schema_re text := current_setting('df.app_schema_regex');
  denied_schema_re text := current_setting('df.denied_schema_regex');
  denied_table_re text := current_setting('df.denied_table_regex');
  denied_table_exc text[] := string_to_array(current_setting('df.denied_table_exceptions'), ',');
  denied_column_re text := current_setting('df.denied_column_regex');
  denied_column_exc text[] := string_to_array(current_setting('df.denied_column_exceptions'), ',');
  denied_roles text[] := string_to_array(current_setting('df.runtime_roles') || ',' || current_setting('df.readonly_roles'), ',');
  append_re text := current_setting('df.append_only_regex');
  db text := current_setting('df.db');
  problems text[] := ARRAY[]::text[];
  role_name text;
  rec record;
  schema_name text;
BEGIN
  FOREACH role_name IN ARRAY (roles || runtime_roles || readonly_roles || ARRAY[migrator]) LOOP
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = role_name) THEN
      problems := problems || format('role %s does not exist', role_name);
    END IF;
  END LOOP;

  -- 1. Attributes: nothing that lets a role escape the grants.
  FOR rec IN SELECT rolname FROM pg_roles
              WHERE rolname = ANY (roles || ARRAY[migrator])
                AND (rolsuper OR rolbypassrls OR rolcreatedb OR rolcreaterole OR rolreplication)
  LOOP
    problems := problems || format('%s has a powerful attribute (superuser, bypassrls, createdb, createrole or replication)', rec.rolname);
  END LOOP;

  -- 2. Membership: an application role belongs to no role at all (pg_read_all_data, pg_write_all_data,
  --    pg_signal_backend, pg_monitor and any inherited role would silently widen it).
  FOR rec IN SELECT m.rolname AS member, g.rolname AS granted
               FROM pg_auth_members am
               JOIN pg_roles m ON m.oid = am.member
               JOIN pg_roles g ON g.oid = am.roleid
              WHERE m.rolname = ANY (roles || ARRAY[migrator])
  LOOP
    problems := problems || format('%s is a member of %s', rec.member, rec.granted);
  END LOOP;

  -- 3. Ownership: serving roles own nothing (the migrator owns the schemas and tables).
  FOR rec IN SELECT r.rolname, 'relation ' || c.oid::regclass::text AS what
               FROM pg_class c JOIN pg_roles r ON r.oid = c.relowner
              WHERE r.rolname = ANY (roles)
             UNION ALL
             SELECT r.rolname, 'function ' || p.oid::regprocedure::text
               FROM pg_proc p JOIN pg_roles r ON r.oid = p.proowner
              WHERE r.rolname = ANY (roles)
             UNION ALL
             SELECT r.rolname, 'schema ' || n.nspname
               FROM pg_namespace n JOIN pg_roles r ON r.oid = n.nspowner
              WHERE r.rolname = ANY (roles)
             UNION ALL
             SELECT r.rolname, 'database ' || d.datname
               FROM pg_database d JOIN pg_roles r ON r.oid = d.datdba
              WHERE r.rolname = ANY (roles)
  LOOP
    problems := problems || format('%s owns %s', rec.rolname, rec.what);
  END LOOP;

  -- 4. No DDL anywhere for serving and read-only roles.
  FOREACH role_name IN ARRAY (roles || readonly_roles) LOOP
    IF has_database_privilege(role_name, db, 'CREATE') THEN
      problems := problems || format('%s can CREATE in database %s', role_name, db);
    END IF;
    IF has_database_privilege(role_name, db, 'TEMP') THEN
      problems := problems || format('%s can create temporary tables in %s', role_name, db);
    END IF;
    FOR rec IN SELECT nspname FROM pg_namespace
                WHERE nspname NOT LIKE 'pg\_%' AND nspname <> 'information_schema'
    LOOP
      IF has_schema_privilege(role_name, rec.nspname, 'CREATE') THEN
        problems := problems || format('%s can CREATE in schema %s', role_name, rec.nspname);
      END IF;
    END LOOP;
  END LOOP;

  -- 5. Append-only relations (tables, views, matviews, foreign tables): no serving role may UPDATE (table OR
  --    column level), DELETE or TRUNCATE a ledger or journal relation. INSERT and SELECT are fine.
  FOR rec IN SELECT r AS role_name, c.oid::regclass::text AS rel, p
               FROM pg_class c
               JOIN pg_namespace n ON n.oid = c.relnamespace
               CROSS JOIN unnest(roles) AS r
               CROSS JOIN unnest(ARRAY['UPDATE', 'DELETE', 'TRUNCATE']) AS p
              WHERE n.nspname ~ schema_re AND c.relkind IN ('r', 'p', 'v', 'm', 'f') AND c.relname ~ append_re
                AND CASE p
                      WHEN 'UPDATE' THEN has_any_column_privilege(r, c.oid, 'UPDATE')
                      ELSE has_table_privilege(r, c.oid, p)
                    END
  LOOP
    problems := problems || format('%s has %s on append-only relation %s', rec.role_name, rec.p, rec.rel);
  END LOOP;

  -- 6. Row security and views for runtime-class roles. Column-level grants count (has_any_column_privilege).
  --    * tables: RLS enabled AND forced;
  --    * views: security_invoker, or the view runs with its owner's rights and ignores the caller's RLS;
  --    * materialized and foreign relations: not allowed at all (no row security can apply).
  FOR rec IN SELECT r AS role_name, c.oid::regclass::text AS rel, c.relkind, c.relrowsecurity, c.relforcerowsecurity,
                    COALESCE(c.reloptions, ARRAY[]::text[]) AS reloptions
               FROM pg_class c
               JOIN pg_namespace n ON n.oid = c.relnamespace
               CROSS JOIN unnest(runtime_roles) AS r
              WHERE n.nspname ~ schema_re AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
                AND (has_any_column_privilege(r, c.oid, 'SELECT') OR has_any_column_privilege(r, c.oid, 'INSERT')
                  OR has_any_column_privilege(r, c.oid, 'UPDATE') OR has_table_privilege(r, c.oid, 'DELETE')
                  OR has_table_privilege(r, c.oid, 'TRUNCATE'))
  LOOP
    IF rec.relkind IN ('r', 'p') AND NOT (rec.relrowsecurity AND rec.relforcerowsecurity) THEN
      problems := problems || format('%s can use table %s but row security is not enabled and forced', rec.role_name, rec.rel);
    ELSIF rec.relkind = 'v' AND NOT (rec.reloptions && ARRAY['security_invoker=true', 'security_invoker=on', 'security_invoker=1']) THEN
      problems := problems || format('%s can use view %s which is not security_invoker', rec.role_name, rec.rel);
    ELSIF rec.relkind IN ('m', 'f') THEN
      problems := problems || format('%s can use %s relation %s, which cannot enforce row security', rec.role_name, rec.relkind, rec.rel);
    END IF;
  END LOOP;

  -- 7. Denied schemas, tables and columns: runtime-class AND read-only roles have no path into account, identity, ticket
  --    or finance data, and hold NO privilege (table or column level) on any token, ticket, credential, secret,
  --    password or session relation (outside the exceptions list) nor on any column NAMED like one, whatever its table.
  FOR rec IN SELECT r AS role_name, n.nspname
               FROM pg_namespace n CROSS JOIN unnest(denied_roles) AS r
              WHERE n.nspname ~ denied_schema_re AND has_schema_privilege(r, n.nspname, 'USAGE')
  LOOP
    problems := problems || format('%s has USAGE on denied schema %s', rec.role_name, rec.nspname);
  END LOOP;
  FOR rec IN SELECT r AS role_name, c.oid::regclass::text AS rel
               FROM pg_class c
               JOIN pg_namespace n ON n.oid = c.relnamespace
               CROSS JOIN unnest(denied_roles) AS r
              WHERE n.nspname ~ schema_re AND c.relkind IN ('r', 'p', 'v', 'm', 'f') AND c.relname ~ denied_table_re
                AND NOT ((n.nspname || '.' || c.relname) = ANY (denied_table_exc))
                AND (has_any_column_privilege(r, c.oid, 'SELECT') OR has_any_column_privilege(r, c.oid, 'INSERT')
                  OR has_any_column_privilege(r, c.oid, 'UPDATE') OR has_table_privilege(r, c.oid, 'DELETE')
                  OR has_table_privilege(r, c.oid, 'TRUNCATE'))
  LOOP
    problems := problems || format('%s holds a privilege on %s, a token/ticket/credential/secret/password/session relation', rec.role_name, rec.rel);
  END LOOP;
  FOR rec IN SELECT r AS role_name, c.oid::regclass::text AS rel, a.attname
               FROM pg_attribute a
               JOIN pg_class c ON c.oid = a.attrelid
               JOIN pg_namespace n ON n.oid = c.relnamespace
               CROSS JOIN unnest(denied_roles) AS r
              WHERE n.nspname ~ schema_re AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
                AND a.attnum > 0 AND NOT a.attisdropped AND a.attname ~ denied_column_re
                AND NOT ((n.nspname || '.' || c.relname || '.' || a.attname) = ANY (denied_column_exc))
                AND (has_column_privilege(r, c.oid, a.attnum, 'SELECT') OR has_column_privilege(r, c.oid, a.attnum, 'INSERT')
                  OR has_column_privilege(r, c.oid, a.attnum, 'UPDATE'))
  LOOP
    problems := problems || format('%s can use column %s of %s, a token/ticket/credential/secret/password column', rec.role_name, rec.attname, rec.rel);
  END LOOP;

  -- 8. Functions in application schemas are not executable by PUBLIC (the PostgreSQL default).
  FOR rec IN SELECT p.oid::regprocedure::text AS fn
               FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
              WHERE n.nspname ~ schema_re
                AND (p.proacl IS NULL
                     OR EXISTS (SELECT 1 FROM aclexplode(p.proacl) a
                                 WHERE a.grantee = 0 AND a.privilege_type = 'EXECUTE'))
  LOOP
    problems := problems || format('function %s is executable by PUBLIC', rec.fn);
  END LOOP;

  -- 9. SECURITY DEFINER functions in application schemas: a pinned search_path that ends in pg_temp, an owner that
  --    is neither superuser nor BYPASSRLS, and no serving role able to CREATE in any schema on that search_path.
  --    (The owner is the migrator, which holds CREATE on the database by design; that is why the rule is about the
  --    schemas on the search_path, not about the owner's database privileges.)
  FOR rec IN SELECT p.oid::regprocedure::text AS fn, p.proconfig, r.rolsuper, r.rolbypassrls, r.rolname AS owner
               FROM pg_proc p
               JOIN pg_namespace n ON n.oid = p.pronamespace
               JOIN pg_roles r ON r.oid = p.proowner
              WHERE p.prosecdef AND n.nspname ~ schema_re
  LOOP
    IF rec.rolsuper OR rec.rolbypassrls THEN
      problems := problems || format('SECURITY DEFINER function %s is owned by %s, which is superuser or BYPASSRLS', rec.fn, rec.owner);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM unnest(COALESCE(rec.proconfig, ARRAY[]::text[])) c WHERE c ~ '^search_path=.*pg_temp\s*$') THEN
      problems := problems || format('SECURITY DEFINER function %s has no pinned search_path ending in pg_temp', rec.fn);
    ELSE
      FOR schema_name IN
        SELECT btrim(s) FROM unnest(string_to_array(
          regexp_replace((SELECT c FROM unnest(rec.proconfig) c WHERE c LIKE 'search_path=%' LIMIT 1), '^search_path=', ''), ',')) AS s
      LOOP
        IF schema_name IN ('pg_catalog', 'pg_temp') OR NOT EXISTS (SELECT 1 FROM pg_namespace WHERE nspname = schema_name) THEN
          CONTINUE;
        END IF;
        FOREACH role_name IN ARRAY (roles || readonly_roles) LOOP
          IF has_schema_privilege(role_name, schema_name, 'CREATE') THEN
            problems := problems || format('%s can CREATE in schema %s, which is on the search_path of SECURITY DEFINER function %s', role_name, schema_name, rec.fn);
          END IF;
        END LOOP;
      END LOOP;
    END IF;
  END LOOP;

  -- 10. Read-only roles hold SELECT and nothing else: no write, reference or trigger privilege on any relation (column
  --     level included) and no UPDATE on a sequence. default_transaction_read_only is not a control (a user can reset it).
  FOR rec IN SELECT r AS role_name, c.oid::regclass::text AS rel, c.relkind, p
               FROM pg_class c
               JOIN pg_namespace n ON n.oid = c.relnamespace
               CROSS JOIN unnest(readonly_roles) AS r
               CROSS JOIN unnest(ARRAY['INSERT', 'UPDATE', 'REFERENCES', 'DELETE', 'TRUNCATE', 'TRIGGER']) AS p
              WHERE n.nspname NOT LIKE 'pg\_%' AND n.nspname <> 'information_schema'
                AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
                AND CASE WHEN p IN ('INSERT', 'UPDATE', 'REFERENCES') THEN has_any_column_privilege(r, c.oid, p)
                         ELSE has_table_privilege(r, c.oid, p) END
  LOOP
    problems := problems || format('read-only role %s has %s on %s', rec.role_name, rec.p, rec.rel);
  END LOOP;
  FOR rec IN SELECT r AS role_name, c.oid::regclass::text AS rel
               FROM pg_class c
               JOIN pg_namespace n ON n.oid = c.relnamespace
               CROSS JOIN unnest(readonly_roles) AS r
              WHERE n.nspname NOT LIKE 'pg\_%' AND n.nspname <> 'information_schema' AND c.relkind = 'S'
                AND (has_sequence_privilege(r, c.oid, 'UPDATE') OR has_sequence_privilege(r, c.oid, 'USAGE'))
  LOOP
    problems := problems || format('read-only role %s has USAGE or UPDATE on sequence %s (nextval)', rec.role_name, rec.rel);
  END LOOP;

  -- 11. The bootstrap migration ran as the migrator: its default privileges remove PUBLIC EXECUTE.
  IF current_setting('df.check_default_acl') = 'on' THEN
    IF NOT EXISTS (SELECT 1 FROM pg_default_acl d JOIN pg_roles r ON r.oid = d.defaclrole
                    WHERE r.rolname = migrator AND d.defaclobjtype = 'f'
                      AND NOT EXISTS (SELECT 1 FROM aclexplode(d.defaclacl) a
                                       WHERE a.grantee = 0 AND a.privilege_type = 'EXECUTE')) THEN
      problems := problems || format('no default ACL removes PUBLIC EXECUTE on functions created by %s (bootstrap migration missing)', migrator);
    END IF;
  END IF;

  IF array_length(problems, 1) > 0 THEN
    RAISE EXCEPTION E'privilege violations:\n%', array_to_string(problems, E'\n');
  END IF;
  RAISE NOTICE 'privileges ok';
END
$verify$;
