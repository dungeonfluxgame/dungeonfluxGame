#![cfg(not(target_arch = "wasm32"))]

//! Campaign image-stage finance admission end to end: the binding rides the admission
//! transaction, the catalog linkage commits or rolls back with the money, and a replay only
//! verifies. The linkage table below is test-owned; finance never writes it directly.

mod support;

use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::time::Duration;

use df_commerce::admission::{AdmissionRefusal, WorkClass};
use df_commerce::finance_ids::{AttemptId, EffectId};
use df_finance::admit::AdmissionOutcome;
use df_finance::held::QuoteCommand;
use df_finance::image_stage::{
    BindingRefusal, ImageAssetId, ImageJobId, ImageStageAdmission, ImageStageBinding,
    ImageStageHookOutcome, ImageStageId, ImageStagePhase, ImageStagePort, MAX_OWNER_SCOPE_BYTES,
    OwnerScope, PublicationAttemptId, StageRefusal, image_stage_quote_digest,
    image_stage_request_digest,
};
use df_finance::ports::MediaRequest;
use df_finance::service::{FinanceConfig, FinanceError};
use support::world::{CURRENCY, Service, World, operation, payer, route};
use tokio_postgres::Transaction;

const LINK_DDL: &str = "CREATE TABLE image_stage_link (
    operation_id bytea PRIMARY KEY,
    owner_scope bytea NOT NULL,
    job_id bytea NOT NULL,
    stage_id bytea NOT NULL,
    asset_id bytea NOT NULL,
    publication_attempt_id bytea NOT NULL,
    tenant_id bytea NOT NULL,
    campaign_id bytea NOT NULL,
    session_id bytea NOT NULL,
    attempt_id bytea NOT NULL,
    effect_id bytea NOT NULL,
    reservation_id bytea NOT NULL,
    retail_maximum_micro_dollars bigint NOT NULL
)";

/// How the test linkage port answers the hook.
#[derive(Clone)]
enum Behavior {
    Admit,
    Refuse(StageRefusal),
    Hang,
    BrokenSql,
    Uncertain,
}

/// A test-only catalog linkage over the open transaction: `Admit` inserts or matches the link
/// row, `Refuse`/`Uncertain` answer without writing, `Hang` never answers, and `BrokenSql`
/// runs a query that the database rejects so the error keeps its SQLSTATE.
struct LinkPort {
    behavior: Mutex<Behavior>,
    calls: Mutex<Vec<ImageStagePhase>>,
}

impl LinkPort {
    fn new(behavior: Behavior) -> Self {
        Self {
            behavior: Mutex::new(behavior),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn set(&self, behavior: Behavior) {
        *self.behavior.lock().expect("behavior") = behavior;
    }

    fn phases(&self) -> Vec<ImageStagePhase> {
        self.calls.lock().expect("calls").clone()
    }

    fn behavior_of(&self) -> Behavior {
        self.behavior.lock().expect("behavior").clone()
    }

    fn push(&self, phase: ImageStagePhase) {
        self.calls.lock().expect("calls").push(phase);
    }

    fn unavailable(error: tokio_postgres::Error) -> FinanceError {
        match error.as_db_error() {
            Some(db) => FinanceError::ImageStageUnavailable {
                sqlstate: Some(db.code().code().to_owned()),
                detail: db.message().to_owned(),
            },
            None => FinanceError::ImageStageUnavailable {
                sqlstate: None,
                detail: error.to_string(),
            },
        }
    }

    async fn stored(
        transaction: &Transaction,
        operation: &[u8; 16],
    ) -> Result<Option<Vec<Vec<u8>>>, tokio_postgres::Error> {
        let row = transaction
            .query_opt(
                "SELECT owner_scope, job_id, stage_id, asset_id, publication_attempt_id, \
                 tenant_id, campaign_id, session_id, attempt_id, effect_id, reservation_id, \
                 retail_maximum_micro_dollars FROM image_stage_link WHERE operation_id = $1",
                &[&operation.to_vec()],
            )
            .await?;
        Ok(row.map(|row| {
            let mut link: Vec<Vec<u8>> = (0..11)
                .map(|index| row.get::<_, Vec<u8>>(index))
                .collect();
            link.push(row.get::<_, i64>(11).to_be_bytes().to_vec());
            link
        }))
    }

    fn expected(
        binding: &ImageStageBinding,
        receipt: &df_commerce::snapshot::ReceiptRow,
    ) -> Vec<Vec<u8>> {
        vec![
            binding.owner_scope.as_bytes().to_vec(),
            binding.job.as_bytes().to_vec(),
            binding.stage.as_bytes().to_vec(),
            binding.asset.as_bytes().to_vec(),
            binding.publication_attempt.as_bytes().to_vec(),
            binding.tenant.as_bytes().to_vec(),
            binding.campaign.as_bytes().to_vec(),
            binding.session.as_bytes().to_vec(),
            binding.attempt.as_bytes().to_vec(),
            binding.effect.as_bytes().to_vec(),
            receipt.reservation.as_bytes().to_vec(),
            receipt
                .retail_maximum_micro_dollars
                .to_be_bytes()
                .to_vec(),
        ]
    }
}

impl ImageStagePort for LinkPort {
    fn admit_image_link<'a>(
        &'a self,
        transaction: &'a Transaction,
        binding: &'a ImageStageBinding,
        receipt: &'a df_commerce::snapshot::ReceiptRow,
    ) -> Pin<Box<dyn Future<Output = Result<ImageStageHookOutcome, FinanceError>> + Send + 'a>>
    {
        Box::pin(async move {
            self.push(ImageStagePhase::New);
            match self.behavior_of() {
                Behavior::Hang => {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    Ok(ImageStageHookOutcome::Accepted)
                }
                Behavior::Refuse(refusal) => Err(FinanceError::ImageStageRefusal(refusal)),
                Behavior::Uncertain => Err(FinanceError::ImageStageUncertain {
                    sqlstate: None,
                    detail: "injected uncertainty".into(),
                }),
                Behavior::BrokenSql => {
                    match transaction.query_one("SELECT * FROM image_stage_link_missing", &[]).await
                    {
                        Ok(_) => Ok(ImageStageHookOutcome::Accepted),
                        Err(error) => Err(Self::unavailable(error)),
                    }
                }
                Behavior::Admit => {
                    let expected = Self::expected(binding, receipt);
                    match Self::stored(transaction, binding.operation.as_bytes())
                        .await
                        .map_err(Self::unavailable)?
                    {
                        None => {
                            let inserted = transaction
                                .execute(
                                    "INSERT INTO image_stage_link (operation_id, owner_scope, \
                                     job_id, stage_id, asset_id, publication_attempt_id, \
                                     tenant_id, campaign_id, session_id, attempt_id, effect_id, \
                                     reservation_id, retail_maximum_micro_dollars) \
                                     VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)",
                                    &[
                                        &binding.operation.as_bytes().to_vec(),
                                        &expected[0],
                                        &expected[1],
                                        &expected[2],
                                        &expected[3],
                                        &expected[4],
                                        &expected[5],
                                        &expected[6],
                                        &expected[7],
                                        &expected[8],
                                        &expected[9],
                                        &expected[10],
                                        &i64::from_be_bytes(
                                            expected[11].clone().try_into().expect("retail"),
                                        ),
                                    ],
                                )
                                .await;
                            match inserted {
                                Ok(1) => Ok(ImageStageHookOutcome::Accepted),
                                Ok(_) => Err(FinanceError::ImageStageConflict),
                                Err(error)
                                    if error.as_db_error().is_some_and(|db| {
                                        db.code().code() == "23505"
                                    }) =>
                                {
                                    match Self::stored(transaction, binding.operation.as_bytes())
                                        .await
                                        .map_err(Self::unavailable)?
                                    {
                                        Some(stored) if stored == expected => {
                                            Ok(ImageStageHookOutcome::Existing)
                                        }
                                        _ => Err(FinanceError::ImageStageConflict),
                                    }
                                }
                                Err(error) => Err(Self::unavailable(error)),
                            }
                        }
                        Some(stored) if stored == expected => Ok(ImageStageHookOutcome::Existing),
                        Some(_) => Err(FinanceError::ImageStageConflict),
                    }
                }
            }
        })
    }

    fn verify_image_link<'a>(
        &'a self,
        transaction: &'a Transaction,
        binding: &'a ImageStageBinding,
        receipt: &'a df_commerce::snapshot::ReceiptRow,
    ) -> Pin<Box<dyn Future<Output = Result<ImageStageHookOutcome, FinanceError>> + Send + 'a>>
    {
        Box::pin(async move {
            self.push(ImageStagePhase::Replay);
            match self.behavior_of() {
                Behavior::Hang => {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    Ok(ImageStageHookOutcome::Existing)
                }
                Behavior::Refuse(refusal) => Err(FinanceError::ImageStageRefusal(refusal)),
                Behavior::Uncertain => Err(FinanceError::ImageStageUncertain {
                    sqlstate: None,
                    detail: "injected uncertainty".into(),
                }),
                Behavior::BrokenSql => {
                    match transaction.query_one("SELECT * FROM image_stage_link_missing", &[]).await
                    {
                        Ok(_) => Ok(ImageStageHookOutcome::Existing),
                        Err(error) => Err(Self::unavailable(error)),
                    }
                }
                Behavior::Admit => {
                    match Self::stored(transaction, binding.operation.as_bytes())
                        .await
                        .map_err(Self::unavailable)?
                    {
                        None => Err(FinanceError::ImageStageMissingLink),
                        Some(stored) if stored == Self::expected(binding, receipt) => {
                            Ok(ImageStageHookOutcome::Existing)
                        }
                        Some(_) => Err(FinanceError::ImageStageConflict),
                    }
                }
            }
        })
    }
}

struct Fixture {
    world: World,
    service: Service,
    command: ImageStageAdmission,
    payload: [u8; 32],
}

fn binding_of(tag: u8, world: &World, admit: &df_finance::admit::AdmitCommand) -> ImageStageBinding {
    ImageStageBinding {
        owner_scope: OwnerScope::new(b"owner-scope-1".to_vec()).expect("scope"),
        job: ImageJobId::new([0x10 + tag; 16]).expect("job"),
        stage: ImageStageId::new([0x20 + tag; 16]).expect("stage"),
        asset: ImageAssetId::new([0x30 + tag; 16]).expect("asset"),
        publication_attempt: PublicationAttemptId::new([0x40 + tag; 16]).expect("publication"),
        tenant: world.tenant,
        campaign: world.campaign,
        session: world.session,
        operation: admit.operation,
        attempt: admit.attempt,
        effect: admit.effect,
    }
}

async fn fixture_with(tag: u8, config: Option<FinanceConfig>) -> Option<(Fixture, LinkPort)> {
    let world = World::open().await?;
    world
        .admin
        .batch_execute(LINK_DDL)
        .await
        .expect("link table");
    world
        .admin
        .batch_execute("GRANT ALL ON image_stage_link TO df_finance")
        .await
        .expect("grant");
    let mut service = match config {
        Some(config) => world.service_with_config(config).await,
        None => world.service().await,
    };
    service
        .grant_trial(payer(1), [tag; 32], CURRENCY)
        .await
        .expect("trial")
        .expect("granted");
    service
        .allocate(payer(1), world.session, operation(100), 10_000_000, world.consent())
        .await
        .expect("allocate");
    let admit = world.admit_command(tag, 2_000_000, 1_000_000);
    let payload = admit.request_digest;
    let binding = binding_of(tag, &world, &admit);
    let port = LinkPort::new(Behavior::Admit);
    Some((
        Fixture {
            world,
            service,
            command: ImageStageAdmission { admit, binding },
            payload,
        },
        port,
    ))
}

async fn fixture(tag: u8) -> Option<(Fixture, LinkPort)> {
    fixture_with(tag, None).await
}

async fn counts(world: &World) -> (i64, i64, i64, i64) {
    (
        world.count("journal_entry").await,
        world.count("provider_attempt").await,
        world.count("operation_receipt").await,
        world
            .admin
            .query_one("SELECT count(*) FROM image_stage_link", &[])
            .await
            .expect("links")
            .get(0),
    )
}

async fn attempt_asset(world: &World, attempt: &AttemptId) -> Option<Vec<u8>> {
    world
        .admin
        .query_opt(
            "SELECT media_asset_id FROM finance.provider_attempt WHERE attempt_id = $1",
            &[&attempt.as_bytes().to_vec()],
        )
        .await
        .expect("attempt")
        .expect("row")
        .get(0)
}

fn sample_binding() -> (ImageStageBinding, [u8; 32]) {
    use df_commerce::finance_ids::{CampaignKey, OperationKey, SessionKey, TenantKey};
    let binding = ImageStageBinding {
        owner_scope: OwnerScope::new(b"owner".to_vec()).expect("scope"),
        job: ImageJobId::new([1; 16]).expect("job"),
        stage: ImageStageId::new([2; 16]).expect("stage"),
        asset: ImageAssetId::new([3; 16]).expect("asset"),
        publication_attempt: PublicationAttemptId::new([4; 16]).expect("publication"),
        tenant: TenantKey::from_array([5; 16]),
        campaign: CampaignKey::from_array([6; 16]),
        session: SessionKey::from_array([7; 16]),
        operation: OperationKey::from_array([8; 16]),
        attempt: AttemptId::from_array([9; 16]),
        effect: EffectId::from_array([10; 16]),
    };
    (binding, [11; 32])
}

#[test]
fn the_stage_digest_changes_when_any_binding_or_payload_field_changes() {
    let (binding, payload) = sample_binding();
    let base = image_stage_request_digest(&binding, &payload);
    let mut changed = binding.clone();
    changed.owner_scope = OwnerScope::new(b"other".to_vec()).expect("scope");
    assert_ne!(image_stage_request_digest(&changed, &payload), base);
    let mut changed = binding.clone();
    changed.job = ImageJobId::new([9; 16]).expect("job");
    assert_ne!(image_stage_request_digest(&changed, &payload), base);
    let mut changed = binding.clone();
    changed.stage = ImageStageId::new([9; 16]).expect("stage");
    assert_ne!(image_stage_request_digest(&changed, &payload), base);
    let mut changed = binding.clone();
    changed.asset = ImageAssetId::new([9; 16]).expect("asset");
    assert_ne!(image_stage_request_digest(&changed, &payload), base);
    let mut changed = binding.clone();
    changed.publication_attempt = PublicationAttemptId::new([9; 16]).expect("publication");
    assert_ne!(image_stage_request_digest(&changed, &payload), base);
    let mut changed = binding.clone();
    changed.tenant = df_commerce::finance_ids::TenantKey::from_array([9; 16]);
    assert_ne!(image_stage_request_digest(&changed, &payload), base);
    let mut changed = binding.clone();
    changed.campaign = df_commerce::finance_ids::CampaignKey::from_array([9; 16]);
    assert_ne!(image_stage_request_digest(&changed, &payload), base);
    let mut changed = binding.clone();
    changed.session = df_commerce::finance_ids::SessionKey::from_array([9; 16]);
    assert_ne!(image_stage_request_digest(&changed, &payload), base);
    let mut changed = binding.clone();
    changed.operation = df_commerce::finance_ids::OperationKey::from_array([9; 16]);
    assert_ne!(image_stage_request_digest(&changed, &payload), base);
    let mut changed = binding.clone();
    changed.attempt = AttemptId::from_array([9; 16]);
    assert_ne!(image_stage_request_digest(&changed, &payload), base);
    let mut changed = binding.clone();
    changed.effect = EffectId::from_array([9; 16]);
    assert_ne!(image_stage_request_digest(&changed, &payload), base);
    assert_ne!(image_stage_request_digest(&binding, &[9; 32]), base);
}

#[test]
fn the_quote_helper_matches_the_admission_digest() {
    let (binding, payload) = sample_binding();
    assert_eq!(
        image_stage_quote_digest(&binding, &payload),
        image_stage_request_digest(&binding, &payload),
    );
}

#[test]
fn checked_ids_and_owner_scope_refuse_empty_zero_and_oversized() {
    assert!(OwnerScope::new(Vec::new()).is_err());
    assert!(OwnerScope::new(vec![7; MAX_OWNER_SCOPE_BYTES + 1]).is_err());
    assert!(OwnerScope::new(vec![7; MAX_OWNER_SCOPE_BYTES]).is_ok());
    assert_eq!(
        OwnerScope::new(Vec::new()),
        Err(BindingRefusal::OwnerScopeEmpty)
    );
    assert!(ImageJobId::new([0; 16]).is_err());
    assert!(ImageStageId::new([0; 16]).is_err());
    assert!(ImageAssetId::new([0; 16]).is_err());
    assert!(PublicationAttemptId::new([0; 16]).is_err());
    assert!(ImageJobId::new([1; 16]).is_ok());
    assert!(ImageStageId::new([1; 16]).is_ok());
    assert!(ImageAssetId::new([1; 16]).is_ok());
    assert!(PublicationAttemptId::new([1; 16]).is_ok());
}

#[test]
fn admits_a_new_image_stage_and_writes_asset_and_link_atomically() {
    support::run(async {
        let Some((mut fixture, port)) = fixture(1).await else {
            return;
        };
        let before = counts(&fixture.world).await;
        let outcome = fixture
            .service
            .admit_image_stage(&fixture.command, &port)
            .await
            .expect("admitted");
        let AdmissionOutcome::Admitted {
            attempt,
            retail_maximum_micro_dollars,
            ..
        } = outcome
        else {
            panic!("expected a fresh admission, got {outcome:?}");
        };
        assert_eq!(retail_maximum_micro_dollars, 1_050_000);
        assert_eq!(
            attempt_asset(&fixture.world, &attempt).await,
            Some(fixture.command.binding.asset.as_bytes().to_vec()),
            "the asset id rides the same admission event",
        );
        assert_eq!(port.phases(), vec![ImageStagePhase::New]);
        let after = counts(&fixture.world).await;
        assert_eq!(after.0, before.0 + 1, "one journal entry");
        assert_eq!(after.1, before.1 + 1, "one attempt row");
        assert_eq!(after.2, before.2 + 1, "one receipt");
        assert_eq!(after.3, before.3 + 1, "one link row");
        fixture.world.finish().await;
    });
}

#[test]
fn an_ordinary_admission_leaves_media_asset_empty() {
    support::run(async {
        let Some((mut fixture, _)) = fixture(2).await else {
            return;
        };
        let outcome = fixture
            .service
            .admit(&fixture.command.admit)
            .await
            .expect("admitted");
        assert!(matches!(outcome, AdmissionOutcome::Admitted { .. }));
        assert_eq!(
            attempt_asset(&fixture.world, &fixture.command.admit.attempt).await,
            None,
            "only the image stage stamps the asset",
        );
        fixture.world.finish().await;
    });
}

#[test]
fn a_replay_verifies_the_identical_link_and_posts_nothing_new() {
    support::run(async {
        let Some((mut fixture, port)) = fixture(3).await else {
            return;
        };
        let first = fixture
            .service
            .admit_image_stage(&fixture.command, &port)
            .await
            .expect("admitted");
        let settled = counts(&fixture.world).await;
        let repeat = fixture
            .service
            .admit_image_stage(&fixture.command, &port)
            .await
            .expect("replayed");
        let AdmissionOutcome::Admitted {
            reservation: first_reservation,
            attempt: first_attempt,
            retail_maximum_micro_dollars: first_maximum,
            ..
        } = first
        else {
            panic!("expected the first admission, got {first:?}");
        };
        let AdmissionOutcome::AlreadyAdmitted {
            reservation,
            attempt,
            retail_maximum_micro_dollars,
            ..
        } = repeat
        else {
            panic!("expected the stored receipt, got {repeat:?}");
        };
        assert_eq!(reservation, first_reservation);
        assert_eq!(attempt, first_attempt);
        assert_eq!(retail_maximum_micro_dollars, first_maximum);
        assert_eq!(counts(&fixture.world).await, settled, "a replay posts nothing");
        assert_eq!(
            port.phases(),
            vec![ImageStagePhase::New, ImageStagePhase::Replay],
            "the replay verifies instead of admitting",
        );
        fixture.world.finish().await;
    });
}

#[test]
fn a_replay_with_a_missing_link_fails_instead_of_repairing() {
    support::run(async {
        let Some((mut fixture, port)) = fixture(4).await else {
            return;
        };
        fixture
            .service
            .admit_image_stage(&fixture.command, &port)
            .await
            .expect("admitted");
        let settled = counts(&fixture.world).await;
        fixture
            .world
            .admin
            .execute("DELETE FROM image_stage_link", &[])
            .await
            .expect("drop the link");
        let error = fixture
            .service
            .admit_image_stage(&fixture.command, &port)
            .await
            .expect_err("no link, no send permission");
        assert!(
            matches!(error, FinanceError::ImageStageMissingLink),
            "{error:?}"
        );
        assert_eq!(counts(&fixture.world).await, settled, "nothing repaired");
        fixture.world.finish().await;
    });
}

#[test]
fn a_replay_with_a_tampered_link_conflicts() {
    support::run(async {
        let Some((mut fixture, port)) = fixture(5).await else {
            return;
        };
        fixture
            .service
            .admit_image_stage(&fixture.command, &port)
            .await
            .expect("admitted");
        fixture
            .world
            .admin
            .execute("UPDATE image_stage_link SET asset_id = $1", &[&vec![9_u8; 16]])
            .await
            .expect("tamper");
        let error = fixture
            .service
            .admit_image_stage(&fixture.command, &port)
            .await
            .expect_err("conflict");
        assert!(matches!(error, FinanceError::ImageStageConflict), "{error:?}");
        fixture.world.finish().await;
    });
}

#[test]
fn each_catalog_refusal_rolls_back_hold_receipt_and_link() {
    support::run(async {
        let Some((mut fixture, port)) = fixture(6).await else {
            return;
        };
        let baseline = counts(&fixture.world).await;
        for refusal in [
            StageRefusal::InFlight,
            StageRefusal::Ready,
            StageRefusal::AudienceEmpty,
            StageRefusal::QuotaFull,
            StageRefusal::Other {
                reason: "catalog paused".into(),
            },
        ] {
            port.set(Behavior::Refuse(refusal.clone()));
            let error = fixture
                .service
                .admit_image_stage(&fixture.command, &port)
                .await
                .expect_err("refused");
            assert!(
                matches!(error, FinanceError::ImageStageRefusal(_)),
                "{error:?}"
            );
            assert_eq!(
                counts(&fixture.world).await,
                baseline,
                "a refusal holds nothing and writes nothing"
            );
        }
        fixture.world.finish().await;
    });
}

#[test]
fn a_hung_link_port_times_out_and_writes_nothing() {
    support::run(async {
        let Some((mut fixture, port)) = fixture_with(
            7,
            Some(FinanceConfig {
                port_deadline_ms: 50,
                ..FinanceConfig::default()
            }),
        )
        .await
        else {
            return;
        };
        let baseline = counts(&fixture.world).await;
        port.set(Behavior::Hang);
        let error = fixture
            .service
            .admit_image_stage(&fixture.command, &port)
            .await
            .expect_err("timeout");
        assert!(matches!(error, FinanceError::PortUnavailable), "{error:?}");
        assert_eq!(
            counts(&fixture.world).await,
            baseline,
            "the timeout rolls the admission back"
        );
        fixture.world.finish().await;
    });
}

#[test]
fn a_broken_link_table_keeps_its_sqlstate() {
    support::run(async {
        let Some((mut fixture, port)) = fixture(8).await else {
            return;
        };
        let baseline = counts(&fixture.world).await;
        port.set(Behavior::BrokenSql);
        let error = fixture
            .service
            .admit_image_stage(&fixture.command, &port)
            .await
            .expect_err("database error");
        let FinanceError::ImageStageUnavailable { sqlstate, detail } = error else {
            panic!("expected unavailable evidence, got {error:?}");
        };
        assert_eq!(sqlstate.as_deref(), Some("42P01"));
        assert!(!detail.is_empty());
        assert_eq!(counts(&fixture.world).await, baseline);
        fixture.world.finish().await;
    });
}

#[test]
fn an_uncertain_link_rolls_back() {
    support::run(async {
        let Some((mut fixture, port)) = fixture(9).await else {
            return;
        };
        let baseline = counts(&fixture.world).await;
        port.set(Behavior::Uncertain);
        let error = fixture
            .service
            .admit_image_stage(&fixture.command, &port)
            .await
            .expect_err("uncertain");
        assert!(
            matches!(error, FinanceError::ImageStageUncertain { .. }),
            "{error:?}"
        );
        assert_eq!(counts(&fixture.world).await, baseline);
        fixture.world.finish().await;
    });
}

#[test]
fn changed_fields_conflict_on_the_same_operation() {
    support::run(async {
        let Some((mut fixture, port)) = fixture(10).await else {
            return;
        };
        fixture
            .service
            .admit_image_stage(&fixture.command, &port)
            .await
            .expect("admitted");
        let settled = counts(&fixture.world).await;
        let mut variants: Vec<(&str, ImageStageAdmission)> = Vec::new();
        let mut job = fixture.command.clone();
        job.binding.job = ImageJobId::new([0x99; 16]).expect("job");
        variants.push(("job", job));
        let mut stage = fixture.command.clone();
        stage.binding.stage = ImageStageId::new([0x99; 16]).expect("stage");
        variants.push(("stage", stage));
        let mut asset = fixture.command.clone();
        asset.binding.asset = ImageAssetId::new([0x99; 16]).expect("asset");
        variants.push(("asset", asset));
        let mut publication = fixture.command.clone();
        publication.binding.publication_attempt =
            PublicationAttemptId::new([0x99; 16]).expect("publication");
        variants.push(("publication attempt", publication));
        let mut owner = fixture.command.clone();
        owner.binding.owner_scope = OwnerScope::new(b"another-owner".to_vec()).expect("scope");
        variants.push(("owner", owner));
        let mut payload = fixture.command.clone();
        payload.admit.request_digest = [9; 32];
        variants.push(("payload digest", payload));
        let mut attempt = fixture.command.clone();
        attempt.binding.attempt = AttemptId::from_array([0x77; 16]);
        attempt.admit.attempt = AttemptId::from_array([0x77; 16]);
        variants.push(("attempt", attempt));
        let mut effect = fixture.command.clone();
        effect.binding.effect = EffectId::from_array([0x88; 16]);
        effect.admit.effect = EffectId::from_array([0x88; 16]);
        variants.push(("effect", effect));
        for (name, command) in variants {
            let error = fixture
                .service
                .admit_image_stage(&command, &port)
                .await
                .expect_err("conflict");
            assert!(
                matches!(error, FinanceError::Admission(AdmissionRefusal::Conflict)),
                "{name}: {error:?}"
            );
        }
        assert_eq!(counts(&fixture.world).await, settled, "conflicts write nothing");
        fixture.world.finish().await;
    });
}

#[test]
fn mismatched_binding_ids_fail_preflight() {
    use df_commerce::finance_ids::{CampaignKey, OperationKey, SessionKey, TenantKey};
    support::run(async {
        let Some((mut fixture, port)) = fixture(11).await else {
            return;
        };
        let baseline = counts(&fixture.world).await;
        let mut command = fixture.command.clone();
        command.binding.tenant = TenantKey::from_array([9; 16]);
        assert!(
            matches!(
                fixture.service.admit_image_stage(&command, &port).await,
                Err(FinanceError::ImageStageBinding(BindingRefusal::TenantMismatch))
            ),
            "tenant must bind the admission"
        );
        let mut command = fixture.command.clone();
        command.binding.campaign = CampaignKey::from_array([9; 16]);
        assert!(
            matches!(
                fixture.service.admit_image_stage(&command, &port).await,
                Err(FinanceError::ImageStageBinding(BindingRefusal::CampaignMismatch))
            ),
            "campaign must bind the admission"
        );
        let mut command = fixture.command.clone();
        command.binding.session = SessionKey::from_array([9; 16]);
        assert!(
            matches!(
                fixture.service.admit_image_stage(&command, &port).await,
                Err(FinanceError::ImageStageBinding(BindingRefusal::SessionMismatch))
            ),
            "session must bind the admission"
        );
        let mut command = fixture.command.clone();
        command.binding.operation = OperationKey::from_array([9; 16]);
        assert!(
            matches!(
                fixture.service.admit_image_stage(&command, &port).await,
                Err(FinanceError::ImageStageBinding(BindingRefusal::OperationMismatch))
            ),
            "operation must bind the admission"
        );
        let mut command = fixture.command.clone();
        command.binding.attempt = AttemptId::from_array([9; 16]);
        assert!(
            matches!(
                fixture.service.admit_image_stage(&command, &port).await,
                Err(FinanceError::ImageStageBinding(BindingRefusal::AttemptMismatch))
            ),
            "attempt must bind the admission"
        );
        let mut command = fixture.command.clone();
        command.binding.effect = EffectId::from_array([9; 16]);
        assert!(
            matches!(
                fixture.service.admit_image_stage(&command, &port).await,
                Err(FinanceError::ImageStageBinding(BindingRefusal::EffectMismatch))
            ),
            "effect must bind the admission"
        );
        assert_eq!(counts(&fixture.world).await, baseline, "preflight writes nothing");
        fixture.world.finish().await;
    });
}

#[test]
fn a_legacy_media_request_is_refused_never_merged() {
    support::run(async {
        let Some((mut fixture, port)) = fixture(12).await else {
            return;
        };
        let baseline = counts(&fixture.world).await;
        let mut command = fixture.command.clone();
        command.admit.media = Some(MediaRequest {
            attempt: command.admit.attempt,
            asset: [5; 16],
        });
        let error = fixture
            .service
            .admit_image_stage(&command, &port)
            .await
            .expect_err("legacy media refused");
        assert!(
            matches!(
                error,
                FinanceError::ImageStageBinding(BindingRefusal::LegacyMediaPresent)
            ),
            "{error:?}"
        );
        assert_eq!(counts(&fixture.world).await, baseline, "nothing merged");
        fixture.world.finish().await;
    });
}

#[test]
fn a_quoted_image_stage_admits_with_the_same_digest() {
    support::run(async {
        let Some((mut fixture, port)) = fixture(13).await else {
            return;
        };
        let full = image_stage_quote_digest(&fixture.command.binding, &fixture.payload);
        let quote = fixture
            .service
            .create_quote(
                &fixture.world.version,
                QuoteCommand {
                    session: fixture.world.session,
                    route: route(),
                    work_class: WorkClass::Essential,
                    maximum_units: fixture.command.admit.maximum_units.clone(),
                    estimate_units: fixture.command.admit.estimate_units.clone(),
                    request_digest: full,
                    lifetime_ms: 3_600_000,
                },
            )
            .await
            .expect("quote");
        fixture
            .service
            .decide_quote(fixture.world.session, quote, true)
            .await
            .expect("approved");
        fixture.command.admit.quote = Some(quote);
        let outcome = fixture
            .service
            .admit_image_stage(&fixture.command, &port)
            .await
            .expect("admitted against the quote");
        assert!(matches!(outcome, AdmissionOutcome::Admitted { .. }));
        fixture.world.finish().await;
    });
}

#[test]
fn the_same_operation_resolves_through_replay() {
    support::run(async {
        let Some((mut fixture, port)) = fixture(14).await else {
            return;
        };
        let first = fixture
            .service
            .admit_image_stage(&fixture.command, &port)
            .await
            .expect("admitted");
        let repeat = fixture
            .service
            .admit_image_stage(&fixture.command, &port)
            .await
            .expect("replay resolves the operation");
        let AdmissionOutcome::Admitted {
            reservation: first_reservation,
            ..
        } = first
        else {
            panic!("expected the first admission, got {first:?}");
        };
        let AdmissionOutcome::AlreadyAdmitted { reservation, .. } = repeat else {
            panic!("expected the stored outcome, got {repeat:?}");
        };
        assert_eq!(reservation, first_reservation);
        fixture.world.finish().await;
    });
}
