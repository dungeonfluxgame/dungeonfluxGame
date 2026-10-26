#![cfg(not(target_arch = "wasm32"))]

mod support;

use df_commerce::finance_ids::{CampaignKey, OperationKey, PayerId, PriceVersionId, SessionKey};
use df_finance::{
    fakes::{FakeClock, FakeFence, FakeMedia, FakeMembers, SeededIds},
    host_funding::{self as funding, ConsentCommand, Error},
    service::{FinanceConfig, FinanceService, PurchaseCommand},
};
use tokio_postgres::{Client, NoTls};

const NOW: i64 = 1_793_000_000_000;
const HOUR: u64 = 3_000_000;
const SESSION: SessionKey = SessionKey::from_array([0x31; 16]);
const CAMPAIGN: CampaignKey = CampaignKey::from_array([0x32; 16]);
const HOST: PayerId = PayerId::from_array([0x33; 16]);

struct Fixture {
    admin: Client,
    client: Client,
    database: String,
    drivers: Vec<tokio::task::JoinHandle<()>>,
}

async fn connect(database: &str) -> (Client, tokio::task::JoinHandle<()>) {
    let (client, connection) = tokio_postgres::connect(
        &format!("host=127.0.0.1 port=55911 user=postgres dbname={database}"),
        NoTls,
    )
    .await
    .unwrap();
    let driver = tokio::spawn(async move {
        connection.await.unwrap();
    });
    (client, driver)
}

impl Fixture {
    async fn new(credit: u64) -> Self {
        let (admin, admin_driver) = connect("postgres").await;
        let database = format!(
            "fundingwire_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        admin
            .batch_execute(&format!("CREATE DATABASE {database}"))
            .await
            .unwrap();
        let (client, driver) = connect(&database).await;
        admin
            .execute("SELECT pg_advisory_lock(7000021)", &[])
            .await
            .unwrap();
        for (_, migration) in support::MIGRATIONS {
            client.batch_execute(migration).await.unwrap();
        }
        admin
            .execute("SELECT pg_advisory_unlock(7000021)", &[])
            .await
            .unwrap();
        // External owners are local fixtures; the finance schema, posting boundary and roles are real.
        client.batch_execute("CREATE SCHEMA accounts; CREATE TABLE accounts.account (account_id bytea PRIMARY KEY, status text); \
            CREATE SCHEMA campaigns; CREATE TABLE campaigns.campaign (campaign_id bytea PRIMARY KEY, tenant_id bytea, status text, \
                spend_preset text, custom_hourly_micro_dollars bigint); \
            CREATE TABLE campaigns.campaign_member (campaign_id bytea, account_id bytea, role text, status text); \
            CREATE SCHEMA df_runtime; CREATE TABLE df_runtime.table_sessions (session_id bytea PRIMARY KEY, \
                campaign_id bytea, tenant_id bytea, phase text, ended_at timestamptz);").await.unwrap();
        client.batch_execute("ALTER TABLE df_runtime.table_sessions ENABLE ROW LEVEL SECURITY; ALTER TABLE df_runtime.table_sessions FORCE ROW LEVEL SECURITY; CREATE POLICY finance_session_read ON df_runtime.table_sessions FOR SELECT TO df_finance USING (true)").await.unwrap();
        client
            .batch_execute(include_str!(
                "../../df-persistence/migrations/0031_host_funding.sql"
            ))
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO accounts.account VALUES ($1,'active')",
                &[&HOST.as_bytes().as_slice()],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO campaigns.campaign VALUES ($1,$1,'active','story',NULL)",
                &[&CAMPAIGN.as_bytes().as_slice()],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO campaigns.campaign_member VALUES ($1,$2,'host','active')",
                &[&CAMPAIGN.as_bytes().as_slice(), &HOST.as_bytes().as_slice()],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO df_runtime.table_sessions (session_id,campaign_id,tenant_id,phase) VALUES ($1,$2,$2,'lobby')",
                &[
                    &SESSION.as_bytes().as_slice(),
                    &CAMPAIGN.as_bytes().as_slice(),
                ],
            )
            .await
            .unwrap();
        let (posting, posting_driver) = connect(&database).await;
        posting.batch_execute("SET ROLE df_finance").await.unwrap();
        let mut service = FinanceService::new(
            posting,
            FakeFence::new(df_finance::ports::FenceVerdict::Current),
            FakeMedia::new(df_finance::ports::MediaVerdict::Admitted),
            FakeMembers::new(vec![HOST]),
            Box::new(FakeClock::new(NOW)),
            Box::new(SeededIds::new(90)),
            FinanceConfig::default(),
        );
        let mut version = support::world::price_version();
        version.id = PriceVersionId::from_array([0x70; 16]);
        service
            .publish_host_price_version(
                &version,
                funding::HostPrices {
                    story_micro_dollars: HOUR,
                    immersive_micro_dollars: 2 * HOUR,
                    cinematic_micro_dollars: 3 * HOUR,
                },
            )
            .await
            .unwrap();
        service
            .apply_purchase(PurchaseCommand {
                payer: HOST,
                processor: "fake",
                payment_ref: "host-credit",
                currency: df_commerce::ledger::CurrencyTag::USD,
                amount_micro_dollars: credit,
            })
            .await
            .unwrap();
        drop(service);
        client
            .batch_execute(
                "SET ROLE df_finance; SET statement_timeout='10s'; SET lock_timeout='2s'",
            )
            .await
            .unwrap();
        Self {
            admin,
            client,
            database,
            drivers: vec![admin_driver, driver, posting_driver],
        }
    }

    async fn service(
        &mut self,
        seed: u64,
        now: i64,
    ) -> FinanceService<FakeFence, FakeMedia, FakeMembers> {
        let (client, driver) = connect(&self.database).await;
        self.drivers.push(driver);
        client
            .batch_execute(
                "SET ROLE df_finance; SET statement_timeout='10s'; SET lock_timeout='2s'",
            )
            .await
            .unwrap();
        FinanceService::new(
            client,
            FakeFence::new(df_finance::ports::FenceVerdict::Current),
            FakeMedia::new(df_finance::ports::MediaVerdict::Admitted),
            FakeMembers::new(vec![HOST]),
            Box::new(FakeClock::new(now)),
            Box::new(SeededIds::new(seed)),
            FinanceConfig::default(),
        )
    }

    async fn finish(self) {
        drop(self.client);
        let mut drivers = self.drivers.into_iter();
        let admin_driver = drivers.next();
        for driver in drivers {
            driver.abort();
            let _cancelled = driver.await;
        }
        self.admin
            .batch_execute(&format!("DROP DATABASE {}", self.database))
            .await
            .unwrap();
        drop(self.admin);
        if let Some(driver) = admin_driver {
            driver.abort();
            let _cancelled = driver.await;
        }
    }

    async fn consent(&mut self) {
        funding::record_consent(&mut self.client, command(), NOW)
            .await
            .unwrap();
    }

    async fn available(&self) -> i64 {
        self.client
            .query_one(
                "SELECT coalesce(sum(b.natural_micro_dollars),0)::bigint \
            FROM finance.account_balance b JOIN finance.account_key k USING(key_id) \
            WHERE k.payer_account_id=$1 AND k.code IN (2110,2210)",
                &[&HOST.as_bytes().as_slice()],
            )
            .await
            .unwrap()
            .get(0)
    }
}

fn command() -> ConsentCommand {
    ConsentCommand {
        session: SESSION,
        account: HOST,
        operation: OperationKey::from_array([0x44; 16]),
        price_version: PriceVersionId::from_array([0x70; 16]),
        cap_micro_dollars: HOUR,
        terms_version: 1,
    }
}

#[test]
fn fundinghook_consent_opens_once_and_cancel_returns_the_allocation() {
    support::run(async {
        let mut fixture = Fixture::new(2 * HOUR).await;
        assert!(
            !funding::start_fundable(&fixture.client, CAMPAIGN, SESSION, NOW)
                .await
                .unwrap()
        );
        assert!(matches!(
            funding::open(&mut fixture.client, CAMPAIGN, SESSION, NOW).await,
            Err(Error::ConsentRequired)
        ));
        fixture.consent().await;
        assert_eq!(fixture.available().await, i64::try_from(2 * HOUR).unwrap());
        assert!(
            funding::start_fundable(&fixture.client, CAMPAIGN, SESSION, NOW)
                .await
                .unwrap()
        );
        funding::open(&mut fixture.client, CAMPAIGN, SESSION, NOW)
            .await
            .unwrap();
        funding::open(&mut fixture.client, CAMPAIGN, SESSION, NOW + 1)
            .await
            .unwrap();
        assert_eq!(fixture.available().await, i64::try_from(HOUR).unwrap());
        let allocations: i64 = fixture
            .client
            .query_one("SELECT count(*) FROM finance.allocation", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(allocations, 1);
        assert!(
            funding::start_fundable(&fixture.client, CAMPAIGN, SESSION, NOW)
                .await
                .unwrap()
        );
        funding::close(&mut fixture.client, SESSION, NOW + 2)
            .await
            .unwrap();
        funding::close(&mut fixture.client, SESSION, NOW + 3)
            .await
            .unwrap();
        assert_eq!(fixture.available().await, i64::try_from(2 * HOUR).unwrap());
        assert!(matches!(
            funding::open(&mut fixture.client, CAMPAIGN, SESSION, NOW + 4).await,
            Err(Error::Closed)
        ));
        fixture.finish().await;
    });
}

#[test]
fn fundinghook_insufficient_credit_never_partially_opens() {
    support::run(async {
        let mut fixture = Fixture::new(HOUR - 1).await;
        fixture.consent().await;
        assert!(
            !funding::start_fundable(&fixture.client, CAMPAIGN, SESSION, NOW)
                .await
                .unwrap()
        );
        assert!(matches!(
            funding::open(&mut fixture.client, CAMPAIGN, SESSION, NOW).await,
            Err(Error::InsufficientCredit)
        ));
        let count: i64 = fixture
            .client
            .query_one("SELECT count(*) FROM finance.session_funding", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(count, 0);
        fixture.finish().await;
    });
}

#[test]
fn fundinghook_restricted_account_and_stale_price_refuse_consent() {
    support::run(async {
        let mut fixture = Fixture::new(HOUR).await;
        let mut stale = command();
        stale.price_version = PriceVersionId::from_array([0x71; 16]);
        assert!(matches!(
            funding::record_consent(&mut fixture.client, stale, NOW).await,
            Err(Error::StalePriceVersion)
        ));
        fixture
            .client
            .batch_execute(
                "RESET ROLE; SET session_replication_role=replica; UPDATE finance.payer SET standing='restricted'; SET session_replication_role=origin; SET ROLE df_finance",
            )
            .await
            .unwrap();
        assert!(matches!(
            funding::record_consent(&mut fixture.client, command(), NOW).await,
            Err(Error::RestrictedAccount)
        ));
        assert!(matches!(
            funding::open(&mut fixture.client, CAMPAIGN, SESSION, NOW).await,
            Err(Error::RestrictedAccount)
        ));
        fixture.finish().await;
    });
}

#[test]
fn fundinghook_nonhost_cannot_consent_and_replay_keeps_original_date() {
    support::run(async {
        let mut fixture = Fixture::new(HOUR).await;
        let mut foreign = command();
        foreign.account = PayerId::from_array([0x55; 16]);
        assert!(matches!(
            funding::record_consent(&mut fixture.client, foreign, NOW).await,
            Err(Error::NotHost)
        ));
        fixture.consent().await;
        let mut changed = command();
        changed.cap_micro_dollars = 2 * HOUR;
        assert!(matches!(
            funding::record_consent(&mut fixture.client, changed, NOW + 1).await,
            Err(Error::InvalidConsent)
        ));
        funding::record_consent(&mut fixture.client, command(), NOW + 1)
            .await
            .unwrap();
        assert_eq!(
            funding::quote(&fixture.client, SESSION, HOST, NOW + 2)
                .await
                .unwrap()
                .consented_at_ms,
            Some(NOW)
        );
        fixture.finish().await;
    });
}

#[test]
fn fundinghook_a_new_price_requires_new_consent_and_cancellation_before_start_is_final() {
    support::run(async {
        let mut fixture = Fixture::new(4 * HOUR).await;
        fixture.consent().await;
        let mut service = fixture.service(91, NOW + 1).await;
        let mut version = support::world::price_version();
        version.id = PriceVersionId::from_array([0x71; 16]);
        version.label = "host-new-price".into();
        service
            .publish_host_price_version(
                &version,
                funding::HostPrices {
                    story_micro_dollars: 2 * HOUR,
                    immersive_micro_dollars: 2 * HOUR,
                    cinematic_micro_dollars: 3 * HOUR,
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            funding::open(&mut fixture.client, CAMPAIGN, SESSION, NOW + 2).await,
            Err(Error::StalePriceVersion)
        ));
        let mut consent = command();
        consent.operation = OperationKey::from_array([0x45; 16]);
        consent.price_version = version.id;
        consent.cap_micro_dollars = 2 * HOUR;
        funding::record_consent(&mut fixture.client, consent, NOW + 3)
            .await
            .unwrap();
        assert!(
            funding::start_fundable(&fixture.client, CAMPAIGN, SESSION, NOW + 4)
                .await
                .unwrap()
        );
        funding::close(&mut fixture.client, SESSION, NOW + 5)
            .await
            .unwrap();
        assert!(matches!(
            funding::open(&mut fixture.client, CAMPAIGN, SESSION, NOW + 6).await,
            Err(Error::Closed)
        ));
        assert_eq!(fixture.available().await, i64::try_from(4 * HOUR).unwrap());
        drop(service);
        fixture.finish().await;
    });
}

#[test]
fn fundinghook_concurrent_start_replays_allocate_the_first_hour_once() {
    support::run(async {
        let mut fixture = Fixture::new(HOUR).await;
        fixture.consent().await;
        let (mut second, driver) = connect(&fixture.database).await;
        fixture.drivers.push(driver);
        second.batch_execute("SET ROLE df_finance").await.unwrap();
        let (first, replayed) = tokio::join!(
            funding::open(&mut fixture.client, CAMPAIGN, SESSION, NOW),
            funding::open(&mut second, CAMPAIGN, SESSION, NOW)
        );
        first.unwrap();
        replayed.unwrap();
        assert_eq!(fixture.available().await, 0);
        assert!(
            funding::start_fundable(&fixture.client, CAMPAIGN, SESSION, NOW)
                .await
                .unwrap()
        );
        drop(second);
        fixture.finish().await;
    });
}

#[test]
fn fundinghook_close_preserves_reserved_credit_until_the_attempt_has_a_known_outcome() {
    support::run(async {
        use df_commerce::finance_ids::{AttemptId, EffectId, TenantKey};
        let mut fixture = Fixture::new(HOUR).await;
        fixture.consent().await;
        funding::open(&mut fixture.client, CAMPAIGN, SESSION, NOW)
            .await
            .unwrap();
        let mut service = fixture.service(92, NOW + 1000).await;
        let command = df_finance::admit::AdmitCommand {
            operation: OperationKey::from_array([0x46; 16]),
            attempt: AttemptId::from_array([0x47; 16]),
            effect: EffectId::from_array([0x48; 16]),
            fence: df_finance::ports::FenceRequest {
                tenant: TenantKey::from_array(*CAMPAIGN.as_bytes()),
                campaign: CAMPAIGN,
                recovery_epoch: 1,
                claim_seq: 1,
                placement_id: [0x49; 16],
            },
            session: SESSION,
            route: support::world::route(),
            work_class: df_commerce::admission::WorkClass::Essential,
            maximum_units: vec![("text_input".into(), 1000), ("text_output".into(), 1000)],
            estimate_units: vec![("text_input".into(), 1000), ("text_output".into(), 1000)],
            quote: None,
            request_digest: [0x50; 32],
            media: None,
        };
        service.admit(&command).await.unwrap();
        let deadlines = df_commerce::lifecycle::Deadlines::default();
        service
            .prepare_dispatch(command.attempt, deadlines)
            .await
            .unwrap();
        service
            .mark_attempt_sent(command.attempt, [0x51; 16], deadlines)
            .await
            .unwrap();
        funding::close(&mut fixture.client, SESSION, NOW + 2000)
            .await
            .unwrap();
        let status: String = fixture
            .client
            .query_one("SELECT status FROM finance.session_funding", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(status, "settling");
        let held:i64=fixture.client.query_one("SELECT coalesce(sum(natural_micro_dollars),0)::bigint             FROM finance.account_balance JOIN finance.account_key USING(key_id) WHERE code IN (2130,2230)",&[]).await.unwrap().get(0);
        assert!(held > 0);
        assert_eq!(
            fixture.available().await + held,
            i64::try_from(HOUR).unwrap()
        );
        funding::close(&mut fixture.client, SESSION, NOW + 3000)
            .await
            .unwrap();
        assert_eq!(
            fixture.available().await + held,
            i64::try_from(HOUR).unwrap()
        );
        drop(service);
        fixture.finish().await;
    });
}

#[test]
fn fundinghook_the_posting_boundary_refuses_reopening_through_a_closure_event() {
    support::run(async {
        let mut fixture = Fixture::new(HOUR).await;
        fixture.consent().await;
        funding::open(&mut fixture.client, CAMPAIGN, SESSION, NOW)
            .await
            .unwrap();
        funding::close(&mut fixture.client, SESSION, NOW + 1)
            .await
            .unwrap();
        let tx = fixture.client.transaction().await.unwrap();
        let lock = serde_json::json!({"payer":HOST.hex(),"session":SESSION.hex()}).to_string();
        tx.query_one(
            "SELECT finance.lock_flow('allocate',$1::text::jsonb)",
            &[&lock],
        )
        .await
        .unwrap();
        let revision: i64 = tx
            .query_one(
                "SELECT revision FROM finance.session_funding WHERE session_id=$1",
                &[&SESSION.as_bytes().as_slice()],
            )
            .await
            .unwrap()
            .get(0);
        let event =
            serde_json::json!({"event_id":"99".repeat(16),"event_kind":"host_funding_closed",
            "payload_schema":1,"entries":[],"changes":[{"kind":"host_session_close_update",
            "expected_revision":revision,"row":{"session_id":format!(r"\x{}",SESSION.hex()),
                "status":"open","settling_at_ms":NOW+1,"closed_at_ms":null}}]})
            .to_string();
        let error = tx
            .query_one("SELECT finance.commit_event($1::text::jsonb)", &[&event])
            .await
            .unwrap_err();
        assert_eq!(error.code().map(|code| code.code()), Some("FN003"));
        tx.rollback().await.unwrap();
        fixture.finish().await;
    });
}

#[test]
fn fundingwire_published_customer_prices_replay_and_refuse_changed_version_payload() {
    support::run(async {
        let mut fixture = Fixture::new(HOUR).await;
        let mut version = support::world::price_version();
        version.id = PriceVersionId::from_array([0x77; 16]);
        version.label = "fundingwire-published-prices".to_owned();
        version.tariff.clear();
        let prices = funding::HostPrices {
            story_micro_dollars: 3_000_000,
            immersive_micro_dollars: 8_000_000,
            cinematic_micro_dollars: 20_000_000,
        };
        funding::publish_host_price_version(&mut fixture.client, &version, prices, NOW + 100)
            .await
            .unwrap();
        funding::publish_host_price_version(&mut fixture.client, &version, prices, NOW + 100)
            .await
            .unwrap();
        let row = fixture.client.query_one("SELECT effective_from_ms, presets::text FROM finance.price_version WHERE price_version_id=$1", &[&version.id.as_bytes().as_slice()]).await.unwrap();
        assert_eq!(row.get::<_, i64>(0), NOW + 100);
        assert_eq!(
            funding::quote(&fixture.client, SESSION, HOST, NOW)
                .await
                .unwrap()
                .price_version,
            PriceVersionId::from_array([0x70; 16])
        );
        assert_eq!(
            funding::quote(&fixture.client, SESSION, HOST, NOW + 100)
                .await
                .unwrap()
                .price_version,
            version.id
        );

        let presets: serde_json::Value = serde_json::from_str(&row.get::<_, String>(1)).unwrap();
        assert_eq!(presets["story"]["hourly_ceiling_micro_dollars"], 3_000_000);
        assert_eq!(
            presets["immersive"]["hourly_ceiling_micro_dollars"],
            8_000_000
        );
        assert_eq!(
            presets["cinematic"]["hourly_ceiling_micro_dollars"],
            20_000_000
        );
        let changed = funding::HostPrices {
            story_micro_dollars: 3_000_001,
            ..prices
        };
        assert!(
            funding::publish_host_price_version(&mut fixture.client, &version, changed, NOW + 100)
                .await
                .is_err()
        );
        assert!(
            funding::publish_host_price_version(&mut fixture.client, &version, prices, NOW + 101)
                .await
                .is_err()
        );
        fixture.finish().await;
    });
}

#[test]
fn fundingwire_durable_cancel_before_start_releases_credit_and_can_finish_after_restart() {
    support::run(async {
        let mut fixture = Fixture::new(2 * HOUR).await;
        fixture.consent().await;
        funding::open(&mut fixture.client, CAMPAIGN, SESSION, NOW)
            .await
            .unwrap();
        assert_eq!(fixture.available().await, i64::try_from(HOUR).unwrap());
        for account in [PayerId::from_array([0x98; 16]), HOST] {
            let cancelled = fixture
                .client
                .query_one(
                    "SELECT df_runtime.cancel_host_lobby($1,$2)",
                    &[
                        &SESSION.as_bytes().as_slice(),
                        &account.as_bytes().as_slice(),
                    ],
                )
                .await
                .unwrap()
                .get::<_, bool>(0);
            assert_eq!(cancelled, account == HOST);
        }
        // The lifecycle commit survives independently of the later finance transaction.
        let mut reopened = fixture.service(234, NOW + 200).await;
        reopened.close_host_session(SESSION).await.unwrap();
        assert_eq!(fixture.available().await, i64::try_from(2 * HOUR).unwrap());
        reopened.close_host_session(SESSION).await.unwrap();
        assert!(
            fixture
                .client
                .query_one(
                    "SELECT df_runtime.cancel_host_lobby($1,$2)",
                    &[&SESSION.as_bytes().as_slice(), &HOST.as_bytes().as_slice()]
                )
                .await
                .unwrap()
                .get::<_, bool>(0)
        );
        assert!(matches!(
            funding::open(&mut fixture.client, CAMPAIGN, SESSION, NOW + 201).await,
            Err(Error::Closed)
        ));
        drop(reopened);
        fixture.finish().await;
    });
}

#[test]
fn fundingwire_cancel_refuses_started_sessions() {
    support::run(async {
        let fixture = Fixture::new(HOUR).await;
        let (admin, driver) = connect(&fixture.database).await;
        admin
            .batch_execute("UPDATE df_runtime.table_sessions SET phase='preparing'")
            .await
            .unwrap();
        assert!(
            !fixture
                .client
                .query_one(
                    "SELECT df_runtime.cancel_host_lobby($1,$2)",
                    &[&SESSION.as_bytes().as_slice(), &HOST.as_bytes().as_slice()]
                )
                .await
                .unwrap()
                .get::<_, bool>(0)
        );
        drop(admin);
        driver.abort();
        let _ = driver.await;
        fixture.finish().await;
    });
}

#[test]
fn fundingwire_force_rls_definer_policies_are_required_for_quote_and_cancel() {
    support::run(async {
        let fixture = Fixture::new(HOUR).await;
        let (admin, driver) = connect(&fixture.database).await;
        admin.batch_execute("DROP POLICY finance_host_funding_read ON df_runtime.table_sessions; DROP POLICY finance_host_lobby_end ON df_runtime.table_sessions").await.unwrap();
        assert!(matches!(
            funding::quote(&fixture.client, SESSION, HOST, NOW).await,
            Err(Error::NotHost)
        ));
        admin.batch_execute("CREATE POLICY finance_host_funding_read ON df_runtime.table_sessions FOR SELECT TO df_finance_definer USING (true)").await.unwrap();
        assert_eq!(
            funding::quote(&fixture.client, SESSION, HOST, NOW)
                .await
                .unwrap()
                .hourly_micro_dollars,
            HOUR
        );
        assert!(
            !fixture
                .client
                .query_one(
                    "SELECT df_runtime.cancel_host_lobby($1,$2)",
                    &[&SESSION.as_bytes().as_slice(), &HOST.as_bytes().as_slice()]
                )
                .await
                .unwrap()
                .get::<_, bool>(0)
        );
        admin.batch_execute("CREATE POLICY finance_host_lobby_end ON df_runtime.table_sessions FOR UPDATE TO df_finance_definer USING (phase IN ('lobby','ended')) WITH CHECK (phase='ended')").await.unwrap();
        assert!(
            fixture
                .client
                .query_one(
                    "SELECT df_runtime.cancel_host_lobby($1,$2)",
                    &[&SESSION.as_bytes().as_slice(), &HOST.as_bytes().as_slice()]
                )
                .await
                .unwrap()
                .get::<_, bool>(0)
        );
        drop(admin);
        driver.abort();
        let _ = driver.await;
        fixture.finish().await;
    });
}
