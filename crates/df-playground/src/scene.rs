//! Live changes to the scene a fixture describes: the caption, the party strip and the phone's
//! choices, made from the dev CLI while the page is open.
//!
//! A fixture is a file; a change here is not saved to it. The changes are kept as an overlay that
//! is drawn on top of the fixture, so `scene reset` (or choosing another fixture) returns to the
//! file. This module is the one place that decides what a change means and what is too large to
//! accept. It does not enforce the table-experience contract: a label of six words is accepted so
//! the playground can show the break, exactly as a fixture that breaks it is.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::fixture::{Caption, Choice, ChoiceGroup, Fixture, PartyMember, UnavailableChoice};

/// Most characters in a caption line.
pub const MAXIMUM_CAPTION_CHARACTERS: usize = 600;
/// Most characters in a name, a speaker or a label.
pub const MAXIMUM_NAME_CHARACTERS: usize = 80;
/// Most members in the party strip.
pub const MAXIMUM_PARTY_MEMBERS: usize = 8;
/// Most choices in one list (suggested, a group or unavailable).
pub const MAXIMUM_CHOICES_PER_LIST: usize = 24;

/// What the overlay does to the caption.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptionOverride {
    Set(Caption),
    Clear,
}

/// The changes drawn on top of a fixture. A field that is `None` leaves the fixture's own.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SceneOverlay {
    pub caption: Option<CaptionOverride>,
    pub party: Option<Vec<PartyMember>>,
    pub suggested: Option<Vec<Choice>>,
    pub groups: Option<Vec<ChoiceGroup>>,
    pub unavailable: Option<Vec<UnavailableChoice>>,
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
    /// Drops every change and returns to the fixture.
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
            SceneChange::Reset {} => *self = Self::default(),
        }
        Ok(())
    }

    /// True when the overlay changes nothing.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
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
    }
}
