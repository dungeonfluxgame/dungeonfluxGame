//! The reads behind the dashboard and the campaign tabs. Each runs on the read pool as plain
//! statements; none locks anything. A view that is a little behind a concurrent join is harmless,
//! and the member snapshot (5.1) remains the one authoritative read for authority.

use df_campaign::{
    draft::{CampaignSettings, FundingModel, PictureDefault, SpendPreset, VoiceDefault},
    errors::{StoreError, StoreResult},
    ids::TicketId,
    lobby_state::{LobbyDisplay, LobbyMemberRow, LobbyRows},
    records::DeviceLabel,
    records::{CampaignStatus, ContentRating, DashboardRow, MemberRow, OverviewRows, parse_role},
};
use df_types::{AccountId, CampaignId, SessionId};

use super::{
    conn::Pool,
    rows::{
        CAMPAIGN_COLUMNS, LOBBY_COLUMNS, MEMBER_COLUMNS, account_id, campaign_from_row,
        campaign_id, lobby_from_row, member_from_row, to_i64, to_u32, to_u64,
    },
};

fn invalid<E>(_: E) -> StoreError {
    StoreError::InvalidRecord
}

fn rating(text: &str) -> StoreResult<ContentRating> {
    match text {
        "gentle" => Ok(ContentRating::Gentle),
        "standard" => Ok(ContentRating::Standard),
        "mature" => Ok(ContentRating::Mature),
        _ => Err(StoreError::InvalidRecord),
    }
}

pub(super) async fn dashboard_campaigns(
    pool: &Pool,
    account: AccountId,
    limit: u32,
) -> StoreResult<Vec<DashboardRow>> {
    let rows = pool
        .read(
            "SELECT c.campaign_id, c.name, c.status, c.content_rating, c.seat_count, \
             (SELECT count(*) FROM campaigns.campaign_member s \
               WHERE s.campaign_id = c.campaign_id AND s.status = 'active' AND s.holds_seat), \
             c.planned_session_at_ms, c.scheduled_delete_at_ms, c.last_event_at_ms, m.role, \
             c.owner_account_id = m.account_id \
             FROM campaigns.campaign_member m \
             JOIN campaigns.campaign c ON c.campaign_id = m.campaign_id \
             WHERE m.account_id = $1 AND m.status = 'active' \
               AND c.status IN ('active', 'archived') \
             ORDER BY c.last_event_at_ms DESC, c.campaign_id LIMIT $2",
            &[&account.as_bytes().as_slice(), &i64::from(limit)],
        )
        .await?;
    rows.iter()
        .map(|row| {
            let status: String = row.try_get(2).map_err(invalid)?;
            let rating_text: String = row.try_get(3).map_err(invalid)?;
            let seat_count: i32 = row.try_get(4).map_err(invalid)?;
            let seats: i64 = row.try_get(5).map_err(invalid)?;
            let planned: Option<i64> = row.try_get(6).map_err(invalid)?;
            let scheduled: Option<i64> = row.try_get(7).map_err(invalid)?;
            let last_event: i64 = row.try_get(8).map_err(invalid)?;
            let role: String = row.try_get(9).map_err(invalid)?;
            Ok(DashboardRow {
                id: campaign_id(row.try_get(0).map_err(invalid)?)?,
                name: row.try_get(1).map_err(invalid)?,
                status: CampaignStatus::parse(&status).ok_or(StoreError::InvalidRecord)?,
                content_rating: rating(&rating_text)?,
                seat_count: to_u32(seat_count)?,
                seats_in_use: u32::try_from(seats).map_err(invalid)?,
                planned_session_at_ms: planned.map(to_u64).transpose()?,
                scheduled_delete_at_ms: scheduled.map(to_u64).transpose()?,
                last_event_at_ms: to_u64(last_event)?,
                role: parse_role(&role).ok_or(StoreError::InvalidRecord)?,
                is_owner: row.try_get(10).map_err(invalid)?,
            })
        })
        .collect()
}

pub(super) async fn campaign_overview(
    pool: &Pool,
    account: AccountId,
    campaign: CampaignId,
) -> StoreResult<Option<OverviewRows>> {
    let sql = format!(
        "SELECT {CAMPAIGN_COLUMNS}, adventure_kind, voice_default, picture_default, spend_preset, \
         custom_hourly_micro_dollars, session_ceiling_micro_dollars, funding_model, \
         sponsor_account_id, \
         (SELECT count(*) FROM campaigns.join_request r \
           WHERE r.campaign_id = campaigns.campaign.campaign_id AND r.state = 'pending') \
         FROM campaigns.campaign WHERE campaign_id = $1 AND status IN ('active', 'archived') \
         AND EXISTS (SELECT 1 FROM campaigns.campaign_member a \
           WHERE a.campaign_id = campaigns.campaign.campaign_id AND a.account_id = $2 \
             AND a.status = 'active')"
    );
    let rows = pool
        .read(
            &sql,
            &[
                &campaign.as_bytes().as_slice(),
                &account.as_bytes().as_slice(),
            ],
        )
        .await?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };
    let base = campaign_from_row(row)?;
    let adventure: String = row.try_get(18).map_err(invalid)?;
    let voice: String = row.try_get(19).map_err(invalid)?;
    let picture: String = row.try_get(20).map_err(invalid)?;
    let preset: String = row.try_get(21).map_err(invalid)?;
    let hourly: Option<i64> = row.try_get(22).map_err(invalid)?;
    let ceiling: Option<i64> = row.try_get(23).map_err(invalid)?;
    let funding: String = row.try_get(24).map_err(invalid)?;
    let sponsor: Option<&[u8]> = row.try_get(25).map_err(invalid)?;
    let pending: i64 = row.try_get(26).map_err(invalid)?;
    let settings = CampaignSettings {
        adventure_kind: adventure,
        voice_default: VoiceDefault::parse(&voice).ok_or(StoreError::InvalidRecord)?,
        picture_default: PictureDefault::parse(&picture).ok_or(StoreError::InvalidRecord)?,
        spend_preset: SpendPreset::parse(&preset).ok_or(StoreError::InvalidRecord)?,
        custom_hourly_micro_dollars: hourly.map(to_u64).transpose()?,
        session_ceiling_micro_dollars: ceiling.map(to_u64).transpose()?,
        funding_model: FundingModel::parse(&funding).ok_or(StoreError::InvalidRecord)?,
        sponsor: sponsor.map(account_id).transpose()?,
    };
    let member_sql = format!(
        "SELECT {MEMBER_COLUMNS} FROM campaigns.campaign_member \
         WHERE campaign_id = $1 AND status = 'active' \
         ORDER BY (role = 'host') DESC, joined_at_ms, member_id"
    );
    let members: Vec<MemberRow> = pool
        .read(&member_sql, &[&campaign.as_bytes().as_slice()])
        .await?
        .iter()
        .map(member_from_row)
        .collect::<StoreResult<_>>()?;
    Ok(Some(OverviewRows {
        campaign: base,
        settings,
        members,
        pending_requests: u32::try_from(pending).map_err(invalid)?,
    }))
}

/// The lobby of one session: the campaign, the seated members with their lobby rows, and the live
/// displays. Plain statements on the read pool; `None` when the campaign is missing or is neither
/// active nor archived.
pub(super) async fn lobby_rows(
    pool: &Pool,
    session: SessionId,
    campaign: CampaignId,
    now_ms: u64,
) -> StoreResult<Option<LobbyRows>> {
    let sql = format!(
        "SELECT {CAMPAIGN_COLUMNS} FROM campaigns.campaign          WHERE campaign_id = $1 AND status IN ('active', 'archived')"
    );
    let rows = pool.read(&sql, &[&campaign.as_bytes().as_slice()]).await?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };
    let campaign_row = campaign_from_row(row)?;
    let member_sql = format!(
        "SELECT {MEMBER_COLUMNS} FROM campaigns.campaign_member          WHERE campaign_id = $1 AND status = 'active' AND holds_seat          ORDER BY (role = 'host') DESC, joined_at_ms, member_id"
    );
    let members: Vec<MemberRow> = pool
        .read(&member_sql, &[&campaign.as_bytes().as_slice()])
        .await?
        .iter()
        .map(member_from_row)
        .collect::<StoreResult<_>>()?;
    let lobby_sql = format!(
        "SELECT {LOBBY_COLUMNS} FROM campaigns.lobby_member          WHERE session_id = $1 AND campaign_id = $2"
    );
    let lobby: Vec<LobbyMemberRow> = pool
        .read(
            &lobby_sql,
            &[
                &session.as_bytes().as_slice(),
                &campaign.as_bytes().as_slice(),
            ],
        )
        .await?
        .iter()
        .map(lobby_from_row)
        .collect::<StoreResult<_>>()?;
    let displays = pool
        .read(
            "SELECT ticket_id, device_label, issued_at_ms, expires_at_ms              FROM campaigns.display_ticket WHERE session_id = $1 AND campaign_id = $2              AND revoked_at_ms IS NULL AND expires_at_ms > $3 ORDER BY issued_at_ms, ticket_id",
            &[
                &session.as_bytes().as_slice(),
                &campaign.as_bytes().as_slice(),
                &to_i64(now_ms)?,
            ],
        )
        .await?
        .iter()
        .map(|row| {
            let ticket: &[u8] = row.try_get(0).map_err(invalid)?;
            let label: String = row.try_get(1).map_err(invalid)?;
            let issued: i64 = row.try_get(2).map_err(invalid)?;
            let expires: i64 = row.try_get(3).map_err(invalid)?;
            Ok(LobbyDisplay {
                ticket: TicketId::from_bytes(ticket).ok_or(StoreError::InvalidRecord)?,
                label: DeviceLabel::parse(&label).ok_or(StoreError::InvalidRecord)?,
                issued_at_ms: to_u64(issued)?,
                expires_at_ms: to_u64(expires)?,
            })
        })
        .collect::<StoreResult<_>>()?;
    let seats = members
        .into_iter()
        .map(|member| {
            let lobby_row = lobby
                .iter()
                .find(|item| item.member == member.member)
                .cloned();
            (member, lobby_row)
        })
        .collect();
    Ok(Some(LobbyRows {
        campaign: campaign_row,
        seats,
        displays,
    }))
}
