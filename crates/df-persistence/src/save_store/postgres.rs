//! PostgreSQL writer for save points (migration 0028). Each mutation is one transaction:
//! the save point, its links, the branch head and the operation result commit together.

use tokio_postgres::Client;

use super::{CreateSavePoint, SaveError, SaveLinkKind, SavePointId, StoredResult};

const MIGRATION: (i32, &str) = (28, include_str!("../../migrations/0028_save_points.sql"));

pub struct PostgresSaveStore {
    client: Client,
}

impl PostgresSaveStore {
    pub async fn open(mut client: Client) -> Result<Self, SaveError> {
        migrate(&mut client).await?;
        Ok(Self { client })
    }

    pub fn client_mut(&mut self) -> &mut Client {
        &mut self.client
    }

    /// SAVE-02: inserts state and the operation result in one transaction; a retry with the
    /// same fingerprint returns the stored result, a different one conflicts.
    pub async fn create_save_point(
        &mut self,
        request: &CreateSavePoint,
    ) -> Result<StoredResult, SaveError> {
        let transaction = self
            .client
            .transaction()
            .await
            .map_err(|_| SaveError::Unavailable)?;
        if let Some(stored) = lookup_operation(&transaction, request).await? {
            transaction
                .commit()
                .await
                .map_err(|_| SaveError::Unavailable)?;
            return Ok(stored);
        }
        transaction
            .execute(
                "INSERT INTO df_save.save_points (save_point_id, tenant_id, campaign_id, \
                 branch_id, parent_save_point_id, codec_version, schema_version, \
                 content_version, checkpoint) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
                &[
                    &request.save_point_id.as_bytes().as_slice(),
                    &request.tenant_id.as_bytes().as_slice(),
                    &request.campaign_id.as_bytes().as_slice(),
                    &request.branch_id.as_bytes().as_slice(),
                    &request
                        .parent_save_point_id
                        .map(|id| id.as_bytes().as_slice().to_vec()),
                    &i32::try_from(request.checkpoint.codec_version)
                        .map_err(|_| SaveError::InvalidId)?,
                    &i32::try_from(request.checkpoint.schema_version)
                        .map_err(|_| SaveError::InvalidId)?,
                    &i32::try_from(request.checkpoint.content_version)
                        .map_err(|_| SaveError::InvalidId)?,
                    &request.checkpoint.bytes.as_slice(),
                ],
            )
            .await
            .map_err(|_| SaveError::Unavailable)?;
        for link in &request.links {
            transaction
                .execute(
                    "INSERT INTO df_save.save_links (save_point_id, link_kind, linked_id) \
                     VALUES ($1,$2,$3) ON CONFLICT DO NOTHING",
                    &[
                        &request.save_point_id.as_bytes().as_slice(),
                        &link.kind.label(),
                        &link.linked_id.as_slice(),
                    ],
                )
                .await
                .map_err(|_| SaveError::Unavailable)?;
        }
        upsert_head(&transaction, request).await?;
        transaction
            .execute(
                "INSERT INTO df_save.save_operations (tenant_id, campaign_id, operation_id, \
                 fingerprint, result_save_point_id, result_bytes) VALUES ($1,$2,$3,$4,$5,$6)",
                &[
                    &request.tenant_id.as_bytes().as_slice(),
                    &request.campaign_id.as_bytes().as_slice(),
                    &request.operation_id.as_bytes().as_slice(),
                    &request.fingerprint.as_slice(),
                    &request.save_point_id.as_bytes().as_slice(),
                    &request.result_bytes.as_slice(),
                ],
            )
            .await
            .map_err(|_| SaveError::Unavailable)?;
        transaction
            .commit()
            .await
            .map_err(|_| SaveError::Unavailable)?;
        Ok(StoredResult {
            save_point_id: request.save_point_id,
            result_bytes: request.result_bytes.clone(),
            committed_now: true,
        })
    }

    /// SAVE-03: a restore/branch creates a new branch head from an older save. It reads the
    /// source checkpoint and links and writes only df_save rows; finance is untouched.
    pub async fn branch_from(
        &mut self,
        source: SavePointId,
        request: &CreateSavePoint,
    ) -> Result<StoredResult, SaveError> {
        let transaction = self
            .client
            .transaction()
            .await
            .map_err(|_| SaveError::Unavailable)?;
        if let Some(stored) = lookup_operation(&transaction, request).await? {
            transaction
                .commit()
                .await
                .map_err(|_| SaveError::Unavailable)?;
            return Ok(stored);
        }
        let row = transaction
            .query_opt(
                "SELECT codec_version, schema_version, content_version, checkpoint \
                 FROM df_save.save_points \
                 WHERE save_point_id = $1 AND tenant_id = $2 AND campaign_id = $3",
                &[
                    &source.as_bytes().as_slice(),
                    &request.tenant_id.as_bytes().as_slice(),
                    &request.campaign_id.as_bytes().as_slice(),
                ],
            )
            .await
            .map_err(|_| SaveError::Unavailable)?
            .ok_or(SaveError::SavePointNotFound)?;
        let checkpoint: Vec<u8> = row.get(3);
        transaction
            .execute(
                "INSERT INTO df_save.save_points (save_point_id, tenant_id, campaign_id, \
                 branch_id, parent_save_point_id, codec_version, schema_version, \
                 content_version, checkpoint) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
                &[
                    &request.save_point_id.as_bytes().as_slice(),
                    &request.tenant_id.as_bytes().as_slice(),
                    &request.campaign_id.as_bytes().as_slice(),
                    &request.branch_id.as_bytes().as_slice(),
                    &source.as_bytes().as_slice(),
                    &row.get::<_, i32>(0),
                    &row.get::<_, i32>(1),
                    &row.get::<_, i32>(2),
                    &checkpoint.as_slice(),
                ],
            )
            .await
            .map_err(|_| SaveError::Unavailable)?;
        transaction
            .execute(
                "INSERT INTO df_save.save_links (save_point_id, link_kind, linked_id) \
                 SELECT $1, link_kind, linked_id FROM df_save.save_links \
                 WHERE save_point_id = $2 ON CONFLICT DO NOTHING",
                &[
                    &request.save_point_id.as_bytes().as_slice(),
                    &source.as_bytes().as_slice(),
                ],
            )
            .await
            .map_err(|_| SaveError::Unavailable)?;
        upsert_head(&transaction, request).await?;
        transaction
            .execute(
                "INSERT INTO df_save.save_operations (tenant_id, campaign_id, operation_id, \
                 fingerprint, result_save_point_id, result_bytes) VALUES ($1,$2,$3,$4,$5,$6)",
                &[
                    &request.tenant_id.as_bytes().as_slice(),
                    &request.campaign_id.as_bytes().as_slice(),
                    &request.operation_id.as_bytes().as_slice(),
                    &request.fingerprint.as_slice(),
                    &request.save_point_id.as_bytes().as_slice(),
                    &request.result_bytes.as_slice(),
                ],
            )
            .await
            .map_err(|_| SaveError::Unavailable)?;
        transaction
            .commit()
            .await
            .map_err(|_| SaveError::Unavailable)?;
        Ok(StoredResult {
            save_point_id: request.save_point_id,
            result_bytes: request.result_bytes.clone(),
            committed_now: true,
        })
    }

    /// Claims the next fence for this campaign in df_save.resume_fences (monotonic claim_seq).
    pub async fn claim_resume_fence(
        &mut self,
        tenant_id: df_types::TenantId,
        campaign_id: df_types::CampaignId,
        placement_id: [u8; 16],
    ) -> Result<super::OwnerFence, SaveError> {
        let epoch: i64 = self
            .client
            .query_one(
                "SELECT COALESCE((SELECT recovery_epoch FROM df_meta.service_state LIMIT 1), 1)",
                &[],
            )
            .await
            .map(|row| row.get(0))
            .unwrap_or(1);
        let row = self
            .client
            .query_one(
                "INSERT INTO df_save.resume_fences (tenant_id, campaign_id, recovery_epoch, \
                 claim_seq, placement_id) \
                 SELECT $1, $2, $3, COALESCE(MAX(claim_seq), 0) + 1, $4 \
                 FROM df_save.resume_fences WHERE tenant_id = $1 AND campaign_id = $2 \
                 RETURNING claim_seq",
                &[
                    &tenant_id.as_bytes().as_slice(),
                    &campaign_id.as_bytes().as_slice(),
                    &epoch,
                    &placement_id.as_slice(),
                ],
            )
            .await
            .map_err(|_| SaveError::Unavailable)?;
        Ok(super::OwnerFence {
            recovery_epoch: u64::try_from(epoch).map_err(|_| SaveError::Unavailable)?,
            claim_seq: u64::try_from(row.get::<_, i64>(0)).map_err(|_| SaveError::Unavailable)?,
            placement_id,
        })
    }

    pub async fn link_count(&mut self, save_point_id: SavePointId) -> Result<i64, SaveError> {
        let row = self
            .client
            .query_one(
                "SELECT count(*) FROM df_save.save_links WHERE save_point_id = $1",
                &[&save_point_id.as_bytes().as_slice()],
            )
            .await
            .map_err(|_| SaveError::Unavailable)?;
        Ok(row.get(0))
    }

    pub fn kind_label(kind: SaveLinkKind) -> &'static str {
        kind.label()
    }
}

async fn lookup_operation(
    transaction: &tokio_postgres::Transaction<'_>,
    request: &CreateSavePoint,
) -> Result<Option<StoredResult>, SaveError> {
    let row = transaction
        .query_opt(
            "SELECT fingerprint, result_save_point_id, result_bytes \
             FROM df_save.save_operations \
             WHERE tenant_id = $1 AND campaign_id = $2 AND operation_id = $3",
            &[
                &request.tenant_id.as_bytes().as_slice(),
                &request.campaign_id.as_bytes().as_slice(),
                &request.operation_id.as_bytes().as_slice(),
            ],
        )
        .await
        .map_err(|_| SaveError::Unavailable)?;
    let Some(row) = row else { return Ok(None) };
    let fingerprint: Vec<u8> = row.get(0);
    if fingerprint.as_slice() != request.fingerprint.as_slice() {
        return Err(SaveError::OperationConflict);
    }
    let bytes: Vec<u8> = row.get(1);
    Ok(Some(StoredResult {
        save_point_id: SavePointId::from_bytes(&bytes)?,
        result_bytes: row.get(2),
        committed_now: false,
    }))
}

async fn upsert_head(
    transaction: &tokio_postgres::Transaction<'_>,
    request: &CreateSavePoint,
) -> Result<(), SaveError> {
    transaction
        .execute(
            "INSERT INTO df_save.branch_heads (tenant_id, campaign_id, branch_id, \
             head_save_point_id) VALUES ($1,$2,$3,$4) \
             ON CONFLICT (tenant_id, campaign_id, branch_id) \
             DO UPDATE SET head_save_point_id = EXCLUDED.head_save_point_id",
            &[
                &request.tenant_id.as_bytes().as_slice(),
                &request.campaign_id.as_bytes().as_slice(),
                &request.branch_id.as_bytes().as_slice(),
                &request.save_point_id.as_bytes().as_slice(),
            ],
        )
        .await
        .map_err(|_| SaveError::Unavailable)?;
    Ok(())
}

async fn migrate(client: &mut Client) -> Result<(), SaveError> {
    let transaction = client
        .transaction()
        .await
        .map_err(|_| SaveError::Unavailable)?;
    transaction
        .batch_execute(
            "SELECT pg_advisory_xact_lock(hashtext('df_save_migration')); \
             CREATE SCHEMA IF NOT EXISTS df_save; \
             CREATE TABLE IF NOT EXISTS df_save.schema_migration ( \
                 version integer PRIMARY KEY, applied_at timestamptz NOT NULL DEFAULT now())",
        )
        .await
        .map_err(|_| SaveError::Unavailable)?;
    let applied = transaction
        .query_opt(
            "SELECT 1 FROM df_save.schema_migration WHERE version = $1",
            &[&MIGRATION.0],
        )
        .await
        .map_err(|_| SaveError::Unavailable)?
        .is_some();
    if !applied {
        transaction
            .batch_execute(MIGRATION.1)
            .await
            .map_err(|_| SaveError::Unavailable)?;
        transaction
            .execute(
                "INSERT INTO df_save.schema_migration (version) VALUES ($1)",
                &[&MIGRATION.0],
            )
            .await
            .map_err(|_| SaveError::Unavailable)?;
    }
    transaction
        .commit()
        .await
        .map_err(|_| SaveError::Unavailable)
}
