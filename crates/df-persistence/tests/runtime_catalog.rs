//! The catalog checks of the hosted database (hosted runtime design 4.3, 4.4) over the WHOLE
//! hosted migration set (every owner's files, in order): no relation granted to a service role
//! escapes forced row security, every definer function is owned by the NOLOGIN definer and fixes
//! its search_path, and nothing a later migration adds is executable by PUBLIC. A later
//! migration cannot add an unprotected granted table, view or function without these failing.
//! Tests that need a server report SKIPPED without one.

mod runtime_support;

use std::{collections::BTreeSet, fs, path::Path};

use runtime_support::*;
use tokio_postgres::Client;

/// Roles a runtime or service connects as, once the deployment gives them a login.
const SERVICE_ROLES: [&str; 9] = [
    "df_campaign_runtime",
    "df_egress",
    "df_importer",
    "df_control",
    "df_runner",
    "df_finance",
    "df_finance_replay",
    "df_story_maintenance",
    "df_runtime_definer",
];

/// Roles that reach tenant data through a shared login: the strict rules apply to every schema.
const TENANT_DATA_ROLES: [&str; 3] = ["df_campaign_runtime", "df_egress", "df_importer"];

/// The schemas this note owns; the other service roles are walked here.
const OWNED_SCHEMAS: [&str; 4] = ["df_runtime", "df_meta", "df_game", "df_story"];

/// Roles whose policies must be row-scoped: the tenant-data roles and the story maintenance role.
const STRICT_POLICY_ROLES: [&str; 4] = [
    "df_campaign_runtime",
    "df_egress",
    "df_importer",
    "df_story_maintenance",
];

/// Policies allowed to be constant, keyed on the policy NAME and COMMAND (`r` select, `a`
/// insert, `w` update, `d` delete, `*` all), never on the table or the role, each with its
/// reason. A new policy with another name, or the same name for another command, is flagged.
const NAMED_EXCEPTIONS: [(&str, char, &str); 2] = [
    (
        "dispatcher_read",
        'r',
        "the re-fetch queue is global by design; claims are scoped by dispatcher instance (10.4)",
    ),
    (
        "maintenance_all",
        '*',
        "erasure and index rebuild act on every campaign by design (4.4 item 5); the role has \
         no login until an operator run",
    ),
];

struct Granted {
    table: String,
    schema: String,
    role: String,
    kind: String,
    protected: bool,
    security_invoker: bool,
}

/// Every relation (table, partitioned table, view, materialised view, foreign table) on which a
/// service role holds any privilege, including a column privilege.
async fn granted_relations(client: &Client) -> Vec<Granted> {
    let roles: Vec<String> = SERVICE_ROLES.iter().map(|role| role.to_string()).collect();
    client
        .query(
            "SELECT n.nspname, n.nspname || '.' || c.relname, r.rolname, c.relkind::text, \
             c.relrowsecurity AND c.relforcerowsecurity AND EXISTS (SELECT 1 FROM pg_policy p \
                 WHERE p.polrelid = c.oid AND (p.polroles = '{0}' OR r.oid = ANY (p.polroles))), \
             COALESCE(c.reloptions @> ARRAY['security_invoker=true'] \
                 OR c.reloptions @> ARRAY['security_invoker=on'], false) \
             FROM pg_class c \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             CROSS JOIN pg_roles r \
             WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f') \
               AND n.nspname NOT IN ('pg_catalog', 'information_schema', 'pg_toast') \
               AND r.rolname = ANY ($1) \
               AND (has_table_privilege(r.oid, c.oid, 'SELECT, INSERT, UPDATE, DELETE, TRUNCATE, REFERENCES, TRIGGER') \
                    OR has_any_column_privilege(r.oid, c.oid, 'SELECT, INSERT, UPDATE, REFERENCES')) \
             ORDER BY 2, 3",
            &[&roles],
        )
        .await
        .expect("catalog walk")
        .iter()
        .map(|row| Granted {
            schema: row.get(0),
            table: row.get(1),
            role: row.get(2),
            kind: row.get(3),
            protected: row.get(4),
            security_invoker: row.get(5),
        })
        .collect()
}

/// What is wrong with each grant, as readable lines.
fn problems(granted: &[Granted]) -> Vec<String> {
    let mut found = Vec::new();
    for grant in granted {
        if grant.role == "df_runtime_definer" {
            // It owns the tables it works on, including the two that carry no row security by design.
            continue;
        }
        let strict = TENANT_DATA_ROLES.contains(&grant.role.as_str());
        if !strict && !OWNED_SCHEMAS.contains(&grant.schema.as_str()) {
            continue;
        }
        let name = format!("{} on {}", grant.role, grant.table);
        match grant.kind.as_str() {
            "r" | "p" if !grant.protected => {
                found.push(format!(
                    "{name}: no forced row security with a policy for the role"
                ));
            }
            "v" if !grant.security_invoker => {
                found.push(format!(
                    "{name}: a view without security_invoker runs as its owner"
                ));
            }
            "m" | "f" => {
                found.push(format!(
                    "{name}: a materialised view or foreign table has no row security"
                ));
            }
            _ => {}
        }
    }
    found
}

/// One policy that applies to a strict role.
struct Policy {
    table: String,
    role: String,
    name: String,
    command: char,
    using: String,
    with_check: String,
    references_a_column: bool,
}

async fn strict_policies(client: &Client) -> Vec<Policy> {
    let roles: Vec<String> = STRICT_POLICY_ROLES
        .iter()
        .map(|role| role.to_string())
        .collect();
    client
        .query(
            "SELECT n.nspname || '.' || c.relname, r.rolname, p.polname::text, p.polcmd::text, \
             COALESCE(pg_get_expr(p.polqual, p.polrelid), ''), \
             COALESCE(pg_get_expr(p.polwithcheck, p.polrelid), ''), \
             EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_policy'::regclass \
                 AND d.objid = p.oid AND d.refclassid = 'pg_class'::regclass \
                 AND d.refobjid = p.polrelid AND d.refobjsubid > 0) \
             FROM pg_policy p \
             JOIN pg_class c ON c.oid = p.polrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             CROSS JOIN pg_roles r \
             WHERE r.rolname = ANY ($1) \
               AND (p.polroles = '{0}' OR r.oid = ANY (p.polroles)) \
               AND n.nspname NOT IN ('pg_catalog', 'information_schema') \
             ORDER BY 1, 2, 3",
            &[&roles],
        )
        .await
        .expect("policy walk")
        .iter()
        .map(|row| Policy {
            table: row.get(0),
            role: row.get(1),
            name: row.get(2),
            command: row.get::<_, String>(3).chars().next().unwrap_or('?'),
            using: row.get(4),
            with_check: row.get(5),
            references_a_column: row.get(6),
        })
        .collect()
}

/// Whether the expression text decides nothing about a row: it is made only of literals,
/// operators and the words true, false, null, and, or, not, is. `pg_get_expr` prints casts as
/// `::type` and quotes literals, both stripped first, so `1 = 1`, `('a'::text = 'a'::text)`
/// and `NOT false` are constant and `(id = 1)` is not.
fn is_constant_expression(text: &str) -> bool {
    if text.trim().is_empty() {
        return false;
    }
    let mut words = Vec::new();
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '\'' => {
                for inner in characters.by_ref() {
                    if inner == '\'' {
                        break;
                    }
                }
            }
            ':' if characters.peek() == Some(&':') => {
                characters.next();
                while characters
                    .peek()
                    .is_some_and(|next| next.is_ascii_alphanumeric() || *next == '_')
                {
                    characters.next();
                }
            }
            letter if letter.is_ascii_alphabetic() || letter == '_' => {
                let mut word = String::from(letter);
                while let Some(next) = characters.peek().copied() {
                    if next.is_ascii_alphanumeric() || next == '_' {
                        word.push(next);
                        characters.next();
                    } else {
                        break;
                    }
                }
                words.push(word.to_ascii_lowercase());
            }
            _ => {}
        }
    }
    words
        .iter()
        .all(|word| ["true", "false", "null", "and", "or", "not", "is"].contains(&word.as_str()))
}

/// Policies of the strict roles that decide nothing about a row, unless a named exception
/// covers that name and command.
fn policy_problems(policies: &[Policy]) -> Vec<String> {
    policies
        .iter()
        .filter(|policy| {
            !policy.references_a_column
                || is_constant_expression(&policy.using)
                || is_constant_expression(&policy.with_check)
        })
        .filter(|policy| {
            !NAMED_EXCEPTIONS
                .iter()
                .any(|(name, command, _)| *name == policy.name && *command == policy.command)
        })
        .map(|policy| {
            format!(
                "{} on {}: policy {} ({}) is constant or row-independent",
                policy.role, policy.table, policy.name, policy.command
            )
        })
        .collect()
}

async fn strings(client: &Client, sql: &str) -> Vec<String> {
    client
        .query(sql, &[])
        .await
        .expect("catalog query")
        .iter()
        .map(|row| row.get(0))
        .collect()
}

#[test]
fn every_relation_granted_to_a_service_role_is_protected_across_the_whole_hosted_migration_set() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let granted = granted_relations(&client).await;
        let found = problems(&granted);
        assert!(found.is_empty(), "unprotected grants: {found:#?}");
        let loose = policy_problems(&strict_policies(&client).await);
        assert!(
            loose.is_empty(),
            "constant or row-independent policies: {loose:#?}"
        );
        let seen: Vec<(&str, &str)> = granted
            .iter()
            .map(|grant| (grant.table.as_str(), grant.role.as_str()))
            .collect();
        for expected in [
            ("df_game.sessions", "df_campaign_runtime"),
            ("df_story.story_memory", "df_campaign_runtime"),
            ("df_runtime.refetch_requests", "df_egress"),
            ("df_runtime.campaign_owners", "df_control"),
            ("df_game.checkpoints", "df_importer"),
        ] {
            assert!(seen.contains(&expected), "the walk must see {expected:?}");
        }
        for forbidden in [
            ("df_runtime.campaign_owners", "df_campaign_runtime"),
            ("df_runtime.placement_secrets", "df_campaign_runtime"),
            ("df_runtime.placement_secrets", "df_control"),
            ("df_runtime.import_jobs", "df_importer"),
            ("df_meta.service_state", "df_campaign_runtime"),
            ("df_game.sessions", "df_control"),
        ] {
            assert!(
                !seen.contains(&forbidden),
                "{forbidden:?} must not be granted"
            );
        }
        scratch.finish().await;
    });
}

#[test]
fn the_catalog_walk_flags_what_a_later_migration_could_get_wrong() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        client
            .batch_execute(
                "CREATE TABLE df_runtime.later_open (id integer); \
                 GRANT SELECT ON df_runtime.later_open TO df_campaign_runtime; \
                 CREATE TABLE df_runtime.later_unforced (id integer); \
                 ALTER TABLE df_runtime.later_unforced ENABLE ROW LEVEL SECURITY; \
                 CREATE POLICY scoped ON df_runtime.later_unforced USING (id = 1); \
                 GRANT SELECT ON df_runtime.later_unforced TO df_egress; \
                 CREATE TABLE df_runtime.later_policyless (id integer); \
                 ALTER TABLE df_runtime.later_policyless ENABLE ROW LEVEL SECURITY; \
                 ALTER TABLE df_runtime.later_policyless FORCE ROW LEVEL SECURITY; \
                 GRANT SELECT (id) ON df_runtime.later_policyless TO df_control; \
                 CREATE TABLE df_runtime.later_other_role (id integer); \
                 ALTER TABLE df_runtime.later_other_role ENABLE ROW LEVEL SECURITY; \
                 ALTER TABLE df_runtime.later_other_role FORCE ROW LEVEL SECURITY; \
                 CREATE POLICY only_egress ON df_runtime.later_other_role TO df_egress USING (id = 1); \
                 GRANT SELECT ON df_runtime.later_other_role TO df_control; \
                 CREATE TABLE df_runtime.later_constant (id integer); \
                 ALTER TABLE df_runtime.later_constant ENABLE ROW LEVEL SECURITY; \
                 ALTER TABLE df_runtime.later_constant FORCE ROW LEVEL SECURITY; \
                 CREATE POLICY everything ON df_runtime.later_constant TO df_campaign_runtime USING (true); \
                 GRANT SELECT ON df_runtime.later_constant TO df_campaign_runtime; \
                 CREATE TABLE df_runtime.later_import (id integer); \
                 ALTER TABLE df_runtime.later_import ENABLE ROW LEVEL SECURITY; \
                 ALTER TABLE df_runtime.later_import FORCE ROW LEVEL SECURITY; \
                 CREATE POLICY everything ON df_runtime.later_import FOR INSERT TO df_importer WITH CHECK (true); \
                 GRANT INSERT ON df_runtime.later_import TO df_importer; \
                 CREATE TABLE df_runtime.later_expressions (id integer); \
                 ALTER TABLE df_runtime.later_expressions ENABLE ROW LEVEL SECURITY; \
                 ALTER TABLE df_runtime.later_expressions FORCE ROW LEVEL SECURITY; \
                 CREATE POLICY one_equals_one ON df_runtime.later_expressions \
                     TO df_campaign_runtime USING (1 = 1); \
                 CREATE POLICY no_column ON df_runtime.later_expressions FOR SELECT \
                     TO df_egress USING (current_setting('x', true) = 'y'); \
                 CREATE POLICY dispatcher_read ON df_runtime.later_expressions FOR SELECT \
                     TO df_importer USING (true); \
                 CREATE POLICY maintenance_all ON df_runtime.later_expressions \
                     TO df_story_maintenance USING (true); \
                 CREATE POLICY erase_all ON df_runtime.later_expressions FOR DELETE \
                     TO df_story_maintenance USING (true); \
                 CREATE POLICY scoped_ok ON df_runtime.later_expressions FOR UPDATE \
                     TO df_campaign_runtime USING (id = 1); \
                 GRANT SELECT, INSERT, UPDATE, DELETE ON df_runtime.later_expressions \
                     TO df_campaign_runtime, df_egress, df_importer, df_story_maintenance; \
                 CREATE TABLE df_runtime.later_dispatcher_insert (id integer); \
                 ALTER TABLE df_runtime.later_dispatcher_insert ENABLE ROW LEVEL SECURITY; \
                 ALTER TABLE df_runtime.later_dispatcher_insert FORCE ROW LEVEL SECURITY; \
                 CREATE POLICY dispatcher_read ON df_runtime.later_dispatcher_insert FOR INSERT \
                     TO df_egress WITH CHECK (true); \
                 GRANT INSERT ON df_runtime.later_dispatcher_insert TO df_egress; \
                 CREATE VIEW df_runtime.later_leak AS SELECT token_digest FROM df_runtime.placement_secrets; \
                 GRANT SELECT ON df_runtime.later_leak TO df_campaign_runtime; \
                 CREATE MATERIALIZED VIEW df_runtime.later_matview AS SELECT 1 AS one; \
                 GRANT SELECT ON df_runtime.later_matview TO df_egress; \
                 CREATE VIEW df_runtime.later_invoker WITH (security_invoker = true) \
                     AS SELECT session_id FROM df_game.sessions; \
                 GRANT SELECT ON df_runtime.later_invoker TO df_campaign_runtime",
            )
            .await
            .expect("later migration");
        let found = problems(&granted_relations(&client).await);
        for expected in [
            "df_campaign_runtime on df_runtime.later_open: no forced row security",
            "df_egress on df_runtime.later_unforced: no forced row security",
            "df_control on df_runtime.later_policyless: no forced row security",
            "df_control on df_runtime.later_other_role: no forced row security",
            "df_campaign_runtime on df_runtime.later_leak: a view without security_invoker",
            "df_egress on df_runtime.later_matview: a materialised view or foreign table",
        ] {
            assert!(
                found.iter().any(|line| line.starts_with(expected)),
                "the walk must flag {expected}: {found:#?}"
            );
        }
        assert!(
            !found.iter().any(|line| line.contains("later_invoker")),
            "a security_invoker view over a protected table is allowed: {found:#?}"
        );
        let loose = policy_problems(&strict_policies(&client).await);
        for expected in [
            "df_campaign_runtime on df_runtime.later_constant: policy everything (*)",
            "df_importer on df_runtime.later_import: policy everything (a)",
            "df_campaign_runtime on df_runtime.later_expressions: policy one_equals_one (*)",
            "df_egress on df_runtime.later_expressions: policy no_column (r)",
            "df_egress on df_runtime.later_dispatcher_insert: policy dispatcher_read (a)",
            "df_story_maintenance on df_runtime.later_expressions: policy erase_all (d)",
        ] {
            assert!(
                loose.iter().any(|line| line.starts_with(expected)),
                "the policy walk must flag {expected}: {loose:#?}"
            );
        }
        for allowed in [
            "later_expressions: policy scoped_ok",
            "later_expressions: policy maintenance_all",
            "df_importer on df_runtime.later_expressions: policy dispatcher_read (r)",
        ] {
            let flagged = loose.iter().any(|line| line.contains(allowed));
            // The first is row-scoped. The other two are covered by a named exception whatever the
            // table or role, which is the ruling; a rename or another command is flagged above.
            assert!(!flagged, "`{allowed}` must not be flagged: {loose:#?}");
        }
        scratch.finish().await;
    });
}

#[test]
fn every_security_definer_function_fixes_a_search_path_that_ends_in_pg_temp() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let functions = client
            .query(
                "SELECT p.oid::regprocedure::text, \
                 (SELECT substr(setting, 13) FROM unnest(p.proconfig) AS setting \
                   WHERE setting LIKE 'search_path=%') \
                 FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
                 WHERE p.prosecdef AND n.nspname NOT IN ('pg_catalog', 'information_schema')",
                &[],
            )
            .await
            .expect("definer functions");
        assert!(
            functions.len() > 20,
            "the walk must see every owner's definer functions"
        );
        let mut unsafe_paths = Vec::new();
        for row in &functions {
            let (name, path): (String, Option<String>) = (row.get(0), row.get(1));
            let ends_in_pg_temp = path
                .as_deref()
                .is_some_and(|path| path.rsplit(',').next().map(str::trim) == Some("pg_temp"));
            if !ends_in_pg_temp {
                unsafe_paths.push(format!("{name}: {path:?}"));
            }
        }
        assert!(
            unsafe_paths.is_empty(),
            "search_path must end in pg_temp: {unsafe_paths:?}"
        );
        scratch.finish().await;
    });
}

#[test]
fn the_definer_never_logs_in_and_no_service_role_is_privileged_or_holds_temp_or_owns_a_relation() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let roles = SERVICE_ROLES.join("', '");
        let privileged = strings(
            &client,
            &format!(
                "SELECT rolname FROM pg_roles WHERE rolname IN ('{roles}') \
                 AND (rolsuper OR rolbypassrls OR rolcreatedb OR rolcreaterole OR rolreplication)"
            ),
        )
        .await;
        assert!(privileged.is_empty(), "{privileged:?}");
        let login = strings(
            &client,
            "SELECT rolname FROM pg_roles WHERE rolname = 'df_runtime_definer' AND rolcanlogin",
        )
        .await;
        assert!(login.is_empty(), "the definer must be NOLOGIN: {login:?}");
        let temp = strings(
            &client,
            &format!(
                "SELECT rolname FROM pg_roles WHERE rolname IN ('{roles}') \
                 AND has_database_privilege(oid, current_database(), 'TEMP')"
            ),
        )
        .await;
        assert!(
            temp.is_empty(),
            "no service role may create pg_temp objects: {temp:?}"
        );
        let owners = strings(
            &client,
            &format!(
                "SELECT DISTINCT r.rolname FROM pg_class c JOIN pg_roles r ON r.oid = c.relowner \
                 WHERE r.rolname IN ('{roles}') AND r.rolname <> 'df_runtime_definer' \
                 AND c.relkind IN ('r', 'p', 'v', 'm', 'f')"
            ),
        )
        .await;
        assert!(
            owners.is_empty(),
            "no login-capable service role owns a relation: {owners:?}"
        );
        let memberships = strings(
            &client,
            &format!(
                "SELECT m.rolname || ' in ' || g.rolname FROM pg_auth_members a \
                 JOIN pg_roles m ON m.oid = a.member JOIN pg_roles g ON g.oid = a.roleid \
                 WHERE m.rolname IN ('{roles}') ORDER BY 1"
            ),
        )
        .await;
        assert_eq!(
            memberships,
            ["df_runtime_definer in pg_signal_backend"],
            "the definer alone may signal backends (the restore terminates a stuck lock holder)"
        );
        scratch.finish().await;
    });
}

#[test]
fn the_definer_owns_every_df_meta_and_df_runtime_object() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        for (what, sql) in [
            (
                "function",
                "SELECT p.oid::regprocedure::text FROM pg_proc p \
                 JOIN pg_namespace n ON n.oid = p.pronamespace \
                 WHERE n.nspname IN ('df_meta', 'df_runtime') \
                   AND pg_get_userbyid(p.proowner) <> 'df_runtime_definer'",
            ),
            (
                "relation",
                "SELECT n.nspname || '.' || c.relname FROM pg_class c \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname IN ('df_meta', 'df_runtime') \
                   AND c.relkind IN ('r', 'p', 'v', 'm', 'f', 'S') \
                   AND pg_get_userbyid(c.relowner) <> 'df_runtime_definer'",
            ),
            (
                "schema",
                "SELECT nspname FROM pg_namespace WHERE nspname IN ('df_meta', 'df_runtime') \
                   AND pg_get_userbyid(nspowner) <> 'df_runtime_definer'",
            ),
            (
                "type",
                "SELECT n.nspname || '.' || t.typname FROM pg_type t \
                 JOIN pg_namespace n ON n.oid = t.typnamespace \
                 WHERE n.nspname IN ('df_meta', 'df_runtime') AND t.typtype IN ('e', 'd') \
                   AND pg_get_userbyid(t.typowner) <> 'df_runtime_definer'",
            ),
        ] {
            let others = strings(&client, sql).await;
            assert!(
                others.is_empty(),
                "{what} not owned by the definer: {others:?}"
            );
        }
        let counted: i64 = client
            .query_one(
                "SELECT count(*) FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
                 WHERE n.nspname IN ('df_meta', 'df_runtime') \
                   AND pg_get_userbyid(p.proowner) = 'df_runtime_definer'",
                &[],
            )
            .await
            .expect("count")
            .get(0);
        assert!(
            counted > 30,
            "the check must see the directory's functions, saw {counted}"
        );
        scratch.finish().await;
    });
}

#[test]
fn a_function_added_later_is_born_without_public_execution_and_adoption_gives_it_to_the_definer() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        client
            .batch_execute(
                "CREATE FUNCTION df_runtime.added_later() RETURNS integer LANGUAGE sql AS 'SELECT 1'",
            )
            .await
            .expect("a later migration's function");
        let public_execute = "SELECT count(*) FROM pg_proc p WHERE p.proname = 'added_later' \
             AND (p.proacl IS NULL OR EXISTS (SELECT 1 FROM aclexplode(p.proacl) a \
                  WHERE a.grantee = 0 AND a.privilege_type = 'EXECUTE'))";
        let open: i64 = client
            .query_one(public_execute, &[])
            .await
            .expect("acl")
            .get(0);
        assert_eq!(
            open, 0,
            "default privileges keep the new function away from PUBLIC"
        );
        let creator = strings(
            &client,
            "SELECT pg_get_userbyid(proowner) FROM pg_proc WHERE proname = 'added_later'",
        )
        .await;
        assert_ne!(creator, ["df_runtime_definer"], "created by the migrator");
        client
            .batch_execute("SELECT df_meta.adopt_objects()")
            .await
            .expect("adopt");
        let owner = strings(
            &client,
            "SELECT pg_get_userbyid(proowner) FROM pg_proc WHERE proname = 'added_later'",
        )
        .await;
        assert_eq!(owner, ["df_runtime_definer"]);
        scratch.finish().await;
    });
}

#[test]
fn no_function_in_a_runtime_schema_is_executable_by_public() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let open = strings(
            &client,
            "SELECT p.oid::regprocedure::text FROM pg_proc p \
             JOIN pg_namespace n ON n.oid = p.pronamespace \
             WHERE n.nspname IN ('df_runtime', 'df_meta') \
               AND (p.proacl IS NULL OR EXISTS (SELECT 1 FROM aclexplode(p.proacl) a \
                    WHERE a.grantee = 0 AND a.privilege_type = 'EXECUTE'))",
        )
        .await;
        assert!(open.is_empty(), "revoke EXECUTE from PUBLIC: {open:?}");
        scratch.finish().await;
    });
}

#[test]
fn the_role_settings_and_connection_limit_are_what_the_design_says() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let settings = strings(
            &client,
            "SELECT r.rolname || ' ' || s FROM pg_db_role_setting d \
             JOIN pg_roles r ON r.oid = d.setrole, unnest(d.setconfig) AS s \
             WHERE d.setdatabase = 0 ORDER BY 1",
        )
        .await;
        for expected in [
            "df_campaign_runtime idle_in_transaction_session_timeout=5s",
            "df_campaign_runtime statement_timeout=30s",
            "df_control idle_in_transaction_session_timeout=5s",
            "df_control statement_timeout=30s",
            "df_runner idle_in_transaction_session_timeout=5s",
            "df_runner statement_timeout=10s",
            "df_egress idle_in_transaction_session_timeout=5s",
            "df_egress statement_timeout=30s",
        ] {
            assert!(
                settings.iter().any(|line| line == expected),
                "missing `{expected}` in {settings:#?}"
            );
        }
        // F's 0024 sets its own, longer bounds for the finance roles (15 s and 60 s idle): they
        // must still carry one, since they take the shared service lock.
        for role in ["df_finance", "df_finance_replay"] {
            assert!(
                settings.iter().any(|line| line
                    .starts_with(&format!("{role} idle_in_transaction_session_timeout="))),
                "{role} needs an idle-in-transaction bound"
            );
        }
        let limit: i32 = client
            .query_one(
                "SELECT rolconnlimit FROM pg_roles WHERE rolname = 'df_control'",
                &[],
            )
            .await
            .expect("limit")
            .get(0);
        assert_eq!(limit, 24);
        scratch.finish().await;
    });
}

#[test]
fn every_hosted_migration_file_is_in_the_harness() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let on_disk: BTreeSet<String> = fs::read_dir(directory)
        .expect("migrations directory")
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.ends_with(".sql"))
        .collect();
    let known: BTreeSet<String> = MIGRATIONS
        .iter()
        .map(|migration| migration.file.to_string())
        .chain(NOT_HOSTED.iter().map(|file| file.to_string()))
        .collect();
    let unlisted: Vec<&String> = on_disk.difference(&known).collect();
    assert!(
        unlisted.is_empty(),
        "add these migrations to runtime_support::MIGRATIONS so the catalog walk covers them: {unlisted:?}"
    );
    let missing: Vec<&String> = known.difference(&on_disk).collect();
    assert!(missing.is_empty(), "listed but absent: {missing:?}");
}

#[test]
fn constant_expressions_are_recognised_by_parsing_and_row_scoped_ones_are_not() {
    for constant in [
        "true",
        "(1 = 1)",
        "1=1",
        "('a'::text = 'a'::text)",
        "(NOT false)",
        "(true AND (2 > 1))",
        "(NULL::boolean IS NOT TRUE)",
    ] {
        assert!(
            is_constant_expression(constant),
            "`{constant}` decides nothing about a row"
        );
    }
    for scoped in [
        "",
        "(id = 1)",
        "((tenant_id, campaign_id) = ( SELECT l.tenant_id, l.campaign_id FROM df_runtime.leased_campaign() l(tenant_id, campaign_id)))",
        "(outcome IS NULL)",
        "(true AND (id = 1))",
    ] {
        assert!(
            !is_constant_expression(scoped),
            "`{scoped}` depends on the row"
        );
    }
}
