//! The lobby's writes on PostgreSQL (8.4): a seated member's character, ready state and microphone.
//! Lock order: the actor's account row (B's share lock, so a closing account cannot change
//! anything), then the campaign row, then the lobby row. Taking the campaign lock is what orders
//! a change against `session_started`, which freezes the rows under the same lock.

use df_campaign::{
    errors::{Failure, Outcome, Refusal},
    lobby::CharacterChoice,
    lobby_state::{LobbyMemberRow, MicrophonePreference, MicrophoneReport},
    records::{AccountStatus, CampaignStatus, MemberStatus},
};
use df_types::{AccountId, CampaignId, SessionId};

use super::{
    conn::Tx,
    queries::{self as q, LobbyChange},
    shared::refuse,
};

/// The checks every lobby change shares; returns the actor's member id.
async fn seated_actor(
    tx: &Tx<'_>,
    actor: AccountId,
    campaign: CampaignId,
    session: SessionId,
) -> Outcome<df_types::MemberId> {
    if q::lock_account(tx, actor).await? != AccountStatus::Active {
        return refuse(Refusal::AccountClosing);
    }
    let Some(row) = q::lock_campaign(tx, campaign).await? else {
        return refuse(Refusal::NotFound);
    };
    let member = match q::member_by_account(tx, campaign, actor).await? {
        Some(member) if member.status == MemberStatus::Active => member,
        // A stranger and a former member are as good as missing.
        _ => return refuse(Refusal::NotFound),
    };
    if row.status != CampaignStatus::Active {
        return refuse(Refusal::CampaignUnavailable);
    }
    if !member.holds_seat {
        return refuse(Refusal::NotSeated);
    }
    // A session that has started has frozen its lobby; a member who joined after cannot slip in.
    if q::lobby_frozen(tx, session).await? {
        return refuse(Refusal::SessionInProgress);
    }
    Ok(member.member)
}

async fn apply(
    tx: &Tx<'_>,
    session: SessionId,
    member: df_types::MemberId,
    campaign: CampaignId,
    change: LobbyChange,
    now_ms: u64,
) -> Outcome<LobbyMemberRow> {
    match q::change_lobby_member(tx, session, member, campaign, &change, now_ms).await? {
        Some(row) => Ok(row),
        // The row exists and is frozen.
        None => refuse(Refusal::SessionInProgress),
    }
}

pub(super) async fn choose_character(
    tx: &Tx<'_>,
    actor: AccountId,
    campaign: CampaignId,
    session: SessionId,
    choice: CharacterChoice,
    reference: Option<[u8; 16]>,
    now_ms: u64,
) -> Outcome<LobbyMemberRow> {
    // A choice names its character; "undecided" names none.
    if choice.is_chosen() != reference.is_some() {
        return Err(Failure::Refused(Refusal::InvalidInput));
    }
    let member = seated_actor(tx, actor, campaign, session).await?;
    apply(
        tx,
        session,
        member,
        campaign,
        LobbyChange::Character(choice, reference),
        now_ms,
    )
    .await
}

pub(super) async fn set_ready(
    tx: &Tx<'_>,
    actor: AccountId,
    campaign: CampaignId,
    session: SessionId,
    ready: bool,
    now_ms: u64,
) -> Outcome<LobbyMemberRow> {
    let member = seated_actor(tx, actor, campaign, session).await?;
    if ready {
        let chosen = q::lobby_member_row(tx, session, member)
            .await?
            .is_some_and(|row| row.character.is_chosen());
        if !chosen {
            return refuse(Refusal::CharacterNotChosen);
        }
    }
    apply(
        tx,
        session,
        member,
        campaign,
        LobbyChange::Ready(ready),
        now_ms,
    )
    .await
}

pub(super) async fn set_microphone(
    tx: &Tx<'_>,
    actor: AccountId,
    campaign: CampaignId,
    session: SessionId,
    preference: MicrophonePreference,
    report: MicrophoneReport,
    now_ms: u64,
) -> Outcome<LobbyMemberRow> {
    let member = seated_actor(tx, actor, campaign, session).await?;
    apply(
        tx,
        session,
        member,
        campaign,
        LobbyChange::Microphone(preference, report),
        now_ms,
    )
    .await
}
