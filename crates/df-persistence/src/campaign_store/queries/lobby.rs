//! `lobby_member` rows and the lobby's reads. A member with no row has the defaults; the first
//! change creates the row. Every change takes the campaign lock first (the caller does), which is
//! also what `session_started` takes to freeze the rows, so a change and the freeze are ordered.

use df_campaign::{
    lobby::CharacterChoice,
    lobby_state::{LobbyMemberRow, MicrophonePreference, MicrophoneReport},
};

use super::prelude::*;
use crate::campaign_store::rows::{LOBBY_COLUMNS, lobby_from_row};

/// What one change writes.
pub(in super::super) enum LobbyChange {
    Character(CharacterChoice, Option<[u8; 16]>),
    Ready(bool),
    Microphone(MicrophonePreference, MicrophoneReport),
}

pub(in super::super) async fn lobby_member_row(
    tx: &Tx<'_>,
    session: SessionId,
    member: MemberId,
) -> StoreResult<Option<LobbyMemberRow>> {
    let sql = format!(
        "SELECT {LOBBY_COLUMNS} FROM campaigns.lobby_member WHERE session_id = $1 AND member_id = $2"
    );
    tx.query_opt(
        &sql,
        &[
            &session.as_bytes().as_slice(),
            &member.as_bytes().as_slice(),
        ],
    )
    .await?
    .map(|row| lobby_from_row(&row))
    .transpose()
}

/// Whether the session's lobby has been frozen (any row carries a freeze stamp).
pub(in super::super) async fn lobby_frozen(tx: &Tx<'_>, session: SessionId) -> StoreResult<bool> {
    let row = tx
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM campaigns.lobby_member \
             WHERE session_id = $1 AND frozen_at_ms IS NOT NULL)",
            &[&session.as_bytes().as_slice()],
        )
        .await?;
    row.try_get(0).map_err(|_| StoreError::InvalidRecord)
}

/// Writes one change and returns the row. A row that exists and is frozen is left alone (`None`).
pub(in super::super) async fn change_lobby_member(
    tx: &Tx<'_>,
    session: SessionId,
    member: MemberId,
    campaign: CampaignId,
    change: &LobbyChange,
    now_ms: u64,
) -> StoreResult<Option<LobbyMemberRow>> {
    let now = to_i64(now_ms)?;
    let key = (
        session.as_bytes().to_vec(),
        member.as_bytes().to_vec(),
        campaign.as_bytes().to_vec(),
    );
    let rows = match change {
        LobbyChange::Character(choice, reference) => {
            let sql = format!(
                "INSERT INTO campaigns.lobby_member (session_id, member_id, campaign_id, \
                 character_choice, character_ref, ready, updated_at_ms) \
                 VALUES ($1, $2, $3, $4, $5, false, $6) \
                 ON CONFLICT (session_id, member_id) DO UPDATE SET \
                 character_choice = EXCLUDED.character_choice, \
                 character_ref = EXCLUDED.character_ref, ready = false, \
                 updated_at_ms = EXCLUDED.updated_at_ms, \
                 revision = campaigns.lobby_member.revision + 1 \
                 WHERE campaigns.lobby_member.frozen_at_ms IS NULL RETURNING {LOBBY_COLUMNS}"
            );
            tx.query(
                &sql,
                &[
                    &key.0,
                    &key.1,
                    &key.2,
                    &choice.text(),
                    &reference.map(|bytes| bytes.to_vec()),
                    &now,
                ],
            )
            .await?
        }
        LobbyChange::Ready(ready) => {
            let sql = format!(
                "INSERT INTO campaigns.lobby_member (session_id, member_id, campaign_id, ready, \
                 updated_at_ms) VALUES ($1, $2, $3, $4, $5) \
                 ON CONFLICT (session_id, member_id) DO UPDATE SET ready = EXCLUDED.ready, \
                 updated_at_ms = EXCLUDED.updated_at_ms, \
                 revision = campaigns.lobby_member.revision + 1 \
                 WHERE campaigns.lobby_member.frozen_at_ms IS NULL RETURNING {LOBBY_COLUMNS}"
            );
            tx.query(&sql, &[&key.0, &key.1, &key.2, ready, &now])
                .await?
        }
        LobbyChange::Microphone(preference, report) => {
            let sql = format!(
                "INSERT INTO campaigns.lobby_member (session_id, member_id, campaign_id, \
                 microphone_preference, microphone_report, updated_at_ms) \
                 VALUES ($1, $2, $3, $4, $5, $6) \
                 ON CONFLICT (session_id, member_id) DO UPDATE SET \
                 microphone_preference = EXCLUDED.microphone_preference, \
                 microphone_report = EXCLUDED.microphone_report, \
                 updated_at_ms = EXCLUDED.updated_at_ms, \
                 revision = campaigns.lobby_member.revision + 1 \
                 WHERE campaigns.lobby_member.frozen_at_ms IS NULL RETURNING {LOBBY_COLUMNS}"
            );
            tx.query(
                &sql,
                &[
                    &key.0,
                    &key.1,
                    &key.2,
                    &preference.text(),
                    &report.text(),
                    &now,
                ],
            )
            .await?
        }
    };
    rows.first().map(lobby_from_row).transpose()
}

/// Freezes the session's lobby: every seated member gets a row (created if missing) stamped
/// frozen. The campaign is locked by the caller.
pub(in super::super) async fn freeze_lobby(
    tx: &Tx<'_>,
    session: SessionId,
    campaign: CampaignId,
    now_ms: u64,
) -> StoreResult<()> {
    tx.execute(
        "INSERT INTO campaigns.lobby_member (session_id, member_id, campaign_id, updated_at_ms, \
         frozen_at_ms) \
         SELECT $1, m.member_id, m.campaign_id, $3, $3 FROM campaigns.campaign_member m \
         WHERE m.campaign_id = $2 AND m.status = 'active' AND m.holds_seat \
         ON CONFLICT (session_id, member_id) DO UPDATE SET \
         frozen_at_ms = COALESCE(campaigns.lobby_member.frozen_at_ms, EXCLUDED.frozen_at_ms)",
        &[
            &session.as_bytes().as_slice(),
            &campaign.as_bytes().as_slice(),
            &to_i64(now_ms)?,
        ],
    )
    .await?;
    Ok(())
}
