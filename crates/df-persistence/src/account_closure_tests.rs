//! Account closure on PostgreSQL: the shared closure scenarios with a test campaign model standing
//! in for the campaign owner (a scratch schema `campaigns_test` the participant reads and writes
//! on the store's own connection, inside the close transaction), plus the checks only a real
//! database can make: the roles, the definer function, and the startup timeout policy.

use df_customer::{
    BlockReason, BlockingCampaign, ClosureCheck, LeaveChunk, OwnedArchivedCampaign,
    ScheduledDeletion, StoreError,
    contract::Fixture,
    fakes::{CampaignSeed, CampaignSeeding},
};
use df_types::{AccountId, CampaignId};
use tokio_postgres::Transaction;

use crate::{
    account_closure::{ClosureParticipant, ParticipantCall, TimeoutPolicy, TimeoutRefusal},
    account_store::{AccountStoreOpenError, PostgresAccountStore},
    postgres_tls::PostgresTransport,
};

pub(crate) const SCHEMA_SQL: &str = "CREATE SCHEMA campaigns_test; \
    CREATE TABLE campaigns_test.campaign (campaign_id bytea PRIMARY KEY, name text NOT NULL, \
        owner bytea NOT NULL, host bytea NOT NULL, archived boolean NOT NULL, \
        scheduled_delete_at_ms bigint); \
    CREATE TABLE campaigns_test.member (campaign_id bytea NOT NULL, account_id bytea NOT NULL, \
        PRIMARY KEY (campaign_id, account_id)); \
    CREATE TABLE campaigns_test.proposal (proposal_id bigserial PRIMARY KEY, \
        from_account bytea NOT NULL, is_open boolean NOT NULL)";

fn failed(_: tokio_postgres::Error) -> StoreError {
    StoreError::Unavailable
}

fn campaign_id(bytes: &[u8]) -> Result<CampaignId, StoreError> {
    CampaignId::from_bytes(bytes).map_err(|_| StoreError::InvalidRecord)
}

fn account(bytes: &[u8]) -> Result<AccountId, StoreError> {
    AccountId::from_bytes(bytes).map_err(|_| StoreError::InvalidRecord)
}

/// Campaigns left per chunk by the test participant (small, so chunking is exercised).
const CHUNK: usize = 2;

/// What the campaign owner's participant does, on test tables.
pub(crate) struct TestCampaigns;

async fn check(transaction: &Transaction<'_>, who: AccountId) -> Result<ClosureCheck, StoreError> {
    let rows = transaction
        .query(
            "SELECT campaign_id, name, owner, host, archived FROM campaigns_test.campaign \
             WHERE owner = $1 OR host = $1 ORDER BY campaign_id",
            &[&&who.as_bytes()[..]],
        )
        .await
        .map_err(failed)?;
    let mut check = ClosureCheck::default();
    for row in rows {
        let id: Vec<u8> = row.try_get(0).map_err(failed)?;
        let name: String = row.try_get(1).map_err(failed)?;
        let owner: Vec<u8> = row.try_get(2).map_err(failed)?;
        let host: Vec<u8> = row.try_get(3).map_err(failed)?;
        let archived: bool = row.try_get(4).map_err(failed)?;
        let owns = owner == who.as_bytes();
        let hosts = host == who.as_bytes();
        if archived {
            if owns {
                check.owned_archived.push(OwnedArchivedCampaign {
                    campaign: campaign_id(&id)?,
                    name,
                });
            }
            continue;
        }
        let reason = match (owns, hosts) {
            (true, true) => BlockReason::OwnsAndHosts,
            (true, false) => BlockReason::Owns,
            _ => BlockReason::Hosts,
        };
        check.blocking.push(BlockingCampaign {
            campaign: campaign_id(&id)?,
            name,
            reason,
        });
    }
    Ok(check)
}

impl ClosureParticipant for TestCampaigns {
    fn closure_campaigns<'a>(
        &'a self,
        transaction: &'a Transaction<'_>,
        who: AccountId,
    ) -> ParticipantCall<'a, ClosureCheck> {
        Box::pin(check(transaction, who))
    }

    fn schedule_archived_deletions<'a>(
        &'a self,
        transaction: &'a Transaction<'_>,
        who: AccountId,
        delete_after_ms: u64,
    ) -> ParticipantCall<'a, Vec<ScheduledDeletion>> {
        Box::pin(async move {
            let at = i64::try_from(delete_after_ms).map_err(|_| StoreError::InvalidRecord)?;
            let rows = transaction
                .query(
                    "UPDATE campaigns_test.campaign SET scheduled_delete_at_ms = $2 \
                     WHERE owner = $1 AND archived AND scheduled_delete_at_ms IS NULL \
                     RETURNING campaign_id, name",
                    &[&&who.as_bytes()[..], &at],
                )
                .await
                .map_err(failed)?;
            transaction
                .execute(
                    "UPDATE campaigns_test.proposal SET is_open = false WHERE from_account = $1",
                    &[&&who.as_bytes()[..]],
                )
                .await
                .map_err(failed)?;
            let mut scheduled = Vec::new();
            for row in rows {
                let id: Vec<u8> = row.try_get(0).map_err(failed)?;
                let members = transaction
                    .query(
                        "SELECT account_id FROM campaigns_test.member WHERE campaign_id = $1 \
                         ORDER BY account_id",
                        &[&id],
                    )
                    .await
                    .map_err(failed)?;
                scheduled.push(ScheduledDeletion {
                    campaign: campaign_id(&id)?,
                    name: row.try_get(1).map_err(failed)?,
                    members: members
                        .iter()
                        .map(|member| {
                            let bytes: Vec<u8> = member.try_get(0).map_err(failed)?;
                            account(&bytes)
                        })
                        .collect::<Result<_, _>>()?,
                });
            }
            scheduled.sort_by_key(|deletion| deletion.campaign);
            Ok(scheduled)
        })
    }

    fn cancel_scheduled_deletions<'a>(
        &'a self,
        transaction: &'a Transaction<'_>,
        who: AccountId,
    ) -> ParticipantCall<'a, ()> {
        Box::pin(async move {
            transaction
                .execute(
                    "UPDATE campaigns_test.campaign SET scheduled_delete_at_ms = NULL \
                     WHERE owner = $1",
                    &[&&who.as_bytes()[..]],
                )
                .await
                .map_err(failed)?;
            Ok(())
        })
    }

    fn leave_chunk<'a>(
        &'a self,
        transaction: &'a Transaction<'_>,
        who: AccountId,
        after: Option<CampaignId>,
    ) -> ParticipantCall<'a, LeaveChunk> {
        Box::pin(async move {
            let start = after.map(|id| id.as_bytes().to_vec()).unwrap_or_default();
            let rows = transaction
                .query(
                    "SELECT campaign_id FROM campaigns_test.member \
                     WHERE account_id = $1 AND campaign_id > $2 ORDER BY campaign_id LIMIT $3",
                    &[
                        &&who.as_bytes()[..],
                        &start,
                        &i64::try_from(CHUNK + 1).map_err(|_| StoreError::InvalidRecord)?,
                    ],
                )
                .await
                .map_err(failed)?;
            let mut left = Vec::new();
            for row in rows.iter().take(CHUNK) {
                let id: Vec<u8> = row.try_get(0).map_err(failed)?;
                transaction
                    .execute(
                        "DELETE FROM campaigns_test.member WHERE account_id = $1 \
                         AND campaign_id = $2",
                        &[&&who.as_bytes()[..], &id],
                    )
                    .await
                    .map_err(failed)?;
                left.push(campaign_id(&id)?);
            }
            match (rows.len() > CHUNK, left.last()) {
                (true, Some(last)) => Ok(LeaveChunk::Continue(*last)),
                _ => Ok(LeaveChunk::Done),
            }
        })
    }
}

impl CampaignSeeding for PostgresAccountStore {
    async fn seed_campaign(&self, seed: CampaignSeed) {
        let id = seed.campaign.as_bytes().to_vec();
        self.test_query(
            "INSERT INTO campaigns_test.campaign (campaign_id, name, owner, host, archived) \
             VALUES ($1, $2, $3, $4, $5)",
            &[
                &id,
                &seed.name,
                &seed.owner.as_bytes().to_vec(),
                &seed.host.as_bytes().to_vec(),
                &seed.archived,
            ],
        )
        .await;
        for member in seed.members {
            self.test_query(
                "INSERT INTO campaigns_test.member (campaign_id, account_id) VALUES ($1, $2)",
                &[&id, &member.as_bytes().to_vec()],
            )
            .await;
        }
    }

    async fn seed_outgoing_proposal(&self, from: AccountId) {
        self.test_query(
            "INSERT INTO campaigns_test.proposal (from_account, is_open) VALUES ($1, true)",
            &[&from.as_bytes().to_vec()],
        )
        .await;
    }

    async fn archive_campaign(&self, campaign: CampaignId) {
        self.test_query(
            "UPDATE campaigns_test.campaign SET archived = true WHERE campaign_id = $1",
            &[&campaign.as_bytes().to_vec()],
        )
        .await;
    }

    async fn scheduled_deletion(&self, campaign: CampaignId) -> Option<u64> {
        let rows = self
            .test_query(
                "SELECT scheduled_delete_at_ms FROM campaigns_test.campaign WHERE campaign_id = $1",
                &[&campaign.as_bytes().to_vec()],
            )
            .await;
        let at: Option<i64> = rows.first()?.get(0);
        at.and_then(|at| u64::try_from(at).ok())
    }

    async fn open_proposals_from(&self, from: AccountId) -> usize {
        let rows = self
            .test_query(
                "SELECT count(*) FROM campaigns_test.proposal WHERE from_account = $1 AND is_open",
                &[&from.as_bytes().to_vec()],
            )
            .await;
        let count: i64 = rows[0].get(0);
        usize::try_from(count).unwrap_or(usize::MAX)
    }

    async fn is_member(&self, campaign: CampaignId, account: AccountId) -> bool {
        !self
            .test_query(
                "SELECT 1 FROM campaigns_test.member WHERE campaign_id = $1 AND account_id = $2",
                &[&campaign.as_bytes().to_vec(), &account.as_bytes().to_vec()],
            )
            .await
            .is_empty()
    }
}

macro_rules! closure_scenarios {
    ($($scenario:ident),+ $(,)?) => {
        $(
            #[test]
            fn $scenario() {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .unwrap();
                runtime.block_on(async {
                    let Some(fixture) = super::tests::fresh_database().await else {
                        return;
                    };
                    df_customer::contract::$scenario(fixture.store()).await;
                    fixture.finish().await;
                });
            }
        )+
    };
}

closure_scenarios!(
    closing_is_blocked_while_the_account_owns_or_hosts_an_active_campaign,
    closing_schedules_archived_deletions_cancels_proposals_and_reopening_undoes_them,
    a_completed_closure_leaves_campaigns_and_new_blockers_keep_the_account_closing,
);

fn run<F: std::future::Future<Output = ()>>(test: F) {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
        .block_on(test);
}

#[test]
fn the_store_refuses_to_open_unless_its_role_has_a_short_transaction_timeout_default() {
    run(async {
        let Some(fixture) = super::tests::fresh_database().await else {
            return;
        };
        // The scratch administrator has no such default.
        let config = fixture.database_config();
        let refused = PostgresAccountStore::open(
            config.clone(),
            PostgresTransport::PlaintextLoopbackOnly,
            1,
            TimeoutPolicy::AtMostMs(10_000),
        )
        .await;
        assert!(matches!(
            refused,
            Err(AccountStoreOpenError::TransactionTimeout(
                TimeoutRefusal::NotSet
            ))
        ));
        // The service role the migration provisions has it, so the same open succeeds. The
        // deployment adds LOGIN; the scratch server does it here.
        fixture
            .admin_client()
            .await
            .batch_execute("ALTER ROLE df_accounts LOGIN")
            .await
            .unwrap();
        let mut as_service = config.clone();
        as_service.user("df_accounts");
        let opened = PostgresAccountStore::open(
            as_service.clone(),
            PostgresTransport::PlaintextLoopbackOnly,
            1,
            TimeoutPolicy::AtMostMs(10_000),
        )
        .await;
        assert!(opened.is_ok(), "{:?}", opened.err());
        let too_strict = PostgresAccountStore::open(
            as_service,
            PostgresTransport::PlaintextLoopbackOnly,
            1,
            TimeoutPolicy::AtMostMs(5_000),
        )
        .await;
        assert!(matches!(
            too_strict,
            Err(AccountStoreOpenError::TransactionTimeout(
                TimeoutRefusal::TooLong
            ))
        ));
        fixture.finish().await;
    });
}

#[test]
fn the_campaign_role_can_share_lock_and_read_status_but_cannot_write_account_rows() {
    run(async {
        let Some(fixture) = super::tests::fresh_database().await else {
            return;
        };
        let admin = fixture.admin_client().await;
        admin
            .batch_execute(
                "INSERT INTO accounts.account (account_id, email_normalized, email_display, \
                 status, credential_generation, created_at_ms) VALUES \
                 ('\\x00000000000000000000000000000001', 'a@b.test', 'a@b.test', 'closing', 1, 0)",
            )
            .await
            .unwrap();
        admin
            .batch_execute("ALTER ROLE df_campaigns LOGIN")
            .await
            .unwrap();
        let mut as_campaigns = fixture.database_config();
        as_campaigns.user("df_campaigns");
        let (campaigns, connection) = as_campaigns.connect(tokio_postgres::NoTls).await.unwrap();
        tokio::spawn(async move {
            let _ended = connection.await;
        });
        let id = vec![0_u8; 15].into_iter().chain([1]).collect::<Vec<u8>>();
        let locked = campaigns
            .query_one("SELECT accounts.lock_account_status($1)", &[&id])
            .await
            .unwrap();
        let status: Option<String> = locked.get(0);
        assert_eq!(status.as_deref(), Some("closing"));
        let viewed = campaigns
            .query_one("SELECT status FROM accounts.account_status", &[])
            .await
            .unwrap();
        assert_eq!(viewed.get::<_, String>(0), "closing");
        for forbidden in [
            "UPDATE accounts.account SET status = 'active'",
            "SELECT * FROM accounts.account",
            "SELECT * FROM accounts.web_session",
            "SELECT 1 FROM accounts.account FOR SHARE",
        ] {
            assert!(
                campaigns.batch_execute(forbidden).await.is_err(),
                "the campaign role ran: {forbidden}"
            );
        }
        drop(campaigns);
        fixture.finish().await;
    });
}

#[test]
fn a_share_locked_account_makes_the_close_wait_and_a_closed_one_reads_as_closing() {
    run(async {
        let Some(fixture) = super::tests::fresh_database().await else {
            return;
        };
        let harness = df_customer::contract::Harness::start(fixture.store()).await;
        let (caller, _) = harness.verified_account("robin@example.test").await;
        let admin = fixture.admin_client().await;
        admin
            .batch_execute("ALTER ROLE df_campaigns LOGIN")
            .await
            .unwrap();
        let mut as_campaigns = fixture.database_config();
        as_campaigns.user("df_campaigns");
        let (mut campaigns, connection) =
            as_campaigns.connect(tokio_postgres::NoTls).await.unwrap();
        tokio::spawn(async move {
            let _ended = connection.await;
        });
        let id = caller.account.id.as_bytes().to_vec();
        // A transfer acceptance holds the share lock first.
        let acceptance = campaigns.transaction().await.unwrap();
        let status: Option<String> = acceptance
            .query_one("SELECT accounts.lock_account_status($1)", &[&id])
            .await
            .unwrap()
            .get(0);
        assert_eq!(status.as_deref(), Some("active"));
        let service = harness.service.clone();
        let closing_caller = caller.clone();
        let close = tokio::spawn(async move {
            service
                .close(
                    &closing_caller,
                    df_customer::Reauthentication::Password(df_customer::contract::PASSWORD),
                )
                .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert!(
            !close.is_finished(),
            "the closure must wait for the share lock"
        );
        acceptance.commit().await.unwrap();
        let closed = close.await.unwrap().unwrap();
        assert!(matches!(closed, df_customer::CloseResult::Closed { .. }));
        let after = campaigns
            .query_one("SELECT accounts.lock_account_status($1)", &[&id])
            .await
            .unwrap();
        assert_eq!(
            after.get::<_, Option<String>>(0).as_deref(),
            Some("closing")
        );
        drop(campaigns);
        drop(harness);
        fixture.finish().await;
    });
}

#[test]
fn completion_leaves_a_long_membership_list_in_chunks() {
    run(async {
        let Some(fixture) = super::tests::fresh_database().await else {
            return;
        };
        let store = fixture.store();
        let harness = df_customer::contract::Harness::start(store.clone()).await;
        let (robin, _) = harness.verified_account("robin@example.test").await;
        let (marian, _) = harness.verified_account("marian@example.test").await;
        for number in 1..=5_u8 {
            store
                .seed_campaign(CampaignSeed {
                    campaign: CampaignId::from_bytes(&[number; 16]).unwrap(),
                    name: format!("Table {number}"),
                    owner: marian.account.id,
                    host: marian.account.id,
                    archived: false,
                    members: vec![robin.account.id, marian.account.id],
                })
                .await;
        }
        harness
            .service
            .close(
                &robin,
                df_customer::Reauthentication::Password(df_customer::contract::PASSWORD),
            )
            .await
            .unwrap();
        harness.clock.advance_ms(31 * 24 * 60 * 60 * 1000);
        harness.service.run_retention_pass().await;
        for number in 1..=5_u8 {
            let campaign = CampaignId::from_bytes(&[number; 16]).unwrap();
            assert!(!store.is_member(campaign, robin.account.id).await);
            assert!(store.is_member(campaign, marian.account.id).await);
        }
        let summary = harness.service.dev_summary().await.unwrap();
        assert_eq!((summary.closing, summary.closed), (0, 1));
        drop(harness);
        fixture.finish().await;
    });
}

/// Gives the provisioned service role a login on the scratch server (a deployment does this),
/// retrying while a parallel test is altering a cluster-wide role.
async fn enable_login(admin: &tokio_postgres::Client, role: &str) {
    for attempt in 0..10 {
        match admin
            .batch_execute(&format!("ALTER ROLE {role} LOGIN"))
            .await
        {
            Ok(()) => return,
            Err(_) if attempt < 9 => {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            Err(error) => panic!("enable login for {role}: {error}"),
        }
    }
}

#[test]
fn the_service_role_holds_exactly_the_table_privileges_the_store_needs() {
    run(async {
        let Some(fixture) = super::tests::fresh_database_with(false).await else {
            return;
        };
        let admin = fixture.admin_client().await;
        let expected: &[(&str, &str)] = &[
            ("account", "SELECT INSERT UPDATE"),
            ("password_credential", "SELECT INSERT UPDATE DELETE"),
            ("identity_link", "SELECT INSERT DELETE"),
            ("web_session", "SELECT INSERT UPDATE DELETE"),
            ("email_change", "SELECT INSERT UPDATE DELETE"),
            ("account_token", "SELECT INSERT UPDATE DELETE"),
            ("operation_receipt", "SELECT INSERT UPDATE DELETE"),
            ("account_event", "SELECT INSERT DELETE"),
            ("schema_migration", "SELECT"),
        ];
        let tables: Vec<String> = admin
            .query(
                "SELECT c.relname FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname = 'accounts' AND c.relkind = 'r' ORDER BY c.relname",
                &[],
            )
            .await
            .unwrap()
            .iter()
            .map(|row| row.get(0))
            .collect();
        let mut named: Vec<&str> = expected.iter().map(|(table, _)| *table).collect();
        named.sort_unstable();
        assert_eq!(
            tables.iter().map(String::as_str).collect::<Vec<_>>(),
            named,
            "every table has a stated privilege set; a new table must be added here and granted"
        );
        for (table, allowed) in expected {
            for privilege in ["SELECT", "INSERT", "UPDATE", "DELETE", "TRUNCATE"] {
                let has: bool = admin
                    .query_one(
                        "SELECT has_table_privilege('df_accounts', $1, $2)",
                        &[&format!("accounts.{table}"), &privilege],
                    )
                    .await
                    .unwrap()
                    .get(0);
                assert_eq!(
                    has,
                    allowed.split(' ').any(|granted| granted == privilege),
                    "{privilege} on {table}"
                );
            }
        }
        let defaults: i64 = admin
            .query_one(
                "SELECT count(*) FROM pg_default_acl d JOIN pg_namespace n \
                 ON n.oid = d.defaclnamespace WHERE n.nspname = 'accounts'",
                &[],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(defaults, 0, "no default-privilege grant");
        // The production schema knows one provider.
        admin
            .batch_execute(
                "INSERT INTO accounts.account (account_id, email_normalized, email_display, \
                 status, credential_generation, created_at_ms) VALUES \
                 ('\\x00000000000000000000000000000002', 'b@c.test', 'b@c.test', 'active', 1, 0)",
            )
            .await
            .unwrap();
        let link = |provider: &str| {
            format!(
                "INSERT INTO accounts.identity_link (provider, subject, account_id, linked_at_ms) \
                 VALUES ('{provider}', 's-{provider}', '\\x00000000000000000000000000000002', 0)"
            )
        };
        assert!(admin.batch_execute(&link("fake")).await.is_err());
        assert!(admin.batch_execute(&link("workos")).await.is_ok());
        fixture.finish().await;
    });
}

/// A fresh database whose store is opened as the provisioned service role, the way a deployment
/// runs it: table privileges only, the startup timeout check on.
async fn service_role_fixture() -> Option<(super::tests::PostgresFixture, PostgresAccountStore)> {
    let fixture = super::tests::fresh_database().await?;
    let admin = fixture.admin_client().await;
    enable_login(&admin, "df_accounts").await;
    // The test campaign model lives in a scratch schema: the role may use it.
    admin
        .batch_execute(
            "GRANT USAGE ON SCHEMA campaigns_test TO df_accounts;              GRANT ALL ON ALL TABLES IN SCHEMA campaigns_test TO df_accounts;              GRANT ALL ON ALL SEQUENCES IN SCHEMA campaigns_test TO df_accounts",
        )
        .await
        .unwrap();
    let mut as_service = fixture.database_config();
    as_service.user("df_accounts");
    let store = PostgresAccountStore::open(
        as_service,
        PostgresTransport::PlaintextLoopbackOnly,
        2,
        TimeoutPolicy::AtMostMs(10_000),
    )
    .await
    .expect("the service role opens the store")
    .with_closure_participant(std::sync::Arc::new(TestCampaigns));
    Some((fixture, store))
}

macro_rules! under_the_service_role {
    ($($scenario:ident),+ $(,)?) => {
        $(
            #[test]
            fn $scenario() {
                run(async {
                    let Some((fixture, store)) = service_role_fixture().await else {
                        return;
                    };
                    df_customer::contract::$scenario(store).await;
                    fixture.finish().await;
                });
            }
        )+
    };
}

mod service_role {
    use super::*;

    under_the_service_role!(
        signup_verify_login_and_logout_work_end_to_end,
        closing_an_account_signs_out_and_logging_in_within_the_window_reopens_it,
        retention_deletes_a_closed_accounts_personal_data_after_the_window,
        the_provider_checks_the_password_for_sensitive_actions_and_closing_deletes_its_user,
        closing_schedules_archived_deletions_cancels_proposals_and_reopening_undoes_them,
        verifying_replaces_the_sign_up_password_and_ends_earlier_sessions,
        a_provider_reset_works_once_and_signs_out_everywhere,
    );
}

#[test]
fn an_unchecked_timeout_policy_is_refused_for_a_database_that_is_not_local() {
    run(async {
        let mut config = tokio_postgres::Config::new();
        config
            .host("db.example.test")
            .port(25060)
            .user("doadmin")
            .dbname("df");
        let refused = PostgresAccountStore::open(
            config,
            PostgresTransport::VerifiedTls {
                ca_file: std::path::PathBuf::from("does-not-matter.pem"),
            },
            1,
            TimeoutPolicy::NotChecked,
        )
        .await;
        assert!(matches!(
            refused,
            Err(AccountStoreOpenError::UncheckedTimeoutOnRemoteHost)
        ));
    });
}
