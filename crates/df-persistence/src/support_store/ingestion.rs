//! Mailbox fencing and durable classification claims. Interrupted model work stays manual;
//! a replay never repeats a possibly billed judgement.
use df_support::{contracts::TicketId, ingest::MailboxLease, ports::SupportStoreError};
use serde_json::{Value, json};

/// Native support sync persistence, using a dedicated bounded connection.
pub struct IngestState {
    client: tokio::sync::Mutex<tokio_postgres::Client>,
}

impl IngestState {
    pub fn new(client: tokio_postgres::Client) -> Self {
        Self {
            client: tokio::sync::Mutex::new(client),
        }
    }

    /// Claims one bounded sync slice. Expired owners cannot advance a successor's cursor.
    pub async fn claim_mailbox(
        &self,
        mailbox_id: &str,
        address: &str,
        now_unix_s: i64,
    ) -> Result<Option<MailboxLease>, SupportStoreError> {
        let client = self.client.lock().await;
        client
            .execute(
                "INSERT INTO support.mailbox(mailbox_id,address,bootstrap_boundary) \
             VALUES($1,$2,'{\"scope\":\"all\",\"cursor\":null}') ON CONFLICT DO NOTHING",
                &[&mailbox_id, &address],
            )
            .await
            .map_err(|_| SupportStoreError::Unavailable)?;
        let row = client.query_opt(
            "UPDATE support.mailbox SET sync_fence=sync_fence+1, \
             sync_lease_until=to_timestamp($3::bigint::double precision)+interval '90 seconds', \
             sync_health='syncing' WHERE mailbox_id=$1 AND address=$2 \
             AND (sync_lease_until IS NULL OR sync_lease_until<=to_timestamp($3::bigint::double precision)) \
             RETURNING sync_fence,bootstrap_boundary",
            &[&mailbox_id, &address, &now_unix_s],
        ).await.map_err(|_| SupportStoreError::Unavailable)?;
        let Some(row) = row else { return Ok(None) };
        let boundary: Option<String> = row.get(1);
        let cursor = match boundary {
            Some(boundary) => serde_json::from_str::<Value>(&boundary)
                .map_err(|_| SupportStoreError::InvalidRequest)?
                .get("cursor")
                .and_then(Value::as_str)
                .map(str::to_owned),
            None => None,
        };
        Ok(Some(MailboxLease {
            fence: row.get(0),
            cursor,
        }))
    }

    /// Preserves transport provenance and incomplete intake without asking the model to infer it.
    pub async fn record_message_metadata(
        &self,
        mailbox_id: &str,
        lease: &MailboxLease,
        mail: &df_support::ingest::InboundMail,
    ) -> Result<(), SupportStoreError> {
        let direction = match mail.direction {
            df_support::contracts::MessageDirection::Inbound => "inbound",
            df_support::contracts::MessageDirection::Outbound => "outbound",
        };
        let labels =
            serde_json::to_string(&mail.labels).map_err(|_| SupportStoreError::InvalidRequest)?;
        let mut client = self.client.lock().await;
        let transaction = client
            .transaction()
            .await
            .map_err(|_| SupportStoreError::Unavailable)?;
        check_fence(&transaction, mailbox_id, lease).await?;
        let changed = transaction.execute(
            "UPDATE support.message m SET direction=$4,gmail_labels=$5::text::jsonb,truncated=$6 \
             FROM support.mailbox b WHERE m.mailbox_id=b.mailbox_id AND b.mailbox_id=$1 \
             AND b.sync_fence=$2 AND m.gmail_message_id=$3",
            &[&mailbox_id,&lease.fence,&mail.provider_message_id,&direction,&labels,&mail.intake_incomplete],
        ).await.map_err(|_| SupportStoreError::Unavailable)?;
        if changed != 1 {
            return Err(SupportStoreError::Conflict);
        }
        transaction
            .commit()
            .await
            .map_err(|_| SupportStoreError::Unavailable)
    }

    /// Platform call evidence uses the shared support meter contract, never a customer debit.
    /// The pending Unknown record survives cancellation; settlement retains supplier usage.
    pub async fn record_model_call(
        &self,
        mailbox_id: &str,
        lease: &MailboxLease,
        call: df_support::contracts::MeteredCall,
        now_unix_s: i64,
    ) -> Result<(), SupportStoreError> {
        use df_support::contracts::MeterOutcome;
        let phase = match call.outcome {
            MeterOutcome::Completed => "completed",
            MeterOutcome::Unusable => "unusable",
            MeterOutcome::Failed => "failed",
            MeterOutcome::Unknown => "unknown",
        };
        let audit_id = format!("{}-{phase}", call.call_id.as_str());
        let data = serde_json::to_string(&call).map_err(|_| SupportStoreError::InvalidRequest)?;
        let mut client = self.client.lock().await;
        let transaction = client
            .transaction()
            .await
            .map_err(|_| SupportStoreError::Unavailable)?;
        if phase == "unknown" {
            check_fence(&transaction, mailbox_id, lease).await?;
            transaction.query_opt(
                "SELECT 1 FROM support.ticket t JOIN support.classification c ON c.ticket_id=t.ticket_id \
                 WHERE t.ticket_id=$1 AND c.classification_id=$2 AND t.revision=c.source_revision \
                 AND c.data->>'attempt_state'='running' FOR UPDATE OF t",
                &[&call.ticket_id.as_str(), &call.job_id.as_str()],
            ).await.map_err(|_| SupportStoreError::Unavailable)?.ok_or(SupportStoreError::Conflict)?;
        }
        transaction
            .execute(
                "INSERT INTO support.audit(audit_id,operation_id,ticket_id,data,created_at_unix_s) \
             VALUES($1,$2,$3,$4::text::jsonb,$5) ON CONFLICT DO NOTHING",
                &[
                    &audit_id,
                    &call.call_id.as_str(),
                    &call.ticket_id.as_str(),
                    &data,
                    &now_unix_s,
                ],
            )
            .await
            .map_err(|_| SupportStoreError::AuditUnavailable)?;
        transaction
            .commit()
            .await
            .map_err(|_| SupportStoreError::Unavailable)
    }

    /// Releases the slice only after admitted work is durable; failure retains its old cursor.
    pub async fn finish_mailbox(
        &self,
        mailbox_id: &str,
        lease: &MailboxLease,
        cursor: Option<&str>,
        succeeded: bool,
        retry_at_unix_s: Option<i64>,
    ) -> Result<(), SupportStoreError> {
        let boundary = json!({"scope":"all","cursor":cursor}).to_string();
        let changed = self
            .client
            .lock()
            .await
            .execute(
                "UPDATE support.mailbox SET sync_lease_until=CASE WHEN $4 THEN NULL ELSE to_timestamp($5::bigint::double precision) END, \
             bootstrap_boundary=CASE WHEN $4 THEN $3 ELSE bootstrap_boundary END, \
             sync_health=CASE WHEN $4 THEN 'healthy' ELSE 'retry_pending' END \
             WHERE mailbox_id=$1 AND sync_fence=$2",
                &[&mailbox_id, &lease.fence, &boundary, &succeeded, &retry_at_unix_s],
            )
            .await
            .map_err(|_| SupportStoreError::Unavailable)?;
        if changed != 1 {
            return Err(SupportStoreError::Conflict);
        }
        Ok(())
    }

    /// Claims the provider-message judgement once, including after a crash between intake and claim.
    /// The initial manual marker is durable before any provider dispatch.
    pub async fn claim_classification(
        &self,
        mailbox_id: &str,
        lease: &MailboxLease,
        ticket_id: &TicketId,
        classification_id: &str,
        now_unix_s: i64,
        subject: &str,
    ) -> Result<Option<u64>, SupportStoreError> {
        let mut client = self.client.lock().await;
        let transaction = client
            .transaction()
            .await
            .map_err(|_| SupportStoreError::Unavailable)?;
        let row = transaction
            .query_opt(
                "SELECT t.revision FROM support.ticket t JOIN support.mailbox b USING(mailbox_id) \
             WHERE t.ticket_id=$1 AND b.mailbox_id=$2 AND b.sync_fence=$3 FOR UPDATE OF t,b",
                &[&ticket_id.as_str(), &mailbox_id, &lease.fence],
            )
            .await
            .map_err(|_| SupportStoreError::Unavailable)?
            .ok_or(SupportStoreError::Conflict)?;
        let revision: i64 = row.get(0);
        let marker = json!({"status":"unclassified","reason":"interrupted_or_pending",
            "attempt_state":"running","source_subject":subject,"schema":"dungeonflux.support-classification/1"}).to_string();
        let inserted = transaction.execute(
            "INSERT INTO support.classification(classification_id,ticket_id,source_revision,data,created_at_unix_s) \
             VALUES($1,$2,$3,$4::text::jsonb,$5) ON CONFLICT DO NOTHING",
            &[&classification_id, &ticket_id.as_str(), &revision, &marker, &now_unix_s],
        ).await.map_err(|_| SupportStoreError::Unavailable)?;
        if inserted == 0 {
            return Ok(None);
        }
        transaction.execute(
            "UPDATE support.ticket SET classification_head=$2,sensitivity='restricted' WHERE ticket_id=$1",
            &[&ticket_id.as_str(), &classification_id],
        ).await.map_err(|_| SupportStoreError::Unavailable)?;
        append_ingest_audit(
            &transaction,
            ticket_id,
            classification_id,
            "model_pending",
            now_unix_s,
        )
        .await?;
        transaction
            .commit()
            .await
            .map_err(|_| SupportStoreError::Unavailable)?;
        Ok(Some(
            u64::try_from(revision).map_err(|_| SupportStoreError::Unavailable)?,
        ))
    }

    /// Commits a model result and its audit under the current fence and source revision.
    pub async fn complete_classification(
        &self,
        mailbox_id: &str,
        lease: &MailboxLease,
        classification_id: &str,
        data: &Value,
        now_unix_s: i64,
    ) -> Result<(), SupportStoreError> {
        let mut client = self.client.lock().await;
        let transaction = client
            .transaction()
            .await
            .map_err(|_| SupportStoreError::Unavailable)?;
        check_fence(&transaction, mailbox_id, lease).await?;
        let encoded = data.to_string();
        let row = transaction.query_opt(
            "UPDATE support.classification c SET data=c.data || $4::text::jsonb FROM support.ticket t,support.mailbox b \
             WHERE c.classification_id=$1 AND t.ticket_id=c.ticket_id AND b.mailbox_id=t.mailbox_id \
             AND b.mailbox_id=$2 AND b.sync_fence=$3 AND t.revision=c.source_revision \
             AND c.data->>'attempt_state'='running' RETURNING c.ticket_id",
            &[&classification_id, &mailbox_id, &lease.fence, &encoded],
        ).await.map_err(|_| SupportStoreError::Unavailable)?
            .ok_or(SupportStoreError::Conflict)?;
        let ticket_id =
            TicketId::new(row.get::<_, String>(0)).map_err(|_| SupportStoreError::Unavailable)?;
        append_ingest_audit(
            &transaction,
            &ticket_id,
            classification_id,
            "model_completed",
            now_unix_s,
        )
        .await?;
        transaction
            .commit()
            .await
            .map_err(|_| SupportStoreError::Unavailable)
    }
}

async fn append_ingest_audit(
    transaction: &tokio_postgres::Transaction<'_>,
    ticket_id: &TicketId,
    operation_id: &str,
    phase: &str,
    now_unix_s: i64,
) -> Result<(), SupportStoreError> {
    let audit_id = format!("{operation_id}-{phase}");
    let data = json!({"kind":"support_ingest","phase":phase}).to_string();
    transaction
        .execute(
            "INSERT INTO support.audit(audit_id,operation_id,ticket_id,data,created_at_unix_s) \
         VALUES($1,$2,$3,$4::text::jsonb,$5) ON CONFLICT DO NOTHING",
            &[
                &audit_id,
                &operation_id,
                &ticket_id.as_str(),
                &data,
                &now_unix_s,
            ],
        )
        .await
        .map_err(|_| SupportStoreError::AuditUnavailable)?;
    Ok(())
}

async fn check_fence(
    transaction: &tokio_postgres::Transaction<'_>,
    mailbox_id: &str,
    lease: &MailboxLease,
) -> Result<(), SupportStoreError> {
    transaction
        .query_opt(
            "SELECT 1 FROM support.mailbox WHERE mailbox_id=$1 AND sync_fence=$2 FOR UPDATE",
            &[&mailbox_id, &lease.fence],
        )
        .await
        .map_err(|_| SupportStoreError::Unavailable)?
        .ok_or(SupportStoreError::Conflict)?;
    Ok(())
}
