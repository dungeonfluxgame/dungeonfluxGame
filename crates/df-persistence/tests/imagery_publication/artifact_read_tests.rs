//! Real byte proofs and native Ready publication precede every artifact read.
use super::*;
use df_imagery::publication::SourceDenial;
use df_imagery::*;
use df_persistence::postgres_tls::PostgresTransport;
use df_types::OperationId;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Option<AcceptCommand>>>);
impl ImageJournal for Capture {
    async fn accept(&self, command: &AcceptCommand) -> Result<AcceptOutcome, JournalError> {
        *self.0.lock().unwrap() = Some(command.clone());
        Err(JournalError::Unavailable)
    }
    async fn lookup_operation(
        &self,
        _: &ImageOwner,
        _: OperationId,
    ) -> Result<Option<OperationView>, JournalError> {
        Ok(None)
    }
    async fn read_job(
        &self,
        _: &ImageOwner,
        _: ImageJobId,
    ) -> Result<Option<ImageJob>, JournalError> {
        panic!("capture read")
    }
    async fn freeze_recipe(&self, _: &FreezeCommand) -> Result<FreezeOutcome, JournalError> {
        panic!("capture freeze")
    }
    async fn withdraw_interest(
        &self,
        _: &ImageOwner,
        _: ImageInterestId,
        _: i64,
    ) -> Result<WithdrawOutcome, JournalError> {
        panic!("capture withdraw")
    }
}
struct Clock;
impl ServiceClock for Clock {
    fn now_ms(&self) -> i64 {
        NOW
    }
}
struct Ids;
impl ServiceIds for Ids {
    fn next_job_id(&self) -> Result<ImageJobId, ServiceIdError> {
        Ok(ImageJobId::from_bytes(&[1; 16]).unwrap())
    }
    fn next_interest_id(&self) -> Result<ImageInterestId, ServiceIdError> {
        Ok(ImageInterestId::from_bytes(&[15; 16]).unwrap())
    }
}
struct RequestAuthority;
impl ImageAuthorizer for RequestAuthority {
    type Principal = ();
    fn authorize_request(&self, _: &(), _: &ImageRequest) -> Result<(), AuthDenied> {
        Ok(())
    }
    fn authorize_owner(&self, _: &(), _: &ImageOwner, _: ServiceAction) -> Result<(), AuthDenied> {
        Ok(())
    }
    fn authorize_stored(
        &self,
        _: &(),
        _: ServiceAction,
        _: &OperationView,
        _: &StoredImageAuthorization<'_>,
    ) -> Result<(), AuthDenied> {
        panic!("capture stored")
    }
}

fn owner(run: u8) -> ImageOwner {
    ImageOwner::Run {
        campaign: CampaignId::from_bytes(&[10; 16]).unwrap(),
        run: RunId::from_bytes(&[run; 16]).unwrap(),
    }
}
fn request(run: u8) -> ImageRequest {
    let ImageOwner::Run {
        campaign,
        run: run_id,
    } = owner(run)
    else {
        unreachable!()
    };
    ImageRequest {
        identity: OperationIdentity {
            owner: owner(run),
            operation: OperationId::from_bytes(&[16; 16]).unwrap(),
            fingerprint: [0; 32],
        },
        purpose: PurposedUse {
            purpose: Purpose::CanonicalPerson,
            role: VariantRole::Primary,
        },
        operation: ImageOperation::Generate(GenerateSpec {
            prompt: TextPrompt::new("the complete stable public hero photograph", 0xdf02_0001)
                .unwrap()
                .with_artwork(ArtworkBinding {
                    spec: CampaignArtworkSpec {
                        campaign,
                        subject: ArtworkSubject::Hero(
                            ArtworkSubjectKey::new("library/public-hero").unwrap(),
                        ),
                        public_revision: [31; 32],
                    },
                    selection: RunSelectionFence {
                        run: run_id,
                        binding: [run; 32],
                    },
                }),
            references: Vec::new(),
        }),
        execution: ExecutionMode::Live,
        freshness: Freshness::ReuseCompatible,
        urgency: Urgency::Visible,
        audience: Audience::CampaignShared(campaign),
        expected_source_revision: 1,
        output: OutputProfile::provider(
            Dimensions::new(1024, 1024).unwrap(),
            RasterOutputSpec::new(
                Dimensions::new(1, 1).unwrap(),
                AlphaHandling::Opaque,
                Encoding::Png,
            )
            .unwrap(),
        ),
    }
}
async fn snapshots(
    mut request: ImageRequest,
    config: ImageConfig,
) -> (ImageRequest, VersionedSnapshot, VersionedSnapshot) {
    let capture = Capture::default();
    let service = ImageService::new(capture.clone(), config, Clock, Ids, RequestAuthority).unwrap();
    request.identity.fingerprint = service.canonical_fingerprint(&request).unwrap();
    assert_eq!(
        service.request(&(), &request).await.unwrap_err(),
        ImageServiceError::Unavailable
    );
    let command = capture.0.lock().unwrap().take().unwrap();
    (request, command.intent, command.config)
}
struct Authority(ImageOwner);
impl Authority {
    fn check(
        &self,
        owner: &ImageOwner,
        output: &AssetRecord,
        facts: &[PublicationSource],
    ) -> Result<(), PublicationError> {
        let ImageOwner::Run { campaign, .. } = owner else {
            return Err(PublicationError::BindingConflict);
        };
        if owner != &self.0
            || output.audience != Audience::CampaignShared(*campaign)
            || output.campaign != Some(*campaign)
        {
            return Err(PublicationError::BindingConflict);
        }
        if output.retention != RetentionClass::CampaignMedia {
            return Err(PublicationError::SourceDenied(
                SourceDenial::RetentionChanged,
            ));
        }
        if facts.iter().any(|fact| {
            Audience::CampaignShared(*campaign).meet(&fact.asset.audience)
                != Audience::CampaignShared(*campaign)
        }) {
            return Err(PublicationError::SourceDenied(
                SourceDenial::AudienceChanged,
            ));
        }
        Ok(())
    }
}
impl PublicationSourceAuthority for Authority {
    fn lock_authority<'a>(
        &'a self,
        tx: &'a tokio_postgres::Transaction<'_>,
        owner: &'a ImageOwner,
        _: i64,
    ) -> PublicationAuthorityFuture<'a> {
        Box::pin(async move {
            if owner != &self.0 {
                return Err(PublicationError::BindingConflict);
            }
            let allowed = tx
                .query_opt(
                    "SELECT allowed FROM artifact_grant WHERE owner_scope=$1 FOR UPDATE",
                    &[&owner_scope_bytes(owner)],
                )
                .await
                .map_err(|_| PublicationError::Unavailable)?;
            if allowed.is_some_and(|row| row.get::<_, bool>(0)) {
                Ok(())
            } else {
                Err(PublicationError::SourceDenied(SourceDenial::GrantRefused))
            }
        })
    }
    fn authorize_sources<'a>(
        &'a self,
        tx: &'a tokio_postgres::Transaction<'_>,
        job: &'a ImageJob,
        _: &'a [PublicationSource],
        now: i64,
    ) -> PublicationAuthorityFuture<'a> {
        Box::pin(async move { self.lock_authority(tx, &job.owner, now).await })
    }
    fn authorize_artifact<'a>(
        &'a self,
        _: &'a tokio_postgres::Transaction<'_>,
        request: &'a ImageRequest,
        output: &'a AssetRecord,
        facts: &'a [PublicationSource],
        _: i64,
    ) -> PublicationAuthorityFuture<'a> {
        Box::pin(async move { self.check(&request.identity.owner, output, facts) })
    }
    fn authorize_artifact_read<'a>(
        &'a self,
        _: &'a tokio_postgres::Transaction<'_>,
        owner: &'a ImageOwner,
        output: &'a AssetRecord,
        facts: &'a [PublicationSource],
        _: i64,
    ) -> PublicationAuthorityFuture<'a> {
        Box::pin(async move { self.check(owner, output, facts) })
    }
}
async fn grant(client: &Client) {
    client.batch_execute("CREATE TABLE artifact_grant(owner_scope bytea PRIMARY KEY,allowed boolean NOT NULL);GRANT SELECT,UPDATE ON artifact_grant TO df_imagery_publication").await.unwrap();
    for run in [12, 13] {
        client
            .execute(
                "INSERT INTO artifact_grant VALUES($1,true)",
                &[&owner_scope_bytes(&owner(run))],
            )
            .await
            .unwrap();
    }
}
async fn publish(
    client: &mut Client,
    fixture: &Fixture,
) -> df_imagery::publication::PublishedImage {
    let authority = Authority(fixture.owner.clone());
    let tx = client.transaction().await.unwrap();
    let mut command = fixture.command();
    command.source_authority = Some(&authority);
    let PublicationOutcome::Recorded(result) = finalize_image_in(&tx, &command).await.unwrap()
    else {
        panic!("real native Ready required")
    };
    tx.commit().await.unwrap();
    result
}
async fn open_store(scratch: &Scratch) -> PostgresImagePublication {
    let mut config = scratch.config.clone();
    config.options("-c role=df_imagery_output");
    PostgresImagePublication::open_existing(config, PostgresTransport::PlaintextLoopbackOnly, 1)
        .await
        .unwrap()
}
async fn counts(client: &Client) -> Vec<i64> {
    let row=client.query_one("SELECT (SELECT count(*) FROM media.asset),(SELECT count(*) FROM media.quota_reservation),(SELECT count(*) FROM imagery.image_job),(SELECT count(*) FROM imagery.image_operation),(SELECT count(*) FROM imagery.image_interest),(SELECT count(*) FROM imagery.image_dispatch_attempt),(SELECT count(*) FROM finance.provider_attempt),(SELECT count(*) FROM imagery.image_artifact_dependency)",&[]).await.unwrap();
    (0..row.len()).map(|index| row.get(index)).collect()
}
async fn fixture(client: &Client) -> (Fixture, VersionedSnapshot) {
    let (_, intent, config) = snapshots(request(12), ImageConfig::defaults().unwrap()).await;
    let recipe = retained_campaign_artifact_recipe(&intent, &config)
        .unwrap()
        .unwrap();
    (
        Fixture::create_with_artifact(client, intent, config.clone(), recipe).await,
        config,
    )
}

#[tokio::test]
async fn actual_ready_reads_across_two_runs_survive_restart_and_retirement_without_new_work() {
    let scratch = Scratch::open().await;
    let mut client = connect(&scratch.config).await;
    grant(&client).await;
    let (fixture, config) = fixture(&client).await;
    let published = publish(&mut client, &fixture).await;
    client
        .execute(
            "UPDATE imagery.image_job SET state='retired' WHERE job_id=$1",
            &[&fixture.command().job.as_bytes().as_slice()],
        )
        .await
        .unwrap();
    let baseline = counts(&client).await;
    let current = request(13);
    let authority = Authority(current.identity.owner.clone());
    let store = open_store(&scratch).await;
    let artifact = store
        .find_ready_artifact_with_authority(&current, &config, NOW + 20, &authority)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(artifact.asset, published.asset);
    assert_eq!(artifact.attempt, published.attempt);
    assert_eq!(artifact.variants, published.variants);
    assert!(artifact.inspection.is_none());
    assert!(artifact.staging.is_empty());
    assert_eq!(
        store
            .read_campaign_artifact_with_authority(
                &current.identity.owner,
                &published.asset,
                NOW + 20,
                &authority
            )
            .await
            .unwrap(),
        Some(artifact.clone())
    );
    assert!(
        store
            .read_artifact_with_authority(
                &current.identity.owner,
                &fixture.command().job,
                NOW + 20,
                &authority
            )
            .await
            .unwrap()
            .is_none()
    );
    drop(store);
    let restarted = open_store(&scratch).await;
    assert_eq!(
        restarted
            .find_ready_artifact_with_authority(&current, &config, NOW + 2000, &authority)
            .await
            .unwrap(),
        Some(artifact)
    );
    assert_eq!(counts(&client).await, baseline);
    drop(restarted);
    scratch.finish().await;
}

#[tokio::test]
async fn changed_complete_prompt_source_revision_and_provider_model_miss_without_writes() {
    let scratch = Scratch::open().await;
    let mut client = connect(&scratch.config).await;
    grant(&client).await;
    let (fixture, config) = fixture(&client).await;
    publish(&mut client, &fixture).await;
    let baseline = counts(&client).await;
    let store = open_store(&scratch).await;
    let authority = Authority(owner(13));
    let mut changed = request(13);
    changed.expected_source_revision += 1;
    assert!(
        store
            .find_ready_artifact_with_authority(&changed, &config, NOW + 20, &authority)
            .await
            .unwrap()
            .is_none()
    );
    let ImageOperation::Generate(spec) = &mut changed.operation else {
        unreachable!()
    };
    let artwork = spec.prompt.artwork().unwrap().clone();
    spec.prompt = TextPrompt::new(
        "different complete provider text",
        spec.prompt.template_version,
    )
    .unwrap()
    .with_artwork(artwork);
    assert!(
        store
            .find_ready_artifact_with_authority(&changed, &config, NOW + 20, &authority)
            .await
            .unwrap()
            .is_none()
    );
    let mut model = ImageConfig::defaults().unwrap();
    model.backends[BackendId::OpenAi.index()].model =
        Some(ModelName::new("different-model").unwrap());
    let (_, _, config) = snapshots(request(13), model).await;
    assert!(
        store
            .find_ready_artifact_with_authority(&request(13), &config, NOW + 20, &authority)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(counts(&client).await, baseline);
    drop(store);
    scratch.finish().await;
}

#[tokio::test]
async fn current_grant_retention_and_exact_winning_version_refuse_live_artifact_reuse() {
    let scratch = Scratch::open().await;
    let mut client = connect(&scratch.config).await;
    grant(&client).await;
    let (fixture, config) = fixture(&client).await;
    let published = publish(&mut client, &fixture).await;
    let store = open_store(&scratch).await;
    let authority = Authority(owner(13));
    client
        .execute(
            "UPDATE artifact_grant SET allowed=false WHERE owner_scope=$1",
            &[&owner_scope_bytes(&owner(13))],
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .find_ready_artifact_with_authority(&request(13), &config, NOW + 20, &authority)
            .await
            .unwrap_err(),
        PublicationError::SourceDenied(SourceDenial::GrantRefused)
    );
    client
        .execute(
            "UPDATE artifact_grant SET allowed=true WHERE owner_scope=$1",
            &[&owner_scope_bytes(&owner(13))],
        )
        .await
        .unwrap();
    client
        .execute(
            "UPDATE media.asset SET retention_class='personal' WHERE asset_id=$1",
            &[&published.asset.as_bytes().as_slice()],
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .read_campaign_artifact_with_authority(
                &owner(13),
                &published.asset,
                NOW + 20,
                &authority
            )
            .await
            .unwrap_err(),
        PublicationError::SourceDenied(SourceDenial::RetentionChanged)
    );
    client
        .execute(
            "UPDATE media.asset SET retention_class='campaign_media' WHERE asset_id=$1",
            &[&published.asset.as_bytes().as_slice()],
        )
        .await
        .unwrap();
    client.execute("UPDATE media.asset_variant SET object_version='changed-winning-version' WHERE asset_id=$1",&[&published.asset.as_bytes().as_slice()]).await.unwrap();
    assert_eq!(
        store
            .find_ready_artifact_with_authority(&request(13), &config, NOW + 20, &authority)
            .await
            .unwrap_err(),
        PublicationError::SourceDenied(SourceDenial::WinnerChanged)
    );
    drop(store);
    scratch.finish().await;
}

#[tokio::test]
async fn a_poisoned_semantic_hint_never_grants_an_actual_ready_asset() {
    let scratch = Scratch::open().await;
    let mut client = connect(&scratch.config).await;
    grant(&client).await;
    let (_, intent, config) = snapshots(request(12), ImageConfig::defaults().unwrap()).await;
    let mut recipe = retained_campaign_artifact_recipe(&intent, &config)
        .unwrap()
        .unwrap();
    let mut bytes = recipe.semantic.bytes().to_vec();
    bytes.push(0);
    recipe.semantic = VersionedSnapshot::new(recipe.semantic.revision(), bytes, 1_048_576).unwrap();
    let fixture = Fixture::create_with_artifact(&client, intent, config.clone(), recipe).await;
    publish(&mut client, &fixture).await;
    let store = open_store(&scratch).await;
    assert_eq!(
        store
            .find_ready_artifact_with_authority(
                &request(13),
                &config,
                NOW + 20,
                &Authority(owner(13))
            )
            .await
            .unwrap_err(),
        PublicationError::InvalidRecord
    );
    drop(store);
    scratch.finish().await;
}

#[tokio::test]
async fn raw_resolved_reference_metadata_cannot_adopt_a_dispatched_producer_job() {
    let scratch = Scratch::open().await;
    let mut client = connect(&scratch.config).await;
    grant(&client).await;
    let (fixture, config) = fixture(&client).await;
    let published = publish(&mut client, &fixture).await;
    let (_, intent, _) = snapshots(request(12), ImageConfig::defaults().unwrap()).await;
    let recipe = retained_campaign_artifact_recipe(&intent, &config)
        .unwrap()
        .unwrap();
    client.execute("INSERT INTO imagery.image_artifact_dependency(owner_scope,job_id,stage_id,claim_generation,semantic_recipe_revision,semantic_recipe_bytes,semantic_key,asset_id,publication_attempt,reference_state,accepted_at_ms,next_check_at_ms,resolved_at_ms,resolved_job_revision) VALUES($1,$2,$3,1,$4,$5,$6,$7,$8,'resolved',$9,$10,$10,$11)", &[&owner_scope_bytes(&fixture.owner),&fixture.command().job.as_bytes().as_slice(),&fixture.command().stage.as_bytes().as_slice(),&i64::from(recipe.semantic.revision()),&recipe.semantic.bytes(),&recipe.key.as_bytes().as_slice(),&published.asset.as_bytes().as_slice(),&published.attempt.as_bytes().as_slice(),&NOW,&(NOW+10),&i64::try_from(published.job_revision).unwrap()]).await.unwrap();
    let store = open_store(&scratch).await;
    assert!(
        store
            .read_artifact_with_authority(
                &fixture.owner,
                &fixture.command().job,
                NOW + 20,
                &Authority(fixture.owner.clone())
            )
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM imagery.image_dispatch_attempt", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        1
    );
    drop(store);
    scratch.finish().await;
}
