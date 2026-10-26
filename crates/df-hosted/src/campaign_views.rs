//! Explicit mapping between the account wizard and the campaign owner's staged contract.

use df_account_app::{wire::PacePreset, wire_campaign as wire};
use df_campaign::{
    draft as domain,
    records::{ContentRating, RoomCodes},
    roles::{Delegation, Delegations},
};
use df_portal::app_api::ports::{PortError, PortResult};

pub(super) fn draft(row: domain::DraftRow) -> PortResult<wire::CampaignDraft> {
    Ok(wire::CampaignDraft {
        name: row.name,
        adventure_id: Some(row.adventure_kind),
        content_rating: Some(match row.content_rating {
            ContentRating::Standard => wire::ContentRating::Teen,
            ContentRating::Mature => wire::ContentRating::Mature,
            ContentRating::Gentle => return Err(PortError::Unavailable),
        }),
        seat_count: u8::try_from(row.seat_count).map_err(|_| PortError::Unavailable)?,
        hero_choice: wire::HeroChoice::InLobby,
        has_voice: row.voice_default == domain::VoiceDefault::On,
        pictures: match row.picture_default {
            domain::PictureDefault::ExistingArt => wire::PictureDefault::ExistingArtOnly,
            domain::PictureDefault::Automatic => wire::PictureDefault::Automatic,
        },
        pace: match row.spend_preset {
            domain::SpendPreset::Story => PacePreset::Story,
            domain::SpendPreset::Immersive => PacePreset::Immersive,
            domain::SpendPreset::Cinematic => PacePreset::Cinematic,
            domain::SpendPreset::Custom => PacePreset::Custom,
        },
        custom_ceiling_cents_per_hour: row
            .custom_hourly_micro_dollars
            .map(|value| {
                if !value.is_multiple_of(10_000) {
                    return Err(PortError::Unavailable);
                }
                u32::try_from(value / 10_000).map_err(|_| PortError::Unavailable)
            })
            .transpose()?,
        co_hosts_can_invite: row
            .default_co_host_delegations
            .contains(Delegation::InvitePlayers),
        co_hosts_can_start: row
            .default_co_host_delegations
            .contains(Delegation::StartOrPauseSession),
        co_hosts_can_export: row
            .default_co_host_delegations
            .contains(Delegation::ExportCampaign),
        join_mode: match row.room_codes {
            RoomCodes::Off => wire::JoinMode::LinkOnly,
            RoomCodes::WithApproval => wire::JoinMode::LinkAndRoomCode,
        },
        funding: match row.funding_model {
            domain::FundingModel::NamedSponsor => wire::FundingModel::Sponsor,
            domain::FundingModel::SharedPot => wire::FundingModel::SharedPot,
        },
    })
}

pub(super) fn step(number: u8, draft: wire::CampaignDraft) -> PortResult<domain::DraftStepData> {
    Ok(match number {
        1 => domain::DraftStepData::About {
            name: draft.name,
            adventure_kind: draft.adventure_id.ok_or(PortError::Forbidden)?,
            content_rating: match draft.content_rating.ok_or(PortError::Forbidden)? {
                wire::ContentRating::Teen => ContentRating::Standard,
                wire::ContentRating::Mature => ContentRating::Mature,
            },
            seat_count: u32::from(draft.seat_count),
        },
        2 => domain::DraftStepData::Hero(domain::HeroChoice::ChooseInLobby),
        3 => domain::DraftStepData::Table {
            voice_default: if draft.has_voice {
                domain::VoiceDefault::On
            } else {
                domain::VoiceDefault::Off
            },
            picture_default: match draft.pictures {
                wire::PictureDefault::Automatic => domain::PictureDefault::Automatic,
                wire::PictureDefault::ExistingArtOnly => domain::PictureDefault::ExistingArt,
            },
            spend_preset: match draft.pace {
                PacePreset::Story => domain::SpendPreset::Story,
                PacePreset::Immersive => domain::SpendPreset::Immersive,
                PacePreset::Cinematic => domain::SpendPreset::Cinematic,
                PacePreset::Custom => domain::SpendPreset::Custom,
            },
            custom_hourly_micro_dollars: draft
                .custom_ceiling_cents_per_hour
                .map(|value| u64::from(value) * 10_000),
            session_ceiling_micro_dollars: None,
        },
        4 => {
            let mut delegations = Delegations::NONE;
            if draft.co_hosts_can_invite {
                delegations = delegations.with(Delegation::InvitePlayers);
            }
            if draft.co_hosts_can_start {
                delegations = delegations.with(Delegation::StartOrPauseSession);
            }
            if draft.co_hosts_can_export {
                delegations = delegations.with(Delegation::ExportCampaign);
            }
            domain::DraftStepData::Joining {
                default_co_host_delegations: delegations,
                room_codes: match draft.join_mode {
                    wire::JoinMode::LinkOnly => RoomCodes::Off,
                    wire::JoinMode::LinkAndRoomCode => RoomCodes::WithApproval,
                },
                funding_model: match draft.funding {
                    wire::FundingModel::Sponsor => domain::FundingModel::NamedSponsor,
                    wire::FundingModel::SharedPot => domain::FundingModel::SharedPot,
                },
            }
        }
        5 => domain::DraftStepData::Review,
        _ => return Err(PortError::NotFound),
    })
}

pub(super) fn lobby(view: df_campaign::service::LobbyView) -> PortResult<wire::LobbyView> {
    use df_campaign::service::StartBlocker;
    Ok(wire::LobbyView {
        session_id: df_commerce::finance_ids::hex(view.session.as_bytes()),
        campaign_id: df_commerce::finance_ids::hex(view.campaign.id.as_bytes()),
        campaign_name: view.campaign.name,
        can_manage: view.can_start,
        start_blockers: view
            .blockers
            .into_iter()
            .map(|blocker| match blocker {
                StartBlocker::CampaignArchived => Ok(wire::StartBlocker::CampaignArchived),
                StartBlocker::SessionNotInLobby => Ok(wire::StartBlocker::SessionNotInLobby),
                StartBlocker::NoSeatedMembers => Ok(wire::StartBlocker::NoSeatedMembers),
                StartBlocker::FundingNotReady => Ok(wire::StartBlocker::FundingNotReady),
                _ => Err(PortError::Unavailable),
            })
            .collect::<PortResult<Vec<_>>>()?,
        members: view
            .seats
            .into_iter()
            .map(|seat| {
                Ok(wire::LobbyMember {
                    display_name: seat.display_name.unwrap_or_else(|| "Player".to_owned()),
                    is_me: seat.member.member == view.you,
                })
            })
            .collect::<PortResult<Vec<_>>>()?,
        funding_note: if view
            .funding
            .is_some_and(|funding| funding.no_charge_until_start && !funding.waiting_for_sponsor)
        {
            wire::FundingNote::NoChargeUntilHostStarts
        } else {
            wire::FundingNote::WaitingForSponsor
        },
        has_started: !matches!(
            view.phase,
            df_campaign::store::SessionPhase::Lobby | df_campaign::store::SessionPhase::Ended
        ),
        poll_seconds: df_campaign::config::Tunables::default().feed_poll_seconds,
    })
}

pub(super) fn settings_update(
    draft: wire::CampaignDraft,
    account: df_types::AccountId,
) -> PortResult<domain::SettingsUpdate> {
    let domain::DraftStepData::About {
        name,
        adventure_kind,
        content_rating,
        seat_count,
    } = step(1, draft.clone())?
    else {
        return Err(PortError::Forbidden);
    };
    let domain::DraftStepData::Table {
        voice_default,
        picture_default,
        spend_preset,
        custom_hourly_micro_dollars,
        session_ceiling_micro_dollars,
    } = step(3, draft.clone())?
    else {
        return Err(PortError::Forbidden);
    };
    let domain::DraftStepData::Joining {
        default_co_host_delegations,
        room_codes,
        funding_model,
    } = step(4, draft)?
    else {
        return Err(PortError::Forbidden);
    };
    Ok(domain::SettingsUpdate {
        name,
        content_rating,
        seat_count,
        room_codes,
        default_co_host_delegations,
        settings: domain::CampaignSettings {
            adventure_kind,
            voice_default,
            picture_default,
            spend_preset,
            custom_hourly_micro_dollars,
            session_ceiling_micro_dollars,
            funding_model,
            sponsor: domain::sponsor_of(funding_model, account),
        },
    })
}

pub(super) fn settings(view: &df_campaign::service::Overview) -> PortResult<wire::CampaignDraft> {
    let row = &view.campaign;
    let settings = &view.settings;
    draft(domain::DraftRow {
        id: row.id,
        owner: row.owner,
        tenant: row.tenant,
        name: row.name.clone(),
        adventure_kind: settings.adventure_kind.clone(),
        content_rating: row.content_rating,
        seat_count: row.seat_count,
        room_codes: row.room_codes,
        default_co_host_delegations: row.default_co_host_delegations,
        voice_default: settings.voice_default,
        picture_default: settings.picture_default,
        spend_preset: settings.spend_preset,
        custom_hourly_micro_dollars: settings.custom_hourly_micro_dollars,
        session_ceiling_micro_dollars: settings.session_ceiling_micro_dollars,
        funding_model: settings.funding_model,
        sponsor: settings.sponsor,
        step: 5,
        revision: row.revision,
        created_at_ms: row.created_at_ms,
        expires_at_ms: 0,
    })
}

pub(super) fn catalog() -> wire::WizardCatalog {
    wire::WizardCatalog {
        adventures: Vec::new(),
        maximum_seats: 6,
        presets: [
            PacePreset::Story,
            PacePreset::Immersive,
            PacePreset::Cinematic,
            PacePreset::Custom,
        ]
        .into_iter()
        .map(|preset| df_account_app::wire::PresetOffer {
            preset,
            ceiling_cents_per_hour: None,
        })
        .collect(),
        is_proposed_pricing: true,
        custom_minimum_cents_per_hour: 100,
        custom_maximum_cents_per_hour: 5_000,
    }
}
