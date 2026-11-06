//! The real native cleanup producer seals terminal authority before canonical unused release.
use super::*;
use df_assets::IdSource;
use df_finance::ports::Clock as _;
use df_imagery::cost::*;
use df_imagery::execution::DispatchId;
use df_imagery::runtime::*;
use df_imagery::{AccountHandle, BackendId, ImageJob, SourceRef};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
#[derive(Clone)]
struct Ids(Arc<AtomicU64>);
impl IdSource for Ids {
    fn next_id_bytes(&self) -> [u8; 16] {
        let next = self.0.fetch_add(1, Ordering::SeqCst) + 1;
        let mut bytes = [0; 16];
        bytes[8..].copy_from_slice(&next.to_be_bytes());
        bytes
    }
}
#[derive(Clone)]
struct Clock;
impl df_assets::Clock for Clock {
    fn now_ms(&self) -> i64 {
        df_finance::ports::Clock::now_ms(&SystemClock)
    }
}
struct NoCalls;
impl ImageSources for NoCalls {
    async fn read_source(
        &self,
        _: &ImageOwner,
        _: &SourceRef,
    ) -> Result<AuthorizedImageSource, RuntimeError> {
        panic!("Terminal cleanup requested external source")
    }
}
impl ImageCredentials for NoCalls {
    fn credential(&self, _: BackendId, _: &AccountHandle) -> Result<String, RuntimeError> {
        panic!("Terminal cleanup requested supplier credential")
    }
}
impl ImageRuntimeCostPolicy for NoCalls {
    fn route(
        &self,
        _: &ImageJob,
        _: &ImageConfig,
        _: BackendId,
        _: &str,
    ) -> Result<CostRoute, RuntimeError> {
        panic!("Terminal cleanup requested supplier route")
    }
}
impl ImageCostProjection for NoCalls {
    fn start_pinned<'a>(
        &'a self,
        _: &'a str,
        _: &'a CostRoute,
    ) -> CostFuture<'a, Result<PinnedStart, CostError>> {
        Box::pin(async { panic!("Terminal cleanup started billed work") })
    }
    fn correct<'a>(
        &'a self,
        _: &'a str,
        _: &'a UsageReport<'a>,
    ) -> CostFuture<'a, Result<PinnedCorrection, CostError>> {
        Box::pin(async { panic!("Terminal cleanup projected billed usage") })
    }
    fn record_unsent<'a>(&'a self, _: &'a str) -> CostFuture<'a, Result<PinnedUnsent, CostError>> {
        Box::pin(async { panic!("Terminal cleanup released money") })
    }
    fn is_durable<'a>(
        &'a self,
        _: &'a str,
        _: Option<UsageRevision>,
    ) -> CostFuture<'a, Result<bool, CostError>> {
        Box::pin(async { panic!("Terminal cleanup read billed usage") })
    }
}
#[derive(Clone, Default)]
struct Observations(Arc<std::sync::Mutex<Vec<df_imagery::telemetry::ImageEvent>>>);
impl ImageRuntimeTelemetry for Observations {
    fn observe(&self, event: df_imagery::telemetry::ImageEvent) {
        self.0.lock().unwrap().push(event);
    }
    fn record(
        &self,
        _: &ImageJob,
        _: &StageView,
        _: Option<DispatchId>,
        _: RuntimePhase,
        _: i64,
    ) -> Result<(), RuntimeError> {
        Ok(())
    }
}

#[test]
fn native_cleanup_preserves_missing_book_ack_after_exact_money_release_and_restart() {
    support::run(async {
        let Some(world) = world().await else { return };
        for migration in [
            crate::imagery_publication::IMAGERY_PUBLICATION_MIGRATION_SQL,
            include_str!("../../migrations/0045_imagery_compatible.sql"),
            include_str!("../../migrations/0046_imagery_analysis.sql"),
        ] {
            world.admin.batch_execute(migration).await.unwrap();
        }
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
        let capacity = CapacityScope::new("funded-adapter").unwrap();
        let token = ClaimToken::from_bytes(&[31; 16]).unwrap();
        let basis = StageBasis {
            kind: StageKind::Classification,
            input_revision: accepted.intent.revision(),
            input_digest: df_commerce::digest::sha256(accepted.intent.bytes()),
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
                capacity.clone(),
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

        assert_eq!(world.natural(2150, Some(payer(1)), None, None).await, 90);
        assert_eq!(world.line_sum(9100).await, 60);
        let publication = crate::imagery_publication::PostgresImagePublication::open_existing(
            world.scratch.config(),
            PostgresTransport::PlaintextLoopbackOnly,
            1,
        )
        .await
        .unwrap();
        assert!(
            publication
                .read_no_delivery(&owner, &job)
                .await
                .unwrap()
                .is_none()
        );
        journal
            .withdraw_interest(
                &owner,
                ImageInterestId::from_bytes(&[10; 16]).unwrap(),
                SystemClock.now_ms(),
            )
            .await
            .unwrap();
        let withdrawn = journal.read_job(&owner, job).await.unwrap().unwrap();
        assert_eq!(withdrawn.state, df_imagery::JobState::Retired);
        let current = execution::PostgresImageExecution::open_existing(
            world.scratch.config(),
            PostgresTransport::PlaintextLoopbackOnly,
            1,
        )
        .await
        .unwrap()
        .read_dispatch(&owner, &dispatch)
        .await
        .unwrap();
        assert_eq!(current.state, DispatchState::Prepared);
        assert_eq!(current.settlement, DispatchSettlement::Held);
        assert_eq!(world.natural(2150, Some(payer(1)), None, None).await, 90);
        let adapter = Arc::new(adapter);
        let objects = Arc::new(
            df_assets::NativeFileStore::new(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../artifacts/tmp/funding-runtime-cleanup")
                    .join(world.scratch.config().get_dbname().unwrap()),
                16_777_216,
            )
            .unwrap(),
        );
        let ids = Ids(Arc::new(AtomicU64::new(190_000)));
        let observations = Observations::default();
        let mut settled_snapshot = None;
        let mut sealed = None;
        for _restart in 0..2 {
            let execution = execution::PostgresImageExecution::open_existing(
                world.scratch.config(),
                PostgresTransport::PlaintextLoopbackOnly,
                1,
            )
            .await
            .unwrap();
            let catalog = Arc::new(
                crate::media_catalog::PostgresMediaCatalog::open_existing(
                    world.scratch.config(),
                    PostgresTransport::PlaintextLoopbackOnly,
                    1,
                )
                .await
                .unwrap(),
            );
            let reopened_publication =
                crate::imagery_publication::PostgresImagePublication::open_existing(
                    world.scratch.config(),
                    PostgresTransport::PlaintextLoopbackOnly,
                    1,
                )
                .await
                .unwrap();
            let runtime = NativeImageRuntime {
                journal: journal.clone(),
                execution,
                sources: NoCalls,
                publisher: crate::imagery_publication::NativeImagePublisher::new(
                    crate::imagery_publication::RuntimePublicationSettings {
                        catalog: df_assets::CatalogConfig::default(),
                        layout: df_assets::KeyLayout::new(
                            df_assets::ObjectEnvironment::new("cleanup-test").unwrap(),
                        ),
                        decode_concurrency: 1,
                    },
                    crate::imagery_publication::RuntimePublicationPorts {
                        catalog,
                        publication: reopened_publication,
                        objects: objects.clone(),
                        ids: Arc::new(ids.clone()),
                        clock: Arc::new(Clock),
                    },
                )
                .unwrap(),
                admission: adapter.clone(),
                clock: Clock,
                ids: ids.clone(),
                credentials: NoCalls,
                costs: NoCalls,
                telemetry: observations.clone(),
                cost_policy: NoCalls,
                usage: crate::imagery_runtime::PostgresImageUsage::open_existing(
                    world.scratch.config(),
                    PostgresTransport::PlaintextLoopbackOnly,
                    1,
                )
                .unwrap(),
                settings: RuntimeSettings {
                    config: config.clone(),
                    active_config_revision: 1,
                    capacity: capacity.clone(),
                    poll_interval: Duration::from_millis(5),
                    codex_home: None,
                },
            };
            assert!(matches!(
                Box::pin(runtime.run_next()).await,
                Err(RuntimeError::Cost(CostError::UnsentSealUnavailable))
            ));
            let pending = runtime
                .execution
                .scan_unsent_prepared(capacity.as_str(), None, 1, SystemClock.now_ms())
                .await
                .unwrap();
            assert!(
                pending.iter().any(|entry| entry.scope.dispatch == dispatch),
                "without an authoritative Book seal the released supplier obligation remains discoverable"
            );
            drop(runtime);
            let receipt = publication
                .read_no_delivery(&owner, &job)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                receipt.tail,
                df_imagery::publication::NoDeliveryStage::Dispatch {
                    scope: ExecutionScope {
                        owner: owner.clone(),
                        job,
                        stage,
                        dispatch
                    },
                    usage_revision: 0,
                }
            );
            assert_eq!(receipt.actor, receipt.tail);
            assert_eq!(receipt.reason, NoDeliveryReason::Retired);
            assert_eq!(receipt.expected_job_revision, withdrawn.revision);
            assert_eq!(receipt.job_revision, withdrawn.revision + 1);
            assert_eq!(world.natural(2150, Some(payer(1)), None, None).await, 0);
            assert_eq!(world.line_sum(9100).await, 0);
            let row=world.admin.query_one("SELECT d.dispatch_state,d.settlement,a.dispatch_state,r.charged_micro_dollars,r.released_micro_dollars,(SELECT count(*) FROM imagery.image_dispatch_usage WHERE dispatch_id=d.dispatch_id),d.cost_unsent_ack FROM imagery.image_dispatch_attempt d JOIN finance.provider_attempt a ON a.attempt_id=d.finance_attempt JOIN finance.reservation r USING(reservation_id) WHERE d.dispatch_id=$1",&[&dispatch.as_bytes().as_slice()]).await.unwrap();
            assert_eq!(row.get::<_, String>(0), "released");
            assert_eq!(row.get::<_, String>(1), "released");
            assert_eq!(row.get::<_, String>(2), "cancelled_unsent");
            assert_eq!(
                (
                    row.get::<_, i64>(3),
                    row.get::<_, i64>(4),
                    row.get::<_, i64>(5)
                ),
                (0, 90, 0)
            );
            assert!(
                !row.get::<_, bool>(6),
                "missing Book capability cannot acknowledge unsent cost"
            );
            {
                let events = observations.0.lock().unwrap();
                let cancellations = events
                    .iter()
                    .filter(|event| event.kind == df_imagery::telemetry::EventKind::Cancelled)
                    .collect::<Vec<_>>();
                assert_eq!(cancellations.len(), 1);
                assert_eq!(cancellations[0].context.job, Some(job));
                assert_eq!(
                    cancellations[0].measurements.supplier_dispatched,
                    Some(false)
                );
                assert_eq!(cancellations[0].measurements.raw_checkpointed, Some(false));
                let settlements = events
                    .iter()
                    .filter(|event| {
                        event.kind == df_imagery::telemetry::EventKind::SettlementRecorded
                    })
                    .collect::<Vec<_>>();
                assert!(!settlements.is_empty());
                assert!(
                    settlements
                        .iter()
                        .all(|event| event.context.attempt == Some(dispatch)
                            && event.measurements.settlement == Some(DispatchSettlement::Released))
                );
            }
            let snapshot = snapshot_rows(&world).await;
            if let Some(prior) = &settled_snapshot {
                assert_eq!(
                    &snapshot, prior,
                    "worker restart cannot repair or release twice"
                );
            }
            if let Some(prior) = &sealed {
                assert_eq!(
                    &receipt, prior,
                    "worker restart retains exact protected NoDelivery"
                );
            }
            settled_snapshot = Some(snapshot);
            sealed = Some(receipt);
        }
        drop(adapter);
        drop(finance);
        drop(publication);
        drop(journal);
        world.finish().await;
    });
}
