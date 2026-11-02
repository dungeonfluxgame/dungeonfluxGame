//! Live changes to the scene a fixture describes: the caption, the party strip, the phone's
//! choices, pictures and sound, made from the dev CLI while the page is open.
//!
//! A fixture is a file; a change here is not saved to it. The changes are kept as an overlay that
//! is drawn on top of the fixture, so `scene reset` (or choosing another fixture) returns to the
//! file. This module is the one place that decides what a change means and what is too large to
//! accept. It does not enforce the table-experience contract: a label of six words is accepted so
//! the playground can show the break, exactly as a fixture that breaks it is.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::controls::Device;
use crate::fixture::{
    Caption, Choice, ChoiceGroup, Fixture, ImageRef, PartyMember, UnavailableChoice,
};

/// Most characters in a caption line.
pub const MAXIMUM_CAPTION_CHARACTERS: usize = 600;
/// Most characters in a name, a speaker or a label.
pub const MAXIMUM_NAME_CHARACTERS: usize = 80;
/// Most members in the party strip.
pub const MAXIMUM_PARTY_MEMBERS: usize = 8;
/// Most choices in one list (suggested, a group or unavailable).
pub const MAXIMUM_CHOICES_PER_LIST: usize = 24;

/// Most sound cues kept for pages to collect; a page that was away longer misses the older ones.
pub const MAXIMUM_CUES_KEPT: usize = 16;
/// Loudest volume, in percent.
pub const MAXIMUM_VOLUME: u8 = 100;

/// What the overlay does to one part of the scene: put something there, or take what is there away.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Override<T> {
    Set(T),
    Clear,
}

/// What the overlay does to the caption.
pub type CaptionOverride = Override<Caption>;

/// Which screens a sound is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    Both,
    Tv,
    Phone,
}

impl Target {
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "both" => Some(Self::Both),
            "tv" => Some(Self::Tv),
            "phone" => Some(Self::Phone),
            _ => None,
        }
    }

    /// Whether a page showing `device` should play a sound meant for this target.
    pub fn plays_on(self, device: Device) -> bool {
        match self {
            Self::Both => true,
            Self::Tv => device.shows_tv(),
            Self::Phone => device.shows_phone(),
        }
    }
}

/// A picture slot on the screens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageSlot {
    /// Behind the figures on the shared screen.
    TvBackdrop,
    /// In the middle of the shared screen.
    TvCard,
    /// At the top of a phone's story.
    PhoneImage,
}

impl ImageSlot {
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "tv-backdrop" => Some(Self::TvBackdrop),
            "tv-card" => Some(Self::TvCard),
            "phone" => Some(Self::PhoneImage),
            _ => None,
        }
    }
}

/// Music that plays until it is stopped or replaced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub media: String,
    pub target: Target,
    /// Percent, 0 to 100.
    pub volume: u8,
    pub looping: bool,
}

/// A sound played once. Pages play each cue they have not played yet, by its id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cue {
    pub id: u64,
    pub target: Target,
    pub media: String,
    pub volume: u8,
}

/// The changes drawn on top of a fixture. A field that is `None` leaves the fixture's own.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SceneOverlay {
    pub caption: Option<CaptionOverride>,
    pub party: Option<Vec<PartyMember>>,
    pub suggested: Option<Vec<Choice>>,
    pub groups: Option<Vec<ChoiceGroup>>,
    pub unavailable: Option<Vec<UnavailableChoice>>,
    pub backdrop: Option<Override<ImageRef>>,
    pub card: Option<Override<ImageRef>>,
    pub phone_image: Option<Override<ImageRef>>,
    pub music: Option<Override<Track>>,
    /// How many cues there have ever been. It only rises, so a page can tell a new cue from one it
    /// has played even after a reset.
    #[serde(default)]
    pub cue_count: u64,
    /// The newest cues, oldest first.
    #[serde(default)]
    pub cues: Vec<Cue>,
}

/// One change to the scene, as the dev CLI sends it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "change", rename_all = "snake_case", deny_unknown_fields)]
pub enum SceneChange {
    Caption {
        speaker: Option<String>,
        text: String,
    },
    ClearCaption {},
    PartyAdd {
        member: PartyMember,
    },
    /// Changes the named member; a field left out stays as it is.
    PartySet {
        name: String,
        hit_points: Option<u32>,
        maximum_hit_points: Option<u32>,
        conditions: Option<Vec<String>>,
    },
    PartyRemove {
        name: String,
    },
    PartyClear {},
    /// Replaces the suggested list.
    Suggested {
        choices: Vec<Choice>,
    },
    /// Replaces one group's choices; an empty list removes the group.
    Group {
        name: String,
        choices: Vec<Choice>,
    },
    /// Replaces the unavailable list.
    Unavailable {
        choices: Vec<UnavailableChoice>,
    },
    /// Empties the suggested list, every group and the unavailable list.
    ClearChoices {},
    /// Shows a picture from the media library in a slot.
    Image {
        slot: ImageSlot,
        media: String,
        caption: Option<String>,
    },
    ClearImage {
        slot: ImageSlot,
    },
    /// Starts music, replacing any that was playing.
    Music {
        media: String,
        target: Target,
        volume: u8,
        looping: bool,
    },
    StopMusic {},
    /// Plays a sound once.
    Sound {
        media: String,
        target: Target,
        volume: u8,
    },
    /// Drops every change and returns to the fixture. Cues already made stay counted.
    Reset {},
}

/// Why a change was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneError {
    Empty(&'static str),
    TooLong { what: &'static str, limit: usize },
    DuplicateMember(String),
    UnknownMember(String),
    TooManyMembers,
    TooManyChoices { limit: usize },
    BadMaximum,
    BadVolume(u8),
}

impl fmt::Display for SceneError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty(what) => write!(formatter, "{what} cannot be empty"),
            Self::TooLong { what, limit } => {
                write!(formatter, "{what} is longer than {limit} characters")
            }
            Self::DuplicateMember(name) => write!(formatter, "'{name}' is already in the party"),
            Self::UnknownMember(name) => write!(formatter, "'{name}' is not in the party"),
            Self::TooManyMembers => write!(
                formatter,
                "the party strip holds at most {MAXIMUM_PARTY_MEMBERS} members"
            ),
            Self::TooManyChoices { limit } => {
                write!(formatter, "a list holds at most {limit} choices")
            }
            Self::BadMaximum => formatter.write_str("maximum hit points must be at least 1"),
            Self::BadVolume(volume) => {
                write!(
                    formatter,
                    "a volume is from 0 to {MAXIMUM_VOLUME} percent, not {volume}"
                )
            }
        }
    }
}

fn text(what: &'static str, value: &str, limit: usize) -> Result<(), SceneError> {
    if value.trim().is_empty() {
        Err(SceneError::Empty(what))
    } else if value.chars().count() > limit {
        Err(SceneError::TooLong { what, limit })
    } else {
        Ok(())
    }
}

fn labelled(what: &'static str, value: &str) -> Result<(), SceneError> {
    text(what, value, MAXIMUM_NAME_CHARACTERS)
}

fn check_volume(volume: u8) -> Result<(), SceneError> {
    if volume > MAXIMUM_VOLUME {
        Err(SceneError::BadVolume(volume))
    } else {
        Ok(())
    }
}

fn check_count(count: usize) -> Result<(), SceneError> {
    if count > MAXIMUM_CHOICES_PER_LIST {
        Err(SceneError::TooManyChoices {
            limit: MAXIMUM_CHOICES_PER_LIST,
        })
    } else {
        Ok(())
    }
}

fn check_choice(choice: &Choice) -> Result<(), SceneError> {
    labelled("a choice label", &choice.label)?;
    if let Some(explain) = &choice.explain {
        labelled("a choice explanation", explain)?;
    }
    Ok(())
}

fn check_member(member: &PartyMember) -> Result<(), SceneError> {
    labelled("a party member's name", &member.name)?;
    if member.maximum_hit_points == 0 {
        return Err(SceneError::BadMaximum);
    }
    for condition in &member.conditions {
        labelled("a condition", condition)?;
    }
    Ok(())
}

impl SceneOverlay {
    /// The party as drawn: the overlay's, or the fixture's.
    fn party_or<'a>(&'a self, base: &'a Fixture) -> &'a [PartyMember] {
        self.party.as_deref().unwrap_or(&base.tv.party)
    }

    /// Makes one change. `base` is the fixture the overlay sits on, so a change to one member or
    /// one group starts from what is drawn now. A refused change leaves the overlay as it was.
    pub fn apply(&mut self, change: SceneChange, base: &Fixture) -> Result<(), SceneError> {
        let mut next = self.clone();
        next.apply_unchecked(change, base)?;
        *self = next;
        Ok(())
    }

    fn apply_unchecked(&mut self, change: SceneChange, base: &Fixture) -> Result<(), SceneError> {
        match change {
            SceneChange::Caption {
                speaker,
                text: line,
            } => {
                text("the caption", &line, MAXIMUM_CAPTION_CHARACTERS)?;
                if let Some(speaker) = &speaker {
                    labelled("the speaker", speaker)?;
                }
                self.caption = Some(CaptionOverride::Set(Caption {
                    speaker,
                    text: line,
                }));
            }
            SceneChange::ClearCaption {} => self.caption = Some(CaptionOverride::Clear),
            SceneChange::PartyAdd { member } => {
                check_member(&member)?;
                let mut party = self.party_or(base).to_vec();
                if party.iter().any(|existing| existing.name == member.name) {
                    return Err(SceneError::DuplicateMember(member.name));
                }
                if party.len() >= MAXIMUM_PARTY_MEMBERS {
                    return Err(SceneError::TooManyMembers);
                }
                party.push(member);
                self.party = Some(party);
            }
            SceneChange::PartySet {
                name,
                hit_points,
                maximum_hit_points,
                conditions,
            } => {
                let mut party = self.party_or(base).to_vec();
                let member = party
                    .iter_mut()
                    .find(|member| member.name == name)
                    .ok_or(SceneError::UnknownMember(name))?;
                if let Some(hit_points) = hit_points {
                    member.hit_points = hit_points;
                }
                if let Some(maximum) = maximum_hit_points {
                    member.maximum_hit_points = maximum;
                }
                if let Some(conditions) = conditions {
                    member.conditions = conditions;
                }
                check_member(member)?;
                self.party = Some(party);
            }
            SceneChange::PartyRemove { name } => {
                let mut party = self.party_or(base).to_vec();
                let before = party.len();
                party.retain(|member| member.name != name);
                if party.len() == before {
                    return Err(SceneError::UnknownMember(name));
                }
                self.party = Some(party);
            }
            SceneChange::PartyClear {} => self.party = Some(Vec::new()),
            SceneChange::Suggested { choices } => {
                check_count(choices.len())?;
                choices.iter().try_for_each(check_choice)?;
                self.suggested = Some(choices);
            }
            SceneChange::Group { name, choices } => {
                labelled("a group name", &name)?;
                check_count(choices.len())?;
                choices.iter().try_for_each(check_choice)?;
                let mut groups = self
                    .groups
                    .clone()
                    .unwrap_or_else(|| base.phone.groups.clone());
                groups.retain(|group| group.name != name);
                if !choices.is_empty() {
                    groups.push(ChoiceGroup { name, choices });
                }
                self.groups = Some(groups);
            }
            SceneChange::Unavailable { choices } => {
                check_count(choices.len())?;
                for choice in &choices {
                    labelled("a choice label", &choice.label)?;
                    labelled("a reason", &choice.reason)?;
                }
                self.unavailable = Some(choices);
            }
            SceneChange::ClearChoices {} => {
                self.suggested = Some(Vec::new());
                self.groups = Some(Vec::new());
                self.unavailable = Some(Vec::new());
            }
            SceneChange::Image {
                slot,
                media,
                caption,
            } => {
                labelled("a media name", &media)?;
                if let Some(caption) = &caption {
                    labelled("a picture caption", caption)?;
                }
                *self.image_slot_mut(slot) = Some(Override::Set(ImageRef { media, caption }));
            }
            SceneChange::ClearImage { slot } => *self.image_slot_mut(slot) = Some(Override::Clear),
            SceneChange::Music {
                media,
                target,
                volume,
                looping,
            } => {
                labelled("a media name", &media)?;
                check_volume(volume)?;
                self.music = Some(Override::Set(Track {
                    media,
                    target,
                    volume,
                    looping,
                }));
            }
            SceneChange::StopMusic {} => self.music = Some(Override::Clear),
            SceneChange::Sound {
                media,
                target,
                volume,
            } => {
                labelled("a media name", &media)?;
                check_volume(volume)?;
                self.cue_count += 1;
                self.cues.push(Cue {
                    id: self.cue_count,
                    target,
                    media,
                    volume,
                });
                if self.cues.len() > MAXIMUM_CUES_KEPT {
                    self.cues.remove(0);
                }
            }
            SceneChange::Reset {} => *self = self.cleared(),
        }
        Ok(())
    }

    fn image_slot_mut(&mut self, slot: ImageSlot) -> &mut Option<Override<ImageRef>> {
        match slot {
            ImageSlot::TvBackdrop => &mut self.backdrop,
            ImageSlot::TvCard => &mut self.card,
            ImageSlot::PhoneImage => &mut self.phone_image,
        }
    }

    /// The overlay with every change dropped but the count of cues, which only rises. Used when a
    /// scene is reset or another fixture is chosen, so a page never mistakes a new cue for an old one.
    pub fn cleared(&self) -> Self {
        Self {
            cue_count: self.cue_count,
            ..Self::default()
        }
    }

    /// True when the overlay changes nothing about what is drawn. Past cues do not count.
    pub fn is_empty(&self) -> bool {
        Self {
            cue_count: 0,
            cues: Vec::new(),
            ..self.clone()
        } == Self::default()
    }

    /// Draws the overlay onto a copy of the fixture.
    pub fn apply_to(&self, fixture: &mut Fixture) {
        match &self.caption {
            Some(CaptionOverride::Set(caption)) => fixture.tv.caption = Some(caption.clone()),
            Some(CaptionOverride::Clear) => fixture.tv.caption = None,
            None => {}
        }
        if let Some(party) = &self.party {
            fixture.tv.party.clone_from(party);
        }
        if let Some(suggested) = &self.suggested {
            fixture.phone.suggested.clone_from(suggested);
        }
        if let Some(groups) = &self.groups {
            fixture.phone.groups.clone_from(groups);
        }
        if let Some(unavailable) = &self.unavailable {
            fixture.phone.unavailable.clone_from(unavailable);
        }
        match &self.music {
            Some(Override::Set(track)) => fixture.music = Some(track.clone()),
            Some(Override::Clear) => fixture.music = None,
            None => {}
        }
        apply_image(&self.backdrop, &mut fixture.tv.backdrop);
        apply_image(&self.card, &mut fixture.tv.card);
        apply_image(&self.phone_image, &mut fixture.phone.image);
    }
}

fn apply_image(change: &Option<Override<ImageRef>>, slot: &mut Option<ImageRef>) {
    match change {
        Some(Override::Set(image)) => *slot = Some(image.clone()),
        Some(Override::Clear) => *slot = None,
        None => {}
    }
}
