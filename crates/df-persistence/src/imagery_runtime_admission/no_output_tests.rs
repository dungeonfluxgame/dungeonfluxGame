//! Known standalone failure uses a real published Canvas source and the canonical money boundary.
use super::*;
use df_imagery::runtime::{
    DispatchNoOutputSettlement, StandaloneUsageCapture, capture_standalone_usage, qualify_no_output,
};
use df_imagery::{
    AnalysisTarget, HeadLocationsSpec, ImageAnalysisOperation, ImageAnalysisRequest,
    PrivateImageText, StoredAnalysisAuthorization,
};
struct AnalysisAuthority {
    grant: Authority,
    source: SourceReceipt,
}
impl RuntimeImageAuthority for AnalysisAuthority {
    async fn funding(&self, input: &DispatchAdmission<'_>) -> Result<RuntimeFunding, RuntimeError> {
        self.grant.funding(input).await
    }
    async fn verify(
        &self,
        tx: &Transaction<'_>,
        payer: StagePayer<'_>,
        owner: &ImageOwner,
        call: &StageCall,
        sources: &[SourceReceipt],
        replay: bool,
    ) -> Result<(), Error> {
        self.grant
            .verify(tx, payer, owner, call, &[], replay)
            .await?;
        if sources != std::slice::from_ref(&self.source) {
            return Err(Error::LinkConflict);
        }
        let facts = crate::imagery_publication::lock_image_sources_in(
            tx,
            std::slice::from_ref(&self.source.pin),
            &self.source.pin.audience,
        )
        .await
        .map_err(|error| match error {
            df_imagery::publication::PublicationError::Unavailable
            | df_imagery::publication::PublicationError::Unknown => Error::PortUnavailable,
            _ => Error::LinkConflict,
        })?;
        let fact = facts.first().ok_or(Error::LinkConflict)?;
        if !crate::imagery_publication::matches_source_receipt(fact, &self.source)
            || df_assets::record_owner(&fact.asset)
                != Some(df_assets::StorageOwner::AccountDraft(self.grant.account))
        {
            return Err(Error::LinkConflict);
        }
        Ok(())
    }
}
struct AnalysisDemandAuthority {
    account: AccountId,
    source: df_imagery::SourceRef,
}
impl ImageAuthorizer for AnalysisDemandAuthority {
    type Principal = AccountId;
    fn authorize_analysis(
        &self,
        principal: &AccountId,
        request: &ImageAnalysisRequest,
    ) -> Result<(), AuthDenied> {
        self.authorize_owner(principal, &request.identity.owner, ServiceAction::Read)?;
        if request.audience != Audience::OwnerPrivate(self.account)
            || request.operation.sources() != vec![&self.source]
            || !request.operation.dependencies().is_empty()
        {
            return Err(AuthDenied);
        }
        Ok(())
    }
    fn authorize_stored_analysis(
        &self,
        principal: &AccountId,
        _action: ServiceAction,
        view: &OperationView,
        stored: &StoredAnalysisAuthorization<'_>,
    ) -> Result<(), AuthDenied> {
        self.authorize_owner(principal, stored.owner, ServiceAction::Read)?;
        if stored.handle.owner != *stored.owner
            || view.job.owner != *stored.owner
            || *stored.audience != Audience::OwnerPrivate(self.account)
            || stored.operation.sources() != vec![&self.source]
        {
            return Err(AuthDenied);
        }
        Ok(())
    }
    fn authorize_request(&self, _: &AccountId, _: &ImageRequest) -> Result<(), AuthDenied> {
        Err(AuthDenied)
    }
    fn authorize_owner(
        &self,
        principal: &AccountId,
        owner: &ImageOwner,
        _action: ServiceAction,
    ) -> Result<(), AuthDenied> {
        if *principal == self.account
            && *owner
                == (ImageOwner::Draft {
                    account: self.account,
                })
        {
            Ok(())
        } else {
            Err(AuthDenied)
        }
    }
    fn authorize_stored(
        &self,
        _: &AccountId,
        _: ServiceAction,
        _: &OperationView,
        _: &StoredImageAuthorization<'_>,
    ) -> Result<(), AuthDenied> {
        Err(AuthDenied)
    }
}
#[test]
fn standalone_known_no_output_releases_customer_and_records_actual_supplier_loss_while_unknown_stays_held()
 {
    support::run(async {
        for evidence in [UsageEvidence::Reported, UsageEvidence::Unknown] {
            let Some(world) = world().await else { return };
            for sql in [
                crate::imagery_publication::IMAGERY_PUBLICATION_MIGRATION_SQL,
                include_str!("../../migrations/0045_imagery_compatible.sql"),
                include_str!("../../migrations/0046_imagery_analysis.sql"),
            ] {
                world.admin.batch_execute(sql).await.unwrap();
            }
            let seal_privileges = world.admin.query_one(
                "SELECT has_column_privilege('df_imagery_admission', 'imagery.image_analysis_checkpoint', 'owner_scope', 'SELECT'), has_column_privilege('df_imagery_admission', 'imagery.image_analysis_checkpoint', 'job_id', 'SELECT'), has_column_privilege('df_imagery_admission', 'imagery.image_analysis_checkpoint', 'result', 'SELECT')",
                &[],
            ).await.unwrap();
            assert!(seal_privileges.get::<_, bool>(0));
            assert!(seal_privileges.get::<_, bool>(1));
            assert!(!seal_privileges.get::<_, bool>(2));
            for table in ["media.tombstone", "media.asset_variant"] {
                let privileges=world.admin.query_one(
                    "SELECT has_table_privilege('df_imagery_admission',$1,'SELECT'),has_any_column_privilege('df_imagery_admission',$1,'UPDATE'),has_table_privilege('df_imagery_admission',$1,'INSERT'),has_table_privilege('df_imagery_admission',$1,'DELETE')",
                    &[&table],
                ).await.unwrap();
                assert!(privileges.get::<_, bool>(0));
                for denied in 1..4 {
                    assert!(!privileges.get::<_, bool>(denied));
                }
            }
            let restricted = world.scratch.connect().await;
            restricted
                .batch_execute("SET ROLE df_imagery_admission")
                .await
                .unwrap();
            for mutation in [
                "DELETE FROM media.tombstone WHERE false",
                "UPDATE media.asset_variant SET object_key=object_key WHERE false",
            ] {
                let error = restricted.execute(mutation, &[]).await.unwrap_err();
                assert_eq!(error.as_db_error().unwrap().code().code(), "42501");
            }
            drop(restricted);
            let account = AccountId::from_bytes(payer(1).as_bytes()).unwrap();
            let owner = ImageOwner::Draft { account };
            let source = Box::pin(
                crate::imagery_draft_admission::tests::stage_tests::native_source::create(
                    &world, &owner,
                ),
            )
            .await;
            let source_bytes = check_generated_source_bound(&world, &source).await;
            let journal = crate::PostgresImageJournal::open(
                world.scratch.config(),
                PostgresTransport::PlaintextLoopbackOnly,
                1,
            )
            .await
            .unwrap();
            let now = SystemClock.now_ms();
            let (address, response_task) = capture_response(evidence).await;
            let mut config = ImageConfig::defaults().unwrap();
            config.accounting.mode = AccountingMode::Hosted;
            config.analysis.head_locations = Some(df_imagery::AnalysisRoute {
                account: df_imagery::AccountHandle::new("funded-heads").unwrap(),
                model: df_imagery::ModelName::new("gpt-6-luna").unwrap(),
                endpoint: df_imagery::ApiEndpoint::new(&format!(
                    "http://{address}/v1/chat/completions"
                ))
                .unwrap(),
                maximum_output_tokens: 100,
            });
            let service = ImageService::new(
                journal.clone(),
                config.clone(),
                FixedClock(now),
                FixedIds,
                AnalysisDemandAuthority {
                    account,
                    source: source.pin.clone(),
                },
            )
            .unwrap();
            let mut request = ImageAnalysisRequest {
                identity: OperationIdentity {
                    owner: owner.clone(),
                    operation: OperationId::from_bytes(&[7; 16]).unwrap(),
                    fingerprint: [0; 32],
                },
                purpose: PurposedUse {
                    purpose: Purpose::ScenePlate,
                    role: VariantRole::Primary,
                },
                operation: ImageAnalysisOperation::HeadLocations(HeadLocationsSpec {
                    source: source.pin.clone(),
                    face: None,
                    targets: vec![AnalysisTarget {
                        id: PrivateImageText::new("adult-one"),
                        description: PrivateImageText::new("An adult fictional traveler"),
                    }],
                    contract_revision: 1,
                }),
                execution: ExecutionMode::Live,
                freshness: Freshness::ReuseCompatible,
                urgency: Urgency::Present,
                audience: Audience::OwnerPrivate(account),
                expected_source_revision: 1,
            };
            request.identity.fingerprint =
                service.canonical_analysis_fingerprint(&request).unwrap();
            assert!(matches!(
                service.request_analysis(&account, &request).await.unwrap(),
                RequestOutcome::Recorded(_)
            ));
            let job = ImageJobId::from_bytes(&[2; 16]).unwrap();
            let stage = ExecutionStageId::from_bytes(&[30; 16]).unwrap();
            let dispatch = DispatchId::from_bytes(&[32; 16]).unwrap();
            let token = ClaimToken::from_bytes(&[31; 16]).unwrap();
            let accepted = journal.read_job(&owner, job).await.unwrap().unwrap();
            assert_eq!(accepted.intent.revision(), 64);
            let basis = StageBasis {
                kind: StageKind::HeadLocations,
                input_revision: accepted.intent.revision(),
                input_digest: df_commerce::digest::sha256(accepted.intent.bytes()),
                config_revision: 1,
                config_digest: df_commerce::digest::sha256(accepted.config.bytes()),
                parent_dispatch: None,
            };
            let scope = StageScope {
                owner: owner.clone(),
                job,
                stage,
            };
            let capacity = CapacityScope::new("funded-no-output".into()).unwrap();
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
                    None::<PublicationIdentity>,
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
            let route = StageRoute {
                key: [1; 32],
                kind: PaidStageKind::HeadLocations,
                provider: "openai".into(),
                model: "gpt-6-luna".into(),
                route: "heads".into(),
                cause: "head_locations".into(),
                detail: "fixture".into(),
                price_version: [0x70; 16],
                maximum_units: vec![("text_input".into(), 100), ("text_output".into(), 100)],
            };
            let grant = JobConsent {
                id: ConsentId([90; 16]),
                scope: JobScope {
                    account,
                    draft: DraftId([1; 16]),
                    draft_revision: 1,
                    job: JobId(*job.as_bytes()),
                    source_digest: basis.input_digest,
                    config_revision: 1,
                    config_digest: basis.config_digest,
                    routes: vec![route.clone()],
                },
                cap_micro_dollars: 500,
                expires_at_ms: accepted.absolute_deadline_ms,
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
            let usage = crate::imagery_runtime::PostgresImageUsage::open_existing(
                world.scratch.config(),
                PostgresTransport::PlaintextLoopbackOnly,
                1,
            )
            .unwrap();
            let adapter = FundedImageAdmission::new(
                RuntimeAdmissionSettings {
                    dispatcher: [88; 16],
                    quote_lifetime_ms: 300_000,
                    catalog: df_assets::CatalogConfig::default(),
                },
                RuntimeAdmissionPorts {
                    finance: finance.clone(),
                    authority: AnalysisAuthority {
                        grant: Authority { account, grant },
                        source: source.clone(),
                    },
                    local: Arc::new(NoLocal),
                    usage: usage.clone(),
                },
            )
            .unwrap();
            let admitted_job = journal.read_job(&owner, job).await.unwrap().unwrap();
            let receipts = [source.clone()];
            let input = DispatchAdmission {
                job: &admitted_job,
                stage: &claimed,
                dispatch,
                claim: token,
                config: &config,
                quote: Some(&cost),
                sources: &receipts,
            };
            let mode = adapter.admit(&input).await.unwrap();
            assert_eq!(claimed.planned_publication, None);
            assert_eq!(
                adapter.mark_sent(&input, mode).await.unwrap(),
                SendAuthorization::New
            );
            let raw = capture_standalone_usage(StandaloneUsageCapture {
                job: &admitted_job,
                stage: &claimed,
                dispatch,
                source: &source_bytes,
                credential: "test-credential",
                route: &cost,
                revision: UsageRevision(1),
                at_ms: u64::try_from(SystemClock.now_ms()).unwrap(),
                timeout: std::time::Duration::from_secs(10),
                maximum_response_bytes: 1_048_576,
            })
            .await
            .unwrap();
            response_task.await.unwrap();
            assert_eq!(raw.evidence, evidence);
            assert!(!raw.usable);
            assert!(
                raw.raw_output.is_some(),
                "actual transport evidence is retained, including reset"
            );
            usage.record_usage(&raw).await.unwrap();
            let claim = StageClaim {
                scope,
                token,
                generation: claimed.claim_generation,
            };
            let mut client = world.scratch.connect().await;
            let tx = client.transaction().await.unwrap();
            let failed = execution::complete_dispatch_in(
                &tx,
                &CompleteCommand::new(
                    claim,
                    dispatch,
                    if evidence == UsageEvidence::Unknown {
                        CompletionOutcome::Unknown
                    } else {
                        CompletionOutcome::Unusable {
                            cause: FailureCause::InvalidOutput,
                            usage_revision: 1,
                        }
                    },
                    SystemClock.now_ms(),
                )
                .unwrap(),
            )
            .await
            .unwrap();
            tx.commit().await.unwrap();
            drop(client);
            let current = journal.read_job(&owner, job).await.unwrap().unwrap();
            if evidence == UsageEvidence::Unknown {
                assert!(qualify_no_output(&current, &failed, &basis, &raw).is_err());
                adapter
                    .outcome(&current, dispatch, mode, None)
                    .await
                    .unwrap();
                assert_eq!(world.natural(2150, Some(payer(1)), None, None).await, 90);
                assert_eq!(world.line_sum(9100).await, 60);
                let retained = usage.latest_usage(&owner, dispatch).await.unwrap().unwrap();
                assert_eq!(retained.evidence, UsageEvidence::Unknown);
                assert_eq!(retained.units, cost.maxima);
            } else {
                let before = snapshot_rows(&world).await;
                assert!(
                    adapter
                        .outcome(&current, dispatch, mode, Some(false))
                        .await
                        .is_err()
                );
                assert_eq!(
                    snapshot_rows(&world).await,
                    before,
                    "naked unusable boolean cannot replace standalone negative proof"
                );
                let proof = qualify_no_output(&current, &failed, &basis, &raw).unwrap();
                let settlement = DispatchNoOutputSettlement {
                    job: &current,
                    mode,
                    usage: &raw,
                    qualified: &proof,
                };
                adapter.outcome_no_output(&settlement).await.unwrap();
                assert_eq!(world.natural(2150, Some(payer(1)), None, None).await, 0);
                assert_eq!(world.line_sum(9100).await, 0);
                let row=world.admin.query_one("SELECT r.charged_micro_dollars,r.released_micro_dollars,a.supplier_cost_micro_dollars,a.usability,a.supplier_state,d.settlement FROM finance.provider_attempt a JOIN finance.reservation r USING(reservation_id) JOIN imagery.image_dispatch_attempt d ON d.finance_attempt=a.attempt_id WHERE d.dispatch_id=$1",&[&dispatch.as_bytes().as_slice()]).await.unwrap();
                assert_eq!(
                    (
                        row.get::<_, i64>(0),
                        row.get::<_, i64>(1),
                        row.get::<_, i64>(2)
                    ),
                    (0, 90, 60)
                );
                assert_eq!(row.get::<_, String>(3), "no_usable_output");
                assert_eq!(row.get::<_, String>(4), "cost_recorded");
                assert_eq!(row.get::<_, String>(5), "unusable_settled");
                assert_eq!(world.admin.query_one("SELECT sum(micro_dollars)::bigint FROM finance.company_loss WHERE attempt_id=$1",&[&mode.finance().unwrap().attempt.as_slice()]).await.unwrap().get::<_,Option<i64>>(0),Some(60));
                let settled = snapshot_rows(&world).await;
                adapter.outcome_no_output(&settlement).await.unwrap();
                assert_eq!(
                    snapshot_rows(&world).await,
                    settled,
                    "negative proof replay cannot post twice"
                );
            }
            let row=world.admin.query_one("SELECT (SELECT count(*) FROM imagery.image_publication_allocation WHERE job_id=$1),(SELECT count(*) FROM imagery.image_no_delivery WHERE job_id=$1),(SELECT count(*) FROM imagery.image_analysis_checkpoint WHERE job_id=$1),(SELECT count(*) FROM media.asset)",&[&job.as_bytes().as_slice()]).await.unwrap();
            assert_eq!(
                (
                    row.get::<_, i64>(0),
                    row.get::<_, i64>(1),
                    row.get::<_, i64>(2),
                    row.get::<_, i64>(3)
                ),
                (0, 0, 0, 1),
                "only the genuinely published Canvas source is media"
            );
            drop(adapter);
            drop(finance);
            drop(usage);
            drop(service);
            drop(journal);
            world.finish().await;
        }
    });
}

async fn check_generated_source_bound(world: &World, receipt: &SourceReceipt) -> Vec<u8> {
    let mut client = world.scratch.connect().await;
    let tx = client.transaction().await.unwrap();
    let sources = crate::imagery_publication::lock_image_sources_in(
        &tx,
        std::slice::from_ref(&receipt.pin),
        &receipt.pin.audience,
    )
    .await
    .unwrap();
    let source = &sources[0];
    let expected = Some(ImageConfig::defaults().unwrap().transforms.max_output_bytes);
    assert_eq!(
        crate::imagery_publication::read_generated_source_bound_in(
            &tx,
            &source.asset,
            &source.master,
        )
        .await
        .unwrap(),
        expected
    );
    let mut changed = source.master.clone();
    changed.manifest.as_mut().unwrap().sha256[0] ^= 1;
    assert_eq!(
        crate::imagery_publication::read_generated_source_bound_in(&tx, &source.asset, &changed,)
            .await
            .unwrap(),
        None,
        "a different digest cannot borrow the producer bound"
    );
    let mut changed = source.asset.clone();
    changed.origin_account = Some(AccountId::from_bytes(&[0x99; 16]).unwrap());
    assert_eq!(
        crate::imagery_publication::read_generated_source_bound_in(&tx, &changed, &source.master,)
            .await
            .unwrap(),
        None,
        "the producer must match the stored catalog owner"
    );
    changed = source.asset.clone();
    changed.provenance = df_assets::Provenance::ClaimedLibrary;
    assert_eq!(
        crate::imagery_publication::read_generated_source_bound_in(&tx, &changed, &source.master,)
            .await
            .unwrap(),
        None,
        "reviewed imports retain their separate bound"
    );
    assert_eq!(
        crate::imagery_publication::read_generated_source_bound_in(
            &tx,
            &source.asset,
            &source.master,
        )
        .await
        .unwrap(),
        expected,
        "the lookup is read-only"
    );
    let object = df_assets::StoredObject {
        key: source.master.key.clone(),
        version: source.master.version.clone().unwrap(),
        manifest: source.master.manifest.unwrap(),
    };
    tx.rollback().await.unwrap();
    let objects = df_assets::NativeFileStore::new(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../artifacts/tmp/funding-native-sources")
            .join(world.scratch.config().get_dbname().unwrap()),
        expected.unwrap(),
    )
    .unwrap();
    df_assets::ObjectStore::get_version(
        &objects,
        &object,
        df_assets::Deadline::after(std::time::Duration::from_secs(10)),
    )
    .await
    .unwrap()
}

async fn capture_response(
    evidence: UsageEvidence,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        tokio::time::timeout(std::time::Duration::from_secs(60), async move {
            let (mut socket, peer) = listener.accept().await.unwrap();
            assert!(peer.ip().is_loopback());
            let mut request = Vec::new();
            let mut chunk = [0u8;8192];
            loop {
                let count = socket.read(&mut chunk).await.unwrap();
                assert!(count > 0, "transport closed before prepared request completed");
                request.extend_from_slice(&chunk[..count]);
                assert!(request.len() <= 4_194_304);
                if let Some(header_end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let header = std::str::from_utf8(&request[..header_end]).unwrap();
                    let length = header.lines().find_map(|line| {
                        let (name,value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    }).expect("prepared JSON request has a content length");
                    if request.len() >= header_end + 4 + length { break; }
                }
            }
            if evidence == UsageEvidence::Unknown {
                socket.shutdown().await.unwrap();
                return;
            }
            assert_eq!(evidence, UsageEvidence::Reported);
            let body = br#"{"choices":[{"message":{"content":"invalid analytical JSON"},"finish_reason":"stop"}],"usage":{"prompt_tokens":100,"completion_tokens":100,"total_tokens":200}}"#;
            let header = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            socket.write_all(header.as_bytes()).await.unwrap();
            socket.write_all(body).await.unwrap();
            socket.shutdown().await.unwrap();
        }).await.unwrap();
    });
    (address, task)
}
