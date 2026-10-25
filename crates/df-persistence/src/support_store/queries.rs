use df_support::{
    contracts::{
        ActionProposalRecord, AuditEvent, MessagePage, MessagePageRequest, ProposalId,
        ProposalStatus, SupportTicket, TicketId,
    },
    ports::SupportStoreError,
};
use tokio_postgres::Client;

use super::rows::{self, ApprovalInsert, InboundMessage, IngestOutcome, RetentionPurge};

pub(super) async fn ingest(
    client: &mut Client,
    input: InboundMessage,
) -> Result<IngestOutcome, SupportStoreError> {
    let transaction = client
        .transaction()
        .await
        .map_err(|_| SupportStoreError::Unavailable)?;
    let recipients = serde_json::to_string(
        &input
            .recipients
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
    )
    .map_err(|_| SupportStoreError::Unavailable)?;
    let retired = transaction
        .query_opt(
            "SELECT 1 FROM support.retired_source WHERE mailbox_id=$1 AND gmail_message_id=$2",
            &[&input.mailbox_id, &input.gmail_message_id],
        )
        .await
        .map_err(|_| SupportStoreError::Unavailable)?;
    if retired.is_some() {
        transaction
            .rollback()
            .await
            .map_err(|_| SupportStoreError::Unavailable)?;
        return Ok(IngestOutcome::Retired);
    }
    let exists = transaction
        .query_opt(
            "SELECT t.ticket_id FROM support.message m JOIN support.ticket t ON t.ticket_id=m.ticket_id \
             WHERE m.mailbox_id=$1 AND m.gmail_message_id=$2",
            &[&input.mailbox_id, &input.gmail_message_id],
        )
        .await
        .map_err(|_| SupportStoreError::Unavailable)?;
    if exists.is_some() {
        let row = exists.as_ref().ok_or(SupportStoreError::Unavailable)?;
        let ticket_id = TicketId::new(
            row.try_get::<_, String>("ticket_id")
                .map_err(|_| SupportStoreError::Unavailable)?,
        )
        .map_err(|_| SupportStoreError::Unavailable)?;
        transaction
            .rollback()
            .await
            .map_err(|_| SupportStoreError::Unavailable)?;
        return Ok(IngestOutcome::Duplicate { ticket_id });
    }

    transaction.execute(
        "INSERT INTO support.mailbox (mailbox_id,address) VALUES ($1,$2) ON CONFLICT (mailbox_id) DO NOTHING",
        &[&input.mailbox_id, &input.mailbox_address],
    ).await.map_err(|_| SupportStoreError::Unavailable)?;
    transaction.execute(
        "INSERT INTO support.ticket (ticket_id,mailbox_id,gmail_thread_id,status,revision,created_at_unix_s,first_received_at_unix_s,last_inbound_at_unix_s) \
         VALUES ($1,$2,$3,'new',0,$4,$4,$4) ON CONFLICT (mailbox_id,gmail_thread_id) DO NOTHING",
        &[&input.ticket_id.as_str(), &input.mailbox_id, &input.gmail_thread_id, &input.received_at_unix_s],
    ).await.map_err(|_| SupportStoreError::Unavailable)?;
    let ticket_row = transaction
        .query_one(
            "SELECT ticket_id FROM support.ticket WHERE mailbox_id=$1 AND gmail_thread_id=$2",
            &[&input.mailbox_id, &input.gmail_thread_id],
        )
        .await
        .map_err(|_| SupportStoreError::Unavailable)?;
    let actual_ticket: String = ticket_row.get(0);
    transaction.execute(
        "INSERT INTO support.party (party_id,ticket_id,address_ref,display_name) VALUES ($1,$2,$3,$4) ON CONFLICT (ticket_id,party_id) DO NOTHING",
        &[&input.sender.as_str(), &actual_ticket, &input.sender_address_ref, &input.sender_display_name],
    ).await.map_err(|_| SupportStoreError::Unavailable)?;
    for (party_id, address_ref) in &input.recipients {
        transaction.execute(
            "INSERT INTO support.party (party_id,ticket_id,address_ref) VALUES ($1,$2,$3) ON CONFLICT (ticket_id,party_id) DO NOTHING",
            &[&party_id.as_str(), &actual_ticket, address_ref],
        ).await.map_err(|_| SupportStoreError::Unavailable)?;
    }
    let inserted = transaction.execute(
        "INSERT INTO support.message (message_id,ticket_id,mailbox_id,gmail_message_id,direction,received_at_unix_s,sender_party_id,recipient_party_ids,body_text) \
         VALUES ($1,$2,$3,$4,'inbound',$5,$6,$7::text::jsonb,$8) ON CONFLICT (mailbox_id,gmail_message_id) DO NOTHING",
        &[&input.message_id.as_str(), &actual_ticket, &input.mailbox_id, &input.gmail_message_id, &input.received_at_unix_s, &input.sender.as_str(), &recipients, &input.body_text],
    ).await.map_err(|_| SupportStoreError::Unavailable)?;
    if inserted == 0 {
        let ticket_id =
            TicketId::new(actual_ticket.clone()).map_err(|_| SupportStoreError::Unavailable)?;
        transaction
            .rollback()
            .await
            .map_err(|_| SupportStoreError::Unavailable)?;
        return Ok(IngestOutcome::Duplicate { ticket_id });
    }
    transaction
        .execute(
            "UPDATE support.ticket SET last_inbound_at_unix_s=GREATEST(last_inbound_at_unix_s,$2), \
         status=CASE WHEN status='resolved' THEN 'open' ELSE status END, revision=revision+1 \
         WHERE ticket_id=$1 AND revision >= 0",
            &[&actual_ticket, &input.received_at_unix_s],
        )
        .await
        .map_err(|_| SupportStoreError::Unavailable)?;
    transaction
        .commit()
        .await
        .map_err(|_| SupportStoreError::Unavailable)?;
    Ok(IngestOutcome::Inserted {
        ticket_id: TicketId::new(actual_ticket).map_err(|_| SupportStoreError::Unavailable)?,
    })
}

pub(super) async fn get_ticket(
    client: &Client,
    id: &TicketId,
) -> Result<SupportTicket, SupportStoreError> {
    let row = client.query_opt(
        "SELECT ticket_id,mailbox_id,status,revision,created_at_unix_s,first_received_at_unix_s,last_inbound_at_unix_s FROM support.ticket WHERE ticket_id=$1",
        &[&id.as_str()],
    ).await.map_err(|_| SupportStoreError::Unavailable)?.ok_or(SupportStoreError::NotFound)?;
    rows::ticket_from_row(&row)
}

pub(super) async fn save_ticket(
    client: &Client,
    ticket: SupportTicket,
    expected_revision: u64,
) -> Result<(), SupportStoreError> {
    let next = rows::ticket_status(ticket.status);
    let expected = i64::try_from(expected_revision).map_err(|_| SupportStoreError::Conflict)?;
    let changed = client.execute(
        "UPDATE support.ticket SET status=$2,revision=revision+1 WHERE ticket_id=$1 AND revision=$3 \
         AND CASE status WHEN 'new' THEN $2 IN ('open','spam') \
         WHEN 'open' THEN $2 IN ('waiting_for_customer','waiting_for_internal','resolved','spam') \
         WHEN 'waiting_for_customer' THEN $2 IN ('open','waiting_for_internal','resolved','spam') \
         WHEN 'waiting_for_internal' THEN $2 IN ('open','waiting_for_customer','resolved','spam') \
         WHEN 'resolved' THEN $2 IN ('open','spam') WHEN 'spam' THEN $2 IN ('new','open','resolved') ELSE false END",
        &[&ticket.id.as_str(), &next, &expected],
    ).await.map_err(|_| SupportStoreError::Unavailable)?;
    if changed == 1 {
        Ok(())
    } else {
        Err(SupportStoreError::Conflict)
    }
}

pub(super) async fn list_messages(
    client: &Client,
    ticket: &TicketId,
    request: MessagePageRequest,
) -> Result<MessagePage, SupportStoreError> {
    let cursor = request.cursor.map(|value| value.as_str().to_owned());
    let (cursor_at, cursor_id) = match cursor.as_deref() {
        Some(value) => {
            let (timestamp, message_id) = value
                .split_once('|')
                .ok_or(SupportStoreError::Unavailable)?;
            (
                Some(
                    timestamp
                        .parse::<i64>()
                        .map_err(|_| SupportStoreError::Unavailable)?,
                ),
                Some(message_id.to_owned()),
            )
        }
        None => (None, None),
    };
    let rows = client.query(
        "SELECT message_id,direction,received_at_unix_s,sender_party_id,recipient_party_ids::text AS recipient_party_ids,body_text \
         FROM support.message WHERE ticket_id=$1 AND ($2::bigint IS NULL OR (received_at_unix_s,message_id) > ($2,$3)) \
         ORDER BY received_at_unix_s,message_id LIMIT $4",
        &[&ticket.as_str(), &cursor_at, &cursor_id, &i64::from(request.page_size)],
    ).await.map_err(|_| SupportStoreError::Unavailable)?;
    let mut messages = rows
        .iter()
        .map(rows::message_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    let next_cursor = if messages.len() == usize::from(request.page_size) {
        messages
            .last()
            .map(|message| {
                let value = format!("{}|{}", message.received_at_unix_s, message.id.as_str());
                df_support::contracts::PageCursor::new(value)
                    .map_err(|_| SupportStoreError::Unavailable)
            })
            .transpose()?
    } else {
        None
    };
    if messages.is_empty() && get_ticket(client, ticket).await.is_err() {
        return Err(SupportStoreError::NotFound);
    }
    Ok(MessagePage {
        messages: std::mem::take(&mut messages),
        next_cursor,
    })
}

pub(super) async fn save_proposal(
    client: &Client,
    proposal: ActionProposalRecord,
) -> Result<(), SupportStoreError> {
    let data = serde_json::to_string(&proposal).map_err(|_| SupportStoreError::Unavailable)?;
    let source_revision =
        i64::try_from(proposal.source_revision).map_err(|_| SupportStoreError::Unavailable)?;
    let status = rows::proposal_status(proposal.status);
    client.execute(
        "INSERT INTO support.proposal (proposal_id,ticket_id,operation_id,source_revision,status,data) \
         VALUES ($1,$2,$3,$4,$5,$6::text::jsonb) ON CONFLICT (proposal_id) DO NOTHING",
        &[&proposal.id.as_str(), &proposal.ticket_id.as_str(), &proposal.operation_id.as_str(), &source_revision, &status, &data],
    ).await.map_err(|_| SupportStoreError::Unavailable)?;
    Ok(())
}

pub(super) async fn transition_proposal(
    client: &Client,
    proposal_id: &ProposalId,
    expected: ProposalStatus,
    next: ProposalStatus,
) -> Result<(), SupportStoreError> {
    expected
        .transition(next)
        .map_err(|_| SupportStoreError::Conflict)?;
    let updated = client
        .execute(
            "UPDATE support.proposal SET status=$3,data=jsonb_set(data,'{status}',to_jsonb($3::text),true) WHERE proposal_id=$1 AND status=$2",
            &[
                &proposal_id.as_str(),
                &rows::proposal_status(expected),
                &rows::proposal_status(next),
            ],
        )
        .await
        .map_err(|_| SupportStoreError::Unavailable)?;
    if updated == 1 {
        Ok(())
    } else {
        Err(SupportStoreError::Conflict)
    }
}

pub(super) async fn record_audit(
    client: &Client,
    event: AuditEvent,
) -> Result<(), SupportStoreError> {
    let data = serde_json::to_string(&event).map_err(|_| SupportStoreError::Unavailable)?;
    let (audit_id, operation_id, ticket_id) = match &event {
        AuditEvent::ContentRead {
            ticket_id,
            audit_id,
            ..
        } => (
            audit_id.as_str().to_owned(),
            audit_id.as_str().to_owned(),
            ticket_id.as_str(),
        ),
        AuditEvent::ClassificationChanged {
            ticket_id,
            revision,
            ..
        } => (
            format!("classification:{revision}"),
            format!("classification:{revision}"),
            ticket_id.as_str(),
        ),
        AuditEvent::ProposalDecided {
            ticket_id,
            proposal_id,
            ..
        } => (
            format!("proposal:{}", proposal_id.as_str()),
            format!("proposal:{}", proposal_id.as_str()),
            ticket_id.as_str(),
        ),
        AuditEvent::ExternalDispatch {
            ticket_id,
            operation_id,
            ..
        } => (
            operation_id.as_str().to_owned(),
            operation_id.as_str().to_owned(),
            ticket_id.as_str(),
        ),
    };
    client
        .execute(
            "INSERT INTO support.audit (audit_id,operation_id,ticket_id,data,created_at_unix_s) \
         VALUES ($1,$2,$3,$4::text::jsonb,floor(extract(epoch FROM clock_timestamp()))::bigint) \
         ON CONFLICT DO NOTHING",
            &[&audit_id, &operation_id, &ticket_id, &data],
        )
        .await
        .map_err(|_| SupportStoreError::AuditUnavailable)?;
    Ok(())
}

pub(super) async fn save_classification(
    client: &Client,
    id: &str,
    ticket: &TicketId,
    revision: u64,
    data: &serde_json::Value,
    at: i64,
) -> Result<(), SupportStoreError> {
    let source_revision = i64::try_from(revision).map_err(|_| SupportStoreError::Unavailable)?;
    let json = serde_json::to_string(data).map_err(|_| SupportStoreError::Unavailable)?;
    client.execute("INSERT INTO support.classification (classification_id,ticket_id,source_revision,data,created_at_unix_s) VALUES ($1,$2,$3,$4::text::jsonb,$5) ON CONFLICT (classification_id) DO NOTHING", &[&id,&ticket.as_str(),&source_revision,&json,&at]).await.map_err(|_| SupportStoreError::Unavailable)?;
    Ok(())
}

pub(super) async fn save_deadline(
    client: &Client,
    ticket: &TicketId,
    kind: &str,
    due: i64,
    based: i64,
) -> Result<(), SupportStoreError> {
    client.execute("INSERT INTO support.deadline (ticket_id,kind,due_at_unix_s,based_on_received_at_unix_s) VALUES ($1,$2,$3,$4) ON CONFLICT (ticket_id,kind) DO UPDATE SET due_at_unix_s=EXCLUDED.due_at_unix_s,based_on_received_at_unix_s=LEAST(support.deadline.based_on_received_at_unix_s,EXCLUDED.based_on_received_at_unix_s)", &[&ticket.as_str(),&kind,&due,&based]).await.map_err(|_| SupportStoreError::Unavailable)?;
    Ok(())
}

pub(super) async fn save_approval(
    client: &Client,
    approval: ApprovalInsert,
) -> Result<(), SupportStoreError> {
    client.execute("INSERT INTO support.approval (approval_id,proposal_id,operator_id,proof_id,approved_payload_digest,decided_at_unix_s,decision) VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT (approval_id) DO NOTHING", &[&approval.approval_id,&approval.proposal_id.as_str(),&approval.operator_id,&approval.proof_id,&approval.payload_digest,&approval.decided_at_unix_s,&approval.decision]).await.map_err(|_| SupportStoreError::Unavailable)?;
    Ok(())
}

pub(super) async fn purge_expired(
    client: &Client,
    now: i64,
    limit: u16,
) -> Result<RetentionPurge, SupportStoreError> {
    let bounded = i64::from(limit.min(500));
    if bounded == 0 {
        return Ok(RetentionPurge { deleted_tickets: 0 });
    }
    let now_seconds = now as f64;
    let count: i64 = client.query_one(
        "WITH doomed AS (SELECT ticket_id FROM support.ticket WHERE retained_until <= to_timestamp($1) ORDER BY retained_until,ticket_id FOR UPDATE SKIP LOCKED LIMIT $2), \
         tombstones AS (INSERT INTO support.retired_source (mailbox_id,gmail_message_id) \
         SELECT m.mailbox_id,m.gmail_message_id FROM support.message m JOIN support.ticket t ON t.ticket_id=m.ticket_id JOIN doomed d ON d.ticket_id=t.ticket_id \
         ON CONFLICT DO NOTHING RETURNING mailbox_id), deleted AS (DELETE FROM support.ticket t USING doomed d WHERE t.ticket_id=d.ticket_id RETURNING t.ticket_id) SELECT count(*) FROM deleted",
        &[&now_seconds,&bounded],
    ).await.map_err(|_| SupportStoreError::Unavailable)?.get(0);
    Ok(RetentionPurge {
        deleted_tickets: count as u64,
    })
}
