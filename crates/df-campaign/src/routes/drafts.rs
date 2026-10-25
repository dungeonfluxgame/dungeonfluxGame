//! Handlers for the create wizard (8.1): start a draft, read it back to resume, save a step, and
//! finish. Only the owner reaches a draft; every other caller sees it as missing.

use df_campaign_wire as wire;
use df_types::CampaignId;

use super::{
    guard::Guard,
    request::{Ctx, co_host_body, delegations_of, hex_of, invalid, not_found, operation_of},
    responses::{Handled, failure_response, ok},
};
use crate::{
    draft::{
        DraftRow, DraftStepData, FundingModel, HeroChoice, PictureDefault, SpendPreset,
        VoiceDefault,
    },
    records::RoomCodes,
    service::Ports,
};

fn draft_id(ctx: &Ctx<'_, impl Ports, impl Guard>) -> Result<CampaignId, super::responses::Failed> {
    ctx.param("draft")
        .and_then(|text| CampaignId::from_hex(text).ok())
        .ok_or_else(not_found)
}

pub(super) fn voice_name(value: VoiceDefault) -> wire::VoiceName {
    match value {
        VoiceDefault::On => wire::VoiceName::On,
        VoiceDefault::Off => wire::VoiceName::Off,
    }
}

pub(super) fn picture_name(value: PictureDefault) -> wire::PictureName {
    match value {
        PictureDefault::ExistingArt => wire::PictureName::ExistingArt,
        PictureDefault::Automatic => wire::PictureName::Automatic,
    }
}

pub(super) fn spend_name(value: SpendPreset) -> wire::SpendPresetName {
    match value {
        SpendPreset::Story => wire::SpendPresetName::Story,
        SpendPreset::Immersive => wire::SpendPresetName::Immersive,
        SpendPreset::Cinematic => wire::SpendPresetName::Cinematic,
        SpendPreset::Custom => wire::SpendPresetName::Custom,
    }
}

pub(super) fn funding_name(value: FundingModel) -> wire::FundingModelName {
    match value {
        FundingModel::NamedSponsor => wire::FundingModelName::NamedSponsor,
        FundingModel::SharedPot => wire::FundingModelName::SharedPot,
    }
}

pub(super) fn draft_view(draft: &DraftRow) -> wire::DraftView {
    wire::DraftView {
        draft_id: hex_of(draft.id.as_bytes()),
        step: draft.step,
        name: draft.name.clone(),
        adventure_kind: draft.adventure_kind.clone(),
        content_rating: draft.content_rating,
        seat_count: draft.seat_count,
        hero: wire::HeroName::ChooseInLobby,
        voice_default: voice_name(draft.voice_default),
        picture_default: picture_name(draft.picture_default),
        spend_preset: spend_name(draft.spend_preset),
        custom_hourly_micro_dollars: draft.custom_hourly_micro_dollars,
        session_ceiling_micro_dollars: draft.session_ceiling_micro_dollars,
        default_co_host: co_host_body(draft.default_co_host_delegations),
        room_codes: draft.room_codes == RoomCodes::WithApproval,
        funding_model: funding_name(draft.funding_model),
        expires_at_ms: draft.expires_at_ms,
    }
}

pub(super) async fn start<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let draft = ctx
        .service()
        .start_draft(ctx.account()?)
        .await
        .map_err(failure_response)?;
    ok(&draft_view(&draft))
}

pub(super) async fn get<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let draft = ctx
        .service()
        .get_draft(ctx.account()?, draft_id(ctx)?)
        .await
        .map_err(failure_response)?;
    ok(&draft_view(&draft))
}

/// The body of step `n` is parsed by that step's own type; a step number outside 1 to 5 is a
/// missing route.
fn step_data<P: Ports, G: Guard>(
    ctx: &Ctx<'_, P, G>,
) -> Result<DraftStepData, super::responses::Failed> {
    let number: u8 = ctx
        .param("step")
        .and_then(|text| text.parse().ok())
        .ok_or_else(not_found)?;
    Ok(match number {
        1 => {
            let body: wire::DraftStep1Body = ctx.json()?;
            DraftStepData::About {
                name: body.name,
                adventure_kind: body.adventure_kind,
                content_rating: body.content_rating,
                seat_count: body.seat_count,
            }
        }
        2 => {
            let body: wire::DraftStep2Body = ctx.json()?;
            match body.hero {
                wire::HeroName::ChooseInLobby => DraftStepData::Hero(HeroChoice::ChooseInLobby),
            }
        }
        3 => {
            let body: wire::DraftStep3Body = ctx.json()?;
            DraftStepData::Table {
                voice_default: match body.voice_default {
                    wire::VoiceName::On => VoiceDefault::On,
                    wire::VoiceName::Off => VoiceDefault::Off,
                },
                picture_default: match body.picture_default {
                    wire::PictureName::ExistingArt => PictureDefault::ExistingArt,
                    wire::PictureName::Automatic => PictureDefault::Automatic,
                },
                spend_preset: match body.spend_preset {
                    wire::SpendPresetName::Story => SpendPreset::Story,
                    wire::SpendPresetName::Immersive => SpendPreset::Immersive,
                    wire::SpendPresetName::Cinematic => SpendPreset::Cinematic,
                    wire::SpendPresetName::Custom => SpendPreset::Custom,
                },
                custom_hourly_micro_dollars: body.custom_hourly_micro_dollars,
                session_ceiling_micro_dollars: body.session_ceiling_micro_dollars,
            }
        }
        4 => {
            let body: wire::DraftStep4Body = ctx.json()?;
            DraftStepData::Joining {
                default_co_host_delegations: delegations_of(&body.default_co_host),
                room_codes: if body.room_codes {
                    RoomCodes::WithApproval
                } else {
                    RoomCodes::Off
                },
                funding_model: match body.funding_model {
                    wire::FundingModelName::NamedSponsor => FundingModel::NamedSponsor,
                    wire::FundingModelName::SharedPot => FundingModel::SharedPot,
                },
            }
        }
        5 => {
            let _: wire::Done = ctx.json()?;
            DraftStepData::Review
        }
        _ => return Err(not_found()),
    })
}

pub(super) async fn save_step<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let data = step_data(ctx)?;
    let draft = ctx
        .service()
        .save_draft_step(ctx.account()?, draft_id(ctx)?, data)
        .await
        .map_err(failure_response)?;
    ok(&draft_view(&draft))
}

pub(super) async fn finish<P: Ports, G: Guard>(ctx: &Ctx<'_, P, G>) -> Handled {
    let body: wire::DraftFinishBody = ctx.json()?;
    let account = ctx.account()?;
    let service = ctx.service();
    let created = service
        .finish_draft(
            account,
            draft_id(ctx)?,
            operation_of(&body.operation).map_err(|_| invalid())?,
        )
        .await
        .map_err(failure_response)?;
    let snapshot = service
        .member_snapshot(account, created.id)
        .await
        .map_err(failure_response)?
        .ok_or_else(not_found)?;
    ok(&wire::CampaignCreated {
        campaign_id: hex_of(created.id.as_bytes()),
        member_id: hex_of(snapshot.member.as_bytes()),
    })
}
