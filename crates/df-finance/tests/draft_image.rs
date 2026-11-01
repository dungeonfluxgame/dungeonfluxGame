#![cfg(not(target_arch = "wasm32"))]
//! Disposable PG 55471 tests; required mode cannot report a skipped pass.
mod support;

use df_commerce::draft_image::{
    AssetId, Binding, Consent, ConsentId, DraftId, JobId, PublicationId, Scope, StageId,
};
use df_commerce::finance_ids::QuoteId;
use df_finance::draft_image::{
    AdmissionCommand, AdmissionPort, AdmissionRefusal, Error, QuoteCommand, Receipt,
    SettlementCommand, Usability,
};
use df_finance::fakes::SystemClock;
use df_finance::ports::Clock;
use df_types::AccountId;
use support::world::{World, operation, payer, purchase};
use tokio_postgres::Transaction;

const MIGRATION: &str =
    include_str!("../../df-persistence/migrations/0043_imagery_draft_funding.sql");

fn account() -> AccountId {
    AccountId::from_bytes(payer(1).as_bytes()).unwrap()
}
fn now() -> i64 {
    SystemClock.now_ms()
}

async fn world() -> Option<World> {
    if std::env::var_os("DUNGEONFLUX_FINANCE_PG_PORT").is_none() {
        assert!(
            std::env::var_os("DUNGEONFLUX_REQUIRE_SCRATCH_PG").is_none(),
            "required real PG port missing"
        );
        return None;
    }
    assert_eq!(
        std::env::var("DUNGEONFLUX_FINANCE_PG_PORT").unwrap(),
        "55471",
        "owned scratch port required"
    );
    let world = World::open().await.unwrap();
    let database: String = world
        .admin
        .query_one("SELECT current_database()", &[])
        .await
        .unwrap()
        .get(0);
    eprintln!(
        "CREATOR_FUNDING_TEST_DB pid={} database={database}",
        std::process::id()
    );
    use std::io::Write;
    let mut registry =
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
                "../../artifacts/tmp/imagery-20261027/wave2/creator-funding-owned-databases.txt",
            ))
            .unwrap();
    writeln!(registry, "pid={} database={database}", std::process::id()).unwrap();
    world.admin.batch_execute(MIGRATION).await.expect("0043");
    world
        .admin
        .batch_execute(
            "CREATE TABLE draft_test_link(attempt bytea PRIMARY KEY, fingerprint bytea NOT NULL); GRANT SELECT,INSERT ON draft_test_link TO df_finance",
        )
        .await
        .unwrap();
    let mut service = world.service().await;
    service
        .apply_purchase(purchase(payer(1), "draft-wallet", 1_000))
        .await
        .unwrap();
    Some(world)
}

fn binding(tag: u8) -> Binding {
    Binding {
        scope: Scope {
            account: account(),
            draft: DraftId([1; 16]),
            draft_revision: 1,
            job: JobId([2; 16]),
            source_digest: [3; 32],
            config_digest: [4; 32],
            provider: "openai".into(),
            model: "gpt-6-luna".into(),
            route: "portrait".into(),
            price_version: [0x70; 16],
            maximum_units: vec![("text_input".into(), 100), ("text_output".into(), 100)],
        },
        stage: StageId([tag; 16]),
        asset: AssetId([tag + 1; 16]),
        publication: PublicationId([tag + 2; 16]),
        operation: [tag + 3; 16],
        attempt: [tag + 4; 16],
        effect: [tag + 5; 16],
        payload_digest: [tag; 32],
    }
}

fn consent(cap: u64) -> Consent {
    Consent {
        id: ConsentId([80; 16]),
        scope: binding(10).scope,
        cap_micro_dollars: cap,
        expires_at_ms: now() + 60_000,
        revision: 1,
        revoked: false,
    }
}

fn command(tag: u8) -> AdmissionCommand {
    AdmissionCommand {
        consent: ConsentId([80; 16]),
        consent_revision: 1,
        quote: QuoteId::from_array([tag + 6; 16]),
        binding: binding(tag),
    }
}

struct Link {
    refuse: bool,
}
impl AdmissionPort for Link {
    async fn bind(
        &self,
        tx: &Transaction<'_>,
        _binding: &Binding,
        receipt: &Receipt,
    ) -> Result<(), Error> {
        self.record(tx, receipt).await
    }
}
impl Link {
    async fn record(&self, tx: &Transaction<'_>, receipt: &Receipt) -> Result<(), Error> {
        if self.refuse {
            return Err(Error::CatalogRefused);
        }
        let stored = tx
            .query_opt(
                "SELECT fingerprint FROM draft_test_link WHERE attempt=$1",
                &[&receipt.attempt.as_bytes().as_slice()],
            )
            .await
            .map_err(|e| Error::Finance(e.into()))?;
        if receipt.replay {
            if stored.as_ref().map(|r| r.get::<_, Vec<u8>>(0)) != Some(receipt.fingerprint.to_vec())
            {
                return Err(Error::LinkConflict);
            }
        } else {
            tx.execute(
                "INSERT INTO draft_test_link VALUES($1,$2)",
                &[
                    &receipt.attempt.as_bytes().as_slice(),
                    &receipt.fingerprint.as_slice(),
                ],
            )
            .await
            .map_err(|e| Error::Finance(e.into()))?;
        }
        Ok(())
    }
}

async fn quote(world: &World, cmd: &AdmissionCommand) {
    let mut service = world.service().await;
    let max = service
        .quote_draft_image(
            account(),
            &QuoteCommand {
                admission: cmd.clone(),
                expires_at_ms: now() + 30_000,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        (max.retail_micro_dollars, max.supplier_micro_dollars),
        (90, 60)
    );
}

#[test]
fn account_admission_replays_exactly_and_catalog_refusal_rolls_back_money() {
    support::run(async {
        let Some(world) = world().await else { return };
        let mut service = world.service().await;
        let terms = consent(200);
        service
            .record_draft_image_consent(account(), &terms)
            .await
            .unwrap();
        let cmd = command(10);
        quote(&world, &cmd).await;
        let refused = service
            .admit_draft_image(account(), &cmd, &Link { refuse: true })
            .await;
        assert!(matches!(refused, Err(Error::CatalogRefused)), "{refused:?}");
        assert_eq!(world.natural(2110, Some(payer(1)), None, None).await, 1_000);
        assert_eq!(world.count("provider_attempt").await, 0);
        let first = service
            .admit_draft_image(account(), &cmd, &Link { refuse: false })
            .await
            .unwrap();
        assert!(!first.replay);
        let replay = world
            .service()
            .await
            .admit_draft_image(account(), &cmd, &Link { refuse: false })
            .await
            .unwrap();
        assert!(replay.replay);
        assert_eq!(first.reservation, replay.reservation);
        assert_eq!(world.natural(2150, Some(payer(1)), None, None).await, 90);
        assert_eq!(world.count("provider_attempt").await, 1);
        let mut changed = cmd.clone();
        changed.binding.payload_digest[0] += 1;
        assert!(matches!(
            service
                .admit_draft_image(account(), &changed, &Link { refuse: false })
                .await,
            Err(Error::Refused(AdmissionRefusal::Conflict))
        ));
        service
            .revoke_draft_image_consent(account(), terms.id)
            .await
            .unwrap();
        assert!(
            service
                .admit_draft_image(account(), &cmd, &Link { refuse: false })
                .await
                .unwrap()
                .replay
        );
        world.finish().await;
    });
}

#[test]
fn concurrent_admissions_share_one_consent_cap_and_one_account_wallet() {
    support::run(async {
        let Some(world) = world().await else { return };
        let mut first = world.service().await;
        let mut second = world.service().await;
        first
            .record_draft_image_consent(account(), &consent(100))
            .await
            .unwrap();
        let a = command(10);
        let b = command(20);
        quote(&world, &a).await;
        quote(&world, &b).await;
        let (a, b) = tokio::join!(
            first.admit_draft_image(account(), &a, &Link { refuse: false }),
            second.admit_draft_image(account(), &b, &Link { refuse: false })
        );
        assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
        assert!(
            matches!(a, Err(Error::Refused(AdmissionRefusal::ConsentCapExceeded)))
                || matches!(b, Err(Error::Refused(AdmissionRefusal::ConsentCapExceeded)))
        );
        assert_eq!(world.natural(2150, Some(payer(1)), None, None).await, 90);
        assert_eq!(world.natural(2110, Some(payer(1)), None, None).await, 910);
        world.finish().await;
    });
}

#[test]
fn unknown_send_retains_full_reserves_and_known_settlement_releases_only_remainder() {
    support::run(async {
        let Some(world) = world().await else { return };
        let mut service = world.service().await;
        service
            .record_draft_image_consent(account(), &consent(100))
            .await
            .unwrap();
        let cmd = command(10);
        quote(&world, &cmd).await;
        let receipt = service
            .admit_draft_image(account(), &cmd, &Link { refuse: false })
            .await
            .unwrap();
        assert!(
            service
                .prepare_draft_image(account(), receipt.attempt)
                .await
                .unwrap()
        );
        assert!(
            service
                .mark_draft_image_sent(account(), receipt.attempt, [99; 16])
                .await
                .unwrap()
        );
        assert!(
            !service
                .mark_draft_image_sent(account(), receipt.attempt, [99; 16])
                .await
                .unwrap()
        );
        service
            .mark_draft_image_unknown(account(), receipt.attempt)
            .await
            .unwrap();
        assert!(matches!(
            service
                .release_unsent_draft_image(account(), receipt.attempt)
                .await,
            Err(Error::Refused(AdmissionRefusal::PotentiallySent))
        ));
        assert_eq!(world.natural(2150, Some(payer(1)), None, None).await, 90);
        assert_eq!(world.line_sum(9100).await, 60);
        let actual = SettlementCommand {
            attempt: receipt.attempt,
            actual_units: vec![("text_input".into(), 20), ("text_output".into(), 20)],
            usability: Usability::Usable,
        };
        let settled = service
            .settle_draft_image(account(), &actual)
            .await
            .unwrap();
        assert_eq!(
            (
                settled.charged_micro_dollars,
                settled.released_micro_dollars
            ),
            (18, 72)
        );
        assert!(
            service
                .settle_draft_image(account(), &actual)
                .await
                .unwrap()
                .replay
        );
        assert_eq!(world.natural(2110, Some(payer(1)), None, None).await, 982);
        assert_eq!(world.natural(2150, Some(payer(1)), None, None).await, 0);
        assert_eq!(world.line_sum(9100).await, 0);
        assert_eq!(world.line_sum(2300).await, -12);
        assert_eq!(world.count("usage_event").await, 1);
        world.finish().await;
    });
}

#[test]
fn billed_failure_releases_customer_reserve_and_keeps_the_real_supplier_cost() {
    support::run(async {
        let Some(world) = world().await else { return };
        let mut service = world.service().await;
        service
            .record_draft_image_consent(account(), &consent(100))
            .await
            .unwrap();
        let cmd = command(10);
        quote(&world, &cmd).await;
        let receipt = service
            .admit_draft_image(account(), &cmd, &Link { refuse: false })
            .await
            .unwrap();
        service
            .prepare_draft_image(account(), receipt.attempt)
            .await
            .unwrap();
        service
            .mark_draft_image_sent(account(), receipt.attempt, [99; 16])
            .await
            .unwrap();
        let result = service
            .settle_draft_image(
                account(),
                &SettlementCommand {
                    attempt: receipt.attempt,
                    actual_units: vec![("text_input".into(), 20), ("text_output".into(), 20)],
                    usability: Usability::NoUsableOutput,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            (result.charged_micro_dollars, result.released_micro_dollars),
            (0, 90)
        );
        assert_eq!(world.natural(2110, Some(payer(1)), None, None).await, 1_000);
        assert_eq!(world.line_sum(2300).await, -12);
        assert_eq!(world.line_sum(5200).await, 12);
        assert_eq!(world.count("company_loss").await, 1);
        world.finish().await;
    });
}

#[test]
fn wrong_account_and_revoked_consent_never_reserve() {
    support::run(async {
        let Some(world) = world().await else { return };
        let mut service = world.service().await;
        let terms = consent(100);
        service
            .record_draft_image_consent(account(), &terms)
            .await
            .unwrap();
        let cmd = command(10);
        quote(&world, &cmd).await;
        let foreign = AccountId::from_bytes(&[88; 16]).unwrap();
        assert!(matches!(
            service
                .admit_draft_image(foreign, &cmd, &Link { refuse: false })
                .await,
            Err(Error::Policy(_))
        ));
        service
            .revoke_draft_image_consent(account(), terms.id)
            .await
            .unwrap();
        assert!(matches!(
            service
                .admit_draft_image(account(), &cmd, &Link { refuse: false })
                .await,
            Err(Error::Refused(AdmissionRefusal::Revoked))
        ));
        assert_eq!(world.count("provider_attempt").await, 0);
        world.finish().await;
    });
}

#[test]
fn campaign_and_account_holds_conserve_the_same_existing_purchased_lot() {
    support::run(async {
        let Some(world) = world().await else { return };
        let mut service = world.service().await;
        service
            .allocate(payer(1), world.session, operation(1), 400, world.consent())
            .await
            .unwrap();
        let campaign = world.admit_command(1, 100, 100);
        service.admit(&campaign).await.unwrap();
        service
            .record_draft_image_consent(account(), &consent(100))
            .await
            .unwrap();
        let cmd = command(10);
        quote(&world, &cmd).await;
        let receipt = service
            .admit_draft_image(account(), &cmd, &Link { refuse: false })
            .await
            .unwrap();
        assert_eq!(world.natural(2110, Some(payer(1)), None, None).await, 510);
        assert_eq!(world.natural(2130, None, None, None).await, 90);
        assert_eq!(world.natural(2150, Some(payer(1)), None, None).await, 90);
        service
            .release_unsent_draft_image(account(), receipt.attempt)
            .await
            .unwrap();
        assert_eq!(world.natural(2110, Some(payer(1)), None, None).await, 600);
        assert_eq!(world.natural(2130, None, None, None).await, 90);
        for lot in world.lots_of(payer(1)).await {
            world
                .admin
                .execute(
                    "SELECT finance.check_lot_conservation($1)",
                    &[&lot.as_bytes().as_slice()],
                )
                .await
                .unwrap();
        }
        world.finish().await;
    });
}

async fn clock_service(
    world: &World,
    clock: std::sync::Arc<df_finance::fakes::FakeClock>,
) -> support::world::Service {
    let client = world.scratch.connect().await;
    client.batch_execute("SET ROLE df_finance").await.unwrap();
    df_finance::service::FinanceService::new(
        client,
        df_finance::fakes::FakeFence::new(df_finance::ports::FenceVerdict::Current),
        df_finance::fakes::FakeMedia::new(df_finance::ports::MediaVerdict::Admitted),
        df_finance::fakes::FakeMembers::new(vec![]),
        Box::new(clock),
        Box::new(df_finance::fakes::SeededIds::new(900_001)),
        Default::default(),
    )
}

#[test]
fn deterministic_expiry_and_empty_available_wallet_refuse_without_creating_an_attempt() {
    support::run(async {
        let Some(world) = world().await else { return };
        let time = now();
        let clock = std::sync::Arc::new(df_finance::fakes::FakeClock::new(time));
        let mut service = clock_service(&world, clock.clone()).await;
        let mut terms = consent(100);
        terms.expires_at_ms = time + 120_000;
        service
            .record_draft_image_consent(account(), &terms)
            .await
            .unwrap();
        let cmd = command(10);
        service
            .quote_draft_image(
                account(),
                &QuoteCommand {
                    admission: cmd.clone(),
                    expires_at_ms: time + 1_000,
                },
            )
            .await
            .unwrap();
        clock.advance(1_000);
        assert!(matches!(
            service
                .admit_draft_image(account(), &cmd, &Link { refuse: false })
                .await,
            Err(Error::Refused(AdmissionRefusal::QuoteExpired))
        ));
        clock.advance(119_000);
        assert!(matches!(
            service
                .admit_draft_image(account(), &cmd, &Link { refuse: false })
                .await,
            Err(Error::Refused(AdmissionRefusal::ConsentExpired))
        ));
        let mut changed = terms.clone();
        changed.cap_micro_dollars += 1;
        // A fresh wall-clock service sees the stored immutable terms, not a replacement.
        assert!(matches!(
            world
                .service()
                .await
                .record_draft_image_consent(account(), &changed)
                .await,
            Err(Error::Refused(AdmissionRefusal::Conflict))
        ));
        let mut normal = world.service().await;
        normal
            .allocate(
                payer(1),
                world.session,
                operation(1),
                1_000,
                world.consent(),
            )
            .await
            .unwrap();
        let mut new_terms = consent(100);
        new_terms.id = ConsentId([81; 16]);
        normal
            .record_draft_image_consent(account(), &new_terms)
            .await
            .unwrap();
        let mut fresh = command(20);
        fresh.consent = new_terms.id;
        quote(&world, &fresh).await;
        assert!(matches!(
            normal
                .admit_draft_image(account(), &fresh, &Link { refuse: false })
                .await,
            Err(Error::Refused(AdmissionRefusal::InsufficientFunds))
        ));
        assert_eq!(world.count("provider_attempt").await, 0);
        world.finish().await;
    });
}

#[test]
fn known_supplier_overrun_books_platform_loss_and_never_charges_above_the_quote() {
    support::run(async {
        let Some(world) = world().await else { return };
        let mut service = world.service().await;
        service
            .record_draft_image_consent(account(), &consent(200))
            .await
            .unwrap();
        let cmd = command(10);
        quote(&world, &cmd).await;
        let receipt = service
            .admit_draft_image(account(), &cmd, &Link { refuse: false })
            .await
            .unwrap();
        service
            .prepare_draft_image(account(), receipt.attempt)
            .await
            .unwrap();
        service
            .mark_draft_image_sent(account(), receipt.attempt, [99; 16])
            .await
            .unwrap();
        let actual = SettlementCommand {
            attempt: receipt.attempt,
            actual_units: vec![("text_input".into(), 200), ("text_output".into(), 200)],
            usability: Usability::Usable,
        };
        let settled = service
            .settle_draft_image(account(), &actual)
            .await
            .unwrap();
        assert_eq!(settled.charged_micro_dollars, 90);
        assert_eq!(world.line_sum(2300).await, -120);
        assert_eq!(world.line_sum(5100).await, 60);
        assert_eq!(world.line_sum(5200).await, 60);
        let next = command(20);
        quote(&world, &next).await;
        assert!(matches!(
            service
                .admit_draft_image(account(), &next, &Link { refuse: false })
                .await,
            Err(Error::Refused(AdmissionRefusal::SupplierBlocked))
        ));
        world.finish().await;
    });
}

/// Drops the backend COMMIT acknowledgement only after PostgreSQL says COMMIT.
/// The forwarded transaction is real; no application error or fake receipt stands in.
async fn lose_commit_ack(listener: tokio::net::TcpListener) -> std::io::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (mut frontend, _) = listener.accept().await?;
    let mut backend = tokio::net::TcpStream::connect(("127.0.0.1", 55471)).await?;
    let startup_len = frontend.read_u32().await?;
    if !(8..=65_536).contains(&startup_len) {
        return Err(std::io::Error::other("startup bound"));
    }
    let mut startup = vec![0; usize::try_from(startup_len - 4).unwrap()];
    frontend.read_exact(&mut startup).await?;
    backend.write_u32(startup_len).await?;
    backend.write_all(&startup).await?;
    let (mut input, mut output) = frontend.into_split();
    let (mut responses, mut commands) = backend.into_split();
    let commit = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let sent = commit.clone();
    let upstream = async move {
        loop {
            let kind = input.read_u8().await?;
            let len = input.read_u32().await?;
            if !(4..=16_777_216).contains(&len) {
                return Err::<(), std::io::Error>(std::io::Error::other("message bound"));
            }
            let mut body = vec![0; usize::try_from(len - 4).unwrap()];
            input.read_exact(&mut body).await?;
            if kind == b'Q' && body == b"COMMIT\0" {
                sent.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            commands.write_u8(kind).await?;
            commands.write_u32(len).await?;
            commands.write_all(&body).await?;
        }
    };
    let downstream = async move {
        loop {
            let kind = responses.read_u8().await?;
            let len = responses.read_u32().await?;
            if !(4..=16_777_216).contains(&len) {
                return Err::<(), std::io::Error>(std::io::Error::other("message bound"));
            }
            let mut body = vec![0; usize::try_from(len - 4).unwrap()];
            responses.read_exact(&mut body).await?;
            if kind == b'C'
                && body == b"COMMIT\0"
                && commit.load(std::sync::atomic::Ordering::SeqCst)
            {
                output.shutdown().await?;
                return Ok::<(), std::io::Error>(());
            }
            output.write_u8(kind).await?;
            output.write_u32(len).await?;
            output.write_all(&body).await?;
        }
    };
    tokio::select! {result=upstream=>result,result=downstream=>result}
}

#[test]
fn lost_real_commit_ack_recovers_the_same_receipt_without_another_reservation() {
    support::run(async {
        let Some(world) = world().await else { return };
        let mut ordinary = world.service().await;
        ordinary
            .record_draft_image_consent(account(), &consent(100))
            .await
            .unwrap();
        let cmd = command(10);
        quote(&world, &cmd).await;
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let proxy = tokio::spawn(lose_commit_ack(listener));
        let mut config = tokio_postgres::Config::new();
        config
            .host("127.0.0.1")
            .port(port)
            .user("postgres")
            .dbname(world.scratch.config().get_dbname().unwrap())
            .ssl_mode(tokio_postgres::config::SslMode::Disable);
        let (client, connection) = config.connect(tokio_postgres::NoTls).await.unwrap();
        let connection = tokio::spawn(connection);
        client.batch_execute("SET ROLE df_finance").await.unwrap();
        let mut service = df_finance::service::FinanceService::new(
            client,
            df_finance::fakes::FakeFence::new(df_finance::ports::FenceVerdict::Current),
            df_finance::fakes::FakeMedia::new(df_finance::ports::MediaVerdict::Admitted),
            df_finance::fakes::FakeMembers::new(vec![]),
            Box::new(SystemClock),
            Box::new(df_finance::fakes::SeededIds::new(950_001)),
            Default::default(),
        );
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            service.admit_draft_image(account(), &cmd, &Link { refuse: false }),
        )
        .await
        .unwrap();
        assert!(
            matches!(result, Err(Error::CommitUnknown { .. })),
            "{result:?}"
        );
        drop(service);
        proxy.await.unwrap().unwrap();
        // The dropped wire is the deliberate fault; the driver error is observed.
        assert!(connection.await.unwrap().is_err());
        let receipt = ordinary
            .admit_draft_image(account(), &cmd, &Link { refuse: false })
            .await
            .unwrap();
        assert!(receipt.replay);
        assert_eq!(world.count("provider_attempt").await, 1);
        assert_eq!(world.natural(2150, Some(payer(1)), None, None).await, 90);
        assert_eq!(world.natural(2110, Some(payer(1)), None, None).await, 910);
        world.finish().await;
    });
}

#[test]
fn consent_revocation_after_admission_blocks_unsent_egress_and_still_allows_release() {
    support::run(async {
        let Some(world) = world().await else { return };
        let mut service = world.service().await;
        let terms = consent(100);
        service
            .record_draft_image_consent(account(), &terms)
            .await
            .unwrap();
        let cmd = command(10);
        quote(&world, &cmd).await;
        let receipt = service
            .admit_draft_image(account(), &cmd, &Link { refuse: false })
            .await
            .unwrap();
        service
            .prepare_draft_image(account(), receipt.attempt)
            .await
            .unwrap();
        service
            .revoke_draft_image_consent(account(), terms.id)
            .await
            .unwrap();
        assert!(matches!(
            service
                .mark_draft_image_sent(account(), receipt.attempt, [99; 16])
                .await,
            Err(Error::Refused(AdmissionRefusal::Revoked))
        ));
        let released = service
            .release_unsent_draft_image(account(), receipt.attempt)
            .await
            .unwrap();
        assert_eq!(released.released_micro_dollars, 90);
        assert_eq!(world.natural(2110, Some(payer(1)), None, None).await, 1_000);
        assert_eq!(world.line_sum(9100).await, 0);
        world.finish().await;
    });
}

fn rational_version(world: &World, per_units: u64) -> df_commerce::price_schedule::PriceVersion {
    let mut version = world.version.clone();
    version.id = df_commerce::finance_ids::PriceVersionId::from_array([0x71; 16]);
    version.label = "pv-rational-fixture".into();
    version.markup_numerator = 1;
    version.markup_denominator = 1;
    version.tariff = vec![df_commerce::price_schedule::TariffLine {
        provider: "openai".into(),
        model: "gpt-6-luna".into(),
        category: "text_input".into(),
        context_tier_max_tokens: 0,
        nano_usd: 1,
        per: per_units,
    }];
    version
}

async fn rational_quote(
    world: &World,
    version: &df_commerce::price_schedule::PriceVersion,
    tag: u8,
    maximum_units: Vec<(String, u64)>,
) -> (AdmissionCommand, df_commerce::draft_image::Maximums) {
    let mut command = command(tag);
    command.binding.scope.price_version = *version.id.as_bytes();
    command.binding.scope.maximum_units = maximum_units;
    command.consent = ConsentId([tag + 50; 16]);
    let mut terms = consent(100);
    terms.id = command.consent;
    terms.scope = command.binding.scope.clone();
    terms.expires_at_ms = now() + 120_000;
    let mut service = world.service().await;
    service
        .record_draft_image_consent(account(), &terms)
        .await
        .unwrap();
    let maximums = service
        .quote_draft_image(
            account(),
            &QuoteCommand {
                admission: command.clone(),
                expires_at_ms: now() + 60_000,
            },
        )
        .await
        .unwrap();
    (command, maximums)
}

#[test]
fn pg_quote_reserves_the_exact_ceiling_just_above_one_micro_dollar() {
    support::run(async {
        let Some(world) = world().await else { return };
        let version = rational_version(&world, 3_000_000_000_000_000);
        world
            .service()
            .await
            .publish_price_version(&version)
            .await
            .unwrap();
        let quantity = 3_000_000_000_000_000_001;
        let units = vec![("text_input".into(), quantity)];
        let (_, maximums) = rational_quote(&world, &version, 10, units.clone()).await;
        let expected = df_commerce::draft_image::Maximums {
            retail_micro_dollars: 2,
            supplier_micro_dollars: 2,
        };
        let pure = (
            version.retail_maximum("openai", "gpt-6-luna", &units),
            version.supplier_maximum("openai", "gpt-6-luna", &units),
        );
        world.finish().await;
        assert_eq!(pure, (Ok(2), Ok(2)));
        assert_eq!(maximums, expected);
    });
}

#[test]
fn pg_known_charge_floors_the_exact_value_just_below_one_micro_dollar() {
    support::run(async {
        let Some(world) = world().await else { return };
        let version = rational_version(&world, 6_000_000_000_000_000);
        let mut service = world.service().await;
        service.publish_price_version(&version).await.unwrap();
        let (command, maximums) = rational_quote(
            &world,
            &version,
            10,
            vec![("text_input".into(), 6_000_000_000_000_000_000)],
        )
        .await;
        let receipt = service
            .admit_draft_image(account(), &command, &Link { refuse: false })
            .await
            .unwrap();
        service
            .prepare_draft_image(account(), receipt.attempt)
            .await
            .unwrap();
        service
            .mark_draft_image_sent(account(), receipt.attempt, [99; 16])
            .await
            .unwrap();
        let units = vec![("text_input".into(), 5_999_999_999_999_999_999)];
        let settled = service
            .settle_draft_image(
                account(),
                &SettlementCommand {
                    attempt: receipt.attempt,
                    actual_units: units.clone(),
                    usability: Usability::Usable,
                },
            )
            .await
            .unwrap();
        let pure = version.retail_charge("openai", "gpt-6-luna", &units, 1);
        let available = world.natural(2110, Some(payer(1)), None, None).await;
        world.finish().await;
        assert_eq!(pure, Ok(0));
        assert_eq!(
            (
                maximums.retail_micro_dollars,
                settled.charged_micro_dollars,
                settled.released_micro_dollars,
                available
            ),
            (1, 0, 1, 1_000),
        );
    });
}

#[test]
fn pg_supplier_cost_rounds_half_up_only_after_the_exact_fraction_is_known() {
    support::run(async {
        let Some(world) = world().await else { return };
        let version = rational_version(&world, 6_000_000_000_000_000);
        let mut service = world.service().await;
        service.publish_price_version(&version).await.unwrap();
        let mut observed = Vec::new();
        for (tag, quantity, expected_cost) in [
            (10, 2_999_999_999_999_999_999, 0),
            (20, 3_000_000_000_000_000_000, 1),
            (30, 3_000_000_000_000_000_001, 1),
        ] {
            let (command, _) = rational_quote(
                &world,
                &version,
                tag,
                vec![("text_input".into(), 3_000_000_000_000_000_001)],
            )
            .await;
            let receipt = service
                .admit_draft_image(account(), &command, &Link { refuse: false })
                .await
                .unwrap();
            service
                .prepare_draft_image(account(), receipt.attempt)
                .await
                .unwrap();
            service
                .mark_draft_image_sent(account(), receipt.attempt, [99; 16])
                .await
                .unwrap();
            let units = vec![("text_input".into(), quantity)];
            service
                .settle_draft_image(
                    account(),
                    &SettlementCommand {
                        attempt: receipt.attempt,
                        actual_units: units.clone(),
                        usability: Usability::Usable,
                    },
                )
                .await
                .unwrap();
            let supplier_cost: i64 = world
                .admin
                .query_one(
                    "SELECT supplier_micro_dollars FROM finance.usage_event WHERE attempt_id=$1",
                    &[&receipt.attempt.as_bytes().as_slice()],
                )
                .await
                .unwrap()
                .get(0);
            observed.push((
                supplier_cost,
                expected_cost,
                version.supplier_cost("openai", "gpt-6-luna", &units),
            ));
        }
        world.finish().await;
        for (supplier_cost, expected_cost, pure_cost) in observed {
            assert_eq!(pure_cost, Ok(expected_cost));
            assert_eq!(supplier_cost, i64::try_from(expected_cost).unwrap());
        }
    });
}

#[test]
fn pg_heterogeneous_tariffs_sum_before_supplier_and_retail_rounding() {
    support::run(async {
        let Some(world) = world().await else { return };
        let mut version = rational_version(&world, 3);
        version.tariff[0].nano_usd = 1_000;
        version
            .tariff
            .push(df_commerce::price_schedule::TariffLine {
                category: "text_output".into(),
                per: 6,
                ..version.tariff[0].clone()
            });
        let mut service = world.service().await;
        service.publish_price_version(&version).await.unwrap();
        let units = vec![("text_input".into(), 1), ("text_output".into(), 1)];
        let (command, maximums) = rational_quote(&world, &version, 10, units.clone()).await;
        let receipt = service
            .admit_draft_image(account(), &command, &Link { refuse: false })
            .await
            .unwrap();
        service
            .prepare_draft_image(account(), receipt.attempt)
            .await
            .unwrap();
        service
            .mark_draft_image_sent(account(), receipt.attempt, [99; 16])
            .await
            .unwrap();
        let settled = service
            .settle_draft_image(
                account(),
                &SettlementCommand {
                    attempt: receipt.attempt,
                    actual_units: units,
                    usability: Usability::Usable,
                },
            )
            .await
            .unwrap();
        let supplier = world.line_sum(2300).await;
        world.finish().await;
        assert_eq!(
            (
                maximums.retail_micro_dollars,
                maximums.supplier_micro_dollars
            ),
            (1, 1)
        );
        assert_eq!(
            (
                settled.charged_micro_dollars,
                settled.released_micro_dollars
            ),
            (0, 1)
        );
        assert_eq!(supplier, -1);
    });
}

#[test]
fn pg_three_category_supplier_rounding_matches_the_overflow_safe_rust_sum() {
    support::run(async {
        let Some(world) = world().await else { return };
        let mut version = rational_version(&world, 1);
        version.tariff = [8_796_093_022_207, 8_796_093_022_205, 4_398_046_511_103]
            .into_iter()
            .enumerate()
            .map(|(index, per)| df_commerce::price_schedule::TariffLine {
                provider: "openai".into(),
                model: "gpt-6-luna".into(),
                category: format!("category_{index}"),
                context_tier_max_tokens: 0,
                nano_usd: 1_000,
                per,
            })
            .collect();
        let units: Vec<_> = (0..3)
            .map(|index| (format!("category_{index}"), 1))
            .collect();
        let mut service = world.service().await;
        service.publish_price_version(&version).await.unwrap();
        let (command, maximums) = rational_quote(&world, &version, 10, units.clone()).await;
        let receipt = service
            .admit_draft_image(account(), &command, &Link { refuse: false })
            .await
            .unwrap();
        service
            .prepare_draft_image(account(), receipt.attempt)
            .await
            .unwrap();
        service
            .mark_draft_image_sent(account(), receipt.attempt, [99; 16])
            .await
            .unwrap();
        let settled = service
            .settle_draft_image(
                account(),
                &SettlementCommand {
                    attempt: receipt.attempt,
                    actual_units: units.clone(),
                    usability: Usability::Usable,
                },
            )
            .await
            .unwrap();
        let supplier = world.line_sum(2300).await;
        let available = world.natural(2110, Some(payer(1)), None, None).await;
        world.finish().await;
        assert_eq!(version.supplier_cost("openai", "gpt-6-luna", &units), Ok(0));
        assert_eq!(
            (
                maximums.retail_micro_dollars,
                maximums.supplier_micro_dollars
            ),
            (1, 1)
        );
        assert_eq!(
            (
                settled.charged_micro_dollars,
                settled.released_micro_dollars
            ),
            (0, 1)
        );
        assert_eq!((supplier, available), (0, 1_000));
    });
}

#[path = "draft_image/stages.rs"]
mod stages;
