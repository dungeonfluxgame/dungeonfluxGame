//! Actual native source bytes and a transactional current-grant fixture.
use super::*;
use df_imagery::runtime::SourceReceipt;

pub(super) struct Authority(pub ImageOwner);
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
            allowed
                .then_some(())
                .ok_or(PublicationError::BindingConflict)
        })
    }
    fn authorize_sources<'a>(
        &'a self,
        tx: &'a tokio_postgres::Transaction<'_>,
        job: &'a df_imagery::ImageJob,
        facts: &'a [PublicationSource],
        now_ms: i64,
    ) -> PublicationAuthorityFuture<'a> {
        Box::pin(async move {
            self.lock_authority(tx, &job.owner, now_ms).await?;
            if facts.iter().any(|fact| {
                fact.asset
                    .ready_at_ms
                    .is_none_or(|ready| now_ms >= ready + 1000)
            }) {
                return Err(PublicationError::BindingConflict);
            }
            Ok(())
        })
    }
}

pub(super) async fn install(client: &Client, fixture: &Fixture) -> SourceReceipt {
    let asset = AssetId::from_bytes(&[21; 16]).unwrap();
    let attempt = AttemptId::from_bytes(&[22; 16]).unwrap();
    let write = WriteId::from_bytes(&[23; 16]).unwrap();
    let owner = ClaimToken::from_bytes(&[24; 16]).unwrap();
    let campaign = CampaignId::from_bytes(&[10; 16]).unwrap();
    let layout = KeyLayout::new(ObjectEnvironment::new("test").unwrap());
    let key = layout.campaign_asset(&campaign, asset, attempt, write, AssetVariant::Master);
    let manifest = AssetManifest {
        byte_len: MASTER.len().try_into().unwrap(),
        sha256: Sha256::digest(MASTER).into(),
    };
    let store = NativeFileStore::new(&fixture.root, 1024 * 1024).unwrap();
    let stored = store
        .put_new(
            &key,
            MASTER,
            manifest,
            Deadline::after(Duration::from_secs(5)),
        )
        .await
        .unwrap();
    client.execute("INSERT INTO media.asset(asset_id,kind,provenance,charge_scope_kind,charge_scope_id,origin_account,campaign_id,audience_kind,audience_campaign,dedupe_scope,state,attempt,retention_class,created_at_ms,updated_at_ms,ready_at_ms,winning_writes,mime,byte_length,sha256,width_px,height_px) SELECT $1,kind,'reviewed',charge_scope_kind,charge_scope_id,origin_account,campaign_id,audience_kind,audience_campaign,dedupe_scope,'ready',$2,retention_class,created_at_ms,updated_at_ms,created_at_ms,ARRAY[$3]::bytea[],'image/png',$4,$5,1,1 FROM media.asset WHERE asset_id=$6", &[&asset.as_bytes().as_slice(),&attempt.as_bytes().as_slice(),&write.as_bytes().as_slice(),&i64::try_from(manifest.byte_len).unwrap(),&manifest.sha256.as_slice(),&fixture.held.asset.as_bytes().as_slice()]).await.unwrap();
    client.execute("INSERT INTO media.asset_variant(asset_id,attempt,write_id,variant,owner,state,object_domain,object_key,object_version,byte_length,sha256,created_at_ms) VALUES($1,$2,$3,'master',$4,'written','private_media',$5,$6,$7,$8,$9)", &[&asset.as_bytes().as_slice(),&attempt.as_bytes().as_slice(),&write.as_bytes().as_slice(),&owner.as_bytes().as_slice(),&stored.key.path(),&stored.version.as_str(),&i64::try_from(manifest.byte_len).unwrap(),&manifest.sha256.as_slice(),&NOW]).await.unwrap();
    let pin = df_imagery::SourceRef {
        asset,
        digest: df_imagery::AssetDigest::from_bytes(manifest.sha256),
        audience: Audience::CampaignShared(campaign),
    };
    let (intent, _) = intent_fixture::snapshots(&fixture.owner, Some(pin.clone())).await;
    client
        .execute(
            "UPDATE imagery.image_job SET intent_revision=$1,intent_bytes=$2",
            &[&i64::from(intent.revision()), &intent.bytes()],
        )
        .await
        .unwrap();
    let receipts=serde_json::json!([{"asset":asset.as_bytes(),"digest":manifest.sha256,"attempt":attempt.as_bytes(),"write":write.as_bytes(),"version":stored.version.as_str(),"suppression_generation":0,"retention":"campaign_media"}]).to_string();
    let receipt = SourceReceipt {
        pin,
        attempt,
        write,
        version: stored.version,
        suppression_generation: 0,
        retention: RetentionClass::CampaignMedia,
    };
    client
        .execute(
            "UPDATE imagery.image_dispatch_sources SET receipts=$1::text::jsonb,source_digest=$2",
            &[
                &receipts,
                &df_imagery::runtime::source_receipts_digest(std::slice::from_ref(&receipt))
                    .unwrap()
                    .as_slice(),
            ],
        )
        .await
        .unwrap();
    client.batch_execute("CREATE TABLE source_grant(allowed boolean NOT NULL);INSERT INTO source_grant VALUES(true); GRANT SELECT,UPDATE ON source_grant TO df_imagery_publication").await.unwrap();
    receipt
}

pub(super) async fn prepare_send(client: &mut Client, fixture: &Fixture) {
    let tx = client.transaction().await.unwrap();
    let scope = owner_scope_bytes(&fixture.owner);
    tx.execute("INSERT INTO imagery.image_operation(owner_scope,operation_id,fingerprint,job_id,interest_id,created_at_ms) VALUES($1,$2,$3,$4,$5,$6)", &[&scope,&[81u8;16].as_slice(),&[82u8;32].as_slice(),&fixture.command().job.as_bytes().as_slice(),&[80u8;16].as_slice(),&NOW]).await.unwrap();
    tx.execute("INSERT INTO imagery.image_interest(interest_id,owner_scope,job_id,operation_id,state,created_at_ms) VALUES($1,$2,$3,$4,'attached',$5)", &[&[80u8;16].as_slice(),&scope,&fixture.command().job.as_bytes().as_slice(),&[81u8;16].as_slice(),&NOW]).await.unwrap();
    df_persistence::imagery_execution::set_capacity_bounds_in(
        &tx,
        "test",
        &df_imagery::execution::CapacityBounds::new(2, 4, 1_048_576).unwrap(),
        NOW,
    )
    .await
    .unwrap();
    df_persistence::imagery_execution::set_execution_gate_in(&tx, "test", 1, true, NOW)
        .await
        .unwrap();
    tx.batch_execute("UPDATE imagery.image_execution_stage SET claim_config_revision=1; UPDATE imagery.image_dispatch_attempt SET dispatch_state='prepared',usage_revision=0; DELETE FROM imagery.image_dispatch_checkpoint").await.unwrap();
    tx.commit().await.unwrap();
}
