//! Column lists and row decoding. A row that fails validation on read is `InvalidRecord`; it is
//! never repaired or defaulted.

use df_campaign::{
    errors::{StoreError, StoreResult},
    ids::{FeedEpoch, InvitationId, PairingId, ProposalId, RequestId, TicketId},
    join_invitation::JoinRole,
    records::{
        CampaignRow, CampaignStatus, CodeRow, DeviceLabel, EventKind, EventRow, EventScope,
        InvitationRow, MemberRow, MemberStatus, OldHostBecomes, PairingRow, ProposalRow,
        ProposalState, RequestRow, RequestState, RevokeReason, RoomCodes, TicketRow, TransferKind,
        parse_pairing_state, parse_role, role_text,
    },
    roles::{CampaignRole, Delegations},
};
use df_types::{AccountId, CampaignId, MemberId, SessionId, TenantId};
use tokio_postgres::{Row, types::FromSql};

pub(super) fn to_i64(value: u64) -> StoreResult<i64> {
    i64::try_from(value).map_err(|_| StoreError::InvalidRecord)
}

pub(super) fn to_u64(value: i64) -> StoreResult<u64> {
    u64::try_from(value).map_err(|_| StoreError::InvalidRecord)
}

pub(super) fn to_u32(value: i32) -> StoreResult<u32> {
    u32::try_from(value).map_err(|_| StoreError::InvalidRecord)
}

pub(super) fn to_i32(value: u32) -> StoreResult<i32> {
    i32::try_from(value).map_err(|_| StoreError::InvalidRecord)
}

fn col<'a, T: FromSql<'a>>(row: &'a Row, index: usize) -> StoreResult<T> {
    row.try_get(index).map_err(|_| StoreError::InvalidRecord)
}

fn digest(bytes: &[u8]) -> StoreResult<[u8; 32]> {
    bytes.try_into().map_err(|_| StoreError::InvalidRecord)
}

pub(super) fn campaign_id(bytes: &[u8]) -> StoreResult<CampaignId> {
    CampaignId::from_bytes(bytes).map_err(|_| StoreError::InvalidRecord)
}

pub(super) fn account_id(bytes: &[u8]) -> StoreResult<AccountId> {
    AccountId::from_bytes(bytes).map_err(|_| StoreError::InvalidRecord)
}

pub(super) fn member_id(bytes: &[u8]) -> StoreResult<MemberId> {
    MemberId::from_bytes(bytes).map_err(|_| StoreError::InvalidRecord)
}

pub(super) fn session_id(bytes: &[u8]) -> StoreResult<SessionId> {
    SessionId::from_bytes(bytes).map_err(|_| StoreError::InvalidRecord)
}

fn tenant_id(bytes: &[u8]) -> StoreResult<TenantId> {
    TenantId::from_bytes(bytes).map_err(|_| StoreError::InvalidRecord)
}

fn feed_epoch(bytes: &[u8]) -> StoreResult<FeedEpoch> {
    FeedEpoch::from_bytes(bytes).ok_or(StoreError::InvalidRecord)
}

fn optional_u64(value: Option<i64>) -> StoreResult<Option<u64>> {
    value.map(to_u64).transpose()
}

pub(super) fn join_role_text(role: JoinRole) -> &'static str {
    match role {
        JoinRole::Player => "player",
        JoinRole::Contributor => "contributor",
    }
}

pub(super) fn parse_join_role(text: &str) -> StoreResult<JoinRole> {
    match text {
        "player" => Ok(JoinRole::Player),
        "contributor" => Ok(JoinRole::Contributor),
        _ => Err(StoreError::InvalidRecord),
    }
}

pub(super) fn stored_role(role: CampaignRole) -> &'static str {
    role_text(role)
}

pub(super) const CAMPAIGN_COLUMNS: &str = "campaign_id, tenant_id, owner_account_id, name, \
    seat_count, content_rating, room_codes, default_co_host_delegations, display_summary_enabled, \
    planned_session_at_ms, status, revision, event_seq, feed_epoch, last_event_at_ms, \
    created_at_ms, archived_at_ms, scheduled_delete_at_ms";

pub(super) fn campaign_from_row(row: &Row) -> StoreResult<CampaignRow> {
    let rating: String = col(row, 5)?;
    let room_codes: String = col(row, 6)?;
    let delegations: i16 = col(row, 7)?;
    let status: String = col(row, 10)?;
    Ok(CampaignRow {
        id: campaign_id(col(row, 0)?)?,
        tenant: tenant_id(col(row, 1)?)?,
        owner: account_id(col(row, 2)?)?,
        name: col(row, 3)?,
        seat_count: to_u32(col(row, 4)?)?,
        content_rating: match rating.as_str() {
            "gentle" => df_campaign::records::ContentRating::Gentle,
            "standard" => df_campaign::records::ContentRating::Standard,
            "mature" => df_campaign::records::ContentRating::Mature,
            _ => return Err(StoreError::InvalidRecord),
        },
        room_codes: RoomCodes::parse(&room_codes).ok_or(StoreError::InvalidRecord)?,
        default_co_host_delegations: u8::try_from(delegations)
            .ok()
            .and_then(Delegations::from_bits)
            .ok_or(StoreError::InvalidRecord)?,
        display_summary_enabled: col(row, 8)?,
        planned_session_at_ms: optional_u64(col(row, 9)?)?,
        status: CampaignStatus::parse(&status).ok_or(StoreError::InvalidRecord)?,
        revision: to_u64(col(row, 11)?)?,
        event_seq: to_u64(col(row, 12)?)?,
        feed_epoch: feed_epoch(col(row, 13)?)?,
        last_event_at_ms: to_u64(col(row, 14)?)?,
        created_at_ms: to_u64(col(row, 15)?)?,
        archived_at_ms: optional_u64(col(row, 16)?)?,
        scheduled_delete_at_ms: optional_u64(col(row, 17)?)?,
    })
}

pub(super) fn content_rating_text(rating: df_campaign::records::ContentRating) -> &'static str {
    match rating {
        df_campaign::records::ContentRating::Gentle => "gentle",
        df_campaign::records::ContentRating::Standard => "standard",
        df_campaign::records::ContentRating::Mature => "mature",
    }
}

pub(super) const MEMBER_COLUMNS: &str = "member_id, campaign_id, account_id, role, holds_seat, \
    delegations, status, may_rejoin, removed_by_role, last_invitation_id, joined_at_ms, \
    ended_at_ms, revision";

pub(super) fn member_from_row(row: &Row) -> StoreResult<MemberRow> {
    let role: String = col(row, 3)?;
    let delegations: i16 = col(row, 5)?;
    let status: String = col(row, 6)?;
    let removed_by: Option<String> = col(row, 8)?;
    let invitation: Option<&[u8]> = col(row, 9)?;
    Ok(MemberRow {
        member: member_id(col(row, 0)?)?,
        campaign: campaign_id(col(row, 1)?)?,
        account: account_id(col(row, 2)?)?,
        role: parse_role(&role).ok_or(StoreError::InvalidRecord)?,
        holds_seat: col(row, 4)?,
        delegations: u8::try_from(delegations)
            .ok()
            .and_then(Delegations::from_bits)
            .ok_or(StoreError::InvalidRecord)?,
        status: MemberStatus::parse(&status).ok_or(StoreError::InvalidRecord)?,
        may_rejoin: col(row, 7)?,
        removed_by_role: removed_by
            .map(|text| parse_role(&text).ok_or(StoreError::InvalidRecord))
            .transpose()?,
        last_invitation: invitation
            .map(|bytes| InvitationId::from_bytes(bytes).ok_or(StoreError::InvalidRecord))
            .transpose()?,
        joined_at_ms: to_u64(col(row, 10)?)?,
        ended_at_ms: optional_u64(col(row, 11)?)?,
        revision: to_u64(col(row, 12)?)?,
    })
}

pub(super) const INVITATION_COLUMNS: &str = "invitation_id, campaign_id, token_digest, \
    created_by_member, joins_as, created_at_ms, expires_at_ms, max_uses, use_count, revoked_at_ms";

pub(super) fn invitation_from_row(row: &Row) -> StoreResult<InvitationRow> {
    let joins_as: String = col(row, 4)?;
    Ok(InvitationRow {
        id: InvitationId::from_bytes(col(row, 0)?).ok_or(StoreError::InvalidRecord)?,
        campaign: campaign_id(col(row, 1)?)?,
        token_digest: digest(col(row, 2)?)?,
        created_by: member_id(col(row, 3)?)?,
        joins_as: parse_join_role(&joins_as)?,
        created_at_ms: to_u64(col(row, 5)?)?,
        expires_at_ms: to_u64(col(row, 6)?)?,
        max_uses: to_u32(col(row, 7)?)?,
        use_count: to_u32(col(row, 8)?)?,
        revoked_at_ms: optional_u64(col(row, 9)?)?,
    })
}

pub(super) const CODE_COLUMNS: &str =
    "code_digest, campaign_id, created_at_ms, expires_at_ms, revoked_at_ms";

pub(super) fn code_from_row(row: &Row) -> StoreResult<CodeRow> {
    Ok(CodeRow {
        code_digest: digest(col(row, 0)?)?,
        campaign: campaign_id(col(row, 1)?)?,
        created_at_ms: to_u64(col(row, 2)?)?,
        expires_at_ms: to_u64(col(row, 3)?)?,
        revoked_at_ms: optional_u64(col(row, 4)?)?,
    })
}

pub(super) const REQUEST_COLUMNS: &str = "request_id, campaign_id, account_id, state, approved_as, \
    created_at_ms, decided_by_member, decided_at_ms";

pub(super) fn request_from_row(row: &Row) -> StoreResult<RequestRow> {
    let state: String = col(row, 3)?;
    let approved_as: Option<String> = col(row, 4)?;
    let decided_by: Option<&[u8]> = col(row, 6)?;
    Ok(RequestRow {
        id: RequestId::from_bytes(col(row, 0)?).ok_or(StoreError::InvalidRecord)?,
        campaign: campaign_id(col(row, 1)?)?,
        account: account_id(col(row, 2)?)?,
        state: RequestState::parse(&state).ok_or(StoreError::InvalidRecord)?,
        approved_as: approved_as.map(|text| parse_join_role(&text)).transpose()?,
        created_at_ms: to_u64(col(row, 5)?)?,
        decided_by: decided_by.map(member_id).transpose()?,
        decided_at_ms: optional_u64(col(row, 7)?)?,
    })
}

pub(super) const PROPOSAL_COLUMNS: &str = "proposal_id, campaign_id, kind, from_member, \
    from_account, to_member, to_account, old_host_becomes, campaign_revision, proposed_at_ms, \
    expires_at_ms, state";

pub(super) fn proposal_from_row(row: &Row) -> StoreResult<ProposalRow> {
    let kind: String = col(row, 2)?;
    let becomes: Option<String> = col(row, 7)?;
    let state: String = col(row, 11)?;
    Ok(ProposalRow {
        id: ProposalId::from_bytes(col(row, 0)?).ok_or(StoreError::InvalidRecord)?,
        campaign: campaign_id(col(row, 1)?)?,
        kind: TransferKind::parse(&kind).ok_or(StoreError::InvalidRecord)?,
        from_member: member_id(col(row, 3)?)?,
        from_account: account_id(col(row, 4)?)?,
        to_member: member_id(col(row, 5)?)?,
        to_account: account_id(col(row, 6)?)?,
        old_host_becomes: becomes
            .map(|text| match text.as_str() {
                "player" => Ok(OldHostBecomes::Player),
                "co_host" => Ok(OldHostBecomes::CoHost),
                _ => Err(StoreError::InvalidRecord),
            })
            .transpose()?,
        campaign_revision: to_u64(col(row, 8)?)?,
        proposed_at_ms: to_u64(col(row, 9)?)?,
        expires_at_ms: to_u64(col(row, 10)?)?,
        state: ProposalState::parse(&state).ok_or(StoreError::InvalidRecord)?,
    })
}

pub(super) const PAIRING_COLUMNS: &str = "pairing_id, campaign_id, user_code, device_secret_digest, \
    device_label, ip_key, created_at_ms, expires_at_ms, state, session_id, approved_by_member, \
    approved_at_ms, redeemed_at_ms, reissue_count, renewable_until_ms, ip_key_loose";

pub(super) fn pairing_from_row(row: &Row) -> StoreResult<PairingRow> {
    let campaign: Option<&[u8]> = col(row, 1)?;
    let label: String = col(row, 4)?;
    let state: String = col(row, 8)?;
    let session: Option<&[u8]> = col(row, 9)?;
    let approver: Option<&[u8]> = col(row, 10)?;
    let reissues: i16 = col(row, 13)?;
    Ok(PairingRow {
        id: PairingId::from_bytes(col(row, 0)?).ok_or(StoreError::InvalidRecord)?,
        campaign: campaign.map(campaign_id).transpose()?,
        user_code: col(row, 2)?,
        device_digest: digest(col(row, 3)?)?,
        device_label: DeviceLabel::parse(&label).ok_or(StoreError::InvalidRecord)?,
        ip_key: digest(col(row, 5)?)?,
        ip_key_loose: digest(col(row, 15)?)?,
        created_at_ms: to_u64(col(row, 6)?)?,
        expires_at_ms: to_u64(col(row, 7)?)?,
        state: parse_pairing_state(&state).ok_or(StoreError::InvalidRecord)?,
        session: session.map(session_id).transpose()?,
        approved_by: approver.map(member_id).transpose()?,
        approved_at_ms: optional_u64(col(row, 11)?)?,
        redeemed_at_ms: optional_u64(col(row, 12)?)?,
        reissue_count: u8::try_from(reissues).map_err(|_| StoreError::InvalidRecord)?,
        renewable_until_ms: optional_u64(col(row, 14)?)?,
    })
}

pub(super) const TICKET_COLUMNS: &str = "ticket_id, ticket_digest, pairing_id, campaign_id, \
    session_id, device_secret_digest, device_label, approved_by_member, issued_at_ms, \
    expires_at_ms, revoked_at_ms, revoke_reason";

pub(super) fn ticket_from_row(row: &Row) -> StoreResult<TicketRow> {
    let label: String = col(row, 6)?;
    let reason: Option<String> = col(row, 11)?;
    Ok(TicketRow {
        id: TicketId::from_bytes(col(row, 0)?).ok_or(StoreError::InvalidRecord)?,
        token_digest: digest(col(row, 1)?)?,
        pairing: PairingId::from_bytes(col(row, 2)?).ok_or(StoreError::InvalidRecord)?,
        campaign: campaign_id(col(row, 3)?)?,
        session: session_id(col(row, 4)?)?,
        device_digest: digest(col(row, 5)?)?,
        device_label: DeviceLabel::parse(&label).ok_or(StoreError::InvalidRecord)?,
        approved_by: member_id(col(row, 7)?)?,
        issued_at_ms: to_u64(col(row, 8)?)?,
        expires_at_ms: to_u64(col(row, 9)?)?,
        revoked_at_ms: optional_u64(col(row, 10)?)?,
        revoke_reason: reason
            .map(|text| RevokeReason::parse(&text).ok_or(StoreError::InvalidRecord))
            .transpose()?,
    })
}

pub(super) const EVENT_COLUMNS: &str = "campaign_id, event_seq, scope, feed_epoch, member_id, \
    member_revision, revoking, at_ms, kind, actor_member, ticket_id, writer_fence";

pub(super) fn event_from_row(row: &Row) -> StoreResult<EventRow> {
    let scope: String = col(row, 2)?;
    let member: Option<&[u8]> = col(row, 4)?;
    let revision: Option<i64> = col(row, 5)?;
    let kind: String = col(row, 8)?;
    let actor: Option<&[u8]> = col(row, 9)?;
    let ticket: Option<&[u8]> = col(row, 10)?;
    Ok(EventRow {
        campaign: campaign_id(col(row, 0)?)?,
        event_seq: to_u64(col(row, 1)?)?,
        scope: EventScope::parse(&scope).ok_or(StoreError::InvalidRecord)?,
        feed_epoch: feed_epoch(col(row, 3)?)?,
        member: member.map(member_id).transpose()?,
        member_revision: optional_u64(revision)?,
        revoking: col(row, 6)?,
        at_ms: to_u64(col(row, 7)?)?,
        kind: EventKind::parse(&kind).ok_or(StoreError::InvalidRecord)?,
        actor: actor.map(member_id).transpose()?,
        ticket: ticket
            .map(|bytes| TicketId::from_bytes(bytes).ok_or(StoreError::InvalidRecord))
            .transpose()?,
        writer_fence: col(row, 11)?,
    })
}

pub(super) fn head_from_row(row: &Row) -> StoreResult<df_campaign::records::CampaignHead> {
    Ok(df_campaign::records::CampaignHead {
        campaign: campaign_id(col(row, 0)?)?,
        event_seq: to_u64(col(row, 1)?)?,
        feed_epoch: feed_epoch(col(row, 2)?)?,
    })
}

pub(super) const DRAFT_COLUMNS: &str = "campaign_id, tenant_id, owner_account_id, name, \
    adventure_kind, content_rating, seat_count, room_codes, default_co_host_delegations, \
    voice_default, picture_default, spend_preset, custom_hourly_micro_dollars, \
    session_ceiling_micro_dollars, funding_model, sponsor_account_id, draft_step, revision, \
    created_at_ms, draft_expires_at_ms";

pub(super) fn draft_from_row(row: &Row) -> StoreResult<df_campaign::draft::DraftRow> {
    use df_campaign::{
        draft::{DraftRow, FundingModel, PictureDefault, SpendPreset, VoiceDefault},
        records::ContentRating,
    };
    let rating: String = col(row, 5)?;
    let room_codes: String = col(row, 7)?;
    let delegations: i16 = col(row, 8)?;
    let voice: String = col(row, 9)?;
    let picture: String = col(row, 10)?;
    let preset: String = col(row, 11)?;
    let funding: String = col(row, 14)?;
    let sponsor: Option<&[u8]> = col(row, 15)?;
    let step: Option<i16> = col(row, 16)?;
    let expires: Option<i64> = col(row, 19)?;
    Ok(DraftRow {
        id: campaign_id(col(row, 0)?)?,
        tenant: tenant_id(col(row, 1)?)?,
        owner: account_id(col(row, 2)?)?,
        name: col(row, 3)?,
        adventure_kind: col(row, 4)?,
        content_rating: match rating.as_str() {
            "gentle" => ContentRating::Gentle,
            "standard" => ContentRating::Standard,
            "mature" => ContentRating::Mature,
            _ => return Err(StoreError::InvalidRecord),
        },
        seat_count: to_u32(col(row, 6)?)?,
        room_codes: RoomCodes::parse(&room_codes).ok_or(StoreError::InvalidRecord)?,
        default_co_host_delegations: u8::try_from(delegations)
            .ok()
            .and_then(Delegations::from_bits)
            .ok_or(StoreError::InvalidRecord)?,
        voice_default: VoiceDefault::parse(&voice).ok_or(StoreError::InvalidRecord)?,
        picture_default: PictureDefault::parse(&picture).ok_or(StoreError::InvalidRecord)?,
        spend_preset: SpendPreset::parse(&preset).ok_or(StoreError::InvalidRecord)?,
        custom_hourly_micro_dollars: optional_u64(col(row, 12)?)?,
        session_ceiling_micro_dollars: optional_u64(col(row, 13)?)?,
        funding_model: FundingModel::parse(&funding).ok_or(StoreError::InvalidRecord)?,
        sponsor: sponsor.map(account_id).transpose()?,
        step: step
            .and_then(|value| u8::try_from(value).ok())
            .ok_or(StoreError::InvalidRecord)?,
        revision: to_u64(col(row, 17)?)?,
        created_at_ms: to_u64(col(row, 18)?)?,
        expires_at_ms: optional_u64(expires)?.ok_or(StoreError::InvalidRecord)?,
    })
}

pub(super) const LOBBY_COLUMNS: &str = "session_id, member_id, campaign_id, character_choice, \
    character_ref, ready, microphone_preference, microphone_report, updated_at_ms, revision, \
    frozen_at_ms";

pub(super) fn lobby_from_row(row: &Row) -> StoreResult<df_campaign::lobby_state::LobbyMemberRow> {
    use df_campaign::{
        lobby::CharacterChoice,
        lobby_state::{LobbyMemberRow, MicrophonePreference, MicrophoneReport},
    };
    let choice: String = col(row, 3)?;
    let reference: Option<&[u8]> = col(row, 4)?;
    let preference: String = col(row, 6)?;
    let report: String = col(row, 7)?;
    let frozen: Option<i64> = col(row, 10)?;
    Ok(LobbyMemberRow {
        session: session_id(col(row, 0)?)?,
        member: member_id(col(row, 1)?)?,
        campaign: campaign_id(col(row, 2)?)?,
        character: CharacterChoice::parse(&choice).ok_or(StoreError::InvalidRecord)?,
        character_ref: reference
            .map(|bytes| <[u8; 16]>::try_from(bytes).map_err(|_| StoreError::InvalidRecord))
            .transpose()?,
        ready: col(row, 5)?,
        microphone_preference: MicrophonePreference::parse(&preference)
            .ok_or(StoreError::InvalidRecord)?,
        microphone_report: MicrophoneReport::parse(&report).ok_or(StoreError::InvalidRecord)?,
        updated_at_ms: to_u64(col(row, 8)?)?,
        revision: to_u64(col(row, 9)?)?,
        frozen: frozen.is_some(),
    })
}
