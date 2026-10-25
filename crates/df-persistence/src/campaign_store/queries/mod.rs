//! The row-level statements the rules in the sibling modules are built from. Each is one
//! parameterized statement. The locking ones are named `lock_*`; the caller takes them in the
//! order of launch decision 29 and never takes an accounts, finance or O1 row after the campaign
//! lock.

mod campaigns;
mod display;
mod drafts;
mod invitations;
mod lobby;
mod members;
mod receipts;
mod transfers;

pub(super) use campaigns::*;
pub(super) use display::*;
pub(super) use drafts::*;
pub(super) use invitations::*;
pub(super) use lobby::*;
pub(super) use members::*;
pub(super) use receipts::*;
pub(super) use transfers::*;

/// What every statement module needs, imported once.
mod prelude {
    pub(super) use df_campaign::{
        errors::{StoreError, StoreResult},
        ids::{InvitationId, PairingId, ProposalId, RequestId, TicketId},
        records::{
            AccountStatus, CampaignRow, CodeRow, EventKind, InvitationRow, MemberRow, NewEvent,
            PairingRow, ProposalRow, RequestRow, RevokeReason, SeatCounts, TicketRow,
            pairing_state_text, role_text,
        },
    };
    pub(super) use df_types::{AccountId, CampaignId, MemberId, SessionId};

    pub(super) use super::super::{
        conn::Tx,
        rows::{
            CAMPAIGN_COLUMNS, CODE_COLUMNS, INVITATION_COLUMNS, MEMBER_COLUMNS, PAIRING_COLUMNS,
            PROPOSAL_COLUMNS, REQUEST_COLUMNS, TICKET_COLUMNS, campaign_from_row, code_from_row,
            content_rating_text, invitation_from_row, join_role_text, member_from_row,
            pairing_from_row, proposal_from_row, request_from_row, stored_role, ticket_from_row,
            to_i32, to_i64, to_u64,
        },
    };
}
