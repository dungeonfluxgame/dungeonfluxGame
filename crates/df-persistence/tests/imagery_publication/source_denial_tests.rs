//! Real locked source changes close delivery; malformed facts and outages do not become denials.
use super::*;
use df_imagery::execution::*;
use df_imagery::publication::{FinalOutputSelection, SourceClosure, SourceDenial};
use df_imagery::runtime::{ImageDispatchAdmission, ImagePublisher, PublicationContext};
use std::sync::Arc;

struct Authority(ImageOwner);
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
                .query_one("SELECT allowed FROM source_grant FOR UPDATE", &[])
                .await
                .map_err(|_| PublicationError::Unavailable)?
                .get::<_, bool>(0);
            if allowed {
                Ok(())
            } else {
                Err(PublicationError::SourceDenied(SourceDenial::GrantRefused))
            }
        })
    }
    fn authorize_sources<'a>(
        &'a self,
        tx: &'a tokio_postgres::Transaction<'_>,
        job: &'a df_imagery::ImageJob,
        _: &'a [PublicationSource],
        now: i64,
    ) -> PublicationAuthorityFuture<'a> {
        self.lock_authority(tx, &job.owner, now)
    }
}
struct Unavailable;
impl PublicationSourceAuthority for Unavailable {
    fn lock_authority<'a>(
        &'a self,
        _: &'a tokio_postgres::Transaction<'_>,
        _: &'a ImageOwner,
        _: i64,
    ) -> PublicationAuthorityFuture<'a> {
        Box::pin(async { Err(PublicationError::Unavailable) })
    }
    fn authorize_sources<'a>(
        &'a self,
        _: &'a tokio_postgres::Transaction<'_>,
        _: &'a df_imagery::ImageJob,
        _: &'a [PublicationSource],
        _: i64,
    ) -> PublicationAuthorityFuture<'a> {
        Box::pin(async { Err(PublicationError::Unavailable) })
    }
}

fn terminal(context: &PublicationContext, selected: &FinalOutputSelection) -> TerminalCommand {
    let scope = ExecutionScope {
        owner: context.job.owner.clone(),
        job: context.job.job,
        stage: context.stage.stage,
        dispatch: context.dispatch,
    };
    TerminalCommand {
        actor: TerminalActor::Dispatch(scope.clone()),
        tail: TerminalActor::Dispatch(scope),
        authority: TerminalAuthority::Claim(StageClaim {
            scope: StageScope {
                owner: context.job.owner.clone(),
                job: context.job.job,
                stage: context.stage.stage,
            },
            token: context.claim,
            generation: context.stage.claim_generation,
        }),
        expected_job_revision: context.job.revision,
        candidate_dispatch: Some(selected.candidate.as_ref().unwrap().scope.dispatch),
        now_ms: NOW + 10,
    }
}

async fn assert_open(client: &Client, fixture: &Fixture) {
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM imagery.image_no_delivery", &[])
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
            .query_one("SELECT reserved_bytes FROM media.quota_usage", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        1000
    );
    assert_eq!(
        client
            .query_one(
                "SELECT state FROM media.asset WHERE asset_id=$1",
                &[&fixture.held.asset.as_bytes().as_slice()]
            )
            .await
            .unwrap()
            .get::<_, String>(0),
        "uploading"
    );
}

#[tokio::test]
async fn locked_source_denial_seals_once_and_preserves_unknown_supplier_liability() {
    for (change, unknown) in [
        ("UPDATE source_grant SET allowed=false", false),
        (
            "UPDATE media.asset_variant SET object_version='changed-version' WHERE asset_id=decode(repeat('15',16),'hex')",
            false,
        ),
        (
            "UPDATE media.asset SET retention_class='personal' WHERE asset_id=decode(repeat('15',16),'hex')",
            true,
        ),
        (
            "INSERT INTO media.tombstone(asset_id,suppression_generation,reason,at_ms) VALUES(decode(repeat('15',16),'hex'),1,'restore_replay',1700000000001)",
            true,
        ),
    ] {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config).await;
        let fixture = Fixture::create(&client, false, true).await;
        source_fixture::install(&client, &fixture).await;
        let (context, selected) = if unknown {
            selection_fixture::prepare_unknown_candidate(&mut client, &fixture).await
        } else {
            selection_fixture::prepare(&mut client, &fixture).await
        };
        let command = terminal(&context, &selected);
        let publisher = selection_fixture::publisher(&scratch, &fixture)
            .await
            .with_source_authority(Arc::new(Authority(fixture.owner.clone())));
        client.batch_execute(change).await.unwrap();
        let SourceClosure::Closed(receipt) = publisher
            .no_delivery_if_sources_denied(&command)
            .await
            .unwrap()
        else {
            panic!("current source denial was not closed")
        };
        assert_eq!(
            receipt.reason,
            df_imagery::runtime::NoDeliveryReason::SourceDenied
        );
        assert_eq!(
            publisher
                .no_delivery_if_sources_denied(&command)
                .await
                .unwrap(),
            SourceClosure::Closed(receipt.clone())
        );
        assert_eq!(
            client
                .query_one(
                    "SELECT reject_reason FROM media.asset WHERE asset_id=$1",
                    &[&fixture.held.asset.as_bytes().as_slice()]
                )
                .await
                .unwrap()
                .get::<_, String>(0),
            "source_denied"
        );
        assert_eq!(
            client
                .query_one("SELECT reserved_bytes FROM media.quota_usage", &[])
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
            1
        );
        assert_eq!(
            client
                .query_one("SELECT count(*) FROM imagery.image_job_result", &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            0
        );
        let candidate = selected.candidate.as_ref().unwrap();
        assert_eq!(
            publisher
                .image_economic_use(&candidate.scope)
                .await
                .unwrap(),
            Some(df_imagery::runtime::ImageEconomicDisposition {
                usage_revision: 1,
                usable: false
            })
        );
        publisher
            .outcome(
                &context.job,
                candidate.scope.dispatch,
                DispatchMode::Local,
                if unknown { None } else { Some(false) },
            )
            .await
            .unwrap();
        assert_eq!(
            client
                .query_one(
                    "SELECT settlement FROM imagery.image_dispatch_attempt WHERE dispatch_id=$1",
                    &[&candidate.scope.dispatch.as_bytes().as_slice()]
                )
                .await
                .unwrap()
                .get::<_, String>(0),
            if unknown {
                "unknown_outstanding"
            } else {
                "unusable_settled"
            }
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
}

#[tokio::test]
async fn current_authorized_sources_return_allowed_without_a_seal_or_ready() {
    let scratch = Scratch::open().await;
    let mut client = connect(&scratch.config).await;
    let fixture = Fixture::create(&client, false, true).await;
    source_fixture::install(&client, &fixture).await;
    let (context, selected) = selection_fixture::prepare(&mut client, &fixture).await;
    let publisher = selection_fixture::publisher(&scratch, &fixture)
        .await
        .with_source_authority(Arc::new(Authority(fixture.owner.clone())));
    assert_eq!(
        publisher
            .no_delivery_if_sources_denied(&terminal(&context, &selected))
            .await
            .unwrap(),
        SourceClosure::PublicationAllowed
    );
    assert_open(&client, &fixture).await;
    drop(publisher);
    scratch.finish().await;
}

#[tokio::test]
async fn malformed_catalog_and_unavailable_authority_never_become_source_denial_seals() {
    for unavailable in [true, false] {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config).await;
        let fixture = Fixture::create(&client, false, true).await;
        source_fixture::install(&client, &fixture).await;
        let (context, selected) = selection_fixture::prepare(&mut client, &fixture).await;
        let authority: Arc<dyn PublicationSourceAuthority> = if unavailable {
            Arc::new(Unavailable)
        } else {
            Arc::new(Authority(fixture.owner.clone()))
        };
        if !unavailable {
            client.batch_execute("INSERT INTO media.asset_variant(asset_id,attempt,write_id,variant,owner,state,object_domain,object_key,object_version,byte_length,sha256,created_at_ms) SELECT asset_id,attempt,decode(repeat('63',16),'hex'),variant,owner,state,object_domain,object_key,object_version,byte_length,sha256,created_at_ms FROM media.asset_variant WHERE asset_id=decode(repeat('15',16),'hex'); UPDATE media.asset SET winning_writes=array_append(winning_writes,decode(repeat('63',16),'hex')) WHERE asset_id=decode(repeat('15',16),'hex')").await.unwrap();
        }
        let publisher = selection_fixture::publisher(&scratch, &fixture)
            .await
            .with_source_authority(authority);
        assert!(
            publisher
                .no_delivery_if_sources_denied(&terminal(&context, &selected))
                .await
                .is_err()
        );
        assert_open(&client, &fixture).await;
        drop(publisher);
        scratch.finish().await;
    }
}

#[tokio::test]
async fn stale_source_closure_authority_does_not_reject_the_current_allocation() {
    let scratch = Scratch::open().await;
    let mut client = connect(&scratch.config).await;
    let fixture = Fixture::create(&client, false, true).await;
    source_fixture::install(&client, &fixture).await;
    let (context, selected) = selection_fixture::prepare(&mut client, &fixture).await;
    client
        .batch_execute("UPDATE source_grant SET allowed=false")
        .await
        .unwrap();
    let publisher = selection_fixture::publisher(&scratch, &fixture)
        .await
        .with_source_authority(Arc::new(Authority(fixture.owner.clone())));
    let mut command = terminal(&context, &selected);
    command.expected_job_revision += 1;
    assert!(
        publisher
            .no_delivery_if_sources_denied(&command)
            .await
            .is_err()
    );
    assert_open(&client, &fixture).await;
    drop(publisher);
    scratch.finish().await;
}

#[tokio::test]
async fn source_denial_locks_and_all_catalog_changes_rollback_together() {
    let scratch = Scratch::open().await;
    let mut client = connect(&scratch.config).await;
    let mut competing = connect(&scratch.config).await;
    let fixture = Fixture::create(&client, false, true).await;
    source_fixture::install(&client, &fixture).await;
    let (context, selected) = selection_fixture::prepare(&mut client, &fixture).await;
    client.batch_execute("UPDATE media.asset_variant SET object_version='changed-version' WHERE asset_id=decode(repeat('15',16),'hex')").await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.batch_execute("SET LOCAL ROLE df_imagery_output")
        .await
        .unwrap();
    assert!(matches!(
        no_delivery_if_sources_denied_in(
            &tx,
            &terminal(&context, &selected),
            &Authority(fixture.owner.clone())
        )
        .await
        .unwrap(),
        SourceClosure::Closed(_)
    ));
    for query in [
        "SELECT allowed FROM source_grant FOR UPDATE NOWAIT",
        "SELECT asset_id FROM media.asset WHERE asset_id=decode(repeat('15',16),'hex') FOR UPDATE NOWAIT",
        "SELECT asset_id FROM media.asset WHERE asset_id=decode(repeat('05',16),'hex') FOR UPDATE NOWAIT",
    ] {
        let second = competing.transaction().await.unwrap();
        assert_eq!(
            second
                .query(query, &[])
                .await
                .unwrap_err()
                .code()
                .unwrap()
                .code(),
            "55P03"
        );
        second.rollback().await.unwrap();
    }
    tx.rollback().await.unwrap();
    assert_open(&client, &fixture).await;
    scratch.finish().await;
}
