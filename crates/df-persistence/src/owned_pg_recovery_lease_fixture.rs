//! Deterministic recovery-lease checks on the parent's registered owned PostgreSQL.
use super::{
    CommitOutcome, Config, Duration, FixtureBoundInput, FixtureGrant, FixtureProof, Instant,
    OperationContext, OperationLookup, RecoverySource, ReferenceInventory, RepositoryError,
    SessionId, SessionRepository, TransactionBounds, actor_block_on, admitted_case,
    bind_registered_scope, codec_limits, compose_registered_repository, connect_fixture_authority,
    content, fixture_database_now, fixture_lost_ack_configuration, limits, pins,
    resource_constraints, retain_fixture_repository_after_retry, retained_fixture_all_family_bytes,
    rule, seed_grant, seed_proof, seed_session, within_deadline,
};

const INSTALL_PROBE: &str = r#"
CREATE SEQUENCE df_fixture_authority.recovery_lease_calls MINVALUE 1 MAXVALUE 64 NO CYCLE;
CREATE TABLE df_fixture_authority.recovery_lease_probe (
    singleton boolean PRIMARY KEY CHECK (singleton),
    expire_on_call integer NOT NULL CHECK (expire_on_call IN (0, 2))
);
INSERT INTO df_fixture_authority.recovery_lease_probe VALUES (true, 0);
CREATE FUNCTION df_fixture_authority.probe_recovery_lease(p_tenant bytea, p_session bytea)
RETURNS boolean LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    call_number bigint;
    expire_on integer;
    changed integer;
BEGIN
    SELECT expire_on_call INTO STRICT expire_on
        FROM df_fixture_authority.recovery_lease_probe WHERE singleton;
    call_number := nextval('df_fixture_authority.recovery_lease_calls');
    IF call_number = expire_on THEN
        UPDATE df_game.sessions SET lease_until = clock_timestamp()
            WHERE tenant_id = p_tenant AND session_id = p_session;
        GET DIAGNOSTICS changed = ROW_COUNT;
        IF changed <> 1 THEN RAISE EXCEPTION 'recovery lease probe did not change one row'; END IF;
    END IF;
    RETURN true;
END;
$$;
REVOKE ALL ON df_fixture_authority.recovery_lease_probe FROM PUBLIC;
REVOKE ALL ON SEQUENCE df_fixture_authority.recovery_lease_calls FROM PUBLIC;
REVOKE ALL ON FUNCTION df_fixture_authority.probe_recovery_lease(bytea, bytea) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION df_fixture_authority.probe_recovery_lease(bytea, bytea)
    TO df_persistence_fixture_runtime_a;
"#;

const REMOVE_PROBE: &str = r#"
DROP FUNCTION df_fixture_authority.probe_recovery_lease(bytea, bytea);
DROP TABLE df_fixture_authority.recovery_lease_probe;
DROP SEQUENCE df_fixture_authority.recovery_lease_calls;
"#;

// The existing issuer still performs every identity, membership and transaction-binding
// check. This predicate only interposes the registered fixture's lease expiry after it.
const PROBED_VERIFIER: &str = r#"
SELECT checked.*
FROM df_fixture_authority.bind_scope($1::bytea, $2::bytea) AS checked
WHERE df_fixture_authority.probe_recovery_lease(checked.tenant_id, $2::bytea)
"#;

pub(super) fn observe_registered_recovery_lease(
    handle: &tokio::runtime::Handle,
    admin: &mut tokio_postgres::Client,
    configuration: &Config,
    port: u16,
    bounds: TransactionBounds,
    stage: &std::cell::Cell<&'static str>,
) -> Result<(), RepositoryError> {
    let tenant = [120; 16];
    let principal = [121; 16];
    let campaign = [122; 16];
    let fence = [123; 16];
    let (baseline, candidate, input) = admitted_case(207);
    let session = baseline.basis().session;
    let rules = vec![rule()];
    let entries = vec![content()];
    let resources = resource_constraints();
    let context = OperationContext {
        trace_parent: String::new(),
        build: "persistence-owned-recovery-lease".to_owned(),
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    stage.set("recovery lease canonical seed and deterministic verifier interposition");
    let proof = actor_block_on(
        handle,
        within_deadline(deadline, async {
            let transaction = admin
                .transaction()
                .await
                .map_err(|_| RepositoryError::Unavailable)?;
            let grant = || FixtureGrant {
                service_role: "df_persistence_fixture_runtime_a",
                tenant: &tenant,
                principal: &principal,
                campaign: &campaign,
                role: b"gm",
                access_revision: b"fixture-recovery-lease-1",
                lifetime_seconds: 120,
            };
            seed_grant(&transaction, &grant()).await?;
            seed_session(
                &transaction,
                &tenant,
                &fence,
                &baseline,
                ReferenceInventory {
                    rules: &rules,
                    content: &entries,
                    resources: &resources,
                    assets: &[],
                },
                (limits(), codec_limits()),
                120,
            )
            .await?;
            let proof = seed_proof(
                &transaction,
                &FixtureProof {
                    grant: grant(),
                    basis: baseline.basis(),
                    operation: &[6; 16],
                    namespace: b"fixture/recovery-lease/v1",
                    canonical_fingerprint: &[124; 32],
                    fence: &fence,
                    mode: 3,
                    lookup_only: false,
                },
            )
            .await?;
            transaction
                .batch_execute(INSTALL_PROBE)
                .await
                .map_err(|_| RepositoryError::Unavailable)?;
            transaction
                .commit()
                .await
                .map_err(|_| RepositoryError::Unavailable)?;
            Ok::<_, RepositoryError>(proof)
        }),
    )?
    .map_err(|_| RepositoryError::Unavailable)??;

    let fixture_configuration = fixture_lost_ack_configuration(configuration, port)?;
    let mut authority =
        connect_fixture_authority(handle, &fixture_configuration, bounds, &context)?;
    let bound_scope = bind_registered_scope(
        &mut authority,
        &proof,
        FixtureBoundInput {
            input,
            pins: pins(),
        },
        fixture_database_now(handle, admin, deadline)?,
    );
    let (scope, mut verifier) = match bound_scope {
        Ok(value) => value,
        Err(error) => {
            actor_block_on(handle, authority.connection.close())??;
            return Err(error);
        }
    };
    let client = authority.connection.take_client()?;
    let prepared = actor_block_on(
        handle,
        within_deadline(deadline, async {
            let statement = client
                .prepare(PROBED_VERIFIER)
                .await
                .map_err(|_| RepositoryError::Unavailable)?;
            let activity_row = client.query_one(
            "SELECT pg_backend_pid(), backend_start::text FROM pg_stat_activity WHERE pid=pg_backend_pid()",
            &[],
        ).await.map_err(|_| RepositoryError::Unavailable)?;
            let backend_pid: i32 = activity_row
                .try_get(0)
                .map_err(|_| RepositoryError::Unavailable)?;
            let backend_start: String = activity_row
                .try_get(1)
                .map_err(|_| RepositoryError::Unavailable)?;
            if backend_pid <= 0 || backend_pid == 80873 || backend_start.is_empty() {
                return Err(RepositoryError::InvalidReceipt);
            }
            Ok::<_, RepositoryError>((statement, backend_pid, backend_start))
        }),
    );
    let returned = authority.connection.return_client(client);
    let prepared = prepared
        .and_then(|value| value.map_err(|_| RepositoryError::Unavailable))
        .and_then(|value| value)
        .and_then(|value| returned.map(|_| value));
    let (statement, backend_pid, backend_start) = match prepared {
        Ok(value) => value,
        Err(error) => {
            actor_block_on(handle, authority.connection.close())??;
            return Err(error);
        }
    };
    verifier.statement = Some(statement);
    let mut repository = compose_registered_repository(
        authority,
        codec_limits(),
        16384,
        verifier,
        RecoverySource {
            rules,
            content: entries,
            resources,
            resource_resolver: None,
            asset_resolver: None,
            assets: vec![],
            limits: limits(),
        },
        bounds,
    )?;

    let observed = (|| {
        let original_lease: String = admin_value(
            handle,
            admin,
            deadline,
            "SELECT lease_until::text FROM df_game.sessions WHERE tenant_id=$1::bytea AND session_id=$2::bytea",
            &[&tenant.as_slice(), &session.as_bytes().as_slice()],
        )?;
        let before = snapshot(handle, admin, &tenant, session, deadline)?;
        stage.set("current owner normal recovery and complete physical bytes");
        configure_probe(handle, admin, 0, deadline)?;
        if repository.load_current(&scope, &context)? != baseline {
            return Err(RepositoryError::InvalidCandidate);
        }
        verify_calls(handle, admin, 2, deadline)?;
        require_snapshot(handle, admin, &tenant, session, deadline, &before)?;

        stage
            .set("lease expires in second verifier after session lock; typed refusal and rollback");
        configure_probe(handle, admin, 2, deadline)?;
        if !matches!(
            repository.load_current(&scope, &context),
            Err(RepositoryError::ExpiredOwner)
        ) {
            return Err(RepositoryError::InvalidReceipt);
        }
        // Sequences do not roll back: two calls prove the injection was actually reached.
        verify_calls(handle, admin, 2, deadline)?;
        require_snapshot(handle, admin, &tenant, session, deadline, &before)?;
        configure_probe(handle, admin, 0, deadline)?;
        if repository.load_current(&scope, &context)? != baseline {
            return Err(RepositoryError::InvalidCandidate);
        }
        verify_calls(handle, admin, 2, deadline)?;
        require_snapshot(handle, admin, &tenant, session, deadline, &before)?;

        stage.set("initial expired owner remains refused without changing stored families");
        change_session(
            handle,
            admin,
            &tenant,
            session,
            deadline,
            "UPDATE df_game.sessions SET lease_until=clock_timestamp() WHERE tenant_id=$1::bytea AND session_id=$2::bytea",
            &[],
        )?;
        let expired = snapshot(handle, admin, &tenant, session, deadline)?;
        configure_probe(handle, admin, 0, deadline)?;
        if !matches!(
            repository.load_current(&scope, &context),
            Err(RepositoryError::ExpiredOwner)
        ) {
            return Err(RepositoryError::InvalidReceipt);
        }
        verify_calls(handle, admin, 2, deadline)?;
        require_snapshot(handle, admin, &tenant, session, deadline, &expired)?;
        change_session(
            handle,
            admin,
            &tenant,
            session,
            deadline,
            "UPDATE df_game.sessions SET lease_until=$3::text::timestamptz WHERE tenant_id=$1::bytea AND session_id=$2::bytea",
            &[&original_lease],
        )?;

        stage.set("wrong current owner fence remains a typed stale-fence refusal");
        change_session(
            handle,
            admin,
            &tenant,
            session,
            deadline,
            "UPDATE df_game.sessions SET owner_fence=$3::bytea WHERE tenant_id=$1::bytea AND session_id=$2::bytea",
            &[&[125_u8; 16].as_slice()],
        )?;
        let fenced = snapshot(handle, admin, &tenant, session, deadline)?;
        configure_probe(handle, admin, 0, deadline)?;
        if !matches!(
            repository.load_current(&scope, &context),
            Err(RepositoryError::StaleFence)
        ) {
            return Err(RepositoryError::InvalidReceipt);
        }
        verify_calls(handle, admin, 2, deadline)?;
        require_snapshot(handle, admin, &tenant, session, deadline, &fenced)?;
        change_session(
            handle,
            admin,
            &tenant,
            session,
            deadline,
            "UPDATE df_game.sessions SET owner_fence=$3::bytea WHERE tenant_id=$1::bytea AND session_id=$2::bytea",
            &[&fence.as_slice()],
        )?;
        require_snapshot(handle, admin, &tenant, session, deadline, &before)?;

        stage.set("real durable commit then retained lookup survives owner lease expiry");
        configure_probe(handle, admin, 0, deadline)?;
        let receipt =
            match repository.commit_decision(&scope, &candidate, baseline.basis(), &context)? {
                CommitOutcome::Confirmed(value) => value,
                _ => return Err(RepositoryError::InvalidReceipt),
            };
        if repository.load_current(&scope, &context)? != candidate {
            return Err(RepositoryError::InvalidCandidate);
        }
        change_session(
            handle,
            admin,
            &tenant,
            session,
            deadline,
            "UPDATE df_game.sessions SET lease_until=clock_timestamp() WHERE tenant_id=$1::bytea AND session_id=$2::bytea",
            &[],
        )?;
        let committed_expired = snapshot(handle, admin, &tenant, session, deadline)?;
        configure_probe(handle, admin, 0, deadline)?;
        if !matches!(repository.lookup_operation(&scope, &context)?,
            OperationLookup::Committed(value) if value == receipt)
        {
            return Err(RepositoryError::InvalidReceipt);
        }
        verify_calls(handle, admin, 2, deadline)?;
        require_snapshot(
            handle,
            admin,
            &tenant,
            session,
            deadline,
            &committed_expired,
        )?;
        configure_probe(handle, admin, 0, deadline)?;
        if !matches!(repository.commit_decision(&scope, &candidate, baseline.basis(), &context)?,
            CommitOutcome::PreviouslyCommitted(value) if value == receipt)
        {
            return Err(RepositoryError::InvalidReceipt);
        }
        verify_calls(handle, admin, 2, deadline)?;
        require_snapshot(
            handle,
            admin,
            &tenant,
            session,
            deadline,
            &committed_expired,
        )?;
        configure_probe(handle, admin, 0, deadline)?;
        if !matches!(
            repository.load_current(&scope, &context),
            Err(RepositoryError::ExpiredOwner)
        ) {
            return Err(RepositoryError::InvalidReceipt);
        }
        verify_calls(handle, admin, 2, deadline)?;
        require_snapshot(
            handle,
            admin,
            &tenant,
            session,
            deadline,
            &committed_expired,
        )?;

        stage.set("revoked current membership still refuses retained lookup and recovery");
        set_grant(handle, admin, &tenant, &principal, false, deadline)?;
        if !matches!(
            repository.lookup_operation(&scope, &context),
            Err(RepositoryError::Unauthorized)
        ) || !matches!(
            repository.load_current(&scope, &context),
            Err(RepositoryError::Unauthorized)
        ) {
            return Err(RepositoryError::InvalidReceipt);
        }
        require_snapshot(
            handle,
            admin,
            &tenant,
            session,
            deadline,
            &committed_expired,
        )?;
        set_grant(handle, admin, &tenant, &principal, true, deadline)?;
        change_session(
            handle,
            admin,
            &tenant,
            session,
            deadline,
            "UPDATE df_game.sessions SET lease_until=$3::text::timestamptz WHERE tenant_id=$1::bytea AND session_id=$2::bytea",
            &[&original_lease],
        )?;
        configure_probe(handle, admin, 0, deadline)?;
        if repository.load_current(&scope, &context)? != candidate {
            return Err(RepositoryError::InvalidCandidate);
        }
        verify_calls(handle, admin, 2, deadline)?;
        Ok(())
    })();
    let first_close = repository.close();
    let retained = retain_fixture_repository_after_retry(repository, first_close, None);
    let closed = match retained {
        Ok(value) => value,
        Err(pending) => pending.fail_fixture_while_retaining_owner(),
    };
    let joined = closed
        .first_close
        .and(closed.retry_close.unwrap_or(Ok(())))
        .and(closed.final_cleanup.unwrap_or(Ok(())));
    let absent = actor_block_on(handle, within_deadline(deadline, async {
        loop {
            let present: bool = admin.query_one(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1::integer AND backend_start::text=$2::text AND datname=current_database())",
                &[&backend_pid, &backend_start],
            ).await.map_err(|_| RepositoryError::Unavailable)?
                .try_get(0).map_err(|_| RepositoryError::Unavailable)?;
            if !present { return Ok::<_, RepositoryError>(()); }
            tokio::task::yield_now().await;
        }
    }))?.map_err(|_| RepositoryError::Unavailable)?;
    let removed = actor_block_on(
        handle,
        within_deadline(deadline, admin.batch_execute(REMOVE_PROBE)),
    )?
    .map_err(|_| RepositoryError::Unavailable)?
    .map_err(|_| RepositoryError::Unavailable);
    observed?;
    joined?;
    absent?;
    removed?;
    stage.set("recovery lease verified second-grant expiry refusal, retained lookup, full physical rollback and joined backend absence");
    eprintln!(
        "registered recovery lease PASS: normal; second-verifier expiry and same-connection retry; initial expiry; stale fence; committed retained lookup and duplicate; revoked membership; full stored families unchanged at every refusal; driver joined; exact backend absent; probe removed"
    );
    Ok(())
}

fn configure_probe(
    handle: &tokio::runtime::Handle,
    admin: &tokio_postgres::Client,
    expire_on_call: i32,
    deadline: Instant,
) -> Result<(), RepositoryError> {
    actor_block_on(handle, within_deadline(deadline, async {
        let changed = admin.execute(
            "UPDATE df_fixture_authority.recovery_lease_probe SET expire_on_call=$1::integer WHERE singleton",
            &[&expire_on_call],
        ).await.map_err(|_| RepositoryError::Unavailable)?;
        if changed != 1 { return Err(RepositoryError::InvalidReceipt); }
        admin.batch_execute("ALTER SEQUENCE df_fixture_authority.recovery_lease_calls RESTART WITH 1")
            .await.map_err(|_| RepositoryError::Unavailable)
    }))?.map_err(|_| RepositoryError::Unavailable)?
}

fn verify_calls(
    handle: &tokio::runtime::Handle,
    admin: &tokio_postgres::Client,
    expected: i64,
    deadline: Instant,
) -> Result<(), RepositoryError> {
    let calls: i64 = admin_value(
        handle,
        admin,
        deadline,
        "SELECT CASE WHEN is_called THEN last_value ELSE 0 END FROM df_fixture_authority.recovery_lease_calls",
        &[],
    )?;
    if calls != expected {
        return Err(RepositoryError::InvalidReceipt);
    }
    Ok(())
}

fn admin_value<T: tokio_postgres::types::FromSqlOwned>(
    handle: &tokio::runtime::Handle,
    admin: &tokio_postgres::Client,
    deadline: Instant,
    query: &str,
    parameters: &[&(dyn tokio_postgres::types::ToSql + Sync)],
) -> Result<T, RepositoryError> {
    actor_block_on(
        handle,
        within_deadline(deadline, admin.query_one(query, parameters)),
    )?
    .map_err(|_| RepositoryError::Unavailable)?
    .map_err(|_| RepositoryError::Unavailable)?
    .try_get(0)
    .map_err(|_| RepositoryError::Unavailable)
}

fn change_session(
    handle: &tokio::runtime::Handle,
    admin: &tokio_postgres::Client,
    tenant: &[u8; 16],
    session: SessionId,
    deadline: Instant,
    query: &str,
    extra: &[&(dyn tokio_postgres::types::ToSql + Sync)],
) -> Result<(), RepositoryError> {
    let tenant_bytes = tenant.as_slice();
    let session_bytes = session.as_bytes().as_slice();
    let mut parameters: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
        vec![&tenant_bytes, &session_bytes];
    parameters.extend_from_slice(extra);
    let changed = actor_block_on(
        handle,
        within_deadline(deadline, admin.execute(query, &parameters)),
    )?
    .map_err(|_| RepositoryError::Unavailable)?
    .map_err(|_| RepositoryError::Unavailable)?;
    if changed != 1 {
        return Err(RepositoryError::InvalidReceipt);
    }
    Ok(())
}

fn set_grant(
    handle: &tokio::runtime::Handle,
    admin: &tokio_postgres::Client,
    tenant: &[u8; 16],
    principal: &[u8; 16],
    active: bool,
    deadline: Instant,
) -> Result<(), RepositoryError> {
    let changed = actor_block_on(handle, within_deadline(deadline, admin.execute(
        "UPDATE df_fixture_authority.grants SET active=$3::boolean
         WHERE tenant_id=$1::bytea AND principal_id=$2::bytea AND service_role='df_persistence_fixture_runtime_a'",
        &[&tenant.as_slice(), &principal.as_slice(), &active],
    )))?.map_err(|_| RepositoryError::Unavailable)?
        .map_err(|_| RepositoryError::Unavailable)?;
    if changed != 1 {
        return Err(RepositoryError::InvalidReceipt);
    }
    Ok(())
}

fn snapshot(
    handle: &tokio::runtime::Handle,
    admin: &tokio_postgres::Client,
    tenant: &[u8; 16],
    session: SessionId,
    deadline: Instant,
) -> Result<Vec<String>, RepositoryError> {
    let mut rows = retained_fixture_all_family_bytes(handle, admin, tenant, session, deadline)?;
    let owner: String = admin_value(
        handle,
        admin,
        deadline,
        "SELECT to_jsonb(s)::text FROM df_game.sessions s WHERE tenant_id=$1::bytea AND session_id=$2::bytea",
        &[&tenant.as_slice(), &session.as_bytes().as_slice()],
    )?;
    rows.push(owner);
    Ok(rows)
}

fn require_snapshot(
    handle: &tokio::runtime::Handle,
    admin: &tokio_postgres::Client,
    tenant: &[u8; 16],
    session: SessionId,
    deadline: Instant,
    expected: &[String],
) -> Result<(), RepositoryError> {
    if snapshot(handle, admin, tenant, session, deadline)? != expected {
        return Err(RepositoryError::InvalidReceipt);
    }
    Ok(())
}
