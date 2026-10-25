use df_support::contracts::{
    MessageDirection, MessageId, PartyId, ProposalStatus, SupportMessage, SupportTicket, TicketId,
    TicketStatus, UntrustedEmailContent,
};

use super::SupportStoreError;

pub struct InboundMessage {
    pub ticket_id: TicketId,
    pub message_id: MessageId,
    pub mailbox_id: String,
    pub mailbox_address: String,
    pub gmail_message_id: String,
    pub gmail_thread_id: String,
    pub received_at_unix_s: i64,
    pub sender: PartyId,
    pub sender_address_ref: String,
    pub sender_display_name: Option<String>,
    pub recipients: Vec<(PartyId, String)>,
    pub body_text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IngestOutcome {
    Inserted { ticket_id: TicketId },
    Duplicate { ticket_id: TicketId },
    Retired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetentionPurge {
    pub deleted_tickets: u64,
}

pub struct ApprovalInsert {
    pub approval_id: String,
    pub proposal_id: df_support::contracts::ProposalId,
    pub operator_id: String,
    pub proof_id: String,
    pub payload_digest: String,
    pub decided_at_unix_s: i64,
    pub decision: String,
}

pub(super) fn ticket_from_row(
    row: &tokio_postgres::Row,
) -> Result<SupportTicket, SupportStoreError> {
    Ok(SupportTicket {
        id: TicketId::new(row.get::<_, String>("ticket_id"))
            .map_err(|_| SupportStoreError::Unavailable)?,
        mailbox_id: df_support::contracts::MailboxId::new(row.get::<_, String>("mailbox_id"))
            .map_err(|_| SupportStoreError::Unavailable)?,
        status: parse_ticket_status(row.get("status"))?,
        revision: row
            .get::<_, i64>("revision")
            .try_into()
            .map_err(|_| SupportStoreError::Unavailable)?,
        created_at_unix_s: row.get("created_at_unix_s"),
        first_received_at_unix_s: row.get("first_received_at_unix_s"),
        last_inbound_at_unix_s: row.get("last_inbound_at_unix_s"),
    })
}

pub(super) fn parse_ticket_status(value: String) -> Result<TicketStatus, SupportStoreError> {
    match value.as_str() {
        "new" => Ok(TicketStatus::New),
        "open" => Ok(TicketStatus::Open),
        "waiting_for_customer" => Ok(TicketStatus::WaitingForCustomer),
        "waiting_for_internal" => Ok(TicketStatus::WaitingForInternal),
        "resolved" => Ok(TicketStatus::Resolved),
        "spam" => Ok(TicketStatus::Spam),
        _ => Err(SupportStoreError::Unavailable),
    }
}

pub(super) fn ticket_status(status: TicketStatus) -> &'static str {
    match status {
        TicketStatus::New => "new",
        TicketStatus::Open => "open",
        TicketStatus::WaitingForCustomer => "waiting_for_customer",
        TicketStatus::WaitingForInternal => "waiting_for_internal",
        TicketStatus::Resolved => "resolved",
        TicketStatus::Spam => "spam",
    }
}

pub(super) fn proposal_status(status: ProposalStatus) -> &'static str {
    match status {
        ProposalStatus::Draft => "draft",
        ProposalStatus::Approved => "approved",
        ProposalStatus::Queued => "queued",
        ProposalStatus::Running => "running",
        ProposalStatus::Succeeded => "succeeded",
        ProposalStatus::Failed => "failed",
        ProposalStatus::Unknown => "unknown",
        ProposalStatus::Rejected => "rejected",
        ProposalStatus::Superseded => "superseded",
        ProposalStatus::Expired => "expired",
    }
}

pub(super) fn message_from_row(
    row: &tokio_postgres::Row,
) -> Result<SupportMessage, SupportStoreError> {
    let recipient_json: String = row.get("recipient_party_ids");
    let recipients: Vec<String> =
        serde_json::from_str(&recipient_json).map_err(|_| SupportStoreError::Unavailable)?;
    Ok(SupportMessage {
        id: MessageId::new(row.get::<_, String>("message_id"))
            .map_err(|_| SupportStoreError::Unavailable)?,
        direction: match row.get::<_, String>("direction").as_str() {
            "inbound" => MessageDirection::Inbound,
            "outbound" => MessageDirection::Outbound,
            _ => return Err(SupportStoreError::Unavailable),
        },
        received_at_unix_s: row.get("received_at_unix_s"),
        sender: PartyId::new(row.get::<_, String>("sender_party_id"))
            .map_err(|_| SupportStoreError::Unavailable)?,
        recipients: recipients
            .into_iter()
            .map(PartyId::new)
            .collect::<Result<_, _>>()
            .map_err(|_| SupportStoreError::Unavailable)?,
        content: UntrustedEmailContent::new(row.get("body_text")),
    })
}
