//! Actual allocated campaign funds follow the same canonical unused cancellation path.
use super::*;
use df_finance::{OwnerScope, ProviderStageAdmission, ProviderStageBinding};
async fn assert_legacy_protocol_denied(
    world: &World,
    finance: &Mutex<FinanceService<FakeFence, FakeMedia, FakeMembers>>,
    attempt: df_commerce::finance_ids::AttemptId,
    units: &[(String, u64)],
) {
    use df_commerce::lifecycle::Deadlines;
    use df_commerce::settlement::{ReleaseReason, UnusableOutcome, UsageSource};
    macro_rules! unchanged_refusal {
        ($call:expr) => {{
            let before = snapshot_rows(world).await;
            assert!(matches!(
                ($call).await,
                Err(df_finance::service::FinanceError::ImageStageConflict)
            ));
            assert_eq!(
                snapshot_rows(world).await,
                before,
                "legacy campaign protocol cannot bypass canonical proof before any replay shortcut"
            );
        }};
    }
    unchanged_refusal!(
        finance
            .lock()
            .await
            .prepare_dispatch(attempt, Deadlines::default())
    );
    unchanged_refusal!(finance.lock().await.mark_attempt_sent(
        attempt,
        [9; 16],
        Deadlines::default()
    ));
    unchanged_refusal!(
        finance
            .lock()
            .await
            .settle(attempt, units.to_vec(), UsageSource::Reported)
    );
    unchanged_refusal!(finance.lock().await.settle_unusable(
        attempt,
        units.to_vec(),
        UsageSource::Reported,
        UnusableOutcome::FailedNoOutput
    ));
    unchanged_refusal!(
        finance
            .lock()
            .await
            .release(attempt, ReleaseReason::NotDispatched)
    );
    use df_commerce::plan::{ChangeKind, EventKind, FinancePlan, Row, RowChange};
    let identities = world.admin.query_one(
        "SELECT a.reservation_id,a.revision,r.revision FROM finance.provider_attempt a JOIN finance.reservation r USING(reservation_id) WHERE a.attempt_id=$1",
        &[&attempt.as_bytes().as_slice()],
    ).await.unwrap();
    let mut attempt_plan = FinancePlan::new(
        df_commerce::finance_ids::EventId::from_array([0xfd; 16]),
        EventKind::AttemptPrepared,
    );
    attempt_plan.changes.push(RowChange::update(
        ChangeKind::ProviderAttemptUpdate,
        Row::new().bytes("attempt_id", attempt.as_bytes()),
        identities.get(1),
    ));
    unchanged_refusal!(finance.lock().await.record_event(&attempt_plan));
    let mut reservation_plan = FinancePlan::new(
        df_commerce::finance_ids::EventId::from_array([0xfc; 16]),
        EventKind::ReleaseApplied,
    );
    reservation_plan.changes.push(RowChange::update(
        ChangeKind::ReservationUpdate,
        Row::new().bytes("reservation_id", &identities.get::<_, Vec<u8>>(0)),
        identities.get(2),
    ));
    unchanged_refusal!(finance.lock().await.record_event(&reservation_plan));
}
struct CampaignAuthority {
    owner: ImageOwner,
    command: ProviderStageAdmission,
}
impl RuntimeImageAuthority for CampaignAuthority {
    async fn funding(&self, input: &DispatchAdmission<'_>) -> Result<RuntimeFunding, RuntimeError> {
        if input.job.owner != self.owner {
            return Err(RuntimeError::SourceDenied);
        }
        Ok(RuntimeFunding::Campaign(Box::new(self.command.clone())))
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
        let StagePayer::Campaign(binding) = payer else {
            return Err(Error::LinkConflict);
        };
        if owner != &self.owner
            || call != &self.command.binding.call
            || !sources.is_empty()
            || binding.owner_scope.as_bytes() != df_imagery::owner_scope_bytes(owner)
            || binding.attempt != self.command.binding.attempt
            || binding.operation != self.command.binding.operation
            || binding.effect != self.command.binding.effect
            || binding.route != self.command.binding.route
        {
            return Err(Error::LinkConflict);
        }
        tx.query_opt("SELECT 1 FROM public.funded_campaign_authority h JOIN finance.session_funding s ON s.session_id=h.session AND s.campaign_id=h.campaign AND s.tenant_id=h.tenant WHERE h.owner_scope=$1 AND h.tenant=$2 AND h.campaign=$3 AND h.session=$4 AND h.active AND s.status='open' FOR SHARE OF h",
    &[&df_imagery::owner_scope_bytes(owner),&binding.tenant.as_bytes().as_slice(),&binding.campaign.as_bytes().as_slice(),&binding.session.as_bytes().as_slice()]).await.map_err(|error|Error::Finance(error.into()))?.map(|_|()).ok_or(Error::LinkConflict)
    }
}
struct CampaignDemandAuthority(ImageOwner);
impl ImageAuthorizer for CampaignDemandAuthority {
    type Principal = ImageOwner;
    fn authorize_request(
        &self,
        principal: &ImageOwner,
        request: &ImageRequest,
    ) -> Result<(), AuthDenied> {
        self.authorize_owner(principal, &request.identity.owner, ServiceAction::Read)?;
        let ImageOwner::Run { campaign, .. } = &self.0 else {
            return Err(AuthDenied);
        };
        if request.audience != Audience::CampaignShared(*campaign)
            || !request.operation.sources().is_empty()
        {
            return Err(AuthDenied);
        }
        Ok(())
    }
    fn authorize_owner(
        &self,
        principal: &ImageOwner,
        owner: &ImageOwner,
        _action: ServiceAction,
    ) -> Result<(), AuthDenied> {
        if principal == &self.0 && owner == &self.0 {
            Ok(())
        } else {
            Err(AuthDenied)
        }
    }
    fn authorize_stored(
        &self,
        principal: &ImageOwner,
        _action: ServiceAction,
        view: &OperationView,
        stored: &StoredImageAuthorization<'_>,
    ) -> Result<(), AuthDenied> {
        self.authorize_owner(principal, stored.owner, ServiceAction::Read)?;
        if stored.handle.owner != *stored.owner || view.job.owner != *stored.owner {
            return Err(AuthDenied);
        }
        let ImageOwner::Run { campaign, .. } = &self.0 else {
            return Err(AuthDenied);
        };
        if *stored.audience != Audience::CampaignShared(*campaign)
            || !stored.operation.sources().is_empty()
        {
            return Err(AuthDenied);
        }
        Ok(())
    }
}
async fn campaign_demand(
    world: &World,
    owner: &ImageOwner,
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
        CampaignDemandAuthority(owner.clone()),
    )
    .unwrap();
    let dimensions = Dimensions::new(1024, 1024).unwrap();
    let mut request = ImageRequest {
        identity: OperationIdentity {
            owner: owner.clone(),
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
        audience: match owner {
            ImageOwner::Run { campaign, .. } => Audience::CampaignShared(*campaign),
            _ => unreachable!(),
        },
        expected_source_revision: 1,
        output: OutputProfile::provider(
            dimensions,
            RasterOutputSpec::new(dimensions, AlphaHandling::Opaque, Encoding::Png).unwrap(),
        ),
    };
    request.identity.fingerprint = service.canonical_fingerprint(&request).unwrap();
    assert!(matches!(
        service.request(owner, &request).await.unwrap(),
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

#[test]
fn campaign_unused_release_matches_draft_atomicity_replay_and_send_refusal() {
    support::run(async {
        for scenario in [
            Scenario::Prepared,
            Scenario::Sent,
            Scenario::Unknown,
            Scenario::SendRace,
        ] {
            let Some(world) = world().await else { return };
            world
                .admin
                .batch_execute(crate::imagery_publication::IMAGERY_PUBLICATION_MIGRATION_SQL)
                .await
                .unwrap();
            let owner = ImageOwner::Run {
                campaign: df_types::CampaignId::from_bytes(world.campaign.as_bytes()).unwrap(),
                run: df_types::RunId::from_bytes(&[55; 16]).unwrap(),
            };
            let job = ImageJobId::from_bytes(&[2; 16]).unwrap();
            let stage = ExecutionStageId::from_bytes(&[30; 16]).unwrap();
            let dispatch = DispatchId::from_bytes(&[32; 16]).unwrap();
            let now = SystemClock.now_ms();
            let (journal, accepted, config) = campaign_demand(&world, &owner, now).await;
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

            let mut funder = world.service().await;
            funder
                .allocate(
                    payer(1),
                    world.session,
                    support::world::operation(1),
                    400,
                    world.consent(),
                )
                .await
                .unwrap();
            drop(funder);
            world.admin.batch_execute("CREATE TABLE public.funded_campaign_authority(owner_scope bytea PRIMARY KEY,tenant bytea NOT NULL,campaign bytea NOT NULL,session bytea NOT NULL,active boolean NOT NULL); GRANT SELECT,UPDATE(active) ON public.funded_campaign_authority TO df_imagery_admission").await.unwrap();
            world
                .admin
                .execute(
                    "INSERT INTO public.funded_campaign_authority VALUES($1,$2,$3,$4,true)",
                    &[
                        &df_imagery::owner_scope_bytes(&owner),
                        &world.tenant.as_bytes().as_slice(),
                        &world.campaign.as_bytes().as_slice(),
                        &world.session.as_bytes().as_slice(),
                    ],
                )
                .await
                .unwrap();
            let client = world.scratch.connect().await;
            client
                .batch_execute("SET ROLE df_imagery_admission")
                .await
                .unwrap();
            let finance = FinanceService::new(
                client,
                FakeFence::new(FenceVerdict::Current),
                FakeMedia::new(MediaVerdict::Admitted),
                FakeMembers::new(world.payers.clone()),
                Box::new(SystemClock),
                Box::new(SeededIds::new(90_000)),
                FinanceConfig::default(),
            );
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
            let call = super::super::super::super::context::call(finance.client(), &input)
                .await
                .unwrap();
            let mut admit = world.admit_command(10, 100, 100);
            admit.route.name = route.route.clone();
            let binding = ProviderStageBinding {
                owner_scope: OwnerScope::new(df_imagery::owner_scope_bytes(&owner)).unwrap(),
                call,
                route_key: route.key,
                route,
                tenant: world.tenant,
                campaign: world.campaign,
                session: world.session,
                operation: admit.operation,
                attempt: admit.attempt,
                effect: admit.effect,
                media_asset: None,
            };
            let command = ProviderStageAdmission { admit, binding };
            let finance = Arc::new(Mutex::new(finance));
            let adapter = FundedImageAdmission::new(
                RuntimeAdmissionSettings {
                    dispatcher: [88; 16],
                    quote_lifetime_ms: 300_000,
                    catalog: df_assets::CatalogConfig::default(),
                },
                RuntimeAdmissionPorts {
                    finance: finance.clone(),
                    authority: CampaignAuthority {
                        owner: owner.clone(),
                        command,
                    },
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
            let mode = adapter.admit(&input).await.unwrap();
            assert!(matches!(mode, DispatchMode::Hosted(_)));
            let held_key = world.admin.query_one(
                "SELECT s.allocation_id,s.lot_id,a.payer_account_id,a.session_id
                 FROM finance.reservation_share s JOIN finance.provider_attempt p USING(reservation_id)
                 JOIN finance.allocation a USING(allocation_id) WHERE p.attempt_id=$1",
                &[&mode.finance().unwrap().attempt.as_slice()],
            ).await.unwrap();
            assert_eq!(
                held_key.get::<_, Vec<u8>>("payer_account_id").as_slice(),
                payer(1).as_bytes().as_slice()
            );
            assert_eq!(
                held_key.get::<_, Vec<u8>>("session_id").as_slice(),
                world.session.as_bytes().as_slice()
            );
            let held_allocation = df_commerce::finance_ids::AllocationId::from_slice(
                &held_key.get::<_, Vec<u8>>("allocation_id"),
            )
            .unwrap();
            let held_lot =
                df_commerce::finance_ids::LotId::from_slice(&held_key.get::<_, Vec<u8>>("lot_id"))
                    .unwrap();
            assert_legacy_protocol_denied(
                &world,
                finance.as_ref(),
                df_commerce::finance_ids::AttemptId::from_array(mode.finance().unwrap().attempt),
                &input
                    .quote
                    .unwrap()
                    .maxima
                    .iter()
                    .map(|unit| (unit.category.clone(), unit.quantity))
                    .collect::<Vec<_>>(),
            )
            .await;
            if matches!(scenario, Scenario::Sent | Scenario::Unknown) {
                assert_eq!(
                    adapter.mark_sent(&input, mode).await.unwrap(),
                    SendAuthorization::New
                );
            }
            if scenario == Scenario::SendRace {
                let client = world.scratch.connect().await;
                client.batch_execute("SET ROLE df_imagery_admission; SET lock_timeout='2s'; SET statement_timeout='30s'").await.unwrap();
                let second = FundedImageAdmission::new(
                    RuntimeAdmissionSettings {
                        dispatcher: [88; 16],
                        quote_lifetime_ms: 300_000,
                        catalog: df_assets::CatalogConfig::default(),
                    },
                    RuntimeAdmissionPorts {
                        finance: Arc::new(Mutex::new(FinanceService::new(
                            client,
                            FakeFence::new(FenceVerdict::Current),
                            FakeMedia::new(MediaVerdict::Admitted),
                            FakeMembers::new(world.payers.clone()),
                            Box::new(SystemClock),
                            Box::new(SeededIds::new(180_000)),
                            FinanceConfig::default(),
                        ))),
                        authority: CampaignAuthority {
                            owner: owner.clone(),
                            command: adapter.ports.authority.command.clone(),
                        },
                        local: Arc::new(NoLocal),
                        usage: adapter.ports.usage.clone(),
                    },
                )
                .unwrap();
                let (sent, receipt) = tokio::join!(second.mark_sent(&input, mode), async {
                    journal
                        .withdraw_interest(
                            &owner,
                            ImageInterestId::from_bytes(&[10; 16]).unwrap(),
                            now,
                        )
                        .await
                        .unwrap();
                    let retired = journal.read_job(&owner, job).await.unwrap().unwrap();
                    let terminal_now = SystemClock.now_ms();
                    let proof =
                        df_imagery::runtime::qualify_demand_ended(&retired, terminal_now).unwrap();
                    let tail = TerminalActor::Dispatch(ExecutionScope {
                        owner: owner.clone(),
                        job,
                        stage,
                        dispatch,
                    });
                    let command = TerminalCommand {
                        actor: tail.clone(),
                        tail,
                        authority: TerminalAuthority::Claim(claim.clone()),
                        expected_job_revision: retired.revision,
                        candidate_dispatch: None,
                        now_ms: terminal_now,
                    };
                    let mut client = world.scratch.connect().await;
                    let tx = client.transaction().await.unwrap();
                    let receipt = crate::imagery_publication::finalize_no_delivery_in(
                        &tx,
                        &NoDeliveryRequest {
                            terminal: &command,
                            cause: NoDeliveryCause::DemandEnded(&proof),
                        },
                    )
                    .await
                    .unwrap()
                    .receipt();
                    tx.commit().await.unwrap();
                    receipt
                });
                match sent {
                    Ok(SendAuthorization::New) => {
                        let before = snapshot_rows(&world).await;
                        assert!(
                            adapter
                                .release_unused(&admitted_job, dispatch, &receipt)
                                .await
                                .is_err()
                        );
                        assert_eq!(snapshot_rows(&world).await, before);
                        assert_eq!(
                            world
                                .natural(2130, None, Some(held_lot), Some(held_allocation))
                                .await,
                            90
                        );
                        assert_eq!(world.line_sum(9100).await, 60);
                    }
                    Err(RuntimeError::InvalidRecord | RuntimeError::SourceDenied) => {
                        adapter
                            .release_unused(&admitted_job, dispatch, &receipt)
                            .await
                            .unwrap();
                        assert_eq!(
                            world
                                .natural(2130, None, Some(held_lot), Some(held_allocation))
                                .await,
                            0
                        );
                        assert_eq!(world.line_sum(9100).await, 0);
                        let before = snapshot_rows(&world).await;
                        adapter
                            .release_unused(&admitted_job, dispatch, &receipt)
                            .await
                            .unwrap();
                        assert_eq!(snapshot_rows(&world).await, before);
                    }
                    Ok(SendAuthorization::AlreadySent) => {
                        panic!("fresh isolated race cannot already be sent")
                    }
                    Err(error) => panic!("unexpected send race failure: {error:?}"),
                }
                let row=world.admin.query_one("SELECT d.dispatch_state,a.dispatch_state,d.settlement FROM imagery.image_dispatch_attempt d JOIN finance.provider_attempt a ON a.attempt_id=d.finance_attempt WHERE d.dispatch_id=$1",&[&dispatch.as_bytes().as_slice()]).await.unwrap();
                let pair = (
                    row.get::<_, String>(0),
                    row.get::<_, String>(1),
                    row.get::<_, String>(2),
                );
                assert!(
                    pair == ("sent".into(), "sent".into(), "held".into())
                        || pair
                            == (
                                "released".into(),
                                "cancelled_unsent".into(),
                                "released".into()
                            ),
                    "no split Sent/money release allowed: {pair:?}"
                );
                drop(second);
                drop(adapter);
                drop(finance);
                drop(journal);
                world.finish().await;
                continue;
            }
            journal
                .withdraw_interest(&owner, ImageInterestId::from_bytes(&[10; 16]).unwrap(), now)
                .await
                .unwrap();
            let retired = journal.read_job(&owner, job).await.unwrap().unwrap();
            assert_eq!(retired.state, df_imagery::JobState::Retired);
            let tail = ExecutionScope {
                owner: owner.clone(),
                job,
                stage,
                dispatch,
            };
            let claimed_receipt = NoDeliveryReceipt {
                owner: owner.clone(),
                job,
                actor: NoDeliveryStage::Dispatch {
                    scope: tail.clone(),
                    usage_revision: 0,
                },
                tail: NoDeliveryStage::Dispatch {
                    scope: tail.clone(),
                    usage_revision: 0,
                },
                candidate: None,
                allocation: NoDeliveryAllocation::NoAllocation,
                reason: df_imagery::runtime::NoDeliveryReason::Retired,
                expected_job_revision: retired.revision - 1,
                job_revision: retired.revision,
                sealed_at_ms: now,
            };
            let before = snapshot_rows(&world).await;
            assert!(
                adapter
                    .release_unused(&admitted_job, dispatch, &claimed_receipt)
                    .await
                    .is_err()
            );
            assert_eq!(
                snapshot_rows(&world).await,
                before,
                "no stored seal cannot release funds"
            );
            let terminal_now = SystemClock.now_ms();
            let proof = df_imagery::runtime::qualify_demand_ended(&retired, terminal_now).unwrap();
            let command = TerminalCommand {
                actor: TerminalActor::Dispatch(tail.clone()),
                tail: TerminalActor::Dispatch(tail),
                authority: TerminalAuthority::Claim(claim.clone()),
                expected_job_revision: retired.revision,
                candidate_dispatch: None,
                now_ms: terminal_now,
            };
            let mut client = world.scratch.connect().await;
            let tx = client.transaction().await.unwrap();
            let receipt = crate::imagery_publication::finalize_no_delivery_in(
                &tx,
                &NoDeliveryRequest {
                    terminal: &command,
                    cause: NoDeliveryCause::DemandEnded(&proof),
                },
            )
            .await
            .unwrap()
            .receipt();
            tx.commit().await.unwrap();
            drop(client);
            let sealed = snapshot_rows(&world).await;
            let mut stale = receipt.clone();
            stale.sealed_at_ms += 1;
            assert!(
                adapter
                    .release_unused(&admitted_job, dispatch, &stale)
                    .await
                    .is_err()
            );
            let mut wrong = receipt.clone();
            wrong.job = ImageJobId::from_bytes(&[77; 16]).unwrap();
            assert!(
                adapter
                    .release_unused(&admitted_job, dispatch, &wrong)
                    .await
                    .is_err()
            );
            assert!(
                adapter
                    .release_unused(
                        &admitted_job,
                        DispatchId::from_bytes(&[78; 16]).unwrap(),
                        &receipt
                    )
                    .await
                    .is_err()
            );
            assert_eq!(snapshot_rows(&world).await, sealed);
            assert_eq!(
                world
                    .natural(2130, None, Some(held_lot), Some(held_allocation))
                    .await,
                90
            );
            assert_eq!(world.line_sum(9100).await, 60);
            if scenario == Scenario::Unknown {
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
                        usable: false,
                        at_ms: u64::try_from(now).unwrap(),
                        raw_output: None,
                    })
                    .await
                    .unwrap();
                let mut client = world.scratch.connect().await;
                let tx = client.transaction().await.unwrap();
                execution::complete_dispatch_in(
                    &tx,
                    &CompleteCommand::new(
                        claim.clone(),
                        dispatch,
                        CompletionOutcome::Unknown,
                        SystemClock.now_ms(),
                    )
                    .unwrap(),
                )
                .await
                .unwrap();
                tx.commit().await.unwrap();
                drop(client);
                adapter
                    .outcome(&admitted_job, dispatch, mode, None)
                    .await
                    .unwrap();
            }
            if scenario == Scenario::Prepared {
                world.admin.batch_execute("CREATE SEQUENCE public.cancel_mirror_probe; GRANT USAGE ON SEQUENCE public.cancel_mirror_probe TO df_imagery_admission;
                  CREATE FUNCTION public.reject_cancel_mirror() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
                   IF NEW.settlement='released' AND OLD.settlement='held' THEN
                    IF NOT EXISTS(SELECT 1 FROM finance.provider_attempt a JOIN finance.reservation r USING(reservation_id)
                       WHERE a.attempt_id=NEW.finance_attempt AND a.dispatch_state='cancelled_unsent' AND a.customer_state='released'
                         AND a.supplier_state='exposure_released' AND r.state='released') THEN
                      RAISE EXCEPTION 'expected finance release before mirror';
                    END IF;
                    PERFORM nextval('public.cancel_mirror_probe'); RAISE EXCEPTION 'injected post-finance mirror failure';
                   END IF; RETURN NEW; END $$;
                  CREATE TRIGGER reject_cancel_mirror BEFORE UPDATE OF settlement ON imagery.image_dispatch_attempt
                    FOR EACH ROW EXECUTE FUNCTION public.reject_cancel_mirror();").await.unwrap();
                let before_fault = snapshot_rows(&world).await;
                assert!(
                    adapter
                        .release_unused(&admitted_job, dispatch, &receipt)
                        .await
                        .is_err()
                );
                assert!(
                    world
                        .admin
                        .query_one("SELECT is_called FROM public.cancel_mirror_probe", &[])
                        .await
                        .unwrap()
                        .get::<_, bool>(0),
                    "fault must execute after actual money release, not fail earlier"
                );
                assert_eq!(
                    snapshot_rows(&world).await,
                    before_fault,
                    "failed mirror must roll back the posted release and every execution change"
                );
                assert_eq!(
                    world
                        .natural(2130, None, Some(held_lot), Some(held_allocation))
                        .await,
                    90
                );
                assert_eq!(world.line_sum(9100).await, 60);
                world.admin.batch_execute("DROP TRIGGER reject_cancel_mirror ON imagery.image_dispatch_attempt; DROP FUNCTION public.reject_cancel_mirror(); DROP SEQUENCE public.cancel_mirror_probe;").await.unwrap();
                adapter
                    .release_unused(&admitted_job, dispatch, &receipt)
                    .await
                    .unwrap();
                assert_eq!(
                    world
                        .natural(2130, None, Some(held_lot), Some(held_allocation))
                        .await,
                    0
                );
                assert_eq!(world.line_sum(9100).await, 0);
                let row=world.admin.query_one("SELECT d.dispatch_state,d.settlement,a.dispatch_state,a.supplier_state,r.state FROM imagery.image_dispatch_attempt d JOIN finance.provider_attempt a ON a.attempt_id=d.finance_attempt JOIN finance.reservation r USING(reservation_id) WHERE d.dispatch_id=$1",&[&dispatch.as_bytes().as_slice()]).await.unwrap();
                assert_eq!(
                    (
                        row.get::<_, String>(0),
                        row.get::<_, String>(1),
                        row.get::<_, String>(2),
                        row.get::<_, String>(3),
                        row.get::<_, String>(4)
                    ),
                    (
                        "released".into(),
                        "released".into(),
                        "cancelled_unsent".into(),
                        "exposure_released".into(),
                        "released".into()
                    )
                );
                let released = snapshot_rows(&world).await;
                assert_legacy_protocol_denied(
                    &world,
                    finance.as_ref(),
                    df_commerce::finance_ids::AttemptId::from_array(
                        mode.finance().unwrap().attempt,
                    ),
                    &input
                        .quote
                        .unwrap()
                        .maxima
                        .iter()
                        .map(|unit| (unit.category.clone(), unit.quantity))
                        .collect::<Vec<_>>(),
                )
                .await;
                adapter
                    .release_unused(&admitted_job, dispatch, &receipt)
                    .await
                    .unwrap();
                assert_eq!(
                    snapshot_rows(&world).await,
                    released,
                    "exact replay must be readonly"
                );
                assert_eq!(
                    world
                        .admin
                        .query_one("SELECT count(*) FROM imagery.image_dispatch_usage", &[])
                        .await
                        .unwrap()
                        .get::<_, i64>(0),
                    0,
                    "unused release creates no billing evidence"
                );
            } else {
                let held = snapshot_rows(&world).await;
                assert!(
                    adapter
                        .release_unused(&admitted_job, dispatch, &receipt)
                        .await
                        .is_err()
                );
                assert_eq!(snapshot_rows(&world).await, held);
                assert_eq!(
                    world
                        .natural(2130, None, Some(held_lot), Some(held_allocation))
                        .await,
                    90
                );
                assert_eq!(world.line_sum(9100).await, 60);
                let mut client = world.scratch.connect().await;
                let tx = client.transaction().await.unwrap();
                let recoverable =
                    execution::scan_recovery_in(&tx, "funded-adapter", None, 8, now + 600_001)
                        .await
                        .unwrap();
                assert!(
                    recoverable
                        .iter()
                        .any(|stage| stage.current.dispatch == dispatch),
                    "terminal output must retain supplier reconciliation"
                );
                tx.rollback().await.unwrap();
                drop(client);
            }
            drop(adapter);
            drop(finance);
            drop(journal);
            world.finish().await;
        }
    });
}
