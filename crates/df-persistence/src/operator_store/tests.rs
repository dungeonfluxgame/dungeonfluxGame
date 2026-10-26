use super::*;
use crate::{account_closure::TimeoutPolicy, account_store::PostgresAccountStore};
use df_customer::{
    AccountStore, Clock, CreateOutcome, EmailAddress, FakeClock, IdentityProviderKind,
    NewIdentityAccount, OperatorService,
};
use std::{
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicU64, Ordering},
};

const NOW_MS: u64 = 1_790_000_000_000;

struct Fixture {
    admin: Connection,
    database: String,
    config: Config,
    owner: Connection,
    accounts: PostgresAccountStore,
    store: Option<PostgresOperatorStore>,
    clock: FakeClock,
}

async fn connection(config: &Config) -> Connection {
    let (client, connection) = tokio::time::timeout(
        Duration::from_secs(5),
        config.connect(tokio_postgres::NoTls),
    )
    .await
    .expect("scratch connect deadline")
    .expect("scratch PostgreSQL 18 at 127.0.0.1:55911");
    Connection {
        client,
        driver: tokio::spawn(async move {
            connection.await.expect("scratch driver");
        }),
    }
}

impl Fixture {
    async fn open() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let database = format!(
            "af01_operator_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let mut config = Config::new();
        config
            .host("127.0.0.1")
            .port(55911)
            .user("postgres")
            .dbname("postgres")
            .options("-c statement_timeout=5s -c lock_timeout=5s -c transaction_timeout=10s");
        let admin = connection(&config).await;
        admin
            .client
            .batch_execute(&format!("CREATE DATABASE {database}"))
            .await
            .unwrap();
        config.dbname(&database);
        let accounts = PostgresAccountStore::open(
            config.clone(),
            PostgresTransport::PlaintextLoopbackOnly,
            1,
            TimeoutPolicy::NotChecked,
        )
        .await
        .unwrap();
        let mut owner = connection(&config).await;
        migrate_operator_access(&mut owner.client).await.unwrap();
        migrate_operator_access(&mut owner.client).await.unwrap();
        let mut runtime_config = config.clone();
        runtime_config.options("-c role=df_accounts -c transaction_timeout=10s");
        let store =
            PostgresOperatorStore::open(runtime_config, PostgresTransport::PlaintextLoopbackOnly)
                .await
                .unwrap();
        Self {
            admin,
            database,
            config,
            owner,
            accounts,
            store: Some(store),
            clock: FakeClock::at(NOW_MS),
        }
    }

    fn store(&self) -> &PostgresOperatorStore {
        self.store.as_ref().unwrap()
    }

    fn service(&self) -> OperatorService<PostgresAccountStore, PostgresOperatorStore> {
        OperatorService::new(
            self.accounts.clone(),
            self.store().clone(),
            Arc::new(self.clock.clone()),
        )
    }

    async fn account(&self, id: u8, verified: bool) -> AccountId {
        let account = account_id(id);
        assert_eq!(
            self.accounts
                .create_identity_account(NewIdentityAccount {
                    id: account,
                    email: EmailAddress::parse(&format!("af01-{id}@example.test")).unwrap(),
                    is_verified: verified,
                    provider: IdentityProviderKind::WorkOs,
                    subject: format!("af01-{id}"),
                    created_at_ms: NOW_MS,
                    adult_attested_at: Some(NOW_MS),
                    terms_version: Some("test".into()),
                    terms_accepted_at: Some(NOW_MS),
                    verification: None,
                })
                .await
                .unwrap(),
            CreateOutcome::Created
        );
        account
    }

    // AF01 has no code verifier: only this local fixture supplies confirmed MFA authority.
    async fn mfa(&self, account: AccountId, state: &str) {
        self.owner.client.execute("INSERT INTO accounts.operator_mfa \
            (account_id,generation,credential_generation,state,seal_key_id,sealed_secret,created_at_ms,confirmed_at_ms,last_used_step) \
            VALUES ($1,1,1,$2,'fixture',decode('01','hex'),$3,$4,$5) \
            ON CONFLICT (account_id) DO UPDATE SET state=$2,confirmed_at_ms=$4,last_used_step=$5",
            &[&&account.as_bytes()[..], &state, &signed(NOW_MS).unwrap(),
                &if state == "active" { Some(signed(NOW_MS).unwrap()) } else { None },
                &if state == "active" { Some(1_i64) } else { None }]).await.unwrap();
    }

    async fn count(&self, table: &str) -> i64 {
        self.owner
            .client
            .query_one(&format!("SELECT count(*) FROM accounts.{table}"), &[])
            .await
            .unwrap()
            .get(0)
    }

    async fn finish(self) {
        let Self {
            admin,
            database,
            owner,
            accounts,
            store,
            ..
        } = self;
        drop(store);
        drop(accounts);
        drop(owner);
        admin
            .client
            .batch_execute(&format!("DROP DATABASE {database} WITH (FORCE)"))
            .await
            .unwrap();
    }
}

fn run(test: impl for<'a> FnOnce(&'a mut Fixture) -> Pin<Box<dyn Future<Output = ()> + 'a>>) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut fixture = runtime.block_on(Fixture::open());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        runtime.block_on(test(&mut fixture))
    }));
    runtime.block_on(fixture.finish());
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

fn account_id(value: u8) -> AccountId {
    AccountId::from_bytes(&[value; 16]).unwrap()
}
fn operation(value: u8) -> OperationId {
    OperationId::from_bytes(&[value; 16]).unwrap()
}
fn grant(account: AccountId, id: u8) -> OperatorCommand {
    OperatorCommand::Grant(GrantOperator {
        account,
        capabilities: OperatorCapabilities::new(vec![OperatorCapability::SupportInspect]).unwrap(),
        scope: OperatorScope::Account(account),
        expires_at_ms: NOW_MS + 60_000,
        reason: OperatorReason::parse("synthetic support case AF01").unwrap(),
        operation: operation(id),
        correlation: operation(250),
    })
}
fn revoke(account: AccountId, receipt: OperatorReceipt, id: u8) -> OperatorCommand {
    OperatorCommand::Revoke(RevokeOperator {
        account,
        grant: receipt.grant,
        expected_version: receipt.version,
        reason: OperatorReason::parse("synthetic revocation").unwrap(),
        operation: operation(id),
        correlation: operation(250),
    })
}

#[test]
fn operator_grant_requires_verified_account_and_confirmed_mfa() {
    run(|fixture| {
        Box::pin(async move {
            let service = fixture.service();
            assert_eq!(
                service.execute_host(grant(account_id(1), 10)).await,
                Err(OperatorError::AccountNotVerified)
            );
            let unverified = fixture.account(1, false).await;
            fixture.mfa(unverified, "active").await;
            assert_eq!(
                service.execute_host(grant(unverified, 10)).await,
                Err(OperatorError::AccountNotVerified)
            );
            assert_eq!(
                fixture.store().execute(grant(unverified, 10), NOW_MS).await,
                Err(OperatorError::AccountNotVerified)
            );
            let account = fixture.account(2, true).await;
            assert_eq!(
                service.execute_host(grant(account, 11)).await,
                Err(OperatorError::ConfirmedMfaRequired)
            );
            for state in ["pending", "revoked"] {
                fixture.mfa(account, state).await;
                assert_eq!(
                    service.execute_host(grant(account, 11)).await,
                    Err(OperatorError::ConfirmedMfaRequired)
                );
            }
            fixture.mfa(account, "active").await;
            let receipt = service.execute_host(grant(account, 11)).await.unwrap();
            let current = fixture
                .store()
                .current_grant(receipt.grant, NOW_MS)
                .await
                .unwrap()
                .unwrap();
            assert!(
                current
                    .capabilities
                    .contains(OperatorCapability::SupportInspect)
            );
            assert!(!current.capabilities.contains(OperatorCapability::Refund));
            fixture.mfa(account, "revoked").await;
            assert!(
                fixture
                    .store()
                    .current_grant(receipt.grant, NOW_MS)
                    .await
                    .unwrap()
                    .is_none()
            );
            fixture.mfa(account, "active").await;
            fixture
                .owner
                .client
                .execute(
                    "UPDATE accounts.operator_mfa SET confirmed_at_ms=$2 WHERE account_id=$1",
                    &[&&account.as_bytes()[..], &signed(NOW_MS + 1).unwrap()],
                )
                .await
                .unwrap();
            assert!(
                fixture
                    .store()
                    .current_grant(receipt.grant, NOW_MS)
                    .await
                    .unwrap()
                    .is_none()
            );
            assert_eq!(
                service.execute_host(grant(account, 12)).await,
                Err(OperatorError::ConfirmedMfaRequired)
            );
            fixture.mfa(account, "active").await;
            fixture
                .owner
                .client
                .execute(
                    "UPDATE accounts.account SET credential_generation=2 WHERE account_id=$1",
                    &[&&account.as_bytes()[..]],
                )
                .await
                .unwrap();
            assert!(
                fixture
                    .store()
                    .current_grant(receipt.grant, NOW_MS)
                    .await
                    .unwrap()
                    .is_none()
            );
            assert_eq!(
                service.execute_host(grant(account, 12)).await,
                Err(OperatorError::ConfirmedMfaRequired)
            );
            fixture
                .owner
                .client
                .execute(
                    "UPDATE accounts.account SET status='closing' WHERE account_id=$1",
                    &[&&account.as_bytes()[..]],
                )
                .await
                .unwrap();
            assert_eq!(
                fixture.store().execute(grant(account, 12), NOW_MS).await,
                Err(OperatorError::AccountNotVerified)
            );
            assert_eq!(fixture.count("operator_grant").await, 1);
            assert_eq!(fixture.count("operator_audit").await, 1);
        })
    });
}

#[test]
fn operator_grant_expiry_and_revocation_are_current() {
    run(|fixture| {
        Box::pin(async move {
            let account = fixture.account(1, true).await;
            fixture.mfa(account, "active").await;
            let service = fixture.service();
            assert_eq!(
                OperatorReason::parse(" \n "),
                Err(OperatorError::InvalidReason)
            );
            assert_eq!(
                OperatorReason::parse(&"a".repeat(501)),
                Err(OperatorError::InvalidReason)
            );
            for expiry in [NOW_MS, NOW_MS - 1, NOW_MS + MAXIMUM_OPERATOR_GRANT_MS + 1] {
                let OperatorCommand::Grant(mut value) = grant(account, 10) else {
                    unreachable!()
                };
                value.expires_at_ms = expiry;
                assert_eq!(
                    service.execute_host(OperatorCommand::Grant(value)).await,
                    Err(OperatorError::InvalidExpiry)
                );
            }
            let OperatorCommand::Grant(mut maximum) = grant(account, 9) else {
                unreachable!()
            };
            maximum.expires_at_ms = NOW_MS + MAXIMUM_OPERATOR_GRANT_MS;
            assert!(
                service
                    .execute_host(OperatorCommand::Grant(maximum))
                    .await
                    .is_ok()
            );
            let receipt = service.execute_host(grant(account, 10)).await.unwrap();
            let scope = fixture
                .store()
                .current_grant(receipt.grant, NOW_MS)
                .await
                .unwrap()
                .unwrap()
                .scope;
            assert!(scope.allows(OperatorScope::Account(account)));
            assert!(!scope.allows(OperatorScope::Account(account_id(2))));
            assert!(!scope.allows(OperatorScope::Global));
            assert!(
                fixture
                    .store()
                    .current_grant(receipt.grant, NOW_MS - 1)
                    .await
                    .unwrap()
                    .is_none()
            );
            fixture.clock.advance_ms(59_999);
            assert!(
                fixture
                    .store()
                    .current_grant(receipt.grant, fixture.clock.now_ms())
                    .await
                    .unwrap()
                    .is_some()
            );
            fixture.clock.advance_ms(1);
            assert!(
                fixture
                    .store()
                    .current_grant(receipt.grant, fixture.clock.now_ms())
                    .await
                    .unwrap()
                    .is_none()
            );
            let OperatorCommand::Revoke(mut stale) = revoke(account, receipt, 11) else {
                unreachable!()
            };
            stale.expected_version = 2;
            assert_eq!(
                service.execute_host(OperatorCommand::Revoke(stale)).await,
                Err(OperatorError::StaleVersion)
            );
            assert_eq!(
                service
                    .execute_host(revoke(account_id(2), receipt, 11))
                    .await,
                Err(OperatorError::NotFound)
            );
            let revoked = service
                .execute_host(revoke(account, receipt, 11))
                .await
                .unwrap();
            assert_eq!(revoked.version, 2);
            assert!(
                fixture
                    .store()
                    .current_grant(receipt.grant, NOW_MS)
                    .await
                    .unwrap()
                    .is_none()
            );
            drop(fixture.store.take());
            let mut config = fixture.config.clone();
            config.options("-c role=df_accounts");
            fixture.store = Some(
                PostgresOperatorStore::open(config, PostgresTransport::PlaintextLoopbackOnly)
                    .await
                    .unwrap(),
            );
            assert!(
                fixture
                    .store()
                    .current_grant(receipt.grant, NOW_MS)
                    .await
                    .unwrap()
                    .is_none()
            );
            assert_eq!(
                fixture
                    .store()
                    .grants(account, None, OperatorPageSize::new(1).unwrap())
                    .await
                    .unwrap()
                    .len(),
                1
            );
        })
    });
}

#[test]
fn operator_grant_retry_is_payload_bound() {
    run(|fixture| {
        Box::pin(async move {
            let account = fixture.account(1, true).await;
            fixture.mfa(account, "active").await;
            let service = fixture.service();
            let command = grant(account, 10);
            let receipt = service.execute_host(command.clone()).await.unwrap();
            assert_eq!(
                service.execute_host(command.clone()).await.unwrap(),
                receipt
            );
            let OperatorCommand::Grant(original) = command.clone() else {
                unreachable!()
            };
            let mut conflicts = Vec::new();
            let mut value = original.clone();
            value.account = account_id(2);
            conflicts.push(value);
            let mut value = original.clone();
            value.scope = OperatorScope::Global;
            conflicts.push(value);
            let mut value = original.clone();
            value.expires_at_ms += 1;
            conflicts.push(value);
            let mut value = original.clone();
            value.reason = OperatorReason::parse("different case").unwrap();
            conflicts.push(value);
            let mut value = original;
            value.capabilities =
                OperatorCapabilities::new(vec![OperatorCapability::Refund]).unwrap();
            conflicts.push(value);
            for value in conflicts {
                assert_eq!(
                    service.execute_host(OperatorCommand::Grant(value)).await,
                    Err(OperatorError::PayloadConflict)
                );
            }
            let revoke = revoke(account, receipt, 11);
            let revoked = service.execute_host(revoke.clone()).await.unwrap();
            assert_eq!(service.execute_host(revoke.clone()).await.unwrap(), revoked);
            let OperatorCommand::Revoke(mut changed) = revoke else {
                unreachable!()
            };
            changed.expected_version += 1;
            assert_eq!(
                service.execute_host(OperatorCommand::Revoke(changed)).await,
                Err(OperatorError::PayloadConflict)
            );
            fixture.clock.advance_ms(MAXIMUM_OPERATOR_GRANT_MS);
            assert_eq!(service.execute_host(command).await.unwrap(), receipt);
            let config = fixture.config.clone();
            let second =
                PostgresOperatorStore::open(config, PostgresTransport::PlaintextLoopbackOnly)
                    .await
                    .unwrap();
            let first = fixture.store().clone();
            let command = grant(account, 20);
            let other = command.clone();
            let a = tokio::spawn(async move { first.execute(command, NOW_MS).await });
            let b = tokio::spawn(async move { second.execute(other, NOW_MS).await });
            assert_eq!(a.await.unwrap().unwrap(), b.await.unwrap().unwrap());
            assert_eq!(fixture.count("operator_grant").await, 2);
            assert_eq!(fixture.count("operator_audit").await, 3);
        })
    });
}

#[test]
fn operator_grant_and_audit_commit_together() {
    run(|fixture| {
        Box::pin(async move {
            let account = fixture.account(1, true).await;
            fixture.mfa(account, "active").await;
            let service = fixture.service();
            fixture.owner.client.batch_execute("CREATE FUNCTION accounts.af01_fail_audit() RETURNS trigger LANGUAGE plpgsql AS $$ \
            BEGIN RAISE EXCEPTION 'synthetic audit failure'; END $$; CREATE TRIGGER af01_fail_audit \
            BEFORE INSERT ON accounts.operator_audit FOR EACH STATEMENT EXECUTE FUNCTION accounts.af01_fail_audit()")
            .await.unwrap();
            let command = grant(account, 10);
            assert_eq!(
                service.execute_host(command.clone()).await,
                Err(OperatorError::Unavailable)
            );
            for table in ["operator_grant", "operator_audit", "operator_receipt"] {
                assert_eq!(fixture.count(table).await, 0);
            }
            fixture
                .owner
                .client
                .batch_execute("DROP TRIGGER af01_fail_audit ON accounts.operator_audit")
                .await
                .unwrap();
            let receipt = service.execute_host(command).await.unwrap();
            fixture
                .owner
                .client
                .batch_execute(
                    "CREATE TRIGGER af01_fail_audit BEFORE INSERT ON accounts.operator_audit \
            FOR EACH STATEMENT EXECUTE FUNCTION accounts.af01_fail_audit()",
                )
                .await
                .unwrap();
            let command = revoke(account, receipt, 11);
            assert_eq!(
                service.execute_host(command.clone()).await,
                Err(OperatorError::Unavailable)
            );
            assert_eq!(
                fixture
                    .store()
                    .current_grant(receipt.grant, NOW_MS)
                    .await
                    .unwrap()
                    .unwrap()
                    .version,
                1
            );
            assert_eq!(fixture.count("operator_receipt").await, 1);
            fixture
                .owner
                .client
                .batch_execute("DROP TRIGGER af01_fail_audit ON accounts.operator_audit")
                .await
                .unwrap();
            service.execute_host(command).await.unwrap();
            assert_eq!(fixture.count("operator_audit").await, 2);
            assert_eq!(fixture.count("operator_receipt").await, 2);
        })
    });
}

#[test]
fn operator_audit_is_append_only_and_paged() {
    run(|fixture| {
        Box::pin(async move {
            let account = fixture.account(1, true).await;
            let other = fixture.account(2, true).await;
            fixture.mfa(account, "active").await;
            fixture.mfa(other, "active").await;
            let service = fixture.service();
            for id in [10, 11, 12] {
                service.execute_host(grant(account, id)).await.unwrap();
            }
            service.execute_host(grant(other, 13)).await.unwrap();
            assert_eq!(OperatorPageSize::new(0), Err(OperatorError::InvalidPage));
            assert_eq!(OperatorPageSize::new(101), Err(OperatorError::InvalidPage));
            let mut ids = Vec::new();
            let mut after = None;
            loop {
                let page = fixture
                    .store()
                    .audit(account, after, OperatorPageSize::new(2).unwrap())
                    .await
                    .unwrap();
                if page.is_empty() {
                    break;
                }
                assert!(page.len() <= 2);
                for record in page {
                    assert_eq!(record.event.at_ms, NOW_MS);
                    after = Some(record.id);
                    ids.push(record.id);
                }
            }
            assert_eq!(ids.len(), 3);
            assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
            let first = fixture
                .store()
                .grants(account, None, OperatorPageSize::new(2).unwrap())
                .await
                .unwrap();
            let second = fixture
                .store()
                .grants(
                    account,
                    Some(first[1].id),
                    OperatorPageSize::new(2).unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(first.len(), 2);
            assert_eq!(second.len(), 1);
            let mut config = fixture.config.clone();
            config.options("-c role=df_accounts");
            let runtime = connection(&config).await;
            for table in ["operator_audit", "operator_receipt"] {
                for sql in [
                    format!("UPDATE accounts.{table} SET operation_id = operation_id"),
                    format!("DELETE FROM accounts.{table}"),
                    format!("TRUNCATE accounts.{table}"),
                ] {
                    let error = runtime.client.batch_execute(&sql).await.unwrap_err();
                    assert_eq!(error.code().unwrap().code(), "42501");
                    assert!(
                        fixture.owner.client.batch_execute(&sql).await.is_err(),
                        "owner trigger: {sql}"
                    );
                }
            }
            // Later owner commands use this same transaction-aware primitive, not post-write audit.
            let mut owner = connection(&fixture.config).await;
            let transaction = owner.client.transaction().await.unwrap();
            let event = NewOperatorAudit {
                basis: OperatorAuditBasis::Session(OperatorSessionBasis {
                    account,
                    session: WebSessionId::from_bytes(&[5; 16]).unwrap(),
                    credential_generation: 1,
                    enrolment_generation: 1,
                    proof: operation(40),
                    verified_at_ms: NOW_MS,
                    grant: first[0].id,
                    grant_version: 1,
                }),
                action: OperatorAuditAction::InspectAccount,
                target: OperatorAuditTarget::Account(account),
                reason: OperatorReason::parse("synthetic protected read").unwrap(),
                operation: operation(41),
                fingerprint: OperatorFingerprint([1; 32]),
                outcome: OperatorAuditOutcome::Applied,
                correlation: operation(42),
                at_ms: NOW_MS,
            };
            let id = append_operator_audit(&transaction, account, &event)
                .await
                .unwrap();
            transaction.commit().await.unwrap();
            let page = fixture
                .store()
                .audit(account, after, OperatorPageSize::new(1).unwrap())
                .await
                .unwrap();
            assert_eq!(page[0], OperatorAuditRecord { id, event });
        })
    });
}

#[test]
fn operator_storage_failure_and_commit_uncertainty_are_typed() {
    run(|fixture| {
        Box::pin(async move {
            let account = fixture.account(1, true).await;
            fixture.mfa(account, "active").await;
            let service = fixture.service();
            fixture
                .owner
                .client
                .batch_execute("REVOKE SELECT ON accounts.operator_audit FROM df_accounts")
                .await
                .unwrap();
            assert_eq!(
                fixture
                    .store()
                    .audit(account, None, OperatorPageSize::new(1).unwrap())
                    .await,
                Err(OperatorError::Unavailable)
            );
            fixture.owner.client.batch_execute("GRANT SELECT ON accounts.operator_audit TO df_accounts; \
            CREATE FUNCTION accounts.af01_fail_commit() RETURNS trigger LANGUAGE plpgsql AS $$ \
            BEGIN RAISE EXCEPTION 'synthetic commit rejection'; END $$; \
            CREATE CONSTRAINT TRIGGER af01_fail_commit AFTER INSERT ON accounts.operator_receipt \
            DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION accounts.af01_fail_commit()")
            .await.unwrap();
            let command = grant(account, 10);
            assert_eq!(
                service.execute_host(command.clone()).await,
                Err(OperatorError::Unknown)
            );
            for table in ["operator_grant", "operator_audit", "operator_receipt"] {
                assert_eq!(fixture.count(table).await, 0);
            }
            fixture
                .owner
                .client
                .batch_execute("DROP TRIGGER af01_fail_commit ON accounts.operator_receipt")
                .await
                .unwrap();
            let receipt = service.execute_host(command.clone()).await.unwrap();
            assert_eq!(service.execute_host(command).await.unwrap(), receipt);
        })
    });
}

#[test]
fn operator_grant_rechecks_account_after_concurrent_closure() {
    run(|fixture| {
        Box::pin(async move {
            let account = fixture.account(1, true).await;
            fixture.mfa(account, "active").await;
            let service = fixture.service();
            let slot = fixture.store().0.checkout().await.unwrap();
            let backend: i32 = slot
                .as_ref()
                .unwrap()
                .client
                .query_one("SELECT pg_backend_pid()", &[])
                .await
                .unwrap()
                .get(0);
            drop(slot);
            let transaction = fixture.owner.client.transaction().await.unwrap();
            transaction
                .execute(
                    "UPDATE accounts.account SET status='closing' WHERE account_id=$1",
                    &[&&account.as_bytes()[..]],
                )
                .await
                .unwrap();
            let pending =
                tokio::spawn(async move { service.execute_host(grant(account, 10)).await });
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let blocked: bool = fixture
                        .admin
                        .client
                        .query_one("SELECT cardinality(pg_blocking_pids($1)) > 0", &[&backend])
                        .await
                        .unwrap()
                        .get(0);
                    if blocked {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("grant reached the current-account row lock");
            transaction.commit().await.unwrap();
            assert_eq!(
                pending.await.unwrap(),
                Err(OperatorError::AccountNotVerified)
            );
            assert_eq!(fixture.count("operator_grant").await, 0);
            assert_eq!(fixture.count("operator_audit").await, 0);
        })
    });
}
