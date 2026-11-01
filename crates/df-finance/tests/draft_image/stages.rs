use super::*;
use df_commerce::draft_image::{JobConsent, JobScope, PaidStageKind, StageCall, StageRoute};
use df_finance::draft_image::{
    StageAdmissionPort, StageQuoteCommand, StageReceipt, StoredStageQuote,
};

fn job_consent(cap: u64) -> JobConsent {
    let b = binding(10);
    let route = |key, kind| StageRoute {
        key: [key; 32],
        kind,
        provider: b.scope.provider.clone(),
        model: b.scope.model.clone(),
        route: b.scope.route.clone(),
        cause: "creator_portrait".into(),
        detail: "portrait".into(),
        price_version: b.scope.price_version,
        maximum_units: b.scope.maximum_units.clone(),
    };
    JobConsent {
        id: ConsentId([90; 16]),
        scope: JobScope {
            account: account(),
            draft: b.scope.draft,
            draft_revision: 1,
            job: b.scope.job,
            source_digest: b.scope.source_digest,
            config_revision: 1,
            config_digest: b.scope.config_digest,
            routes: vec![
                route(1, PaidStageKind::Classification),
                route(2, PaidStageKind::Generation),
            ],
        },
        cap_micro_dollars: cap,
        expires_at_ms: now() + 60_000,
        revision: 1,
    }
}
fn stage_command(grant: &JobConsent, tag: u8, route: usize) -> StageQuoteCommand {
    let selected = grant.scope.routes[route].clone();
    StageQuoteCommand {
        consent: grant.id,
        consent_revision: 1,
        scope: grant.scope.clone(),
        call: StageCall {
            job: grant.scope.job,
            stage: StageId([tag; 16]),
            dispatch: [tag + 1; 16],
            parent_dispatch: (route > 0).then_some([11; 16]),
            kind: selected.kind,
            input_revision: 1,
            input_digest: [3; 32],
            config_revision: 1,
            config_digest: grant.scope.config_digest,
            planned_publication: Some(df_commerce::draft_image::StagePublication {
                asset: AssetId([20; 16]),
                publication: PublicationId([21; 16]),
            }),
            payload_digest: [tag; 32],
        },
        route: selected,
        quote_lifetime_ms: 30_000,
    }
}
struct StageLink;
impl StageAdmissionPort for StageLink {
    async fn bind(&self, tx: &Transaction<'_>, receipt: &StageReceipt) -> Result<(), Error> {
        Link { refuse: false }.record(tx, &receipt.money).await
    }
}
async fn stage_quote(world: &World, command: &StageQuoteCommand) -> StoredStageQuote {
    world
        .service()
        .await
        .quote_draft_stage(account(), command)
        .await
        .unwrap()
}
#[test]
fn one_job_grant_caps_different_stage_routes_and_recovers_the_original_quote_tuple() {
    support::run(async {
        let Some(world) = world().await else { return };
        let grant = job_consent(150);
        world
            .service()
            .await
            .record_draft_job_consent(account(), &grant)
            .await
            .unwrap();
        let first_command = stage_command(&grant, 10, 0);
        assert!(
            world
                .service()
                .await
                .read_draft_stage_quote(account(), &first_command.call)
                .await
                .unwrap()
                .is_none()
        );
        let first = stage_quote(&world, &first_command).await;
        let read = world
            .service()
            .await
            .read_draft_stage_quote(account(), &first_command.call)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(read.quote(), first.quote());
        assert_eq!(read.attempt(), first.attempt());
        let mut changed = first_command.call.clone();
        changed.parent_dispatch = Some([98; 16]);
        assert!(matches!(
            world
                .service()
                .await
                .read_draft_stage_quote(account(), &changed)
                .await,
            Err(Error::LinkConflict)
        ));
        let replay = stage_quote(&world, &first_command).await;
        assert_eq!(first.quote(), replay.quote());
        assert_eq!(first.attempt(), replay.attempt());
        assert_eq!(first.operation(), replay.operation());
        assert_eq!(first.expires_at_ms(), replay.expires_at_ms());
        let second = stage_quote(&world, &stage_command(&grant, 30, 1)).await;
        let mut left = world.service().await;
        let mut right = world.service().await;
        let (a, b) = tokio::join!(
            left.admit_draft_stage(account(), &first, &StageLink),
            right.admit_draft_stage(account(), &second, &StageLink)
        );
        let admitted = usize::from(a.is_ok()) + usize::from(b.is_ok());
        let refused = usize::from(matches!(
            a,
            Err(Error::Refused(AdmissionRefusal::ConsentCapExceeded))
        )) + usize::from(matches!(
            b,
            Err(Error::Refused(AdmissionRefusal::ConsentCapExceeded))
        ));
        assert_eq!((admitted, refused), (1, 1), "{a:?} {b:?}");
        assert_eq!(world.natural(2150, Some(payer(1)), None, None).await, 90);
        assert_eq!(world.count("provider_attempt").await, 1);
        drop(left);
        drop(right);
        world.finish().await;
    });
}
#[test]
fn classification_has_no_media_and_changed_parent_or_kind_cannot_reuse_a_quote() {
    support::run(async {
        let Some(world) = world().await else { return };
        let grant = job_consent(300);
        let mut service = world.service().await;
        service
            .record_draft_job_consent(account(), &grant)
            .await
            .unwrap();
        let command = stage_command(&grant, 10, 0);
        let quote = stage_quote(&world, &command).await;
        let receipt = service
            .admit_draft_stage(account(), &quote, &StageLink)
            .await
            .unwrap();
        let asset: Option<Vec<u8>> = world
            .admin
            .query_one(
                "SELECT media_asset_id FROM finance.provider_attempt WHERE attempt_id=$1",
                &[&receipt.money.attempt.as_bytes().as_slice()],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(asset, None);
        let mut changed = command.clone();
        changed.call.parent_dispatch = Some([99; 16]);
        assert!(matches!(
            service.quote_draft_stage(account(), &changed).await,
            Err(Error::Refused(AdmissionRefusal::Conflict))
        ));
        changed = command.clone();
        changed.call.kind = PaidStageKind::Generation;
        assert!(matches!(
            service.quote_draft_stage(account(), &changed).await,
            Err(Error::Policy(_))
        ));
        let replay = service
            .admit_draft_stage(account(), &quote, &StageLink)
            .await
            .unwrap();
        assert!(replay.money.replay);
        service
            .revoke_draft_image_consent(account(), grant.id)
            .await
            .unwrap();
        let child = stage_command(&grant, 30, 1);
        assert!(matches!(
            service.quote_draft_stage(account(), &child).await,
            Err(Error::Refused(AdmissionRefusal::Revoked))
        ));
        drop(service);
        world.finish().await;
    });
}
#[test]
fn child_route_maxima_cannot_exceed_the_explicit_job_envelope() {
    support::run(async {
        let Some(world) = world().await else { return };
        let grant = job_consent(300);
        let mut service = world.service().await;
        service
            .record_draft_job_consent(account(), &grant)
            .await
            .unwrap();
        let mut command = stage_command(&grant, 10, 0);
        command.route.maximum_units[0].1 += 1;
        assert!(matches!(
            service.quote_draft_stage(account(), &command).await,
            Err(Error::Policy(_))
        ));
        command = stage_command(&grant, 10, 0);
        command.route.cause = "unconsented".into();
        assert!(matches!(
            service.quote_draft_stage(account(), &command).await,
            Err(Error::Policy(_))
        ));
        assert_eq!(world.count("provider_attempt").await, 0);
        drop(service);
        world.finish().await;
    });
}

struct CampaignPort;
impl df_finance::ProviderStagePort for CampaignPort {
    fn bind<'a>(
        &'a self,
        tx: &'a Transaction<'_>,
        binding: &'a df_finance::ProviderStageBinding,
        phase: df_finance::ImageStagePhase,
        receipt: &'a df_commerce::snapshot::ReceiptRow,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        df_finance::ImageStageHookOutcome,
                        df_finance::service::FinanceError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            if phase == df_finance::ImageStagePhase::Replay {
                let row = tx
                    .query_opt(
                        "SELECT 1 FROM draft_test_link WHERE attempt=$1 AND fingerprint=$2",
                        &[
                            &binding.attempt.as_bytes().as_slice(),
                            &receipt.fingerprint.as_slice(),
                        ],
                    )
                    .await?;
                if row.is_none() {
                    return Err(df_finance::service::FinanceError::ImageStageConflict);
                }
                return Ok(df_finance::ImageStageHookOutcome::Existing);
            }
            tx.execute(
                "INSERT INTO draft_test_link VALUES($1,$2)",
                &[
                    &binding.attempt.as_bytes().as_slice(),
                    &receipt.fingerprint.as_slice(),
                ],
            )
            .await?;
            Ok(df_finance::ImageStageHookOutcome::Accepted)
        })
    }
}
impl df_finance::ProviderSendPort for CampaignPort {
    async fn verify(
        &self,
        tx: &Transaction<'_>,
        binding: &df_finance::ProviderStageBinding,
        _new_send: bool,
    ) -> Result<(), df_finance::service::FinanceError> {
        if tx
            .query_opt(
                "SELECT 1 FROM draft_test_link WHERE attempt=$1",
                &[&binding.attempt.as_bytes().as_slice()],
            )
            .await?
            .is_none()
        {
            return Err(df_finance::service::FinanceError::ImageStageConflict);
        }
        Ok(())
    }
}
impl df_finance::ProviderSettlementPort for CampaignPort {
    async fn verify(
        &self,
        tx: &Transaction<'_>,
        command: &df_finance::ProviderStageSettlement,
    ) -> Result<(), df_finance::service::FinanceError> {
        let units = serde_json::json!(command.actual_units).to_string();
        if tx.query_opt("SELECT 1 FROM draft_test_usage WHERE attempt=$1 AND revision=$2 AND units=$3::text::jsonb",
        &[&command.binding.attempt.as_bytes().as_slice(),&i64::from(command.usage_revision),&units]).await?.is_none() {
        return Err(df_finance::service::FinanceError::ImageStageConflict);
    }
        Ok(())
    }
    async fn mirror(
        &self,
        _tx: &Transaction<'_>,
        _command: &df_finance::ProviderStageSettlement,
        _phase: df_finance::ImageStagePhase,
    ) -> Result<(), df_finance::service::FinanceError> {
        Ok(())
    }
}
#[test]
fn campaign_decision_admits_without_media_and_settles_only_the_persisted_selected_usage() {
    support::run(async {
        let Some(world) = world().await else { return };
        let mut service = world.service().await;
        service
            .allocate(payer(1), world.session, operation(1), 400, world.consent())
            .await
            .unwrap();
        world.admin.batch_execute("CREATE TABLE draft_test_usage(attempt bytea PRIMARY KEY,revision bigint,units jsonb); GRANT SELECT ON draft_test_usage TO df_finance").await.unwrap();
        let grant = job_consent(300);
        let quote = stage_command(&grant, 30, 0);
        let mut admit = world.admit_command(10, 100, 100);
        admit.route.name = quote.route.route.clone();
        let binding = df_finance::ProviderStageBinding {
            owner_scope: df_finance::OwnerScope::new(vec![1; 34]).unwrap(),
            call: quote.call,
            route_key: quote.route.key,
            route: quote.route,
            tenant: world.tenant,
            campaign: world.campaign,
            session: world.session,
            operation: admit.operation,
            attempt: admit.attempt,
            effect: admit.effect,
            media_asset: None,
        };
        let command = df_finance::ProviderStageAdmission {
            admit,
            binding: binding.clone(),
        };
        service
            .admit_provider_stage(&command, &CampaignPort)
            .await
            .unwrap();
        assert!(matches!(
            service
                .admit_provider_stage(&command, &CampaignPort)
                .await
                .unwrap(),
            df_finance::admit::AdmissionOutcome::AlreadyAdmitted { .. }
        ));
        assert!(
            service
                .mark_provider_stage_sent(&binding, [77; 16], &CampaignPort)
                .await
                .unwrap()
        );
        assert!(
            !service
                .mark_provider_stage_sent(&binding, [77; 16], &CampaignPort)
                .await
                .unwrap()
        );
        let settlement = df_finance::ProviderStageSettlement {
            binding: binding.clone(),
            usage_revision: 1,
            evidence: df_finance::draft_image::StageUsageEvidence::Reported,
            actual_units: vec![("text_input".into(), 50), ("text_output".into(), 50)],
            usability: Usability::Usable,
        };
        assert!(matches!(
            service
                .settle_provider_stage(&settlement, &CampaignPort)
                .await,
            Err(df_finance::service::FinanceError::ImageStageConflict)
        ));
        world
            .admin
            .execute(
                "INSERT INTO draft_test_usage VALUES($1,1,$2::text::jsonb)",
                &[
                    &binding.attempt.as_bytes().as_slice(),
                    &serde_json::json!(settlement.actual_units).to_string(),
                ],
            )
            .await
            .unwrap();
        assert!(matches!(
            service
                .settle_provider_stage(&settlement, &CampaignPort)
                .await
                .unwrap(),
            df_finance::ProviderSettlementOutcome::Applied(_)
        ));
        assert!(matches!(
            service
                .settle_provider_stage(&settlement, &CampaignPort)
                .await
                .unwrap(),
            df_finance::ProviderSettlementOutcome::Replay
        ));
        let asset: Option<Vec<u8>> = world
            .admin
            .query_one(
                "SELECT media_asset_id FROM finance.provider_attempt WHERE attempt_id=$1",
                &[&binding.attempt.as_bytes().as_slice()],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(asset, None);
        drop(service);
        world.finish().await;
    });
}

#[test]
fn one_job_cap_refuses_mixed_currency_and_unpublished_pinned_route_versions() {
    support::run(async {
        let Some(world) = world().await else { return };
        let mut service = world.service().await;
        let mut version = world.version.clone();
        version.id = df_commerce::finance_ids::PriceVersionId::from_array([0x71; 16]);
        version.currency = df_commerce::ledger::CurrencyTag::parse("EUR").unwrap();
        version.label = "pv-stage-eur".into();
        service.publish_price_version(&version).await.unwrap();
        let mut grant = job_consent(300);
        grant.scope.routes[1].price_version = *version.id.as_bytes();
        let protected:String=world.admin.query_one("SELECT pg_get_functiondef('finance.draft_image_command(bytea,jsonb)'::regprocedure)",&[]).await.unwrap().get(0);
        let unprotected=protected.replace("IF p->'scope'->>'scope_kind'='job' THEN PERFORM finance.draft_job_currency(p->'scope'); END IF;","")
            .replace("IF c.scope->>'scope_kind'='job' THEN PERFORM finance.draft_job_currency(c.scope); END IF;","");
        world.admin.batch_execute(&unprotected).await.unwrap();
        let before = service.record_draft_job_consent(account(), &grant).await;
        world.admin.batch_execute(&protected).await.unwrap();
        grant.id = ConsentId([92; 16]);
        let mixed = service.record_draft_job_consent(account(), &grant).await;
        grant.id = ConsentId([91; 16]);
        grant.scope.routes[1].price_version = [0x72; 16];
        let missing = service.record_draft_job_consent(account(), &grant).await;
        let attempts = world.count("provider_attempt").await;
        let available = world.natural(2110, Some(payer(1)), None, None).await;
        drop(service);
        world.finish().await;
        assert!(before.is_ok(), "before guard: {before:?}");
        assert!(
            matches!(
                mixed,
                Err(Error::Refused(AdmissionRefusal::CurrencyMismatch))
            ),
            "{mixed:?}"
        );
        assert!(
            matches!(missing, Err(Error::Refused(AdmissionRefusal::Unpriced))),
            "{missing:?}"
        );
        assert_eq!((attempts, available), (0, 1000));
    });
}

#[test]
fn standalone_stage_reserves_existing_money_without_any_planned_or_catalog_identity() {
    support::run(async {
        let Some(world) = world().await else { return };
        let mut grant = job_consent(200);
        grant.scope.routes[0].kind = PaidStageKind::HeadLocations;
        let mut service = world.service().await;
        service
            .record_draft_job_consent(account(), &grant)
            .await
            .unwrap();
        let mut command = stage_command(&grant, 10, 0);
        command.call.planned_publication = None;
        let quote = service
            .quote_draft_stage(account(), &command)
            .await
            .unwrap();
        assert!(quote.binding().planned_publication.is_none());
        let receipt = service
            .admit_draft_stage(account(), &quote, &StageLink)
            .await
            .unwrap();
        assert!(!receipt.money.replay);
        let row=world.admin.query_one("SELECT a.media_asset_id,q.draft_binding->'asset'='null'::jsonb, \
            q.draft_binding->'publication'='null'::jsonb,q.draft_binding->'call'->'asset'='null'::jsonb \
            FROM finance.provider_attempt a JOIN finance.reservation r USING(reservation_id) JOIN finance.quote q USING(quote_id) WHERE a.attempt_id=$1",
            &[&receipt.money.attempt.as_bytes().as_slice()]).await.unwrap();
        assert!(row.get::<_, Option<Vec<u8>>>(0).is_none());
        for index in 1..4 {
            assert!(row.get::<_, bool>(index));
        }
        assert!(matches!(
            service
                .release_unsent_draft_image(account(), receipt.money.attempt)
                .await,
            Err(Error::Refused(AdmissionRefusal::BindingChanged))
        ));
        let recovered = service
            .read_draft_stage_quote(account(), &command.call)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(recovered.operation(), quote.operation());
        assert!(recovered.call().planned_publication.is_none());
        command.call.planned_publication = Some(df_commerce::draft_image::StagePublication {
            asset: AssetId([20; 16]),
            publication: PublicationId([21; 16]),
        });
        assert!(matches!(
            service.quote_draft_stage(account(), &command).await,
            Err(Error::Policy(
                df_commerce::draft_image::Refusal::InvalidBinding
            ))
        ));
        world.finish().await;
    });
}
