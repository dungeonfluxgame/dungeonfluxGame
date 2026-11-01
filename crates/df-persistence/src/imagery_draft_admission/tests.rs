#[path = "../../../df-finance/tests/support/mod.rs"]
pub(crate) mod support;

use df_assets::{
    Admission, AssetKind, Audience, DedupeScope, MediaRequestKey, Provenance, QuotaScope,
    RetentionClass,
};
use df_commerce::draft_image::{
    AssetId, Binding, Consent, ConsentId, DraftId, JobId, PublicationId, Scope, StageId,
};
use df_commerce::finance_ids::QuoteId;
use df_finance::draft_image::{AdmissionCommand, Error, QuoteCommand};
use df_finance::fakes::{FakeFence, FakeMedia, FakeMembers, SeededIds, SystemClock};
use df_finance::ports::{Clock, FenceVerdict, MediaVerdict};
use df_finance::service::{FinanceConfig, FinanceService};
use df_imagery::{
    AcceptCommand, AcceptTarget, FreezeCommand, FreezeOutcome, ImageInterestId, ImageJobId,
    ImageJournal, ImageOwner, OperationIdentity, SharingIdentity, VersionedSnapshot,
};
use df_types::{AccountId, OperationId};
use tokio_postgres::Transaction;

use super::{Config, DraftAdmissionPort, DraftAuthority};
use support::world::{World, payer, purchase};

struct Authority {
    refuse: bool,
}
impl DraftAuthority for Authority {
    async fn verify(
        &self,
        _tx: &Transaction<'_>,
        _binding: &Binding,
        _replay: bool,
    ) -> Result<(), Error> {
        if self.refuse {
            Err(Error::LinkConflict)
        } else {
            Ok(())
        }
    }
}

fn account() -> AccountId {
    AccountId::from_bytes(payer(1).as_bytes()).unwrap()
}
fn snapshot(byte: u8) -> VersionedSnapshot {
    VersionedSnapshot::new(1, vec![byte; 16], 1_024).unwrap()
}

async fn world() -> Option<World> {
    if std::env::var_os("DUNGEONFLUX_FINANCE_PG_PORT").is_none() {
        assert!(
            std::env::var_os("DUNGEONFLUX_REQUIRE_SCRATCH_PG").is_none(),
            "required PG missing"
        );
        return None;
    }
    assert_eq!(
        std::env::var("DUNGEONFLUX_FINANCE_PG_PORT").unwrap(),
        "55471"
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
    let transport = crate::postgres_tls::PostgresTransport::PlaintextLoopbackOnly;
    crate::media_catalog::PostgresMediaCatalog::open(world.scratch.config(), transport.clone(), 2)
        .await
        .unwrap();
    crate::PostgresImageJournal::open(world.scratch.config(), transport.clone(), 2)
        .await
        .unwrap();
    world
        .admin
        .batch_execute(include_str!(
            "../../migrations/0039_imagery_admission_bindings.sql"
        ))
        .await
        .unwrap();
    world
        .admin
        .batch_execute(crate::imagery_execution::IMAGERY_EXECUTION_MIGRATION_SQL)
        .await
        .unwrap();
    world
        .admin
        .batch_execute(include_str!(
            "../../migrations/0042_imagery_publication.sql"
        ))
        .await
        .unwrap();
    world
        .admin
        .batch_execute(super::MIGRATION_SQL)
        .await
        .unwrap();
    world
        .service()
        .await
        .apply_purchase(purchase(payer(1), "account-wallet", 1_000))
        .await
        .unwrap();
    Some(world)
}

async fn run_actual_admission(refuse_authority: bool) {
    let Some(world) = world().await else { return };
    let now = SystemClock.now_ms();
    let deadline = now + 60_000;
    let config = snapshot(4);
    let recipe = snapshot(5);
    let job = ImageJobId::from_bytes(&[2; 16]).unwrap();
    let key = MediaRequestKey::from_bytes([6; 32]);
    let owner = ImageOwner::Draft { account: account() };
    let journal = crate::PostgresImageJournal::open(
        world.scratch.config(),
        crate::postgres_tls::PostgresTransport::PlaintextLoopbackOnly,
        2,
    )
    .await
    .unwrap();
    let accepted = journal
        .accept(&AcceptCommand {
            identity: OperationIdentity {
                owner: owner.clone(),
                operation: OperationId::from_bytes(&[7; 16]).unwrap(),
                fingerprint: [8; 32],
            },
            intent: snapshot(3),
            config: config.clone(),
            sharing: SharingIdentity::from_bytes(&[9; 32]).unwrap(),
            interest: ImageInterestId::from_bytes(&[10; 16]).unwrap(),
            target: AcceptTarget::Create {
                job,
                absolute_deadline_ms: deadline,
            },
            now_ms: now,
        })
        .await
        .unwrap();
    let FreezeOutcome::Frozen { job: frozen } = journal
        .freeze_recipe(&FreezeCommand {
            owner: owner.clone(),
            job,
            expected_revision: accepted.view().job.revision,
            key,
            recipe: recipe.clone(),
            now_ms: now,
        })
        .await
        .unwrap()
    else {
        panic!("first freeze")
    };
    let binding = Binding {
        scope: Scope {
            account: account(),
            draft: DraftId([1; 16]),
            draft_revision: 1,
            job: JobId([2; 16]),
            source_digest: [3; 32],
            config_digest: df_commerce::digest::sha256(config.bytes()),
            provider: "openai".into(),
            model: "gpt-6-luna".into(),
            route: "portrait".into(),
            price_version: [0x70; 16],
            maximum_units: vec![("text_input".into(), 100), ("text_output".into(), 100)],
        },
        stage: StageId([11; 16]),
        asset: AssetId([12; 16]),
        publication: PublicationId([13; 16]),
        operation: [14; 16],
        attempt: [15; 16],
        effect: [16; 16],
        payload_digest: [17; 32],
    };
    let admission = Admission {
        asset: df_assets::AssetId::from_bytes(&binding.asset.0).unwrap(),
        attempt: df_assets::AttemptId::from_bytes(&binding.publication.0).unwrap(),
        kind: AssetKind::Image,
        provenance: Provenance::Generated,
        charge_scope: QuotaScope::Account(account()),
        origin_account: Some(account()),
        campaign: None,
        audience: Audience::OwnerPrivate(account()),
        dedupe_scope: DedupeScope::account(&account()),
        request_key: Some(key),
        retention: RetentionClass::Personal,
        reserve_bytes: 100,
        byte_limit: 1_000,
        in_flight_upload_limit: None,
    };
    let port = DraftAdmissionPort::new(
        Authority {
            refuse: refuse_authority,
        },
        Config {
            admission,
            recipe_key: key,
            recipe,
            config,
            expected_job_revision: frozen.revision,
            absolute_deadline_ms: deadline,
            now_ms: now,
        },
    )
    .unwrap();
    let client = world.scratch.connect().await;
    client
        .batch_execute("SET ROLE df_imagery_admission")
        .await
        .unwrap();
    let mut service = FinanceService::new(
        client,
        FakeFence::new(FenceVerdict::Current),
        FakeMedia::new(MediaVerdict::Admitted),
        FakeMembers::new(vec![]),
        Box::new(SystemClock),
        Box::new(SeededIds::new(80_000)),
        FinanceConfig::default(),
    );
    let consent = Consent {
        id: ConsentId([18; 16]),
        scope: binding.scope.clone(),
        cap_micro_dollars: 100,
        expires_at_ms: deadline,
        revision: 1,
        revoked: false,
    };
    service
        .record_draft_image_consent(account(), &consent)
        .await
        .unwrap();
    let command = AdmissionCommand {
        consent: consent.id,
        consent_revision: 1,
        quote: QuoteId::from_array([19; 16]),
        binding,
    };
    service
        .quote_draft_image(
            account(),
            &QuoteCommand {
                admission: command.clone(),
                expires_at_ms: deadline - 1,
            },
        )
        .await
        .unwrap();
    let first = service.admit_draft_image(account(), &command, &port).await;
    if refuse_authority {
        assert!(matches!(first, Err(Error::LinkConflict)));
        assert_eq!(world.natural(2110, Some(payer(1)), None, None).await, 1_000);
        let n: i64 = world
            .admin
            .query_one("SELECT count(*) FROM media.asset", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(n, 0);
    } else {
        let first = first.unwrap();
        assert!(!first.replay);
        assert!(
            service
                .admit_draft_image(account(), &command, &port)
                .await
                .unwrap()
                .replay
        );
        let row=world.admin.query_one("SELECT count(*) FROM imagery.image_publication_allocation a \
            JOIN imagery.image_finance_admission_binding f USING(publication_attempt,job_id,stage_id,asset_id,owner_scope) \
            WHERE f.account_id=$1 AND f.campaign_id IS NULL AND f.session_id IS NULL",
            &[&account().as_bytes().as_slice()]).await.unwrap();
        assert_eq!(row.get::<_, i64>(0), 1);
        assert_eq!(world.natural(2150, Some(payer(1)), None, None).await, 90);
    }
    world.finish().await;
}

#[test]
fn private_account_catalog_and_common_allocation_commit_with_finance_and_replay_read_only() {
    support::run(run_actual_admission(false));
}

#[test]
fn canonical_draft_authority_refusal_rolls_back_catalog_and_finance() {
    support::run(run_actual_admission(true));
}

#[path = "stage_tests.rs"]
pub(crate) mod stage_tests;
