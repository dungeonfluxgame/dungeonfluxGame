//! Real PostgreSQL publication tests; explicit scratch flag required, no skipped success.
use df_assets::*;
use df_imagery::execution::{ClaimToken as ExecutionClaim, DispatchId, ExecutionStageId};
use df_imagery::publication::{FrozenOutput, PublicationError, recipe_digest};
use df_imagery::{ImageJobId, ImageOwner, owner_scope_bytes};
use df_persistence::imagery_publication::*;
use df_types::{AccountId, CampaignId, RunId};
use sha2::{Digest, Sha256};
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio_postgres::{Client, Config, NoTls};

#[path = "imagery_publication/artifact_read_tests.rs"]
mod artifact_read_tests;
#[path = "imagery_publication/intent_fixture.rs"]
mod intent_fixture;
#[path = "imagery_publication/reusable_tests.rs"]
mod reusable_tests;
#[path = "imagery_publication/selection_fixture.rs"]
mod selection_fixture;
#[path = "imagery_publication/source_denial_tests.rs"]
mod source_denial_tests;
#[path = "imagery_publication/source_fixture.rs"]
mod source_fixture;
#[path = "imagery_publication/terminal_fixture.rs"]
mod terminal_fixture;
#[path = "imagery_publication/terminal_tests.rs"]
mod terminal_tests;

const NOW: i64 = 1_700_000_000_000;
const MASTER: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 224, 18, 145, 251, 15, 0, 1,
    164, 1, 60, 147, 139, 14, 183, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];
const DISPLAY: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 244, 34, 127, 138, 0, 0, 0, 14, 73, 68, 65, 84, 120, 156, 99, 224, 18, 145, 99, 0, 97, 0,
    2, 57, 0, 121, 107, 35, 42, 216, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

struct Connection {
    client: Client,
    task: tokio::task::JoinHandle<()>,
}
impl Deref for Connection {
    type Target = Client;
    fn deref(&self) -> &Client {
        &self.client
    }
}
impl DerefMut for Connection {
    fn deref_mut(&mut self) -> &mut Client {
        &mut self.client
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn connect(config: &Config) -> Connection {
    let (client, connection) = config.connect(NoTls).await.expect("owned PG55471");
    let task = tokio::spawn(async move {
        // A dropped database ends its own connection with an administrator error; only the test sees it.
        if let Err(error) = connection.await {
            eprintln!("PG connection closed: {error}");
        }
    });
    Connection { client, task }
}
struct Scratch {
    config: Config,
    admin: Config,
}
impl Scratch {
    async fn open() -> Self {
        assert_eq!(
            std::env::var("DUNGEONFLUX_REQUIRE_SCRATCH_PG").as_deref(),
            Ok("1")
        );
        let mut admin = Config::new();
        admin
            .host("127.0.0.1")
            .port(55471)
            .user("postgres")
            .dbname("postgres");
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let database = format!(
            "df_publication_{}_{}_{}",
            std::process::id(),
            unique,
            SERIAL.fetch_add(1, Ordering::SeqCst)
        );
        let migration_owner = connect(&admin).await;
        migration_owner
            .batch_execute("SELECT pg_advisory_lock(hashtext('df_imagery_test_role_migrations'))")
            .await
            .unwrap();
        connect(&admin)
            .await
            .batch_execute(&format!("CREATE DATABASE {database}"))
            .await
            .unwrap();
        let mut config = admin.clone();
        config.dbname(&database);
        let client = connect(&config).await;
        let mut client = client;
        let transaction = client.transaction().await.unwrap();
        df_persistence::imagery_migrations::migrate_local_in(&transaction)
            .await
            .unwrap();
        transaction.commit().await.unwrap();
        migration_owner
            .batch_execute("SELECT pg_advisory_unlock(hashtext('df_imagery_test_role_migrations'))")
            .await
            .unwrap();
        Self { config, admin }
    }
    async fn finish(self) {
        let database = self.config.get_dbname().unwrap();
        connect(&self.admin)
            .await
            .batch_execute(&format!("DROP DATABASE {database} WITH (FORCE)"))
            .await
            .unwrap();
    }
}

struct Fixture {
    owner: ImageOwner,
    held: HeldStep,
    proofs: Vec<ImageProof>,
    finalize: FinalizeRequest,
    root: std::path::PathBuf,
}
impl Fixture {
    fn command(&self) -> PublicationCommand<'_> {
        PublicationCommand {
            owner: &self.owner,
            job: ImageJobId::from_bytes(&[1; 16]).unwrap(),
            stage: ExecutionStageId::from_bytes(&[2; 16]).unwrap(),
            dispatch: DispatchId::from_bytes(&[3; 16]).unwrap(),
            claim: ExecutionClaim::from_bytes(&[4; 16]).unwrap(),
            claim_generation: 1,
            held: self.held,
            proofs: &self.proofs,
            finalize: &self.finalize,
            now_ms: NOW + 10,
            recovery: None,
            source_authority: None,
            qualified: None,
        }
    }
    async fn create(client: &Client, multi: bool, local: bool) -> Self {
        Self::create_with_pipeline(client, multi, local, None).await
    }
    async fn create_with_pipeline(
        client: &Client,
        multi: bool,
        local: bool,
        pipeline: Option<df_imagery::ImagePipeline>,
    ) -> Self {
        Self::create_with_inputs(client, multi, local, pipeline, None).await
    }
    async fn create_with_artifact(
        client: &Client,
        intent: df_imagery::VersionedSnapshot,
        config: df_imagery::VersionedSnapshot,
        recipe: df_imagery::CampaignArtifactRecipe,
    ) -> Self {
        Self::create_with_inputs(client, false, true, None, Some((intent, config, recipe))).await
    }
    async fn create_with_inputs(
        client: &Client,
        multi: bool,
        local: bool,
        pipeline: Option<df_imagery::ImagePipeline>,
        artifact: Option<(
            df_imagery::VersionedSnapshot,
            df_imagery::VersionedSnapshot,
            df_imagery::CampaignArtifactRecipe,
        )>,
    ) -> Self {
        let campaign = CampaignId::from_bytes(&[10; 16]).unwrap();
        let account = AccountId::from_bytes(&[11; 16]).unwrap();
        let owner = ImageOwner::Run {
            campaign,
            run: RunId::from_bytes(&[12; 16]).unwrap(),
        };
        let asset = AssetId::from_bytes(&[5; 16]).unwrap();
        let attempt = AttemptId::from_bytes(&[6; 16]).unwrap();
        let token = ClaimToken::from_bytes(&[7; 16]).unwrap();
        let master = ImageIntent::new("image/png", 1, 1, ImageAlpha::Opaque).unwrap();
        let display = ImageIntent::new("image/png", 2, 1, ImageAlpha::Transparent).unwrap();
        let recipe = FrozenOutput::new(
            master,
            multi.then_some(display),
            b"canonical execution inputs config model profile transforms revisions".to_vec(),
        )
        .unwrap()
        .snapshot()
        .unwrap();
        let digest = recipe_digest(&recipe);
        let scope = owner_scope_bytes(&owner);
        let key = artifact
            .as_ref()
            .map_or([13; 32], |(_, _, recipe)| *recipe.key.as_bytes());
        let semantic_revision = artifact
            .as_ref()
            .map(|(_, _, recipe)| i64::from(recipe.semantic.revision()));
        let semantic_bytes = artifact
            .as_ref()
            .map(|(_, _, recipe)| recipe.semantic.bytes().to_vec());
        let (intent, config) = if let Some((intent, config, _)) = artifact {
            (intent, config)
        } else if let Some(pipeline) = pipeline {
            intent_fixture::pipeline_snapshots(&owner, pipeline).await
        } else {
            intent_fixture::snapshots(&owner, None).await
        };
        client.execute("INSERT INTO imagery.image_job(job_id,owner_scope,sharing_identity,intent_revision,intent_bytes,config_revision,config_bytes,absolute_deadline_ms,state,revision,resolved_recipe_key,recipe_revision,recipe_bytes,created_at_ms,updated_at_ms,planned_publication_asset,planned_publication_attempt,planned_output_stage) VALUES($1,$2,$3,$11,$4,$12,$13,$5,'recipe_frozen',2,$14,1,$6,$7,$7,$8,$9,$10)", &[&[1u8;16].as_slice(), &scope, &[13u8;32].as_slice(), &intent.bytes(), &(NOW+1000), &recipe.bytes(), &NOW, &asset.as_bytes().as_slice(), &attempt.as_bytes().as_slice(), &[2u8;16].as_slice(),&i64::from(intent.revision()),&i64::from(config.revision()),&config.bytes(),&key.as_slice()]).await.unwrap();
        client.execute("INSERT INTO media.quota_usage(scope_kind,scope_id,reserved_bytes) VALUES('campaign',$1,1000)", &[&campaign.as_bytes().as_slice()]).await.unwrap();
        client.execute("INSERT INTO media.asset(asset_id,kind,provenance,charge_scope_kind,charge_scope_id,origin_account,campaign_id,audience_kind,audience_campaign,dedupe_scope,request_key,state,attempt,step,step_owner,step_lease_until_ms,retention_class,created_at_ms,updated_at_ms) VALUES($1,'image','generated','campaign',$2,$3,$2,'campaign_shared',$2,$2,$8,'validated',$4,'publish',$5,$6,'campaign_media',$7,$7)", &[&asset.as_bytes().as_slice(), &campaign.as_bytes().as_slice(), &account.as_bytes().as_slice(), &attempt.as_bytes().as_slice(), &token.as_bytes().as_slice(), &(NOW+1000), &NOW,&key.as_slice()]).await.unwrap();
        client.execute("INSERT INTO media.quota_reservation(asset_id,scope_kind,scope_id,reserved_bytes,counts_upload) VALUES($1,'campaign',$2,1000,false)", &[&asset.as_bytes().as_slice(), &campaign.as_bytes().as_slice()]).await.unwrap();
        client.execute("INSERT INTO imagery.image_publication_allocation(publication_attempt,asset_id,job_id,stage_id,owner_scope,original_recipe_key,original_recipe_revision,original_recipe_bytes,original_config_revision,original_config_bytes,created_at_ms,semantic_recipe_revision,semantic_recipe_bytes) VALUES($1,$2,$3,$4,$5,$6,1,$7,$10,$8,$9,$11,$12)", &[&attempt.as_bytes().as_slice(), &asset.as_bytes().as_slice(), &[1u8;16].as_slice(), &[2u8;16].as_slice(), &scope, &key.as_slice(), &recipe.bytes(), &config.bytes(), &NOW,&i64::from(config.revision()),&semantic_revision,&semantic_bytes]).await.unwrap();
        let config_digest: [u8; 32] = Sha256::digest(config.bytes()).into();
        client.execute("INSERT INTO imagery.image_execution_stage(job_id,stage_id,owner_scope,stage_kind,stage_state,claim_token,claim_generation,lease_until_ms,input_revision,input_digest,config_revision,config_digest,capacity_scope,local_accounting,created_at_ms,updated_at_ms) VALUES($1,$2,$3,'generation','admitted',$4,1,$5,1,$6,1,$9,'test',$7,$8,$8)", &[&[1u8;16].as_slice(), &[2u8;16].as_slice(), &scope, &[4u8;16].as_slice(), &(NOW+1000), &[14u8;32].as_slice(), &local, &NOW,&config_digest.as_slice()]).await.unwrap();
        client.execute("INSERT INTO imagery.image_dispatch_attempt(dispatch_id,job_id,stage_id,dispatch_seq,dispatch_state,usage_revision,book_call_id,claim_token,claim_generation,claim_lease_until_ms,local_accounting,created_at_ms,updated_at_ms) VALUES($1,$2,$3,0,'succeeded',1,$4,$5,1,$8,$6,$7,$7)", &[&[3u8;16].as_slice(), &[1u8;16].as_slice(), &[2u8;16].as_slice(), &format!("img-{}",DispatchId::from_bytes(&[3;16]).unwrap().to_hex()), &[4u8;16].as_slice(), &local, &NOW,&(NOW+1000)]).await.unwrap();
        client.execute("INSERT INTO imagery.image_stage_cursor(job_id,stage_id,current_dispatch,updated_at_ms) VALUES($1,$2,$3,$4)", &[&[1u8;16].as_slice(), &[2u8;16].as_slice(), &[3u8;16].as_slice(), &NOW]).await.unwrap();
        client.execute("INSERT INTO imagery.image_dispatch_sources(dispatch_id,owner_scope,job_id,stage_id,receipts,source_digest) VALUES($1,$2,$3,$4,'[]'::jsonb,$5)", &[&[3u8;16].as_slice(),&scope,&[1u8;16].as_slice(),&[2u8;16].as_slice(),&df_imagery::runtime::source_receipts_digest(&[]).unwrap().as_slice()]).await.unwrap();
        let root = std::env::current_dir()
            .unwrap()
            .join("../../artifacts/tmp/atomic-publication")
            .join(format!(
                "{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        let store = NativeFileStore::new(&root, 1024 * 1024).unwrap();
        let layout = KeyLayout::new(ObjectEnvironment::new("test").unwrap());
        let deadline = Deadline::after(Duration::from_secs(30));
        let mut proofs = Vec::new();
        for (variant, bytes, intent, write) in [
            (AssetVariant::Master, MASTER, master, 8),
            (AssetVariant::Display, DISPLAY, display, 9),
        ]
        .into_iter()
        .take(if multi { 2 } else { 1 })
        {
            let write = WriteId::from_bytes(&[write; 16]).unwrap();
            let key = layout.campaign_asset(&campaign, asset, attempt, write, variant);
            let manifest = AssetManifest {
                byte_len: bytes.len().try_into().unwrap(),
                sha256: Sha256::digest(bytes).into(),
            };
            let stored = store
                .put_new(&key, bytes, manifest, deadline)
                .await
                .unwrap();
            let record = VariantRecord {
                asset,
                attempt,
                write,
                variant,
                owner: token,
                state: VariantState::Written,
                key,
                version: Some(stored.version.clone()),
                manifest: Some(manifest),
                created_at_ms: NOW,
            };
            client.execute("INSERT INTO media.asset_variant(asset_id,attempt,write_id,variant,owner,state,object_domain,object_key,object_version,byte_length,sha256,created_at_ms) VALUES($1,$2,$3,$4,$5,'written','private_media',$6,$7,$8,$9,$10)", &[&asset.as_bytes().as_slice(),&attempt.as_bytes().as_slice(),&write.as_bytes().as_slice(),&variant.label(),&token.as_bytes().as_slice(),&record.key.path(),&stored.version.as_str(),&i64::try_from(manifest.byte_len).unwrap(),&manifest.sha256.as_slice(),&NOW]).await.unwrap();
            let reopened = NativeFileStore::new(&root, 1024 * 1024).unwrap();
            let durable_bytes = reopened.get_version(&stored, deadline).await.unwrap();
            proofs.push(
                ImageProof::decode(
                    &record,
                    &durable_bytes,
                    digest,
                    intent,
                    ImageProofLimits::default(),
                )
                .unwrap(),
            );
        }
        let record = proofs[0].record();
        let manifest = record.manifest.unwrap();
        client.execute("INSERT INTO imagery.image_dispatch_checkpoint(dispatch_id,storage_key,object_version,byte_len,sha256,recipe_digest,mime,width_px,height_px,alpha,usage_revision,book_call_id,recorded_at_ms) VALUES($1,$2,$3,$4,$5,$6,'image/png',1,1,'opaque',1,$7,$8)", &[&[3u8;16].as_slice(),&record.key.path(),&record.version.as_ref().unwrap().as_str(),&i64::try_from(manifest.byte_len).unwrap(),&manifest.sha256.as_slice(),&digest.as_slice(),&format!("img-{}",DispatchId::from_bytes(&[3;16]).unwrap().to_hex()),&NOW]).await.unwrap();
        let finalize = FinalizeRequest {
            winners: proofs
                .iter()
                .map(|proof| Winner {
                    variant: proof.variant(),
                    write: proof.record().write,
                })
                .collect(),
            mime: "image/png".into(),
            width_px: Some(1),
            height_px: Some(1),
            byte_limit: 10000,
        };
        Self {
            owner,
            held: HeldStep {
                asset,
                attempt,
                owner: token,
            },
            proofs,
            finalize,
            root,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).expect("owned fixture cleanup");
    }
}

async fn assert_unpublished(client: &Client) {
    assert_eq!(client.query_one("SELECT media.asset.state, imagery.image_job.revision FROM media.asset CROSS JOIN imagery.image_job",&[]).await.unwrap().get::<_,String>(0),"validated");
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM imagery.image_job_result", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    assert_eq!(
        client
            .query_one("SELECT revision FROM imagery.image_job", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        2
    );
}

#[tokio::test]
async fn durable_multivariant_publication_replays_and_recovers_lost_ack() {
    let scratch = Scratch::open().await;
    let mut client = connect(&scratch.config).await;
    let fixture = Fixture::create(&client, true, true).await;
    let tx = client.transaction().await.unwrap();
    let outcome = finalize_image_in(&tx, &fixture.command()).await.unwrap();
    let PublicationOutcome::Recorded(result) = outcome else {
        panic!("not recorded")
    };
    assert_eq!(result.variants.len(), 2);
    assert_eq!(result.job_revision, 3);
    commit_publication(tx).await.unwrap();
    let tx = client.transaction().await.unwrap();
    assert_eq!(
        read_result_in(&tx, &fixture.owner, &result.job)
            .await
            .unwrap(),
        Some(result.clone())
    );
    assert_eq!(
        finalize_image_in(&tx, &fixture.command()).await.unwrap(),
        PublicationOutcome::Existing(result)
    );
    tx.commit().await.unwrap();
    assert_eq!(
        client
            .query_one("SELECT revision FROM imagery.image_job", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        3
    );
    scratch.finish().await;
}

#[tokio::test]
async fn binding_checkpoint_and_claim_mismatches_roll_back_before_ready() {
    let scratch = Scratch::open().await;
    let mut client = connect(&scratch.config).await;
    let mut fixture = Fixture::create(&client, false, true).await;
    for mismatch in 0..5 {
        let mut command = fixture.command();
        let wrong = ImageOwner::Run {
            campaign: CampaignId::from_bytes(&[20; 16]).unwrap(),
            run: RunId::from_bytes(&[12; 16]).unwrap(),
        };
        match mismatch {
            0 => command.owner = &wrong,
            1 => command.dispatch = DispatchId::from_bytes(&[20; 16]).unwrap(),
            2 => command.claim_generation = 2,
            3 => command.claim = ExecutionClaim::from_bytes(&[20; 16]).unwrap(),
            _ => command.stage = ExecutionStageId::from_bytes(&[20; 16]).unwrap(),
        };
        let tx = client.transaction().await.unwrap();
        assert!(finalize_image_in(&tx, &command).await.is_err());
        tx.rollback().await.unwrap();
        assert_unpublished(&client).await;
    }
    for sql in [
        "UPDATE imagery.image_dispatch_checkpoint SET alpha='transparent'",
        "UPDATE imagery.image_dispatch_checkpoint SET object_version='wrong'",
        "UPDATE imagery.image_dispatch_checkpoint SET sha256=decode(repeat('ab',32),'hex')",
        "UPDATE imagery.image_dispatch_checkpoint SET recipe_digest=decode(repeat('ab',32),'hex')",
        "UPDATE media.asset_variant SET object_version='wrong'",
    ] {
        let tx = client.transaction().await.unwrap();
        tx.batch_execute(sql).await.unwrap();
        assert!(finalize_image_in(&tx, &fixture.command()).await.is_err());
        tx.rollback().await.unwrap();
        assert_unpublished(&client).await;
    }
    fixture.finalize.winners[0].write = WriteId::from_bytes(&[20; 16]).unwrap();
    let tx = client.transaction().await.unwrap();
    assert!(finalize_image_in(&tx, &fixture.command()).await.is_err());
    tx.rollback().await.unwrap();
    assert_unpublished(&client).await;
    scratch.finish().await;
}

#[tokio::test]
async fn result_insert_failure_rolls_back_catalog_ready_quota_outbox_and_revision() {
    let scratch = Scratch::open().await;
    let mut client = connect(&scratch.config).await;
    let fixture = Fixture::create(&client, false, true).await;
    client.batch_execute("CREATE FUNCTION imagery.fail_result() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected result insert failure'; END $$; CREATE TRIGGER fail_result BEFORE INSERT ON imagery.image_job_result FOR EACH ROW EXECUTE FUNCTION imagery.fail_result();").await.unwrap();
    let tx = client.transaction().await.unwrap();
    assert_eq!(
        finalize_image_in(&tx, &fixture.command()).await,
        Err(PublicationError::Unavailable)
    );
    tx.rollback().await.unwrap();
    assert_unpublished(&client).await;
    assert_eq!(
        client
            .query_one(
                "SELECT committed_bytes,reserved_bytes FROM media.quota_usage",
                &[]
            )
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM media.settlement_outbox", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    scratch.finish().await;
}

#[tokio::test]
async fn retired_paid_result_remains_private_and_demand_is_not_revived() {
    let scratch = Scratch::open().await;
    let mut client = connect(&scratch.config).await;
    let fixture = Fixture::create(&client, false, false).await;
    client
        .batch_execute("UPDATE imagery.image_job SET state='retired'")
        .await
        .unwrap();
    let tx = client.transaction().await.unwrap();
    let result = finalize_image_in(&tx, &fixture.command()).await.unwrap();
    let PublicationOutcome::Recorded(result) = result else {
        panic!("not recorded")
    };
    assert!(result.private_reconciliation);
    tx.commit().await.unwrap();
    assert_eq!(
        client
            .query_one("SELECT state FROM imagery.image_job", &[])
            .await
            .unwrap()
            .get::<_, String>(0),
        "retired"
    );
    assert_eq!(
        client
            .query_one("SELECT audience_kind FROM media.asset", &[])
            .await
            .unwrap()
            .get::<_, String>(0),
        "owner_private"
    );
    let tx = client.transaction().await.unwrap();
    assert!(
        read_recipe_result_in(
            &tx,
            &fixture.owner,
            &result.recipe_digest,
            &Audience::OwnerPrivate(AccountId::from_bytes(&[11; 16]).unwrap())
        )
        .await
        .unwrap()
        .is_none()
    );
    let command = fixture.command();
    tx.query_one(
        "SELECT job_id FROM imagery.image_job WHERE job_id=$1 FOR UPDATE",
        &[&command.job.as_bytes().as_slice()],
    )
    .await
    .unwrap();
    assert_eq!(
        stage_economic_use_in(
            &tx,
            &fixture.owner,
            &command.job,
            &command.stage,
            &command.dispatch
        )
        .await
        .unwrap(),
        Some(StageEconomicUse::UndeliveredCandidate { usage_revision: 1 })
    );
    tx.commit().await.unwrap();
    scratch.finish().await;
}

#[tokio::test]
async fn concurrent_duplicate_publications_increment_revision_once_and_conflict_refuses() {
    let scratch = Scratch::open().await;
    let mut first = connect(&scratch.config).await;
    let mut second = connect(&scratch.config).await;
    let fixture = Fixture::create(&first, false, true).await;
    let (left, right) = tokio::join!(
        publish(&mut first, &fixture),
        publish(&mut second, &fixture)
    );
    assert!(matches!(
        (left, right),
        (
            PublicationOutcome::Recorded(_),
            PublicationOutcome::Existing(_)
        ) | (
            PublicationOutcome::Existing(_),
            PublicationOutcome::Recorded(_)
        )
    ));
    let tx = first.transaction().await.unwrap();
    let mut conflict = fixture.command();
    conflict.claim_generation = 2;
    assert_eq!(
        finalize_image_in(&tx, &conflict).await,
        Err(PublicationError::BindingConflict)
    );
    tx.rollback().await.unwrap();
    scratch.finish().await;
}

async fn publish(client: &mut Connection, fixture: &Fixture) -> PublicationOutcome {
    let tx = client.transaction().await.unwrap();
    let result = finalize_image_in(&tx, &fixture.command()).await.unwrap();
    tx.commit().await.unwrap();
    result
}

#[tokio::test]
async fn restricted_role_publishes_without_finance_or_rekey_privileges() {
    let scratch = Scratch::open().await;
    let mut client = connect(&scratch.config).await;
    let fixture = Fixture::create(&client, true, true).await;
    let tx = client.transaction().await.unwrap();
    tx.batch_execute("SET LOCAL ROLE df_imagery_publication")
        .await
        .unwrap();
    assert!(matches!(
        finalize_image_in(&tx, &fixture.command()).await.unwrap(),
        PublicationOutcome::Recorded(_)
    ));
    commit_publication(tx).await.unwrap();
    for sql in [
        "SELECT * FROM finance.provider_attempt",
        "UPDATE media.asset SET attempt=decode(repeat('ab',16),'hex')",
        "UPDATE imagery.image_job_result SET job_revision=99",
        "DELETE FROM imagery.image_result_variant",
    ] {
        let tx = client.transaction().await.unwrap();
        tx.batch_execute("SET LOCAL ROLE df_imagery_publication")
            .await
            .unwrap();
        let error = tx.batch_execute(sql).await.unwrap_err();
        assert_eq!(error.code().map(|code| code.code()), Some("42501"));
        tx.rollback().await.unwrap();
    }
    scratch.finish().await;
}

#[tokio::test]
async fn recovery_routes_allocated_output_despite_later_inspection_touch_and_unknown_pair_refuses()
{
    let scratch = Scratch::open().await;
    let mut client = connect(&scratch.config).await;
    let fixture = Fixture::create(&client, false, false).await;
    client.batch_execute("INSERT INTO imagery.image_execution_stage(job_id,stage_id,owner_scope,stage_kind,stage_state,claim_token,claim_generation,lease_until_ms,input_revision,input_digest,config_revision,config_digest,capacity_scope,local_accounting,created_at_ms,updated_at_ms) SELECT job_id,decode(repeat('ab',16),'hex'),owner_scope,'inspection',stage_state,claim_token,claim_generation,lease_until_ms,input_revision,input_digest,config_revision,config_digest,capacity_scope,local_accounting,created_at_ms,updated_at_ms+100 FROM imagery.image_execution_stage").await.unwrap();
    let tx = client.transaction().await.unwrap();
    let recovery = resolve_publication_in(
        &tx,
        &fixture.owner,
        fixture.held.asset,
        fixture.held.attempt,
    )
    .await
    .unwrap();
    assert_eq!(
        recovery.execution.stage,
        ExecutionStageId::from_bytes(&[2; 16]).unwrap()
    );
    assert_eq!(
        recovery.execution.current.unwrap().dispatch,
        DispatchId::from_bytes(&[3; 16]).unwrap()
    );
    assert!(recovery.output.is_some());
    assert_eq!(
        resolve_publication_in(
            &tx,
            &fixture.owner,
            fixture.held.asset,
            AttemptId::from_bytes(&[99; 16]).unwrap()
        )
        .await,
        Err(PublicationError::NotFound)
    );
    tx.rollback().await.unwrap();
    scratch.finish().await;
}

#[tokio::test]
async fn native_result_port_reads_scoped_ready_results_and_hides_archived_output() {
    use df_imagery::publication::ImageResults;
    use df_persistence::postgres_tls::PostgresTransport;
    let scratch = Scratch::open().await;
    let client = connect(&scratch.config).await;
    let fixture = Fixture::create(&client, false, true).await;
    let store = PostgresImagePublication::open_existing(
        scratch.config.clone(),
        PostgresTransport::PlaintextLoopbackOnly,
        1,
    )
    .await
    .unwrap();
    let outcome = store.publish(&fixture.command()).await.unwrap();
    let PublicationOutcome::Recorded(result) = outcome else {
        panic!("not recorded")
    };
    assert_eq!(
        store
            .read_result(&fixture.owner, &result.job)
            .await
            .unwrap(),
        Some(result.clone())
    );
    let wrong = ImageOwner::Draft {
        account: AccountId::from_bytes(&[22; 16]).unwrap(),
    };
    assert_eq!(store.read_result(&wrong, &result.job).await.unwrap(), None);
    client
        .batch_execute("UPDATE media.asset SET state='archived'")
        .await
        .unwrap();
    assert_eq!(
        store
            .read_result(&fixture.owner, &result.job)
            .await
            .unwrap(),
        None
    );
    drop(store);
    scratch.finish().await;
}

#[tokio::test]
async fn concurrent_conflicting_publications_leave_one_result_and_one_revision_increment() {
    use df_persistence::postgres_tls::PostgresTransport;
    let scratch = Scratch::open().await;
    let client = connect(&scratch.config).await;
    let fixture = Fixture::create(&client, false, true).await;
    let store = PostgresImagePublication::open_existing(
        scratch.config.clone(),
        PostgresTransport::PlaintextLoopbackOnly,
        2,
    )
    .await
    .unwrap();
    let command = fixture.command();
    let mut conflicting = fixture.command();
    conflicting.claim_generation = 99;
    let (good, bad) = tokio::join!(store.publish(&command), store.publish(&conflicting));
    assert!(matches!(good, Ok(PublicationOutcome::Recorded(_))));
    assert!(matches!(
        bad,
        Err(PublicationError::StaleClaim | PublicationError::BindingConflict)
    ));
    assert_eq!(
        client
            .query_one("SELECT revision FROM imagery.image_job", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        3
    );
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM imagery.image_job_result", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        1
    );
    drop(store);
    scratch.finish().await;
}

#[tokio::test]
async fn retired_paid_reconciliation_accepts_expired_execution_lease_but_keeps_exact_claim_and_live_catalog_lease()
 {
    let scratch = Scratch::open().await;
    let mut client = connect(&scratch.config).await;
    let fixture = Fixture::create(&client, false, false).await;
    client.batch_execute("UPDATE imagery.image_job SET state='retired'; UPDATE imagery.image_execution_stage SET lease_until_ms=1").await.unwrap();
    let tx = client.transaction().await.unwrap();
    let mut wrong = fixture.command();
    wrong.claim_generation = 99;
    assert_eq!(
        finalize_image_in(&tx, &wrong).await,
        Err(PublicationError::StaleClaim)
    );
    tx.rollback().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.batch_execute("INSERT INTO imagery.image_dispatch_attempt(dispatch_id,job_id,stage_id,dispatch_seq,dispatch_state,usage_revision,book_call_id,claim_token,claim_generation,local_accounting,created_at_ms,updated_at_ms) SELECT decode(repeat('ab',16),'hex'),job_id,stage_id,1,dispatch_state,usage_revision,book_call_id,claim_token,claim_generation,local_accounting,created_at_ms,updated_at_ms FROM imagery.image_dispatch_attempt; UPDATE imagery.image_stage_cursor SET current_dispatch=decode(repeat('ab',16),'hex')").await.unwrap();
    assert_eq!(
        finalize_image_in(&tx, &fixture.command()).await,
        Err(PublicationError::BindingConflict)
    );
    tx.rollback().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.batch_execute("UPDATE media.asset SET step_lease_until_ms=1")
        .await
        .unwrap();
    assert_eq!(
        finalize_image_in(&tx, &fixture.command()).await,
        Ok(PublicationOutcome::NotRecorded(FinalizeResult::LeaseLost))
    );
    tx.rollback().await.unwrap();
    assert_unpublished(&client).await;
    let tx = client.transaction().await.unwrap();
    tx.batch_execute("SET LOCAL ROLE df_imagery_publication")
        .await
        .unwrap();
    let result = finalize_image_in(&tx, &fixture.command()).await.unwrap();
    let PublicationOutcome::Recorded(result) = result else {
        panic!("not recorded")
    };
    assert!(result.private_reconciliation);
    tx.commit().await.unwrap();
    assert_eq!(
        client
            .query_one("SELECT state FROM imagery.image_job", &[])
            .await
            .unwrap()
            .get::<_, String>(0),
        "retired"
    );
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM imagery.image_interest", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    scratch.finish().await;
}

#[tokio::test]
async fn output_role_cannot_create_hosted_allocation_or_promote_durable_mode() {
    let scratch = Scratch::open().await;
    let mut client = connect(&scratch.config).await;
    let fixture = Fixture::create(&client, false, false).await;
    let tx = client.transaction().await.unwrap();
    tx.batch_execute("SET LOCAL ROLE df_imagery_publication")
        .await
        .unwrap();
    let error = mirror_local_outcome(&tx, &fixture, Some(true))
        .await
        .unwrap_err();
    assert_eq!(error.code().map(|code| code.code()), Some("42501"));
    tx.rollback().await.unwrap();
    for sql in [
        "INSERT INTO imagery.image_publication_allocation SELECT * FROM imagery.image_publication_allocation",
        "UPDATE imagery.image_execution_stage SET local_accounting=true",
        "UPDATE imagery.image_dispatch_attempt SET settlement='settled'",
        "UPDATE imagery.image_dispatch_attempt SET dispatch_state='sent'",
        "SELECT * FROM finance.provider_attempt",
    ] {
        let tx = client.transaction().await.unwrap();
        tx.batch_execute("SET LOCAL ROLE df_imagery_output")
            .await
            .unwrap();
        let error = tx.batch_execute(sql).await.unwrap_err();
        assert_eq!(error.code().map(|code| code.code()), Some("42501"));
        tx.rollback().await.unwrap();
    }
    let tx = client.transaction().await.unwrap();
    tx.batch_execute("SET LOCAL ROLE df_imagery_output")
        .await
        .unwrap();
    let error = tx
        .query_one(
            "SELECT imagery.allocate_local_output($1,$2,$3,$4,$5,$6)",
            &[
                &owner_scope_bytes(&fixture.owner),
                &[1u8; 16].as_slice(),
                &[2u8; 16].as_slice(),
                &fixture.held.asset.as_bytes().as_slice(),
                &fixture.held.attempt.as_bytes().as_slice(),
                &NOW,
            ],
        )
        .await
        .unwrap_err();
    assert_eq!(error.code().map(|code| code.code()), Some("23514"));
    tx.rollback().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.batch_execute("SET LOCAL ROLE df_imagery_output")
        .await
        .unwrap();
    let error = mirror_local_outcome(&tx, &fixture, Some(true))
        .await
        .unwrap_err();
    assert_eq!(error.code().map(|code| code.code()), Some("23514"));
    tx.rollback().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.batch_execute("SET LOCAL ROLE df_imagery_output")
        .await
        .unwrap();
    let command = fixture.command();
    let sent = df_imagery::execution::MarkSentCommand::new(
        fixture.owner.clone(),
        command.job,
        command.stage,
        command.dispatch,
        command.claim,
        1,
        NOW + 11,
    )
    .unwrap();
    assert!(
        df_persistence::imagery_execution::mark_local_sent_in(&tx, &sent)
            .await
            .is_err()
    );
    tx.rollback().await.unwrap();
    assert_unpublished(&client).await;
    scratch.finish().await;
}

async fn mirror_local_outcome(
    tx: &tokio_postgres::Transaction<'_>,
    fixture: &Fixture,
    usable: Option<bool>,
) -> Result<bool, tokio_postgres::Error> {
    tx.query_one(
        "SELECT imagery.mirror_local_outcome($1,$2,$3,$4,$5)",
        &[
            &owner_scope_bytes(&fixture.owner),
            &fixture.command().job.as_bytes().as_slice(),
            &fixture.command().dispatch.as_bytes().as_slice(),
            &usable,
            &(NOW + 11),
        ],
    )
    .await
    .map(|row| row.get(0))
}

#[tokio::test]
async fn local_outcome_replays_once_and_refuses_conflicting_terminal_and_stale_dispatches() {
    let scratch = Scratch::open().await;
    let mut client = connect(&scratch.config).await;
    let fixture = Fixture::create(&client, false, true).await;
    let tx = client.transaction().await.unwrap();
    tx.batch_execute("SET LOCAL ROLE df_imagery_output")
        .await
        .unwrap();
    assert!(
        mirror_local_outcome(&tx, &fixture, Some(true))
            .await
            .unwrap()
    );
    assert!(
        !mirror_local_outcome(&tx, &fixture, Some(true))
            .await
            .unwrap()
    );
    tx.commit().await.unwrap();
    let row = client.query_one(
        "SELECT settlement,revision FROM imagery.image_dispatch_attempt CROSS JOIN imagery.image_job", &[],
    ).await.unwrap();
    assert_eq!(row.get::<_, String>(0), "settled");
    assert_eq!(row.get::<_, i64>(1), 3);
    for usable in [None, Some(false)] {
        let tx = client.transaction().await.unwrap();
        tx.batch_execute("SET LOCAL ROLE df_imagery_output")
            .await
            .unwrap();
        let error = mirror_local_outcome(&tx, &fixture, usable)
            .await
            .unwrap_err();
        assert_eq!(error.code().map(|code| code.code()), Some("23514"));
        tx.rollback().await.unwrap();
    }
    let tx = client.transaction().await.unwrap();
    tx.batch_execute("UPDATE imagery.image_dispatch_attempt SET dispatch_state='failed',failure_cause='refused'; SET LOCAL ROLE df_imagery_output").await.unwrap();
    let error = mirror_local_outcome(&tx, &fixture, Some(false))
        .await
        .unwrap_err();
    assert_eq!(error.code().map(|code| code.code()), Some("23514"));
    tx.rollback().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.batch_execute("INSERT INTO imagery.image_dispatch_attempt(dispatch_id,job_id,stage_id,dispatch_seq,dispatch_state,usage_revision,book_call_id,claim_token,claim_generation,local_accounting,created_at_ms,updated_at_ms) SELECT decode(repeat('ab',16),'hex'),job_id,stage_id,1,dispatch_state,usage_revision,book_call_id,claim_token,claim_generation,local_accounting,created_at_ms,updated_at_ms FROM imagery.image_dispatch_attempt; UPDATE imagery.image_stage_cursor SET current_dispatch=decode(repeat('ab',16),'hex'); SET LOCAL ROLE df_imagery_output").await.unwrap();
    let error = mirror_local_outcome(&tx, &fixture, Some(true))
        .await
        .unwrap_err();
    assert_eq!(error.code().map(|code| code.code()), Some("23514"));
    tx.rollback().await.unwrap();
    scratch.finish().await;
}

#[tokio::test]
async fn local_unknown_stays_outstanding_until_a_proven_terminal_outcome() {
    let scratch = Scratch::open().await;
    let mut client = connect(&scratch.config).await;
    let fixture = Fixture::create(&client, false, true).await;
    client
        .batch_execute("UPDATE imagery.image_dispatch_attempt SET dispatch_state='unknown'")
        .await
        .unwrap();
    let tx = client.transaction().await.unwrap();
    tx.batch_execute("SET LOCAL ROLE df_imagery_output")
        .await
        .unwrap();
    assert!(mirror_local_outcome(&tx, &fixture, None).await.unwrap());
    assert!(!mirror_local_outcome(&tx, &fixture, None).await.unwrap());
    tx.commit().await.unwrap();
    assert_eq!(
        client
            .query_one("SELECT settlement FROM imagery.image_dispatch_attempt", &[])
            .await
            .unwrap()
            .get::<_, String>(0),
        "unknown_outstanding"
    );
    let tx = client.transaction().await.unwrap();
    tx.batch_execute("SET LOCAL ROLE df_imagery_output")
        .await
        .unwrap();
    let error = mirror_local_outcome(&tx, &fixture, Some(true))
        .await
        .unwrap_err();
    assert_eq!(error.code().map(|code| code.code()), Some("23514"));
    tx.rollback().await.unwrap();
    client.batch_execute("UPDATE imagery.image_dispatch_attempt SET dispatch_state='failed',failure_cause='refused'").await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.batch_execute("SET LOCAL ROLE df_imagery_output")
        .await
        .unwrap();
    assert!(
        mirror_local_outcome(&tx, &fixture, Some(false))
            .await
            .unwrap()
    );
    tx.commit().await.unwrap();
    assert_eq!(
        client
            .query_one("SELECT settlement FROM imagery.image_dispatch_attempt", &[])
            .await
            .unwrap()
            .get::<_, String>(0),
        "unusable_settled"
    );
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM finance.provider_attempt", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    scratch.finish().await;
}

#[tokio::test]
async fn recovery_ownership_fences_publication_and_completed_ack_requires_real_ready_receipt() {
    use df_imagery::execution::{ClaimRecoveryCommand, FinishRecoveryCommand, RecoveryDisposition};
    use df_persistence::imagery_execution::{claim_recovery_in, finish_recovery_in};
    let scratch = Scratch::open().await;
    let mut client = connect(&scratch.config).await;
    let fixture = Fixture::create(&client, false, true).await;
    client.batch_execute("UPDATE imagery.image_execution_stage SET lease_until_ms=1,claim_token=decode(repeat('aa',16),'hex'),claim_generation=2").await.unwrap();
    let tx = client.transaction().await.unwrap();
    let claim = ClaimRecoveryCommand {
        owner: fixture.owner.clone(),
        job: fixture.command().job,
        stage: fixture.command().stage,
        dispatch: fixture.command().dispatch,
        claim: ExecutionClaim::from_bytes(&[19; 16]).unwrap(),
        lease_ms: 100,
        now_ms: NOW + 10,
    };
    let recovery = claim_recovery_in(&tx, &claim).await.unwrap();
    tx.commit().await.unwrap();
    let finish = FinishRecoveryCommand {
        owner: fixture.owner.clone(),
        job: claim.job,
        stage: claim.stage,
        dispatch: claim.dispatch,
        claim: recovery.token,
        claim_generation: recovery.generation,
        disposition: RecoveryDisposition::Completed,
        now_ms: NOW + 12,
    };
    let tx = client.transaction().await.unwrap();
    assert!(finish_recovery_in(&tx, &finish).await.is_err());
    tx.rollback().await.unwrap();
    let tx = client.transaction().await.unwrap();
    let mut wrong = recovery.clone();
    wrong.token = ExecutionClaim::from_bytes(&[20; 16]).unwrap();
    let mut command = fixture.command();
    command.recovery = Some(&wrong);
    assert!(finalize_image_in(&tx, &command).await.is_err());
    tx.rollback().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.batch_execute("SET LOCAL ROLE df_imagery_publication")
        .await
        .unwrap();
    let mut command = fixture.command();
    command.recovery = Some(&recovery);
    assert!(matches!(
        finalize_image_in(&tx, &command).await.unwrap(),
        PublicationOutcome::Recorded(_)
    ));
    tx.commit().await.unwrap();
    let tx = client.transaction().await.unwrap();
    assert_eq!(
        finish_recovery_in(&tx, &finish).await,
        Err(df_imagery::execution::ExecutionError::OperationConflict)
    );
    tx.rollback().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.batch_execute("SET LOCAL ROLE df_imagery_output")
        .await
        .unwrap();
    assert!(
        mirror_local_outcome(&tx, &fixture, Some(true))
            .await
            .unwrap()
    );
    tx.commit().await.unwrap();
    let tx = client.transaction().await.unwrap();
    finish_recovery_in(&tx, &finish).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        client
            .query_one(
                "SELECT recovery_state FROM imagery.image_dispatch_recovery",
                &[]
            )
            .await
            .unwrap()
            .get::<_, String>(0),
        "completed"
    );
    scratch.finish().await;
}

#[tokio::test]
async fn source_winner_suppression_expiry_and_current_grant_are_rechecked_before_ready() {
    let scratch = Scratch::open().await;
    let mut client = connect(&scratch.config).await;
    let fixture = Fixture::create(&client, false, true).await;
    let receipt = source_fixture::install(&client, &fixture).await;
    let authority = source_fixture::Authority(fixture.owner.clone());
    for sql in [
        "UPDATE media.asset SET state='archived' WHERE asset_id=decode(repeat('15',16),'hex')",
        "UPDATE media.asset SET suppression_generation=suppression_generation+1 WHERE asset_id=decode(repeat('15',16),'hex')",
        "UPDATE media.asset SET retention_class='intermediate' WHERE asset_id=decode(repeat('15',16),'hex')",
        "UPDATE media.asset SET audience_kind='host' WHERE asset_id=decode(repeat('15',16),'hex')",
        "UPDATE media.asset_variant SET object_version='changed-version' WHERE asset_id=decode(repeat('15',16),'hex')",
        "UPDATE source_grant SET allowed=false",
        "DELETE FROM imagery.image_dispatch_sources",
    ] {
        let tx = client.transaction().await.unwrap();
        tx.batch_execute(sql).await.unwrap();
        let mut command = fixture.command();
        command.source_authority = Some(&authority);
        assert!(
            finalize_image_in(&tx, &command).await.is_err(),
            "accepted {sql}"
        );
        tx.rollback().await.unwrap();
        assert_eq!(
            client
                .query_one(
                    "SELECT state FROM media.asset WHERE asset_id=$1",
                    &[&fixture.held.asset.as_bytes().as_slice()]
                )
                .await
                .unwrap()
                .get::<_, String>(0),
            "validated"
        );
        assert_eq!(
            client
                .query_one("SELECT count(*) FROM imagery.image_job_result", &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            0
        );
    }
    let tx = client.transaction().await.unwrap();
    assert_eq!(
        finalize_image_in(&tx, &fixture.command()).await,
        Err(PublicationError::BindingConflict)
    );
    tx.rollback().await.unwrap();
    let tx = client.transaction().await.unwrap();
    let mut command = fixture.command();
    command.source_authority = Some(&authority);
    command.now_ms = NOW + 1001;
    assert_eq!(
        finalize_image_in(&tx, &command).await,
        Err(PublicationError::BindingConflict)
    );
    tx.rollback().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.batch_execute("SET LOCAL ROLE df_imagery_publication")
        .await
        .unwrap();
    let mut command = fixture.command();
    command.source_authority = Some(&authority);
    assert!(matches!(
        finalize_image_in(&tx, &command).await.unwrap(),
        PublicationOutcome::Recorded(_)
    ));
    tx.commit().await.unwrap();
    assert_eq!(
        receipt.pin.digest.as_bytes(),
        &fixture.proofs[0].record().manifest.unwrap().sha256
    );
    scratch.finish().await;
}

#[tokio::test]
async fn source_catalog_and_grant_locks_remain_held_until_publication_commits() {
    let scratch = Scratch::open().await;
    let mut first = connect(&scratch.config).await;
    let mut second = connect(&scratch.config).await;
    let fixture = Fixture::create(&first, false, true).await;
    source_fixture::install(&first, &fixture).await;
    let authority = source_fixture::Authority(fixture.owner.clone());
    let tx = first.transaction().await.unwrap();
    let mut command = fixture.command();
    command.source_authority = Some(&authority);
    finalize_image_in(&tx, &command).await.unwrap();
    for sql in [
        "SELECT asset_id FROM media.asset WHERE asset_id=decode(repeat('15',16),'hex') FOR UPDATE NOWAIT",
        "SELECT allowed FROM source_grant FOR UPDATE NOWAIT",
    ] {
        let competing = second.transaction().await.unwrap();
        let error = competing.query(sql, &[]).await.unwrap_err();
        assert_eq!(error.code().map(|code| code.code()), Some("55P03"));
        competing.rollback().await.unwrap();
    }
    tx.commit().await.unwrap();
    scratch.finish().await;
}

#[tokio::test]
async fn final_cpu_selection_publishes_exact_candidate_without_replacing_the_original_allocation() {
    use df_imagery::runtime::{ImageDispatchAdmission, ImagePublisher, PreparedImageOutput};
    let scratch = Scratch::open().await;
    let mut client = connect(&scratch.config).await;
    let fixture = Fixture::create(&client, false, true).await;
    let (context, selected) = selection_fixture::prepare(&mut client, &fixture).await;
    let publisher = selection_fixture::publisher(&scratch, &fixture).await;
    let mut wrong = selected.clone();
    wrong.allocation_stage = context.stage.stage;
    assert!(publisher.select_output(&context, &wrong).await.is_err());
    let mut wrong = selected.clone();
    wrong.candidate.as_mut().unwrap().descriptor.sha256[0] ^= 1;
    assert!(publisher.select_output(&context, &wrong).await.is_err());
    let mut wrong = selected.clone();
    wrong.candidate = None;
    assert!(publisher.select_output(&context, &wrong).await.is_err());
    let mut wrong = selected.clone();
    let mut bytes = wrong.resolved_recipe.bytes().to_vec();
    *bytes.last_mut().unwrap() ^= 1;
    wrong.resolved_recipe = df_imagery::VersionedSnapshot::new(
        wrong.resolved_recipe.revision(),
        bytes,
        df_imagery::publication::MAXIMUM_RECIPE_BYTES,
    )
    .unwrap();
    assert!(publisher.select_output(&context, &wrong).await.is_err());
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM imagery.image_final_selection", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    publisher.select_output(&context, &selected).await.unwrap();
    let revision = client
        .query_one("SELECT revision FROM imagery.image_job", &[])
        .await
        .unwrap()
        .get::<_, i64>(0);
    publisher.select_output(&context, &selected).await.unwrap();
    assert_eq!(
        client
            .query_one("SELECT revision FROM imagery.image_job", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        revision
    );
    let checkpoint = publisher
        .store_output(
            &context,
            &PreparedImageOutput {
                master: MASTER.to_vec(),
                display: None,
            },
        )
        .await
        .unwrap();
    selection_fixture::complete_final(&mut client, &context, checkpoint).await;
    publisher
        .outcome(
            &context.job,
            context.dispatch,
            df_imagery::execution::DispatchMode::Local,
            Some(true),
        )
        .await
        .unwrap();
    let result = publisher.publish(&context).await.unwrap();
    assert_eq!(result.stage, context.stage.stage);
    assert_eq!(result.dispatch, context.dispatch);
    assert_eq!(
        result.recipe_digest,
        recipe_digest(&selected.resolved_recipe)
    );
    assert_eq!(result.asset, fixture.held.asset);
    assert_eq!(result.attempt, fixture.held.attempt);
    assert_eq!(publisher.publish(&context).await.unwrap(), result);
    let row = client
        .query_one(
            "SELECT stage_id,original_recipe_key FROM imagery.image_publication_allocation",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(
        row.get::<_, Vec<u8>>(0),
        selected.allocation_stage.as_bytes()
    );
    assert_eq!(row.get::<_, Vec<u8>>(1), [13; 32]);
    assert_eq!(
        client
            .query_one(
                "SELECT settlement FROM imagery.image_dispatch_attempt WHERE dispatch_id=$1",
                &[&fixture.command().dispatch.as_bytes().as_slice()]
            )
            .await
            .unwrap()
            .get::<_, String>(0),
        "unknown_outstanding"
    );
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM imagery.image_job_result", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        1
    );
    let tx = client.transaction().await.unwrap();
    let recovered = resolve_publication_in(
        &tx,
        &fixture.owner,
        fixture.held.asset,
        fixture.held.attempt,
    )
    .await
    .unwrap();
    assert_eq!(recovered.execution.stage, context.stage.stage);
    assert_eq!(
        recovered.execution.current.unwrap().dispatch,
        context.dispatch
    );
    assert!(recovered.output.is_some());
    tx.commit().await.unwrap();
    drop(publisher);
    scratch.finish().await;
}

#[tokio::test]
async fn retired_recordonly_supplier_publication_requires_exact_known_usable_raw_evidence() {
    let scratch = Scratch::open().await;
    let mut client = connect(&scratch.config).await;
    let fixture = Fixture::create(&client, false, true).await;
    client
        .batch_execute("UPDATE imagery.image_job SET state='retired'")
        .await
        .unwrap();
    let tx = client.transaction().await.unwrap();
    assert_eq!(
        finalize_image_in(&tx, &fixture.command()).await,
        Err(PublicationError::BindingConflict)
    );
    tx.rollback().await.unwrap();
    client.execute("INSERT INTO imagery.image_dispatch_usage(dispatch_id,owner_scope,job_id,stage_id,revision,provider,model,cause,detail,maxima,evidence,units,usable,observed_at_ms,raw_output) VALUES($1,$2,$3,$4,1,'fixture','fixture','image','generation','[{\"category\":\"output\",\"quantity\":1}]','reported','[{\"category\":\"output\",\"quantity\":1}]',true,$5,$6)", &[&fixture.command().dispatch.as_bytes().as_slice(),&owner_scope_bytes(&fixture.owner),&fixture.command().job.as_bytes().as_slice(),&fixture.command().stage.as_bytes().as_slice(),&NOW,&MASTER]).await.unwrap();
    for sql in [
        "UPDATE imagery.image_dispatch_usage SET usable=false",
        "UPDATE imagery.image_dispatch_usage SET evidence='unknown'",
        "UPDATE imagery.image_dispatch_usage SET raw_output=NULL",
    ] {
        let tx = client.transaction().await.unwrap();
        tx.batch_execute(sql).await.unwrap();
        assert_eq!(
            finalize_image_in(&tx, &fixture.command()).await,
            Err(PublicationError::BindingConflict)
        );
        tx.rollback().await.unwrap();
        assert_unpublished(&client).await;
    }
    let tx = client.transaction().await.unwrap();
    tx.batch_execute("SET LOCAL ROLE df_imagery_publication")
        .await
        .unwrap();
    let PublicationOutcome::Recorded(result) =
        finalize_image_in(&tx, &fixture.command()).await.unwrap()
    else {
        panic!("private output missing");
    };
    assert!(result.private_reconciliation);
    tx.commit().await.unwrap();
    assert_eq!(
        client
            .query_one("SELECT audience_kind FROM media.asset", &[])
            .await
            .unwrap()
            .get::<_, String>(0),
        "owner_private"
    );
    assert_eq!(
        client
            .query_one("SELECT state FROM imagery.image_job", &[])
            .await
            .unwrap()
            .get::<_, String>(0),
        "retired"
    );
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM finance.provider_attempt", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    scratch.finish().await;
}

#[tokio::test]
async fn native_local_sent_rechecks_exact_source_and_grant_before_one_committed_send_marker() {
    use df_imagery::execution::DispatchMode;
    use df_imagery::runtime::{DispatchAdmission, ImageDispatchAdmission, SendAuthorization};
    use std::sync::Arc;
    let scratch = Scratch::open().await;
    let mut client = connect(&scratch.config).await;
    let fixture = Fixture::create(&client, false, true).await;
    let receipt = source_fixture::install(&client, &fixture).await;
    source_fixture::prepare_send(&mut client, &fixture).await;
    let publisher = selection_fixture::publisher(&scratch, &fixture)
        .await
        .with_source_authority(Arc::new(source_fixture::Authority(fixture.owner.clone())));
    let tx = client.transaction().await.unwrap();
    let job = df_persistence::read_in(&tx, &fixture.owner, fixture.command().job)
        .await
        .unwrap()
        .unwrap();
    let stage = df_persistence::imagery_execution::read_stage_in(
        &tx,
        &fixture.owner,
        &fixture.command().job,
        &fixture.command().stage,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let config = df_imagery::runtime::retained_config(&job).unwrap();
    let receipts = [receipt.clone()];
    let input = DispatchAdmission {
        job: &job,
        stage: &stage,
        dispatch: fixture.command().dispatch,
        claim: fixture.command().claim,
        config: &config,
        quote: None,
        sources: &receipts,
    };
    client
        .batch_execute("UPDATE source_grant SET allowed=false")
        .await
        .unwrap();
    assert!(
        publisher
            .mark_sent(&input, DispatchMode::Local)
            .await
            .is_err()
    );
    client.batch_execute("UPDATE source_grant SET allowed=true; UPDATE media.asset_variant SET object_version='changed-version' WHERE asset_id=decode(repeat('15',16),'hex')").await.unwrap();
    assert!(
        publisher
            .mark_sent(&input, DispatchMode::Local)
            .await
            .is_err()
    );
    assert_eq!(
        client
            .query_one(
                "SELECT dispatch_state FROM imagery.image_dispatch_attempt",
                &[]
            )
            .await
            .unwrap()
            .get::<_, String>(0),
        "prepared"
    );
    client
        .execute(
            "UPDATE media.asset_variant SET object_version=$1 WHERE asset_id=$2",
            &[
                &receipt.version.as_str(),
                &receipt.pin.asset.as_bytes().as_slice(),
            ],
        )
        .await
        .unwrap();
    assert_eq!(
        publisher
            .mark_sent(&input, DispatchMode::Local)
            .await
            .unwrap(),
        SendAuthorization::New
    );
    let revision = client
        .query_one("SELECT revision FROM imagery.image_job", &[])
        .await
        .unwrap()
        .get::<_, i64>(0);
    assert_eq!(
        publisher
            .mark_sent(&input, DispatchMode::Local)
            .await
            .unwrap(),
        SendAuthorization::AlreadySent
    );
    assert_eq!(
        client
            .query_one("SELECT revision FROM imagery.image_job", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        revision
    );
    assert_eq!(
        client
            .query_one(
                "SELECT dispatch_state FROM imagery.image_dispatch_attempt",
                &[]
            )
            .await
            .unwrap()
            .get::<_, String>(0),
        "sent"
    );
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM finance.provider_attempt", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    drop(publisher);
    scratch.finish().await;
}

#[tokio::test]
async fn required_inspection_refuses_missing_rejected_and_explicit_unjudged_evidence() {
    use df_imagery::runtime::{ImagePublisher, RuntimeError};
    for outcome in [
        selection_fixture::Inspection::Rejected,
        selection_fixture::Inspection::Unjudged,
    ] {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config).await;
        let pipeline =
            intent_fixture::inspection_pipeline(df_imagery::InspectionRequirement::Required);
        let fixture = Fixture::create_with_pipeline(&client, false, true, Some(pipeline)).await;
        let tx = client.transaction().await.unwrap();
        assert_eq!(
            finalize_image_in(&tx, &fixture.command()).await,
            Err(PublicationError::BindingConflict)
        );
        tx.rollback().await.unwrap();
        let (context, mut selected) = selection_fixture::prepare(&mut client, &fixture).await;
        let publisher = selection_fixture::publisher(&scratch, &fixture).await;
        assert!(publisher.select_output(&context, &selected).await.is_err());
        let reference = selection_fixture::inspect(&mut client, &context, &selected, outcome).await;
        selected.decisions.push(reference);
        assert_eq!(
            publisher.select_output(&context, &selected).await,
            Err(RuntimeError::ProviderRefused)
        );
        assert_eq!(
            client
                .query_one("SELECT count(*) FROM imagery.image_final_selection", &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            0
        );
        assert_eq!(
            client
                .query_one("SELECT count(*) FROM imagery.image_job_result", &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            0
        );
        assert_eq!(
            client
                .query_one("SELECT state FROM media.asset", &[])
                .await
                .unwrap()
                .get::<_, String>(0),
            "uploading"
        );
        drop(publisher);
        scratch.finish().await;
    }
}

#[tokio::test]
async fn final_selection_qualifies_actual_decisions_and_preserves_unknown_best_effort_liability() {
    use df_imagery::runtime::{ImageDispatchAdmission, ImagePublisher, PreparedImageOutput};
    for (requirement, outcome, private) in [
        (
            df_imagery::InspectionRequirement::Required,
            selection_fixture::Inspection::Pass,
            false,
        ),
        (
            df_imagery::InspectionRequirement::BestEffort,
            selection_fixture::Inspection::Unjudged,
            false,
        ),
        (
            df_imagery::InspectionRequirement::Required,
            selection_fixture::Inspection::Pass,
            true,
        ),
    ] {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config).await;
        let fixture = Fixture::create_with_pipeline(
            &client,
            false,
            true,
            Some(intent_fixture::inspection_pipeline(requirement)),
        )
        .await;
        let (context, mut selected) = selection_fixture::prepare(&mut client, &fixture).await;
        let reference = selection_fixture::inspect(&mut client, &context, &selected, outcome).await;
        selected.decisions.push(reference.clone());
        let decision_proof =
            selection_fixture::economic_proof(&mut client, &context, &selected, &reference).await;
        let candidate_scope = selected.candidate.as_ref().unwrap().scope.clone();
        let tx = client.transaction().await.unwrap();
        tx.query_one(
            "SELECT job_id FROM imagery.image_job WHERE job_id=$1 FOR UPDATE",
            &[&context.job.job.as_bytes().as_slice()],
        )
        .await
        .unwrap();
        assert_eq!(
            stage_economic_use_in(
                &tx,
                &candidate_scope.owner,
                &candidate_scope.job,
                &candidate_scope.stage,
                &candidate_scope.dispatch
            )
            .await
            .unwrap(),
            None
        );
        assert_eq!(
            decision_economic_use_in(
                &tx,
                &reference.scope.owner,
                &reference.scope.job,
                &reference.scope.stage,
                &reference.scope.dispatch,
                &decision_proof.proof
            )
            .await
            .unwrap(),
            Some(StageEconomicUse::Decision { usage_revision: 1 })
        );
        tx.rollback().await.unwrap();
        let publisher = selection_fixture::publisher(&scratch, &fixture).await;
        let mut wrong = selected.clone();
        wrong.decisions[0].payload_sha256[0] ^= 1;
        assert!(publisher.select_output(&context, &wrong).await.is_err());
        let mut wrong = selected.clone();
        wrong.decisions[0].usage_revision = 2;
        assert!(publisher.select_output(&context, &wrong).await.is_err());
        publisher.select_output(&context, &selected).await.unwrap();
        let checkpoint = publisher
            .store_output(
                &context,
                &PreparedImageOutput {
                    master: MASTER.to_vec(),
                    display: None,
                },
            )
            .await
            .unwrap();
        selection_fixture::complete_final(&mut client, &context, checkpoint).await;
        publisher
            .outcome(
                &context.job,
                context.dispatch,
                df_imagery::execution::DispatchMode::Local,
                Some(true),
            )
            .await
            .unwrap();
        if private {
            client
                .batch_execute("UPDATE imagery.image_job SET state='retired'")
                .await
                .unwrap();
        }
        let result = publisher.publish(&context).await.unwrap();
        assert_eq!(result.stage, context.stage.stage);
        assert_eq!(result.private_reconciliation, private);
        let tx = client.transaction().await.unwrap();
        tx.query_one(
            "SELECT job_id FROM imagery.image_job WHERE job_id=$1 FOR UPDATE",
            &[&context.job.job.as_bytes().as_slice()],
        )
        .await
        .unwrap();
        assert_eq!(
            stage_economic_use_in(
                &tx,
                &candidate_scope.owner,
                &candidate_scope.job,
                &candidate_scope.stage,
                &candidate_scope.dispatch
            )
            .await
            .unwrap(),
            Some(if private {
                StageEconomicUse::UndeliveredCandidate { usage_revision: 1 }
            } else {
                StageEconomicUse::SelectedCandidate { usage_revision: 1 }
            })
        );
        assert_eq!(
            stage_economic_use_in(
                &tx,
                &reference.scope.owner,
                &reference.scope.job,
                &reference.scope.stage,
                &reference.scope.dispatch
            )
            .await
            .unwrap(),
            None
        );
        assert_eq!(
            decision_economic_use_in(
                &tx,
                &reference.scope.owner,
                &reference.scope.job,
                &reference.scope.stage,
                &reference.scope.dispatch,
                &decision_proof.proof
            )
            .await
            .unwrap(),
            Some(StageEconomicUse::Decision { usage_revision: 1 })
        );
        assert_eq!(
            stage_economic_use_in(
                &tx,
                &context.job.owner,
                &context.job.job,
                &context.stage.stage,
                &context.dispatch
            )
            .await
            .unwrap(),
            None
        );
        tx.rollback().await.unwrap();
        assert_eq!(
            client
                .query_one(
                    "SELECT settlement FROM imagery.image_dispatch_attempt WHERE dispatch_id=$1",
                    &[&candidate_scope.dispatch.as_bytes().as_slice()]
                )
                .await
                .unwrap()
                .get::<_, String>(0),
            "unknown_outstanding"
        );
        assert!(
            publisher
                .outcome(
                    &context.job,
                    candidate_scope.dispatch,
                    df_imagery::execution::DispatchMode::Local,
                    Some(private)
                )
                .await
                .is_err()
        );
        publisher
            .outcome(
                &context.job,
                candidate_scope.dispatch,
                df_imagery::execution::DispatchMode::Local,
                Some(!private),
            )
            .await
            .unwrap();
        let settled_revision = client
            .query_one("SELECT revision FROM imagery.image_job", &[])
            .await
            .unwrap()
            .get::<_, i64>(0);
        publisher
            .outcome(
                &context.job,
                candidate_scope.dispatch,
                df_imagery::execution::DispatchMode::Local,
                Some(!private),
            )
            .await
            .unwrap();
        assert_eq!(
            client
                .query_one("SELECT revision FROM imagery.image_job", &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            settled_revision
        );
        assert_eq!(
            client
                .query_one(
                    "SELECT settlement FROM imagery.image_dispatch_attempt WHERE dispatch_id=$1",
                    &[&candidate_scope.dispatch.as_bytes().as_slice()]
                )
                .await
                .unwrap()
                .get::<_, String>(0),
            if private {
                "unusable_settled"
            } else {
                "settled"
            }
        );
        assert_eq!(
            client
                .query_one("SELECT count(*) FROM imagery.image_job_result", &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            1
        );
        if matches!(outcome, selection_fixture::Inspection::Unjudged) {
            assert_eq!(client.query_one("SELECT settlement FROM imagery.image_dispatch_attempt WHERE dispatch_id=$1", &[&reference.scope.dispatch.as_bytes().as_slice()]).await.unwrap().get::<_,String>(0),"unknown_outstanding");
        }
        drop(publisher);
        scratch.finish().await;
    }
}

#[tokio::test]
async fn local_decision_projection_uses_exact_protected_receipts_and_never_settles_unknown() {
    use df_imagery::runtime::{DispatchDecisionSettlement, ImageDispatchAdmission};
    for (outcome, expected) in [
        (selection_fixture::Inspection::Pass, "settled"),
        (selection_fixture::Inspection::Rejected, "settled"),
        (
            selection_fixture::Inspection::UnjudgedKnown,
            "unusable_settled",
        ),
        (
            selection_fixture::Inspection::Unjudged,
            "unknown_outstanding",
        ),
    ] {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config).await;
        let fixture = Fixture::create_with_pipeline(
            &client,
            false,
            true,
            Some(intent_fixture::inspection_pipeline(
                df_imagery::InspectionRequirement::BestEffort,
            )),
        )
        .await;
        let (context, selected) = selection_fixture::prepare(&mut client, &fixture).await;
        let reference = selection_fixture::inspect(&mut client, &context, &selected, outcome).await;
        let evidence =
            selection_fixture::economic_proof(&mut client, &context, &selected, &reference).await;
        let publisher = selection_fixture::publisher(&scratch, &fixture).await;
        client
            .execute(
                "UPDATE imagery.image_dispatch_attempt SET settlement='held' WHERE dispatch_id=$1",
                &[&reference.scope.dispatch.as_bytes().as_slice()],
            )
            .await
            .unwrap();
        let before = client
            .query_one("SELECT revision FROM imagery.image_job", &[])
            .await
            .unwrap()
            .get::<_, i64>(0);
        assert!(
            publisher
                .outcome(
                    &context.job,
                    reference.scope.dispatch,
                    df_imagery::execution::DispatchMode::Local,
                    Some(true)
                )
                .await
                .is_err()
        );
        let tx = client.transaction().await.unwrap();
        tx.batch_execute("SET LOCAL ROLE df_imagery_output")
            .await
            .unwrap();
        let wrong = tx
            .query_one(
                "SELECT imagery.mirror_local_decision_outcome($1,$2,$3,$4,$5,$6,$7,$8)",
                &[
                    &owner_scope_bytes(&reference.scope.owner),
                    &reference.scope.job.as_bytes().as_slice(),
                    &reference.scope.stage.as_bytes().as_slice(),
                    &reference.scope.dispatch.as_bytes().as_slice(),
                    &1i32,
                    &[0u8; 32].as_slice(),
                    &evidence.proof.raw_sha256().as_slice(),
                    &(NOW + 10),
                ],
            )
            .await
            .unwrap_err();
        assert_eq!(wrong.code().unwrap().code(), "23514");
        tx.rollback().await.unwrap();
        let input = DispatchDecisionSettlement {
            job: &context.job,
            mode: df_imagery::execution::DispatchMode::Local,
            usage: &evidence.usage,
            qualified: &evidence.proof,
        };
        publisher.outcome_decision(&input).await.unwrap();
        publisher.outcome_decision(&input).await.unwrap();
        assert_eq!(
            client
                .query_one(
                    "SELECT settlement FROM imagery.image_dispatch_attempt WHERE dispatch_id=$1",
                    &[&reference.scope.dispatch.as_bytes().as_slice()]
                )
                .await
                .unwrap()
                .get::<_, String>(0),
            expected
        );
        assert_eq!(
            client
                .query_one("SELECT revision FROM imagery.image_job", &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            before + 1
        );
        assert_eq!(
            client
                .query_one("SELECT count(*) FROM finance.provider_attempt", &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            0
        );
        assert!(!client.query_one("SELECT has_function_privilege('df_finance','imagery.mirror_local_decision_outcome(bytea,bytea,bytea,bytea,integer,bytea,bytea,bigint)','EXECUTE')",&[]).await.unwrap().get::<_,bool>(0));
        drop(publisher);
        scratch.finish().await;
    }
}
