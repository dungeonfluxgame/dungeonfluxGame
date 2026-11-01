//! Actual money/execution transactions exercise the adapter's Sent hook, without provider calls.
use crate::imagery_draft_admission::tests::support;

use super::super::{
    DraftFunding, FundedImageAdmission, RuntimeAdmissionPorts, RuntimeAdmissionSettings,
    RuntimeFunding,
};
use super::*;
use crate::imagery_draft_admission::StagePayer;
use crate::imagery_execution as execution;
use crate::postgres_tls::PostgresTransport;
use df_commerce::draft_image::{
    ConsentId, DraftId, JobConsent, JobId, JobScope, PaidStageKind, StageCall, StageRoute,
};
use df_finance::fakes::{FakeFence, FakeMedia, FakeMembers, SeededIds, SystemClock};
use df_finance::ports::{Clock, FenceVerdict, MediaVerdict};
use df_finance::service::{FinanceConfig, FinanceService};
use df_imagery::cost::{UsageEvidence, UsageRevision};
use df_imagery::execution::*;
use df_imagery::runtime::SourceReceipt;
use df_imagery::runtime::{ImageDispatchAdmission, ImageUsageJournal, RuntimeUsage};
use df_imagery::{
    AccountingMode, AlphaHandling, Audience, AuthDenied, Dimensions, Encoding, ExecutionMode,
    Freshness, GenerateSpec, ImageAuthorizer, ImageConfig, ImageOperation, ImageRequest,
    ImageService, OutputProfile, Purpose, PurposedUse, RasterOutputSpec, RequestOutcome,
    ServiceAction, ServiceClock, ServiceIdError, ServiceIds, StoredImageAuthorization, TextPrompt,
    Urgency, VariantRole,
};
use df_imagery::{
    ImageInterestId, ImageJobId, ImageJournal, ImageOwner, OperationIdentity, OperationView,
};
use df_types::{AccountId, OperationId};
use std::sync::Arc;
use support::world::{World, payer, purchase};
use tokio::sync::Mutex;

struct Authority {
    account: AccountId,
    grant: JobConsent,
}
impl RuntimeImageAuthority for Authority {
    async fn funding(&self, input: &DispatchAdmission<'_>) -> Result<RuntimeFunding, RuntimeError> {
        if input.job.owner
            != (ImageOwner::Draft {
                account: self.account,
            })
        {
            return Err(RuntimeError::SourceDenied);
        }
        Ok(RuntimeFunding::Draft(Box::new(DraftFunding {
            account: self.account,
            consent: self.grant.id,
            consent_revision: self.grant.revision,
            scope: self.grant.scope.clone(),
            route: self.grant.scope.routes[0].clone(),
        })))
    }
    async fn verify(
        &self,
        tx: &Transaction<'_>,
        payer: StagePayer<'_>,
        owner: &ImageOwner,
        call: &StageCall,
        sources: &[SourceReceipt],
        _replay: bool,
    ) -> Result<(), Error> {
        if !matches!(payer,StagePayer::Account(account) if account==self.account)
            || *owner
                != (ImageOwner::Draft {
                    account: self.account,
                })
            || !sources.is_empty()
        {
            return Err(Error::LinkConflict);
        }
        // This fixture's host grant is stored and locked in the very same finance transaction.
        let row = tx
            .query_opt(
                "SELECT 1 FROM public.funded_adapter_authority h \
             JOIN finance.draft_image_consent c ON c.consent_id=h.consent_id \
             WHERE h.active AND c.account_id=$1 AND c.consent_id=$2 AND NOT c.revoked \
             AND c.scope->'job'=$3::text::jsonb FOR SHARE OF h",
                &[
                    &self.account.as_bytes().as_slice(),
                    &self.grant.id.0.as_slice(),
                    &serde_json::json!(call.job).to_string(),
                ],
            )
            .await
            .map_err(|_| Error::PortUnavailable)?;
        row.map(|_| ()).ok_or(Error::LinkConflict)
    }
}
struct FixedClock(i64);
impl ServiceClock for FixedClock {
    fn now_ms(&self) -> i64 {
        self.0
    }
}
struct FixedIds;
impl ServiceIds for FixedIds {
    fn next_job_id(&self) -> Result<ImageJobId, ServiceIdError> {
        Ok(ImageJobId::from_bytes(&[2; 16]).unwrap())
    }
    fn next_interest_id(&self) -> Result<ImageInterestId, ServiceIdError> {
        Ok(ImageInterestId::from_bytes(&[10; 16]).unwrap())
    }
}
struct DemandAuthority(AccountId);
impl ImageAuthorizer for DemandAuthority {
    type Principal = AccountId;
    fn authorize_request(
        &self,
        principal: &AccountId,
        request: &ImageRequest,
    ) -> Result<(), AuthDenied> {
        if request.operation.sources().is_empty()
            && request.audience == Audience::OwnerPrivate(self.0)
        {
            self.authorize_owner(principal, &request.identity.owner, ServiceAction::Read)
        } else {
            Err(AuthDenied)
        }
    }
    fn authorize_owner(
        &self,
        principal: &AccountId,
        owner: &ImageOwner,
        _action: ServiceAction,
    ) -> Result<(), AuthDenied> {
        if *principal == self.0 && *owner == (ImageOwner::Draft { account: self.0 }) {
            Ok(())
        } else {
            Err(AuthDenied)
        }
    }
    fn authorize_stored(
        &self,
        principal: &AccountId,
        action: ServiceAction,
        view: &OperationView,
        stored: &StoredImageAuthorization<'_>,
    ) -> Result<(), AuthDenied> {
        if *stored.audience != Audience::OwnerPrivate(self.0) || stored.owner != &view.job.owner {
            return Err(AuthDenied);
        }
        self.authorize_owner(principal, &view.job.owner, action)
    }
}
struct NoLocal;
impl ImageDispatchAdmission for NoLocal {
    async fn admit(&self, _input: &DispatchAdmission<'_>) -> Result<DispatchMode, RuntimeError> {
        Err(RuntimeError::UnsupportedCapability)
    }
    async fn mark_sent(
        &self,
        _input: &DispatchAdmission<'_>,
        _mode: DispatchMode,
    ) -> Result<SendAuthorization, RuntimeError> {
        Err(RuntimeError::UnsupportedCapability)
    }
    async fn outcome(
        &self,
        _job: &df_imagery::ImageJob,
        _dispatch: DispatchId,
        _mode: DispatchMode,
        _usable: Option<bool>,
    ) -> Result<(), RuntimeError> {
        Err(RuntimeError::UnsupportedCapability)
    }
}
async fn demand(
    world: &World,
    account: AccountId,
    now: i64,
) -> (
    crate::PostgresImageJournal,
    df_imagery::ImageJob,
    ImageConfig,
) {
    let journal = crate::PostgresImageJournal::open(
        world.scratch.config(),
        PostgresTransport::PlaintextLoopbackOnly,
        1,
    )
    .await
    .unwrap();
    let mut config = ImageConfig::defaults().unwrap();
    config.accounting.mode = AccountingMode::Hosted;
    let service = ImageService::new(
        journal.clone(),
        config.clone(),
        FixedClock(now),
        FixedIds,
        DemandAuthority(account),
    )
    .unwrap();
    let dimensions = Dimensions::new(1024, 1024).unwrap();
    let mut request = ImageRequest {
        identity: OperationIdentity {
            owner: ImageOwner::Draft { account },
            operation: OperationId::from_bytes(&[7; 16]).unwrap(),
            fingerprint: [0; 32],
        },
        purpose: PurposedUse {
            purpose: Purpose::ScenePlate,
            role: VariantRole::Primary,
        },
        operation: ImageOperation::Generate(GenerateSpec {
            prompt: TextPrompt::new("a fixture portrait", 3).unwrap(),
            references: vec![],
        }),
        execution: ExecutionMode::Live,
        freshness: Freshness::ReuseCompatible,
        urgency: Urgency::Present,
        audience: Audience::OwnerPrivate(account),
        expected_source_revision: 1,
        output: OutputProfile::provider(
            dimensions,
            RasterOutputSpec::new(dimensions, AlphaHandling::Opaque, Encoding::Png).unwrap(),
        ),
    };
    request.identity.fingerprint = service.canonical_fingerprint(&request).unwrap();
    assert!(matches!(
        service.request(&account, &request).await.unwrap(),
        RequestOutcome::Recorded(_)
    ));
    let job = journal
        .read_job(
            &request.identity.owner,
            ImageJobId::from_bytes(&[2; 16]).unwrap(),
        )
        .await
        .unwrap()
        .unwrap();
    (journal, job, config)
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
    use std::io::Write;
    let mut registry =
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
                "../../artifacts/tmp/imagery-finish-20261029/funded-adapter/owned-databases.txt",
            ))
            .unwrap();
    writeln!(registry, "pid={} database={database}", std::process::id()).unwrap();
    let transport = PostgresTransport::PlaintextLoopbackOnly;
    crate::media_catalog::PostgresMediaCatalog::open(world.scratch.config(), transport.clone(), 1)
        .await
        .unwrap();
    crate::PostgresImageJournal::open(world.scratch.config(), transport, 1)
        .await
        .unwrap();
    for sql in [
        include_str!("../../migrations/0039_imagery_admission_bindings.sql"),
        execution::IMAGERY_EXECUTION_MIGRATION_SQL,
        crate::imagery_draft_admission::MIGRATION_SQL,
        crate::imagery_runtime::IMAGERY_USAGE_MIGRATION_SQL,
        include_str!("../../migrations/0053_imagery_source_facts.sql"),
    ] {
        world.admin.batch_execute(sql).await.unwrap();
    }
    if world
        .admin
        .query_one(
            "SELECT to_regprocedure('finance.image_stage_protocol_required(bytea)') IS NULL",
            &[],
        )
        .await
        .unwrap()
        .get::<_, bool>(0)
    {
        world
            .admin
            .batch_execute(include_str!(
                "../../migrations/0050_imagery_campaign_release_guard.sql"
            ))
            .await
            .unwrap();
    }
    world
        .admin
        .batch_execute(
            "CREATE TABLE public.funded_adapter_authority(\
             consent_id bytea PRIMARY KEY,active boolean NOT NULL); \
         GRANT SELECT,UPDATE(active) ON public.funded_adapter_authority TO df_imagery_admission",
        )
        .await
        .unwrap();
    world
        .service()
        .await
        .apply_purchase(purchase(payer(1), "funded-adapter-wallet", 1000))
        .await
        .unwrap();
    Some(world)
}
async fn snapshot_rows(world: &World) -> String {
    world
        .admin
        .query_one(
            "SELECT jsonb_build_object( \
        'job',(SELECT jsonb_agg(to_jsonb(j)) FROM imagery.image_job j), \
        'stage',(SELECT jsonb_agg(to_jsonb(s)) FROM imagery.image_execution_stage s), \
        'dispatch',(SELECT jsonb_agg(to_jsonb(d)) FROM imagery.image_dispatch_attempt d), \
        'sources',(SELECT jsonb_agg(to_jsonb(s)) FROM imagery.image_dispatch_sources s), \
        'attempt',(SELECT jsonb_agg(to_jsonb(a)) FROM finance.provider_attempt a), \
        'receipt',(SELECT jsonb_agg(to_jsonb(r)) FROM finance.operation_receipt r), \
        'quote',(SELECT jsonb_agg(to_jsonb(q)) FROM finance.quote q), \
        'event',(SELECT jsonb_agg(to_jsonb(e)) FROM finance.event_marker e), \
        'entry',(SELECT jsonb_agg(to_jsonb(e)) FROM finance.journal_entry e), \
        'line',(SELECT jsonb_agg(to_jsonb(l)) FROM finance.journal_line l))::text",
            &[],
        )
        .await
        .unwrap()
        .get(0)
}

#[test]
fn first_send_commits_both_journals_and_refusal_or_replay_never_sends_again() {
    support::run(async {
        let Some(world) = world().await else { return };
        let account = AccountId::from_bytes(payer(1).as_bytes()).unwrap();
        let owner = ImageOwner::Draft { account };
        let job = ImageJobId::from_bytes(&[2; 16]).unwrap();
        let stage = ExecutionStageId::from_bytes(&[30; 16]).unwrap();
        let dispatch = DispatchId::from_bytes(&[32; 16]).unwrap();
        let now = SystemClock.now_ms();
        let (journal, accepted, config) = demand(&world, account, now).await;
        let deadline = accepted.absolute_deadline_ms;
        let scope = StageScope {
            owner: owner.clone(),
            job,
            stage,
        };
        let capacity = CapacityScope::new("funded-adapter".into()).unwrap();
        let token = ClaimToken::from_bytes(&[31; 16]).unwrap();
        let basis = StageBasis {
            kind: StageKind::Classification,
            input_revision: 1,
            input_digest: [3; 32],
            config_revision: 1,
            config_digest: df_commerce::digest::sha256(accepted.config.bytes()),
            parent_dispatch: None,
        };
        let mut client = world.scratch.connect().await;
        let tx = client.transaction().await.unwrap();
        execution::set_capacity_bounds_in(
            &tx,
            capacity.as_str(),
            &CapacityBounds::new(10, 20, 1_048_576).unwrap(),
            now,
        )
        .await
        .unwrap();
        execution::set_execution_gate_in(&tx, capacity.as_str(), 1, true, now)
            .await
            .unwrap();
        execution::enqueue_stage_in(
            &tx,
            &EnqueueCommand::new(
                scope.clone(),
                basis.clone(),
                ExecutionIdentity::new([20; 16], [21; 16]).unwrap(),
                capacity,
                false,
                now,
            )
            .unwrap(),
        )
        .await
        .unwrap();
        let claimed = execution::claim_stage_in(
            &tx,
            &ClaimCommand::new(
                owner.clone(),
                job,
                stage,
                1,
                token,
                (accepted.absolute_deadline_ms - now).min(60_000),
                now,
            )
            .unwrap(),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        drop(client);
        let claim = StageClaim {
            scope,
            token,
            generation: claimed.claim_generation,
        };
        let route = StageRoute {
            key: [1; 32],
            kind: PaidStageKind::Classification,
            provider: "openai".into(),
            model: "gpt-6-luna".into(),
            route: "portrait".into(),
            cause: "creator_portrait".into(),
            detail: "portrait".into(),
            price_version: [0x70; 16],
            maximum_units: vec![("text_input".into(), 100), ("text_output".into(), 100)],
        };
        let grant = JobConsent {
            id: ConsentId([90; 16]),
            scope: JobScope {
                account,
                draft: DraftId([1; 16]),
                draft_revision: 1,
                job: JobId([2; 16]),
                source_digest: basis.input_digest,
                config_revision: 1,
                config_digest: basis.config_digest,
                routes: vec![route.clone()],
            },
            cap_micro_dollars: 500,
            expires_at_ms: deadline,
            revision: 1,
        };
        let client = world.scratch.connect().await;
        client
            .batch_execute("SET ROLE df_imagery_admission")
            .await
            .unwrap();
        let mut finance = FinanceService::new(
            client,
            FakeFence::new(FenceVerdict::Current),
            FakeMedia::new(MediaVerdict::Admitted),
            FakeMembers::new(vec![]),
            Box::new(SystemClock),
            Box::new(SeededIds::new(90_000)),
            FinanceConfig::default(),
        );
        finance
            .record_draft_job_consent(account, &grant)
            .await
            .unwrap();
        world
            .admin
            .execute(
                "INSERT INTO public.funded_adapter_authority VALUES($1,true)",
                &[&grant.id.0.as_slice()],
            )
            .await
            .unwrap();
        let cost = df_imagery::cost::CostRoute {
            provider: route.provider.clone(),
            model: route.model.clone(),
            cause: route.cause.clone(),
            detail: route.detail.clone(),
            maxima: route
                .maximum_units
                .iter()
                .map(|(category, quantity)| df_imagery::cost::CostUnits {
                    category: category.clone(),
                    quantity: *quantity,
                })
                .collect(),
        };
        let finance = Arc::new(Mutex::new(finance));
        let adapter = FundedImageAdmission::new(
            RuntimeAdmissionSettings {
                dispatcher: [88; 16],
                quote_lifetime_ms: 300_000,
                catalog: df_assets::CatalogConfig::default(),
            },
            RuntimeAdmissionPorts {
                finance: finance.clone(),
                authority: Authority { account, grant },
                local: Arc::new(NoLocal),
                usage: crate::imagery_runtime::PostgresImageUsage::open_existing(
                    world.scratch.config(),
                    PostgresTransport::PlaintextLoopbackOnly,
                    1,
                )
                .unwrap(),
            },
        )
        .unwrap();
        let admitted_job = journal.read_job(&owner, job).await.unwrap().unwrap();
        let input = DispatchAdmission {
            job: &admitted_job,
            stage: &claimed,
            dispatch,
            claim: token,
            config: &config,
            quote: Some(&cost),
            sources: &[],
        };
        let mode = adapter.admit(&input).await.unwrap();
        assert!(matches!(mode, DispatchMode::Hosted(_)));
        let after_admission = snapshot_rows(&world).await;
        assert_eq!(adapter.admit(&input).await.unwrap(), mode);
        assert_eq!(snapshot_rows(&world).await, after_admission);
        {
            let finance = finance.lock().await;
            let recovered = records::read_admitted_funding(&finance, &admitted_job, dispatch)
                .await
                .unwrap();
            assert_eq!(recovered.call().payload_digest, basis.input_digest);
            assert_eq!(recovered.finance().unwrap(), mode.finance().unwrap());
        }
        let source_row = world
            .admin
            .query_one(
                "SELECT receipts::text FROM imagery.image_dispatch_sources WHERE dispatch_id=$1",
                &[&dispatch.as_bytes().as_slice()],
            )
            .await
            .unwrap();
        assert_eq!(source_row.get::<_, String>(0), "[]");
        assert_eq!(
            world
                .admin
                .query_one("SELECT count(*) FROM media.asset", &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            0
        );
        world
            .admin
            .execute(
                "UPDATE public.funded_adapter_authority SET active=false",
                &[],
            )
            .await
            .unwrap();
        let before = snapshot_rows(&world).await;
        assert!(matches!(
            adapter.mark_sent(&input, mode).await,
            Err(RuntimeError::InvalidRecord)
        ));
        assert_eq!(snapshot_rows(&world).await, before);
        world
            .admin
            .execute(
                "UPDATE public.funded_adapter_authority SET active=true",
                &[],
            )
            .await
            .unwrap();
        assert_eq!(
            adapter.mark_sent(&input, mode).await.unwrap(),
            SendAuthorization::New
        );
        let states = world.admin.query_one(
            "SELECT d.dispatch_state,a.dispatch_state FROM imagery.image_dispatch_attempt d \
             JOIN finance.provider_attempt a ON a.attempt_id=d.finance_attempt WHERE d.dispatch_id=$1",
            &[&dispatch.as_bytes().as_slice()],
        ).await.unwrap();
        assert_eq!(
            (states.get::<_, String>(0), states.get::<_, String>(1)),
            ("sent".into(), "sent".into())
        );
        let sent = snapshot_rows(&world).await;
        assert_eq!(
            adapter.mark_sent(&input, mode).await.unwrap(),
            SendAuthorization::AlreadySent
        );
        assert_eq!(snapshot_rows(&world).await, sent);
        // Missing explicit empty receipts are a conflict, never silently recreated on replay.
        world
            .admin
            .execute(
                "DELETE FROM imagery.image_dispatch_sources WHERE dispatch_id=$1",
                &[&dispatch.as_bytes().as_slice()],
            )
            .await
            .unwrap();
        let missing = snapshot_rows(&world).await;
        assert!(matches!(
            adapter.mark_sent(&input, mode).await,
            Err(RuntimeError::InvalidRecord)
        ));
        assert_eq!(snapshot_rows(&world).await, missing);
        // Semantic success never turns uncertain billing into a definitive settlement.
        let raw = b"{\"result\":\"valid\"}".to_vec();
        adapter
            .ports
            .usage
            .record_usage(&RuntimeUsage {
                owner: owner.clone(),
                job,
                stage,
                dispatch,
                revision: UsageRevision(1),
                route: cost.clone(),
                evidence: UsageEvidence::Unknown,
                units: cost.maxima.clone(),
                usable: true,
                at_ms: u64::try_from(now).unwrap(),
                raw_output: Some(raw.clone()),
            })
            .await
            .unwrap();
        let mut client = world.scratch.connect().await;
        let tx = client.transaction().await.unwrap();
        execution::record_decision_observation_in(
            &tx,
            &DecisionObservation::new(
                DecisionScope {
                    owner: owner.clone(),
                    job,
                    stage,
                    dispatch,
                },
                DecisionKind::Classification,
                1,
                DecisionBasis {
                    schema_revision: 1,
                    validator_revision: 1,
                    stage: basis.clone(),
                },
                raw,
                DecisionReceipt {
                    usage_revision: 1,
                    observed_at_ms: now,
                },
                1024,
            )
            .unwrap(),
        )
        .await
        .unwrap();
        execution::complete_dispatch_in(
            &tx,
            &CompleteCommand::new(
                claim,
                dispatch,
                CompletionOutcome::Decision {
                    observation_version: 1,
                    usage_revision: 1,
                },
                SystemClock.now_ms(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        drop(client);
        world
            .admin
            .execute(
                "UPDATE public.funded_adapter_authority SET active=false",
                &[],
            )
            .await
            .unwrap();
        adapter
            .outcome(&admitted_job, dispatch, mode, Some(true))
            .await
            .unwrap();
        let held = snapshot_rows(&world).await;
        adapter
            .outcome(&admitted_job, dispatch, mode, None)
            .await
            .unwrap();
        assert_eq!(snapshot_rows(&world).await, held);
        let state = world.admin.query_one(
            "SELECT dispatch_state,settlement FROM imagery.image_dispatch_attempt WHERE dispatch_id=$1",
            &[&dispatch.as_bytes().as_slice()],
        ).await.unwrap();
        assert_eq!(
            (state.get::<_, String>(0), state.get::<_, String>(1)),
            ("succeeded".into(), "unknown_outstanding".into())
        );
        let raw = adapter
            .ports
            .usage
            .latest_usage(&owner, dispatch)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(raw.evidence, UsageEvidence::Unknown);
        assert_eq!(raw.units, cost.maxima);
        assert!(raw.usable);
        assert_eq!(world.natural(2150, Some(payer(1)), None, None).await, 90);
        assert_eq!(world.line_sum(9100).await, 60);
        drop(adapter);
        drop(finance);
        drop(journal);
        world.finish().await;
    });
}

#[path = "cancellation_tests.rs"]
mod cancellation_tests;

#[path = "no_output_tests.rs"]
mod no_output_tests;

#[path = "runtime_cleanup_tests.rs"]
mod runtime_cleanup_tests;
