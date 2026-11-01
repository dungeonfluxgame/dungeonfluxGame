//! Current catalog/source authority, not execution interest age, governs historical Ready reuse.
use super::*;
use df_persistence::postgres_tls::PostgresTransport;

#[tokio::test]
async fn historical_ready_reuse_checks_real_grants_versions_retention_and_tombstones() {
    let scratch = Scratch::open().await;
    let mut client = connect(&scratch.config).await;
    let fixture = Fixture::create(&client, false, true).await;
    let source = source_fixture::install(&client, &fixture).await;
    let authority = source_fixture::Authority(fixture.owner.clone());
    let tx = client.transaction().await.unwrap();
    let mut command = fixture.command();
    command.source_authority = Some(&authority);
    let PublicationOutcome::Recorded(result) = finalize_image_in(&tx, &command).await.unwrap()
    else {
        panic!("actual Ready missing")
    };
    tx.commit().await.unwrap();
    client
        .execute(
            "UPDATE imagery.image_job SET state='retired',absolute_deadline_ms=$1",
            &[&(NOW + 5)],
        )
        .await
        .unwrap();
    let mut config = scratch.config.clone();
    config.options("-c role=df_imagery_output");
    let store = PostgresImagePublication::open_existing(
        config,
        PostgresTransport::PlaintextLoopbackOnly,
        1,
    )
    .await
    .unwrap();
    assert_eq!(
        store
            .read_reusable_result_with_authority(&fixture.owner, &result.job, NOW + 20, &authority)
            .await
            .unwrap(),
        Some(result.clone())
    );
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM imagery.image_interest", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    assert_eq!(
        client
            .query_one("SELECT state FROM imagery.image_job", &[])
            .await
            .unwrap()
            .get::<_, String>(0),
        "retired"
    );
    client
        .batch_execute("UPDATE source_grant SET allowed=false")
        .await
        .unwrap();
    assert!(
        store
            .read_reusable_result_with_authority(&fixture.owner, &result.job, NOW + 20, &authority)
            .await
            .unwrap()
            .is_none()
    );
    client
        .batch_execute("UPDATE source_grant SET allowed=true")
        .await
        .unwrap();
    let master = result
        .variants
        .iter()
        .find(|variant| variant.variant == AssetVariant::Master)
        .unwrap();
    client
        .execute(
            "UPDATE media.asset_variant SET object_version='changed-version' WHERE asset_id=$1",
            &[&result.asset.as_bytes().as_slice()],
        )
        .await
        .unwrap();
    assert!(
        store
            .read_reusable_result_with_authority(&fixture.owner, &result.job, NOW + 20, &authority)
            .await
            .unwrap()
            .is_none()
    );
    client
        .execute(
            "UPDATE media.asset_variant SET object_version=$1 WHERE asset_id=$2",
            &[
                &master.object_version.as_str(),
                &result.asset.as_bytes().as_slice(),
            ],
        )
        .await
        .unwrap();
    client
        .execute(
            "UPDATE media.asset SET retention_class='personal' WHERE asset_id=$1",
            &[&source.pin.asset.as_bytes().as_slice()],
        )
        .await
        .unwrap();
    assert!(
        store
            .read_reusable_result_with_authority(&fixture.owner, &result.job, NOW + 20, &authority)
            .await
            .unwrap()
            .is_none()
    );
    client
        .execute(
            "UPDATE media.asset SET retention_class='campaign_media' WHERE asset_id=$1",
            &[&source.pin.asset.as_bytes().as_slice()],
        )
        .await
        .unwrap();
    assert!(
        store
            .read_reusable_result_with_authority(&fixture.owner, &result.job, NOW + 20, &authority)
            .await
            .unwrap()
            .is_some()
    );
    client.execute("INSERT INTO media.tombstone(asset_id,suppression_generation,reason,at_ms) VALUES($1,1,'restore_replay',$2)", &[&source.pin.asset.as_bytes().as_slice(),&(NOW+19)]).await.unwrap();
    assert!(
        store
            .read_reusable_result_with_authority(&fixture.owner, &result.job, NOW + 20, &authority)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        client
            .query_one(
                "SELECT state FROM media.asset WHERE asset_id=$1",
                &[&source.pin.asset.as_bytes().as_slice()]
            )
            .await
            .unwrap()
            .get::<_, String>(0),
        "ready"
    );
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM imagery.image_dispatch_attempt", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        1
    );
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM finance.provider_attempt", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    drop(store);
    scratch.finish().await;
}
