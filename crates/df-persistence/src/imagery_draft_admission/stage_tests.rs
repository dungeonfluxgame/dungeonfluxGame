use super::super::{
    CanonicalStagePort, CanonicalStageSend, CanonicalStageSettlement, StageAllocation,
    StageAuthority, StageConfig, StagePayer,
};
use super::*;
use crate::imagery_execution as execution;
use df_commerce::draft_image::{JobConsent, JobScope, PaidStageKind, StageCall, StageRoute};
use df_finance::draft_image::{
    StageQuoteCommand, StageSendPort, StageSettlementCommand, StageUsageEvidence, StoredStageQuote,
    Usability,
};
use df_imagery::cost::{CostRoute, CostUnits, UsageEvidence, UsageRevision};
use df_imagery::execution::*;
use df_imagery::runtime::{
    ImageUsageJournal, PublicationDecisionEvidence, QualifiedDecision, RuntimeUsage,
    qualify_economic_decision,
};
#[path = "../../tests/imagery_publication/intent_fixture.rs"]
mod intent_fixture;
#[path = "stage_tests/native_source.rs"]
pub(crate) mod native_source;
const NOW: i64 = 1_700_000_000_000;

struct StageAuth {
    refuse: bool,
}
impl StageAuthority for StageAuth {
    async fn verify(
        &self,
        _tx: &Transaction<'_>,
        _payer: StagePayer<'_>,
        _owner: &ImageOwner,
        _call: &StageCall,
        _replay: bool,
    ) -> Result<(), Error> {
        if self.refuse {
            Err(Error::LinkConflict)
        } else {
            Ok(())
        }
    }
    async fn record_prepared_sources(
        &self,
        tx: &Transaction<'_>,
        owner: &ImageOwner,
        call: &StageCall,
        replay: bool,
    ) -> Result<(), Error> {
        // The actual snapshot helper fingerprints the exact no-source intent and verifies Prepared.
        let scope = ExecutionScope {
            owner: owner.clone(),
            job: ImageJobId::from_bytes(&call.job.0).map_err(|_| Error::LinkConflict)?,
            stage: ExecutionStageId::from_bytes(&call.stage.0).map_err(|_| Error::LinkConflict)?,
            dispatch: DispatchId::from_bytes(&call.dispatch).map_err(|_| Error::LinkConflict)?,
        };
        if replay {
            let receipts = crate::imagery_runtime::read_sources_in(tx, &scope)
                .await
                .map_err(|_| Error::LinkConflict)?
                .ok_or(Error::LinkConflict)?;
            if !receipts.is_empty() {
                return Err(Error::LinkConflict);
            }
        } else {
            crate::imagery_runtime::record_sources_in(tx, &scope, &[])
                .await
                .map_err(|_| Error::LinkConflict)?;
        }
        Ok(())
    }
}
struct AtomicSender {
    canonical: CanonicalStageSend<StageAuth>,
}
impl StageSendPort for AtomicSender {
    async fn verify(
        &self,
        tx: &Transaction<'_>,
        quote: &StoredStageQuote,
        new_send: bool,
    ) -> Result<(), Error> {
        StageSendPort::verify(&self.canonical, tx, quote, new_send).await?;
        let binding = quote.binding();
        let call = quote.call();
        let finance =
            FinanceTuple::new(binding.attempt, binding.operation, binding.effect).unwrap();
        let dispatch = DispatchId::from_bytes(&call.dispatch).unwrap();
        if new_send {
            let claim = &self.canonical.claim;
            let command = MarkSentCommand::new(
                claim.scope.owner.clone(),
                claim.scope.job,
                claim.scope.stage,
                dispatch,
                claim.token,
                claim.generation,
                SystemClock.now_ms(),
            )
            .unwrap();
            execution::mark_hosted_sent_in(tx, &command, &finance)
                .await
                .map_err(|_| Error::LinkConflict)?;
        } else {
            let stored =
                execution::read_dispatch_in(tx, &self.canonical.claim.scope.owner, &dispatch)
                    .await
                    .map_err(|_| Error::LinkConflict)?;
            if stored.state != DispatchState::Sent || stored.finance != Some(finance) {
                return Err(Error::LinkConflict);
            }
        }
        Ok(())
    }
}
struct Pipeline {
    world: World,
    service: support::world::Service,
    owner: ImageOwner,
    job: ImageJobId,
    config: VersionedSnapshot,
    recipe: VersionedSnapshot,
    key: MediaRequestKey,
    grant: JobConsent,
    capacity: CapacityScope,
    now: i64,
    deadline: i64,
}
struct PaidStage {
    quote: StoredStageQuote,
    claim: StageClaim,
    basis: StageBasis,
}
impl Pipeline {
    async fn new() -> Option<Self> {
        let world = world().await?;
        world
            .admin
            .batch_execute(crate::imagery_runtime::IMAGERY_USAGE_MIGRATION_SQL)
            .await
            .unwrap();
        let owner = ImageOwner::Draft { account: account() };
        let job = ImageJobId::from_bytes(&[2; 16]).unwrap();
        let mut pipeline =
            intent_fixture::inspection_pipeline(df_imagery::InspectionRequirement::Required);
        let route = &mut pipeline.inspection.as_mut().unwrap().route;
        route.model = df_imagery::ModelName::new("gpt-6-luna").unwrap();
        route.maximum_output_tokens = 100;
        pipeline.creator = Some(df_imagery::CreatorClassification {
            route: route.clone(),
            contract_revision: 1,
            public_appearance: df_imagery::PrivateImageText::new("An adult fictional traveler."),
            general_prompt: None,
        });
        let (intent, config) = intent_fixture::pipeline_snapshots(&owner, pipeline).await;
        let recipe = snapshot(5);
        let key = MediaRequestKey::from_bytes([6; 32]);
        let now = SystemClock.now_ms();
        let deadline = now + 600_000;
        let capacity = CapacityScope::new("funded-stages").unwrap();
        let mut admin = world.scratch.connect().await;
        let tx = admin.transaction().await.unwrap();
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
        tx.commit().await.unwrap();
        drop(admin);
        let journal = crate::PostgresImageJournal::open(
            world.scratch.config(),
            crate::postgres_tls::PostgresTransport::PlaintextLoopbackOnly,
            1,
        )
        .await
        .unwrap();
        journal
            .accept(&AcceptCommand {
                identity: OperationIdentity {
                    owner: owner.clone(),
                    operation: OperationId::from_bytes(&[7; 16]).unwrap(),
                    fingerprint: [8; 32],
                },
                intent,
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
        drop(journal);
        let route = |tag, kind| StageRoute {
            key: [tag; 32],
            kind,
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
                account: account(),
                draft: DraftId([1; 16]),
                draft_revision: 1,
                job: JobId([2; 16]),
                source_digest: [3; 32],
                config_revision: 1,
                config_digest: df_commerce::digest::sha256(config.bytes()),
                routes: vec![
                    route(1, PaidStageKind::Classification),
                    route(2, PaidStageKind::Generation),
                    route(3, PaidStageKind::Inspection),
                    route(4, PaidStageKind::Correction),
                    route(5, PaidStageKind::Extraction),
                ],
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
        let mut service = FinanceService::new(
            client,
            FakeFence::new(FenceVerdict::Current),
            FakeMedia::new(MediaVerdict::Admitted),
            FakeMembers::new(vec![]),
            Box::new(SystemClock),
            Box::new(SeededIds::new(90_000)),
            FinanceConfig::default(),
        );
        service
            .record_draft_job_consent(account(), &grant)
            .await
            .unwrap();
        Some(Self {
            world,
            service,
            owner,
            job,
            config,
            recipe,
            key,
            grant,
            capacity,
            now,
            deadline,
        })
    }
    async fn stage(&mut self, kind: StageKind, tag: u8, parent: Option<DispatchId>) -> PaidStage {
        let stage = ExecutionStageId::from_bytes(&[tag; 16]).unwrap();
        let token = ClaimToken::from_bytes(&[tag + 1; 16]).unwrap();
        let basis = StageBasis {
            kind,
            input_revision: 1,
            input_digest: [3; 32],
            config_revision: 1,
            config_digest: self.grant.scope.config_digest,
            parent_dispatch: parent,
        };
        let scope = StageScope {
            owner: self.owner.clone(),
            job: self.job,
            stage,
        };
        let mut client = self.world.scratch.connect().await;
        let tx = client.transaction().await.unwrap();
        execution::enqueue_stage_in(
            &tx,
            &EnqueueCommand::new(
                scope.clone(),
                basis.clone(),
                ExecutionIdentity::new([20; 16], [21; 16]).unwrap(),
                self.capacity.clone(),
                false,
                self.now,
            )
            .unwrap(),
        )
        .await
        .unwrap();
        let claimed = execution::claim_stage_in(
            &tx,
            &ClaimCommand::new(
                self.owner.clone(),
                self.job,
                stage,
                1,
                token,
                600_000,
                self.now,
            )
            .unwrap(),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        drop(client);
        let route = self
            .grant
            .scope
            .routes
            .iter()
            .find(|route| route.kind.label() == kind.key())
            .unwrap()
            .clone();
        let call = StageCall {
            job: self.grant.scope.job,
            stage: StageId([tag; 16]),
            dispatch: [tag + 2; 16],
            parent_dispatch: parent.map(|id| *id.as_bytes()),
            kind: route.kind,
            input_revision: 1,
            input_digest: [3; 32],
            config_revision: 1,
            config_digest: self.grant.scope.config_digest,
            planned_publication: Some(df_commerce::draft_image::StagePublication {
                asset: AssetId([20; 16]),
                publication: PublicationId([21; 16]),
            }),
            payload_digest: [tag; 32],
        };
        let quote = self
            .service
            .quote_draft_stage(
                account(),
                &StageQuoteCommand {
                    consent: self.grant.id,
                    consent_revision: 1,
                    scope: self.grant.scope.clone(),
                    call,
                    route,
                    quote_lifetime_ms: 300_000,
                },
            )
            .await
            .unwrap();
        let claim = StageClaim {
            scope,
            token,
            generation: claimed.claim_generation,
        };
        PaidStage {
            quote,
            claim,
            basis,
        }
    }
    fn output_config(&self, revision: u64) -> Config {
        Config {
            admission: Admission {
                asset: df_assets::AssetId::from_bytes(&[20; 16]).unwrap(),
                attempt: df_assets::AttemptId::from_bytes(&[21; 16]).unwrap(),
                kind: AssetKind::Image,
                provenance: Provenance::Generated,
                charge_scope: QuotaScope::Account(account()),
                origin_account: Some(account()),
                campaign: None,
                audience: Audience::OwnerPrivate(account()),
                dedupe_scope: DedupeScope::account(&account()),
                request_key: Some(self.key),
                retention: RetentionClass::Personal,
                reserve_bytes: 100,
                byte_limit: 1_000,
                in_flight_upload_limit: None,
            },
            recipe_key: self.key,
            recipe: self.recipe.clone(),
            config: self.config.clone(),
            expected_job_revision: revision,
            absolute_deadline_ms: self.deadline,
            now_ms: self.now,
        }
    }
    async fn admit(&mut self, stage: &PaidStage) {
        let revision: i64 = self
            .world
            .admin
            .query_one(
                "SELECT revision FROM imagery.image_job WHERE job_id=$1",
                &[&self.job.as_bytes().as_slice()],
            )
            .await
            .unwrap()
            .get(0);
        let allocation = if stage.basis.kind == StageKind::Classification {
            StageAllocation::Decision
        } else {
            StageAllocation::Output(Box::new(
                self.output_config(u64::try_from(revision).unwrap()),
            ))
        };
        let port = CanonicalStagePort::new(
            StageAuth { refuse: false },
            StageConfig {
                claim: stage.claim.clone(),
                allocation,
                now_ms: self.now,
            },
        )
        .unwrap();
        self.service
            .admit_draft_stage(account(), &stage.quote, &port)
            .await
            .unwrap();
        assert!(
            self.service
                .admit_draft_stage(account(), &stage.quote, &port)
                .await
                .unwrap()
                .money
                .replay
        );
    }
    async fn send(&mut self, stage: &PaidStage) {
        assert!(matches!(
            self.service
                .prepare_draft_image(account(), stage.quote.attempt())
                .await,
            Err(Error::Refused(
                df_finance::draft_image::AdmissionRefusal::BindingChanged
            ))
        ));
        let refused = AtomicSender {
            canonical: CanonicalStageSend {
                authority: StageAuth { refuse: true },
                claim: stage.claim.clone(),
                now_ms: self.now,
            },
        };
        assert!(matches!(
            self.service
                .mark_draft_stage_sent(account(), &stage.quote, [88; 16], &refused)
                .await,
            Err(Error::LinkConflict)
        ));
        let allowed = AtomicSender {
            canonical: CanonicalStageSend {
                authority: StageAuth { refuse: false },
                claim: stage.claim.clone(),
                now_ms: self.now,
            },
        };
        assert!(
            self.service
                .mark_draft_stage_sent(account(), &stage.quote, [88; 16], &allowed)
                .await
                .unwrap()
        );
        assert!(
            !self
                .service
                .mark_draft_stage_sent(account(), &stage.quote, [88; 16], &allowed)
                .await
                .unwrap()
        );
    }
    async fn observe(
        &self,
        stage: &PaidStage,
        evidence: UsageEvidence,
        usable: bool,
        raw: Option<Vec<u8>>,
    ) {
        let values = if evidence == UsageEvidence::Unknown {
            stage.quote.route().maximum_units.clone()
        } else {
            vec![("text_input".into(), 50), ("text_output".into(), 50)]
        };
        self.observe_units(stage, evidence, usable, raw, values)
            .await;
    }
    async fn observe_units(
        &self,
        stage: &PaidStage,
        evidence: UsageEvidence,
        usable: bool,
        raw: Option<Vec<u8>>,
        values: Vec<(String, u64)>,
    ) {
        let route = stage.quote.route();
        let journal = crate::imagery_runtime::PostgresImageUsage::open_existing(
            self.world.scratch.config(),
            crate::postgres_tls::PostgresTransport::PlaintextLoopbackOnly,
            1,
        )
        .unwrap();
        journal
            .record_usage(&RuntimeUsage {
                owner: self.owner.clone(),
                job: self.job,
                stage: stage.claim.scope.stage,
                dispatch: DispatchId::from_bytes(&stage.quote.call().dispatch).unwrap(),
                revision: UsageRevision(1),
                route: CostRoute {
                    provider: route.provider.clone(),
                    model: route.model.clone(),
                    cause: route.cause.clone(),
                    detail: route.detail.clone(),
                    maxima: route
                        .maximum_units
                        .iter()
                        .rev()
                        .map(|(category, quantity)| CostUnits {
                            category: category.clone(),
                            quantity: *quantity,
                        })
                        .collect(),
                },
                evidence,
                units: values
                    .into_iter()
                    .rev()
                    .map(|(category, quantity)| CostUnits { category, quantity })
                    .collect(),
                usable,
                at_ms: u64::try_from(self.now).unwrap(),
                raw_output: raw,
            })
            .await
            .unwrap();
    }
    async fn finish_decision(&self, stage: &PaidStage) {
        let raw = decision_raw(stage.basis.kind);
        self.observe(stage, UsageEvidence::Reported, true, Some(raw.clone()))
            .await;
        self.select_decision(stage).await;
    }
    async fn select_decision(&self, stage: &PaidStage) {
        let payload = decision_payload(stage.basis.kind);
        let dispatch = DispatchId::from_bytes(&stage.quote.call().dispatch).unwrap();
        let mut client = self.world.scratch.connect().await;
        let tx = client.transaction().await.unwrap();
        let kind = if stage.basis.kind == StageKind::Classification {
            DecisionKind::Classification
        } else {
            DecisionKind::Inspection
        };
        execution::record_decision_observation_in(
            &tx,
            &DecisionObservation::new(
                DecisionScope {
                    owner: self.owner.clone(),
                    job: self.job,
                    stage: stage.claim.scope.stage,
                    dispatch,
                },
                kind,
                1,
                DecisionBasis {
                    schema_revision: 1,
                    validator_revision: 1,
                    stage: stage.basis.clone(),
                },
                payload,
                DecisionReceipt {
                    usage_revision: 1,
                    observed_at_ms: SystemClock.now_ms(),
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
                stage.claim.clone(),
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
    }
    async fn settle(
        &mut self,
        stage: &PaidStage,
        evidence: StageUsageEvidence,
        usable: bool,
    ) -> Result<df_finance::draft_image::Settlement, Error> {
        let values = if evidence == StageUsageEvidence::Unknown {
            stage.quote.route().maximum_units.clone()
        } else {
            vec![("text_input".into(), 50), ("text_output".into(), 50)]
        };
        let command = StageSettlementCommand {
            quote: stage.quote.clone(),
            usage_revision: 1,
            evidence,
            actual_units: values,
            usability: if usable {
                Usability::Usable
            } else {
                Usability::NoUsableOutput
            },
        };
        let port = CanonicalStageSettlement {
            owner: self.owner.clone(),
            now_ms: SystemClock.now_ms(),
        };
        if evidence != StageUsageEvidence::Unknown
            && matches!(
                stage.basis.kind,
                StageKind::Classification | StageKind::Inspection
            )
        {
            let proof = self.decision_proof(stage).await?;
            self.service
                .settle_draft_stage(account(), &command, &port.with_decision(&proof))
                .await
        } else {
            self.service
                .settle_draft_stage(account(), &command, &port)
                .await
        }
    }
    async fn decision_proof(&self, stage: &PaidStage) -> Result<QualifiedDecision, Error> {
        let dispatch = DispatchId::from_bytes(&stage.quote.call().dispatch)
            .map_err(|_| Error::InvalidRecord)?;
        let journal = crate::imagery_runtime::PostgresImageUsage::open_existing(
            self.world.scratch.config(),
            crate::postgres_tls::PostgresTransport::PlaintextLoopbackOnly,
            1,
        )
        .map_err(|_| Error::PortUnavailable)?;
        let usage = journal
            .latest_usage(&self.owner, dispatch)
            .await
            .map_err(|_| Error::PortUnavailable)?
            .ok_or(Error::LinkConflict)?;
        let mut client = self.world.scratch.connect().await;
        let tx = client
            .transaction()
            .await
            .map_err(|e| Error::Finance(e.into()))?;
        let decision = execution::read_decision_checkpoint_in(
            &tx,
            &self.owner,
            &self.job,
            &stage.claim.scope.stage,
            &dispatch,
        )
        .await
        .map_err(|_| Error::LinkConflict)?
        .ok_or(Error::LinkConflict)?;
        let source_candidate = if let Some(parent) = stage.basis.parent_dispatch {
            let view = execution::read_dispatch_in(&tx, &self.owner, &parent)
                .await
                .map_err(|_| Error::LinkConflict)?;
            execution::read_candidate_checkpoint_in(
                &tx,
                &self.owner,
                &self.job,
                &view.stage,
                &parent,
            )
            .await
            .map_err(|_| Error::LinkConflict)?
        } else {
            None
        };
        let job = crate::read_in(&tx, &self.owner, self.job)
            .await
            .map_err(|_| Error::PortUnavailable)?
            .ok_or(Error::LinkConflict)?;
        tx.commit().await.map_err(|e| Error::Finance(e.into()))?;
        qualify_economic_decision(
            &job,
            &PublicationDecisionEvidence {
                decision,
                usage,
                source_candidate,
            },
        )
        .map_err(|_| Error::LinkConflict)
    }
}
#[test]
fn actual_classification_requires_durable_usage_and_source_verified_sent_without_catalog_rows() {
    support::run(async {
        let Some(mut pipeline) = Pipeline::new().await else {
            return;
        };
        let stage = pipeline.stage(StageKind::Classification, 30, None).await;
        pipeline.admit(&stage).await;
        let assets: i64 = pipeline
            .world
            .admin
            .query_one("SELECT count(*) FROM media.asset", &[])
            .await
            .unwrap()
            .get(0);
        let allocations: i64 = pipeline
            .world
            .admin
            .query_one(
                "SELECT count(*) FROM imagery.image_publication_allocation",
                &[],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!((assets, allocations), (0, 0));
        pipeline.send(&stage).await;
        assert!(matches!(
            pipeline
                .settle(&stage, StageUsageEvidence::Reported, true)
                .await,
            Err(Error::LinkConflict)
        ));
        assert_eq!(
            pipeline
                .world
                .natural(2150, Some(payer(1)), None, None)
                .await,
            90
        );
        pipeline.finish_decision(&stage).await;
        let known = pipeline
            .settle(&stage, StageUsageEvidence::Reported, true)
            .await
            .unwrap();
        assert_eq!(
            (known.charged_micro_dollars, known.released_micro_dollars),
            (45, 45)
        );
        assert!(
            pipeline
                .settle(&stage, StageUsageEvidence::Reported, true)
                .await
                .unwrap()
                .replay
        );
        let mut foreign = StageSettlementCommand {
            quote: stage.quote.clone(),
            usage_revision: 2,
            evidence: StageUsageEvidence::Reported,
            actual_units: vec![("text_input".into(), 50), ("text_output".into(), 50)],
            usability: Usability::Usable,
        };
        assert!(matches!(
            pipeline
                .service
                .settle_draft_stage(
                    account(),
                    &foreign,
                    &CanonicalStageSettlement {
                        owner: pipeline.owner.clone(),
                        now_ms: pipeline.now
                    }
                )
                .await,
            Err(Error::LinkConflict)
        ));
        foreign.usage_revision = 1;
        foreign.actual_units[0].1 = 49;
        assert!(matches!(
            pipeline
                .service
                .settle_draft_stage(
                    account(),
                    &foreign,
                    &CanonicalStageSettlement {
                        owner: pipeline.owner.clone(),
                        now_ms: pipeline.now
                    }
                )
                .await,
            Err(Error::LinkConflict)
        ));
        drop(pipeline.service);
        pipeline.world.finish().await;
    });
}
#[test]
fn actual_unknown_classification_retains_full_hold_and_supplier_exposure_after_revocation() {
    support::run(async {
        let Some(mut pipeline) = Pipeline::new().await else {
            return;
        };
        let stage = pipeline.stage(StageKind::Classification, 30, None).await;
        pipeline.admit(&stage).await;
        pipeline.send(&stage).await;
        pipeline
            .observe(&stage, UsageEvidence::Unknown, false, None)
            .await;
        let mut client = pipeline.world.scratch.connect().await;
        let tx = client.transaction().await.unwrap();
        execution::complete_dispatch_in(
            &tx,
            &CompleteCommand::new(
                stage.claim.clone(),
                DispatchId::from_bytes(&stage.quote.call().dispatch).unwrap(),
                CompletionOutcome::Unknown,
                SystemClock.now_ms(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        drop(client);
        let unknown = pipeline
            .settle(&stage, StageUsageEvidence::Unknown, false)
            .await
            .unwrap();
        assert_eq!(
            (
                unknown.charged_micro_dollars,
                unknown.released_micro_dollars
            ),
            (0, 0)
        );
        pipeline
            .service
            .revoke_draft_image_consent(account(), pipeline.grant.id)
            .await
            .unwrap();
        assert!(
            pipeline
                .service
                .release_unsent_draft_image(account(), stage.quote.attempt())
                .await
                .is_err()
        );
        assert_eq!(
            pipeline
                .world
                .natural(2150, Some(payer(1)), None, None)
                .await,
            90
        );
        assert_eq!(pipeline.world.line_sum(9100).await, 60);
        drop(pipeline.service);
        pipeline.world.finish().await;
    });
}

const IMAGE: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 224, 18, 145, 251, 15, 0, 1,
    164, 1, 60, 147, 139, 14, 183, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];
impl Pipeline {
    async fn freeze(&self) {
        let journal = crate::PostgresImageJournal::open(
            self.world.scratch.config(),
            crate::postgres_tls::PostgresTransport::PlaintextLoopbackOnly,
            1,
        )
        .await
        .unwrap();
        let revision: i64 = self
            .world
            .admin
            .query_one(
                "SELECT revision FROM imagery.image_job WHERE job_id=$1",
                &[&self.job.as_bytes().as_slice()],
            )
            .await
            .unwrap()
            .get(0);
        assert!(matches!(
            journal
                .freeze_recipe(&FreezeCommand {
                    owner: self.owner.clone(),
                    job: self.job,
                    expected_revision: u64::try_from(revision).unwrap(),
                    key: self.key,
                    recipe: self.recipe.clone(),
                    now_ms: SystemClock.now_ms()
                })
                .await
                .unwrap(),
            FreezeOutcome::Frozen { .. }
        ));
    }
    async fn finish_candidate(&self, stage: &PaidStage) {
        self.observe(stage, UsageEvidence::Reported, true, Some(IMAGE.to_vec()))
            .await;
        let dispatch = DispatchId::from_bytes(&stage.quote.call().dispatch).unwrap();
        let mut client = self.world.scratch.connect().await;
        let tx = client.transaction().await.unwrap();
        let scope = ExecutionScope {
            owner: self.owner.clone(),
            job: self.job,
            stage: stage.claim.scope.stage,
            dispatch,
        };
        let candidate = CandidateObservation::inspect(
            scope,
            stage.basis.clone(),
            1,
            ObservationReceipt {
                usage_revision: 1,
                observed_at_ms: SystemClock.now_ms(),
            },
            IMAGE,
            df_assets::ImageProofLimits::new(1024, 4, 16).unwrap(),
            df_assets::ImageIntent::new(
                "image/png",
                1,
                1,
                df_assets::ImageAlphaRequirement::Opaque,
            )
            .unwrap(),
        )
        .unwrap();
        execution::record_candidate_observation_in(&tx, &candidate)
            .await
            .unwrap();
        execution::complete_dispatch_in(
            &tx,
            &CompleteCommand::new(
                stage.claim.clone(),
                dispatch,
                CompletionOutcome::Candidate {
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
    }
}
#[test]
fn generation_inspection_and_correction_share_one_actual_catalog_allocation() {
    support::run(async {
        let Some(mut pipeline) = Pipeline::new().await else {
            return;
        };
        let classifier = pipeline.stage(StageKind::Classification, 30, None).await;
        pipeline.admit(&classifier).await;
        pipeline.send(&classifier).await;
        pipeline.finish_decision(&classifier).await;
        pipeline
            .settle(&classifier, StageUsageEvidence::Reported, true)
            .await
            .unwrap();
        pipeline.freeze().await;
        let generation = pipeline
            .stage(
                StageKind::Generation,
                40,
                Some(DispatchId::from_bytes(&classifier.quote.call().dispatch).unwrap()),
            )
            .await;
        pipeline.admit(&generation).await;
        pipeline.send(&generation).await;
        pipeline.finish_candidate(&generation).await;
        assert!(matches!(
            pipeline
                .settle(&generation, StageUsageEvidence::Reported, true)
                .await,
            Err(Error::EconomicProofPending)
        ));
        assert_eq!(
            pipeline
                .world
                .natural(2150, Some(payer(1)), None, None)
                .await,
            90
        );
        let inspection = pipeline
            .stage(
                StageKind::Inspection,
                50,
                Some(DispatchId::from_bytes(&generation.quote.call().dispatch).unwrap()),
            )
            .await;
        pipeline.admit(&inspection).await;
        assert_eq!(
            pipeline
                .world
                .natural(2150, Some(payer(1)), None, None)
                .await,
            180
        );
        pipeline.send(&inspection).await;
        pipeline.finish_decision(&inspection).await;
        pipeline
            .settle(&inspection, StageUsageEvidence::Reported, true)
            .await
            .unwrap();
        let correction = pipeline
            .stage(
                StageKind::Correction,
                60,
                Some(DispatchId::from_bytes(&inspection.quote.call().dispatch).unwrap()),
            )
            .await;
        pipeline.admit(&correction).await;
        let row=pipeline.world.admin.query_one("SELECT (SELECT count(*) FROM media.asset), \
        (SELECT count(*) FROM imagery.image_publication_allocation), \
        (SELECT count(*) FROM media.quota_reservation), \
        (SELECT count(*) FROM imagery.image_finance_admission_binding WHERE asset_id IS NOT NULL AND stage_id=$1 AND execution_stage_id<>$1)",
        &[&generation.claim.scope.stage.as_bytes().as_slice()]).await.unwrap();
        assert_eq!(
            (
                row.get::<_, i64>(0),
                row.get::<_, i64>(1),
                row.get::<_, i64>(2),
                row.get::<_, i64>(3)
            ),
            (1, 1, 1, 2)
        );
        let stage: Vec<u8> = pipeline
            .world
            .admin
            .query_one(
                "SELECT planned_output_stage FROM imagery.image_job WHERE job_id=$1",
                &[&pipeline.job.as_bytes().as_slice()],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(stage, generation.claim.scope.stage.as_bytes());
        drop(pipeline.service);
        pipeline.world.finish().await;
    });
}

#[test]
fn empty_known_usage_uses_pinned_maxima_but_explicit_zero_lines_remain_zero() {
    support::run(async {
        for (evidence, raw_units, effective_units, charged) in [
            (
                UsageEvidence::Estimated,
                Vec::new(),
                vec![("text_input".into(), 100), ("text_output".into(), 100)],
                90,
            ),
            (
                UsageEvidence::Reported,
                vec![("text_input".into(), 0), ("text_output".into(), 0)],
                vec![("text_input".into(), 0), ("text_output".into(), 0)],
                0,
            ),
        ] {
            let Some(mut pipeline) = Pipeline::new().await else {
                return;
            };
            let stage = pipeline.stage(StageKind::Classification, 30, None).await;
            pipeline.admit(&stage).await;
            pipeline.send(&stage).await;
            let raw = decision_raw(stage.basis.kind);
            pipeline
                .observe_units(&stage, evidence, true, Some(raw.clone()), raw_units)
                .await;
            pipeline.select_decision(&stage).await;
            let command = StageSettlementCommand {
                quote: stage.quote.clone(),
                usage_revision: 1,
                evidence: if evidence == UsageEvidence::Estimated {
                    StageUsageEvidence::Estimated
                } else {
                    StageUsageEvidence::Reported
                },
                actual_units: effective_units,
                usability: Usability::Usable,
            };
            let proof = pipeline.decision_proof(&stage).await.unwrap();
            let port = CanonicalStageSettlement {
                owner: pipeline.owner.clone(),
                now_ms: SystemClock.now_ms(),
            };
            let known = pipeline
                .service
                .settle_draft_stage(account(), &command, &port.with_decision(&proof))
                .await
                .unwrap();
            assert_eq!(
                (known.charged_micro_dollars, known.released_micro_dollars),
                (charged, 90 - charged)
            );
            let source: String = pipeline
                .world
                .admin
                .query_one(
                    "SELECT usage_source FROM finance.provider_attempt WHERE attempt_id=$1",
                    &[&stage.quote.attempt().as_bytes().as_slice()],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(source, command.evidence.label());
            drop(pipeline.service);
            pipeline.world.finish().await;
        }
    });
}
#[test]
fn duplicate_raw_usage_categories_refuse_settlement_and_leave_the_entire_hold() {
    support::run(async {
        let Some(mut pipeline) = Pipeline::new().await else {
            return;
        };
        let stage = pipeline.stage(StageKind::Classification, 30, None).await;
        pipeline.admit(&stage).await;
        pipeline.send(&stage).await;
        let raw = decision_raw(stage.basis.kind);
        pipeline
            .observe_units(
                &stage,
                UsageEvidence::Reported,
                true,
                Some(raw.clone()),
                vec![("text_input".into(), 50), ("text_input".into(), 50)],
            )
            .await;
        pipeline.select_decision(&stage).await;
        assert!(matches!(
            pipeline
                .settle(&stage, StageUsageEvidence::Reported, true)
                .await,
            Err(Error::LinkConflict)
        ));
        assert_eq!(
            pipeline
                .world
                .natural(2150, Some(payer(1)), None, None)
                .await,
            90
        );
        drop(pipeline.service);
        pipeline.world.finish().await;
    });
}

#[test]
fn successful_decision_with_unknown_billing_retains_all_financial_liability() {
    support::run(async {
        let Some(mut pipeline) = Pipeline::new().await else {
            return;
        };
        let stage = pipeline.stage(StageKind::Classification, 30, None).await;
        pipeline.admit(&stage).await;
        pipeline.send(&stage).await;
        let raw = decision_raw(stage.basis.kind);
        pipeline
            .observe_units(
                &stage,
                UsageEvidence::Unknown,
                true,
                Some(raw.clone()),
                stage.quote.route().maximum_units.clone(),
            )
            .await;
        pipeline.select_decision(&stage).await;
        let unknown = pipeline
            .settle(&stage, StageUsageEvidence::Unknown, true)
            .await
            .unwrap();
        assert_eq!(
            (
                unknown.charged_micro_dollars,
                unknown.released_micro_dollars
            ),
            (0, 0)
        );
        assert_eq!(
            pipeline
                .world
                .natural(2150, Some(payer(1)), None, None)
                .await,
            90
        );
        assert_eq!(pipeline.world.line_sum(9100).await, 60);
        let row=pipeline.world.admin.query_one("SELECT dispatch_state,settlement FROM imagery.image_dispatch_attempt WHERE dispatch_id=$1",&[&stage.quote.call().dispatch.as_slice()]).await.unwrap();
        assert_eq!(
            (row.get::<_, String>(0), row.get::<_, String>(1)),
            ("succeeded".into(), "unknown_outstanding".into())
        );
        drop(pipeline.service);
        pipeline.world.finish().await;
    });
}

fn decision_raw(kind: StageKind) -> Vec<u8> {
    let body=match kind {
        StageKind::Classification=>serde_json::json!({"answers":{"q1":{"type":"choice","choice":"o1"}}}),
        StageKind::Inspection=>{
            let answer=serde_json::json!({"frame":"single_photograph","medium":"photograph","text":"none","dress":"no_people","setting":"period","people_present":"none"});
            serde_json::json!({"choices":[{"message":{"content":answer.to_string()}}]})
        }
        _=>panic!("not a decision stage"),
    }.to_string().into_bytes();
    serde_json::json!({"schema_revision":1,"status":200,"body":body})
        .to_string()
        .into_bytes()
}
fn decision_payload(kind: StageKind) -> Vec<u8> {
    match kind {
        StageKind::Classification=>serde_json::json!({"schema_revision":1,"contract_revision":1,"category":"general","route":"general"}),
        StageKind::Inspection=>serde_json::json!({"schema_revision":1,"contract_revision":1,"verdict":"pass","failure":null,"unjudged_reason":null,"visible_features":[],"source_binding":null}),
        _=>panic!("not a decision stage"),
    }.to_string().into_bytes()
}

#[test]
fn canonical_local_intent_cannot_forge_a_paid_classification_qualification() {
    support::run(async {
        let Some(mut pipeline) = Pipeline::new().await else {
            return;
        };
        let stage = pipeline.stage(StageKind::Classification, 30, None).await;
        pipeline.admit(&stage).await;
        pipeline.send(&stage).await;
        pipeline.finish_decision(&stage).await;
        let good = pipeline.decision_proof(&stage).await.unwrap();
        let (intent, config) = intent_fixture::snapshots(&pipeline.owner, None).await;
        let mut client = pipeline.world.scratch.connect().await;
        let tx = client.transaction().await.unwrap();
        let mut wrong = crate::read_in(&tx, &pipeline.owner, pipeline.job)
            .await
            .unwrap()
            .unwrap();
        let reference = execution::read_decision_checkpoint_in(
            &tx,
            &pipeline.owner,
            &pipeline.job,
            &stage.claim.scope.stage,
            &DispatchId::from_bytes(&stage.quote.call().dispatch).unwrap(),
        )
        .await
        .unwrap()
        .unwrap();
        tx.commit().await.unwrap();
        drop(client);
        wrong.intent = intent;
        wrong.config = config;
        assert!(!good.matches(
            &wrong,
            &df_imagery::publication::FinalDecisionRef::from_observation(&reference),
            None
        ));
        drop(pipeline.service);
        pipeline.world.finish().await;
    });
}
