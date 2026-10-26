//! The create wizard's pure rules (`planning/campaigns-and-table-research.md` 8.1): the values a
//! draft holds, what each of the five steps may save, which steps may be saved when, and what
//! finishing needs. The store applies a validated step with one statement; this module never
//! touches a database.

use df_types::{AccountId, CampaignId, TenantId};

use crate::{
    config::Tunables,
    errors::Refusal,
    records::{ContentRating, RoomCodes},
    roles::Delegations,
};

/// The adventure kinds at launch: one bounded adventure bundled with the product.
///
/// TODO(S1-08): the validated `AdventureCatalog` (content owner) replaces this list and
/// supplies each adventure's allowed ratings.
pub const ADVENTURE_KINDS: [&str; 1] = ["bundled"];

/// The ratings an adventure kind allows. An unknown kind allows none.
pub fn allowed_ratings(kind: &str) -> &'static [ContentRating] {
    match kind {
        "bundled" => &[
            ContentRating::Gentle,
            ContentRating::Standard,
            ContentRating::Mature,
        ],
        _ => &[],
    }
}

/// The last step; a draft whose `step` is this one has every step before it saved.
pub const LAST_STEP: u8 = 5;

/// The largest hourly figure a custom spend preset may carry (micro-dollars: 1,000 dollars).
pub const MAXIMUM_CUSTOM_HOURLY_MICRO_DOLLARS: u64 = 1_000_000_000;
/// The largest per-session ceiling a custom spend preset may carry (10,000 dollars).
pub const MAXIMUM_SESSION_CEILING_MICRO_DOLLARS: u64 = 10_000_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VoiceDefault {
    On,
    Off,
}

impl VoiceDefault {
    pub const fn text(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::Off => "off",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "on" => Some(Self::On),
            "off" => Some(Self::Off),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PictureDefault {
    ExistingArt,
    Automatic,
}

impl PictureDefault {
    pub const fn text(self) -> &'static str {
        match self {
            Self::ExistingArt => "existing_art",
            Self::Automatic => "automatic",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "existing_art" => Some(Self::ExistingArt),
            "automatic" => Some(Self::Automatic),
            _ => None,
        }
    }
}

/// The spend presets are F's catalog; this is the closed set the table stores. F supplies the
/// labels and the amounts, and a preset other than `Custom` carries no figures here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpendPreset {
    Story,
    Immersive,
    Cinematic,
    Custom,
}

impl SpendPreset {
    pub const fn text(self) -> &'static str {
        match self {
            Self::Story => "story",
            Self::Immersive => "immersive",
            Self::Cinematic => "cinematic",
            Self::Custom => "custom",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "story" => Some(Self::Story),
            "immersive" => Some(Self::Immersive),
            "cinematic" => Some(Self::Cinematic),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }
}

/// An intention only: F records the sponsor's consent, and Start stays blocked until F's
/// `start_readiness` says so.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FundingModel {
    NamedSponsor,
    SharedPot,
}

impl FundingModel {
    pub const fn text(self) -> &'static str {
        match self {
            Self::NamedSponsor => "named_sponsor",
            Self::SharedPot => "shared_pot",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "named_sponsor" => Some(Self::NamedSponsor),
            "shared_pot" => Some(Self::SharedPot),
            _ => None,
        }
    }
}

/// A draft campaign: every value the wizard collects, with the column defaults until a step saves
/// its own. It becomes a campaign when `finish` flips it, and has no members and no events until
/// then.
#[derive(Clone, Debug)]
pub struct DraftRow {
    pub id: CampaignId,
    pub owner: AccountId,
    pub tenant: TenantId,
    pub name: String,
    pub adventure_kind: String,
    pub content_rating: ContentRating,
    pub seat_count: u32,
    pub room_codes: RoomCodes,
    pub default_co_host_delegations: Delegations,
    pub voice_default: VoiceDefault,
    pub picture_default: PictureDefault,
    pub spend_preset: SpendPreset,
    pub custom_hourly_micro_dollars: Option<u64>,
    pub session_ceiling_micro_dollars: Option<u64>,
    pub funding_model: FundingModel,
    pub sponsor: Option<AccountId>,
    /// The step to resume at: one past the highest step saved, at most [`LAST_STEP`].
    pub step: u8,
    pub revision: u64,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
}

/// The wizard's values on a finished campaign, as the settings tab shows them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CampaignSettings {
    pub adventure_kind: String,
    pub voice_default: VoiceDefault,
    pub picture_default: PictureDefault,
    pub spend_preset: SpendPreset,
    pub custom_hourly_micro_dollars: Option<u64>,
    pub session_ceiling_micro_dollars: Option<u64>,
    pub funding_model: FundingModel,
    pub sponsor: Option<AccountId>,
}

/// What the person chose for their own hero (step 2).
///
/// TODO(S1-09): a template choice belongs to the characters owner (O2); until their template
/// reference exists the only choice is to pick in the lobby.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeroChoice {
    ChooseInLobby,
}

/// One step's values. Saving a step writes only the columns of that step.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DraftStepData {
    About {
        name: String,
        adventure_kind: String,
        content_rating: ContentRating,
        seat_count: u32,
    },
    Hero(HeroChoice),
    Table {
        voice_default: VoiceDefault,
        picture_default: PictureDefault,
        spend_preset: SpendPreset,
        custom_hourly_micro_dollars: Option<u64>,
        session_ceiling_micro_dollars: Option<u64>,
    },
    Joining {
        default_co_host_delegations: Delegations,
        room_codes: RoomCodes,
        funding_model: FundingModel,
    },
    /// The summary: nothing to save. Saving it only confirms the person reached it.
    Review,
}

impl DraftStepData {
    /// The step number this data belongs to.
    pub const fn number(&self) -> u8 {
        match self {
            Self::About { .. } => 1,
            Self::Hero(_) => 2,
            Self::Table { .. } => 3,
            Self::Joining { .. } => 4,
            Self::Review => 5,
        }
    }

    /// The values a step may save: the closed sets, the lengths and the seat bound.
    pub fn validate(&self, tunables: &Tunables) -> Result<(), Refusal> {
        match self {
            Self::About {
                name,
                adventure_kind,
                content_rating,
                seat_count,
            } => {
                let length = name.trim().chars().count();
                if !(1..=80).contains(&length)
                    || !ADVENTURE_KINDS.contains(&adventure_kind.as_str())
                    || !allowed_ratings(adventure_kind).contains(content_rating)
                    || *seat_count == 0
                    || *seat_count > tunables.maximum_seats
                {
                    return Err(Refusal::InvalidInput);
                }
                Ok(())
            }
            Self::Hero(HeroChoice::ChooseInLobby) | Self::Review => Ok(()),
            Self::Table {
                spend_preset,
                custom_hourly_micro_dollars,
                session_ceiling_micro_dollars,
                ..
            } => match (
                spend_preset,
                custom_hourly_micro_dollars,
                session_ceiling_micro_dollars,
            ) {
                (SpendPreset::Custom, Some(hourly), Some(ceiling))
                    if (1..=MAXIMUM_CUSTOM_HOURLY_MICRO_DOLLARS).contains(hourly)
                        && (*hourly..=MAXIMUM_SESSION_CEILING_MICRO_DOLLARS).contains(ceiling) =>
                {
                    Ok(())
                }
                (SpendPreset::Custom, _, _) => Err(Refusal::InvalidInput),
                (_, None, None) => Ok(()),
                // A preset other than custom carries no figures of its own.
                _ => Err(Refusal::InvalidInput),
            },
            Self::Joining { .. } => Ok(()),
        }
    }
}

/// Whether a step may be saved now: any step up to the one the draft has reached (going back
/// never loses input), never one beyond it.
pub fn may_save(current_step: u8, step: u8) -> bool {
    (1..=LAST_STEP).contains(&step) && step <= current_step.clamp(1, LAST_STEP)
}

/// The step a draft is at after saving `saved`: one past the highest step saved, capped at the
/// last. Saving an earlier step again never moves it back.
pub fn step_after(current_step: u8, saved: u8) -> u8 {
    current_step.max(saved.saturating_add(1)).min(LAST_STEP)
}

/// Finishing needs every step before the summary saved.
pub fn may_finish(current_step: u8) -> bool {
    current_step >= LAST_STEP
}

/// The sponsor a funding model records: the creator for a named sponsor, nobody for a shared pot.
pub fn sponsor_of(model: FundingModel, owner: AccountId) -> Option<AccountId> {
    match model {
        FundingModel::NamedSponsor => Some(owner),
        FundingModel::SharedPot => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn about(name: &str, seats: u32) -> DraftStepData {
        DraftStepData::About {
            name: name.to_owned(),
            adventure_kind: "bundled".to_owned(),
            content_rating: ContentRating::Standard,
            seat_count: seats,
        }
    }

    #[test]
    fn step_one_checks_the_name_the_adventure_the_rating_and_the_seat_bound() {
        let tunables = Tunables::default();
        assert_eq!(about("The Sunken Bell", 4).validate(&tunables), Ok(()));
        assert_eq!(
            about("   ", 4).validate(&tunables),
            Err(Refusal::InvalidInput)
        );
        assert_eq!(
            about(&"x".repeat(81), 4).validate(&tunables),
            Err(Refusal::InvalidInput)
        );
        assert_eq!(
            about("A", 0).validate(&tunables),
            Err(Refusal::InvalidInput)
        );
        assert_eq!(
            about("A", tunables.maximum_seats + 1).validate(&tunables),
            Err(Refusal::InvalidInput)
        );
        let unknown = DraftStepData::About {
            name: "A".to_owned(),
            adventure_kind: "homebrew".to_owned(),
            content_rating: ContentRating::Standard,
            seat_count: 4,
        };
        assert_eq!(unknown.validate(&tunables), Err(Refusal::InvalidInput));
    }

    #[test]
    fn a_custom_spend_preset_needs_both_figures_inside_the_bounds_and_a_named_preset_none() {
        let tunables = Tunables::default();
        let table = |preset, hourly, ceiling| DraftStepData::Table {
            voice_default: VoiceDefault::On,
            picture_default: PictureDefault::ExistingArt,
            spend_preset: preset,
            custom_hourly_micro_dollars: hourly,
            session_ceiling_micro_dollars: ceiling,
        };
        assert_eq!(
            table(SpendPreset::Story, None, None).validate(&tunables),
            Ok(())
        );
        assert_eq!(
            table(SpendPreset::Custom, Some(2_000_000), Some(10_000_000)).validate(&tunables),
            Ok(())
        );
        for bad in [
            table(SpendPreset::Custom, None, None),
            table(SpendPreset::Custom, Some(2_000_000), None),
            table(SpendPreset::Custom, Some(0), Some(10)),
            // A ceiling below the hourly figure could never fund an hour.
            table(SpendPreset::Custom, Some(5_000_000), Some(1_000_000)),
            table(
                SpendPreset::Custom,
                Some(MAXIMUM_CUSTOM_HOURLY_MICRO_DOLLARS + 1),
                Some(MAXIMUM_SESSION_CEILING_MICRO_DOLLARS),
            ),
            table(SpendPreset::Story, Some(1), Some(1)),
        ] {
            assert_eq!(
                bad.validate(&tunables),
                Err(Refusal::InvalidInput),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_step_can_be_saved_up_to_the_one_reached_and_going_back_never_moves_the_draft_back() {
        assert!(may_save(1, 1));
        assert!(!may_save(1, 2));
        assert!(may_save(3, 1));
        assert!(may_save(3, 3));
        assert!(!may_save(3, 4));
        assert!(may_save(5, 5));
        assert!(!may_save(5, 6));
        assert!(!may_save(3, 0));
        assert_eq!(step_after(1, 1), 2);
        assert_eq!(step_after(4, 2), 4);
        assert_eq!(step_after(4, 4), 5);
        assert_eq!(step_after(5, 5), 5);
        assert!(!may_finish(4));
        assert!(may_finish(5));
    }

    #[test]
    fn every_step_number_matches_its_data_and_each_closed_set_round_trips_through_its_text() {
        let steps = [
            about("A", 4),
            DraftStepData::Hero(HeroChoice::ChooseInLobby),
            DraftStepData::Joining {
                default_co_host_delegations: Delegations::NONE,
                room_codes: RoomCodes::Off,
                funding_model: FundingModel::SharedPot,
            },
            DraftStepData::Review,
        ];
        let numbers: Vec<u8> = steps.iter().map(DraftStepData::number).collect();
        assert_eq!(numbers, [1, 2, 4, 5]);
        for preset in [
            SpendPreset::Story,
            SpendPreset::Immersive,
            SpendPreset::Cinematic,
            SpendPreset::Custom,
        ] {
            assert_eq!(SpendPreset::parse(preset.text()), Some(preset));
        }
        for model in [FundingModel::NamedSponsor, FundingModel::SharedPot] {
            assert_eq!(FundingModel::parse(model.text()), Some(model));
        }
        assert_eq!(VoiceDefault::parse("on"), Some(VoiceDefault::On));
        assert_eq!(
            PictureDefault::parse("automatic"),
            Some(PictureDefault::Automatic)
        );
        assert_eq!(SpendPreset::parse("free"), None);
    }

    #[test]
    fn a_named_sponsor_is_the_creator_and_a_shared_pot_has_none() {
        let owner = AccountId::from_bytes(&[1; 16]).unwrap();
        assert_eq!(sponsor_of(FundingModel::NamedSponsor, owner), Some(owner));
        assert_eq!(sponsor_of(FundingModel::SharedPot, owner), None);
    }
}

/// Host-edited campaign defaults; finance still owns admission and consent for each session.
#[derive(Clone, Debug)]
pub struct SettingsUpdate {
    pub name: String,
    pub content_rating: ContentRating,
    pub seat_count: u32,
    pub room_codes: RoomCodes,
    pub default_co_host_delegations: Delegations,
    pub settings: CampaignSettings,
}

impl SettingsUpdate {
    pub fn validate(&self, tunables: &Tunables) -> Result<(), Refusal> {
        DraftStepData::About {
            name: self.name.clone(),
            adventure_kind: self.settings.adventure_kind.clone(),
            content_rating: self.content_rating,
            seat_count: self.seat_count,
        }
        .validate(tunables)?;
        DraftStepData::Table {
            voice_default: self.settings.voice_default,
            picture_default: self.settings.picture_default,
            spend_preset: self.settings.spend_preset,
            custom_hourly_micro_dollars: self.settings.custom_hourly_micro_dollars,
            session_ceiling_micro_dollars: self.settings.session_ceiling_micro_dollars,
        }
        .validate(tunables)
    }
}
