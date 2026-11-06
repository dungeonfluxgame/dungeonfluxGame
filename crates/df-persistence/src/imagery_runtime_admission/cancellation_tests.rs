//! Actual account release and terminal execution proof share the restricted money transaction.
use super::*;
use df_imagery::publication::{
    NoDeliveryAllocation, NoDeliveryCause, NoDeliveryReceipt, NoDeliveryRequest, NoDeliveryStage,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Scenario {
    Prepared,
    Sent,
    Unknown,
    SendRace,
}
#[test]
fn terminal_unused_release_is_exact_atomic_and_never_releases_sent_or_unknown() {
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

            let before_legacy = snapshot_rows(&world).await;
            let attempt = mode.finance().unwrap().attempt;
            let refusal = finance
                .lock()
                .await
                .release_unsent_draft_image(
                    account,
                    df_commerce::finance_ids::AttemptId::from_array(attempt),
                )
                .await;
            assert!(matches!(
                refusal,
                Err(Error::Refused(
                    df_finance::draft_image::AdmissionRefusal::BindingChanged
                ))
            ));
            assert_eq!(
                snapshot_rows(&world).await,
                before_legacy,
                "legacy attempt-only API cannot bypass canonical terminal authority"
            );
            if matches!(scenario, Scenario::Sent | Scenario::Unknown) {
                assert_eq!(
                    adapter.mark_sent(&input, mode).await.unwrap(),
                    SendAuthorization::New
                );
                let before_unknown = snapshot_rows(&world).await;
                assert!(matches!(
                    finance
                        .lock()
                        .await
                        .mark_draft_image_unknown(
                            account,
                            df_commerce::finance_ids::AttemptId::from_array(
                                mode.finance().unwrap().attempt
                            ),
                        )
                        .await,
                    Err(Error::Refused(
                        df_finance::draft_image::AdmissionRefusal::BindingChanged
                    ))
                ));
                assert_eq!(
                    snapshot_rows(&world).await,
                    before_unknown,
                    "legacy account unknown cannot bypass exact stage binding after actual Sent"
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
                            FakeMembers::new(vec![]),
                            Box::new(SystemClock),
                            Box::new(SeededIds::new(180_000)),
                            FinanceConfig::default(),
                        ))),
                        authority: Authority {
                            account,
                            grant: adapter.ports.authority.grant.clone(),
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
                        assert_eq!(world.natural(2150, Some(payer(1)), None, None).await, 90);
                        assert_eq!(world.line_sum(9100).await, 60);
                    }
                    Err(RuntimeError::InvalidRecord | RuntimeError::SourceDenied) => {
                        adapter
                            .release_unused(&admitted_job, dispatch, &receipt)
                            .await
                            .unwrap();
                        assert_eq!(world.natural(2150, Some(payer(1)), None, None).await, 0);
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
            assert_eq!(world.natural(2150, Some(payer(1)), None, None).await, 90);
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
                assert_eq!(world.natural(2150, Some(payer(1)), None, None).await, 90);
                assert_eq!(world.line_sum(9100).await, 60);
                world.admin.batch_execute("DROP TRIGGER reject_cancel_mirror ON imagery.image_dispatch_attempt; DROP FUNCTION public.reject_cancel_mirror(); DROP SEQUENCE public.cancel_mirror_probe;").await.unwrap();
                adapter
                    .release_unused(&admitted_job, dispatch, &receipt)
                    .await
                    .unwrap();
                assert_eq!(world.natural(2150, Some(payer(1)), None, None).await, 0);
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
                assert_eq!(world.natural(2150, Some(payer(1)), None, None).await, 90);
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

#[path = "campaign_cancellation_tests.rs"]
mod campaign_tests;
