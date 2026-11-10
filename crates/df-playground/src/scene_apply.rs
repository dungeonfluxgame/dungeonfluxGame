//! What each live change to the scene means: the one place that turns a `SceneChange` into a new
//! overlay, or refuses it with the reason.
//!
//! A change is all or nothing: it is made on a copy, and a refusal leaves the overlay as it was.
//! The range checks are in `checks`, shared with the scenario files. Each kind of change has its
//! own small method here.

use crate::checks;
use crate::clues::{ClueBoard, ClueNote};
use crate::fixture::{Camera, UnavailableChoice};
use crate::fixture::{Caption, Choice, ChoiceGroup, Fixture, ImageRef, PartyMember};
use crate::limits::{MAXIMUM_CAPTION_CHARACTERS, MAXIMUM_CUES_KEPT, MAXIMUM_PARTY_MEMBERS};
use crate::overlay::Edit;
use crate::scene::{Cue, ImageSlot, SceneChange, SceneError, SceneOverlay, Target, Track};

mod ui;

impl SceneOverlay {
    /// The party as drawn: the overlay's, or the fixture's.
    pub(crate) fn party_now(&self, base: &Fixture) -> Vec<PartyMember> {
        let mut party = base.tv.party.clone();
        self.party.apply_to_value(&mut party);
        party
    }

    /// Makes one change. `base` is the fixture the overlay sits on, so a change to one member or
    /// one group starts from what is drawn now. A refused change leaves the overlay as it was.
    /// Moves made this way begin at the host's clock reading of 0; the host uses `apply_at`.
    pub fn apply(&mut self, change: SceneChange, base: &Fixture) -> Result<(), SceneError> {
        self.apply_at(change, base, 0)
    }

    /// `apply` for a host whose clock reads `now_ms` (milliseconds since it started): a slide or a
    /// camera move made now is stamped with it, so a later reader can say how long ago it began.
    pub fn apply_at(
        &mut self,
        change: SceneChange,
        base: &Fixture,
        now_ms: u64,
    ) -> Result<(), SceneError> {
        let mut next = self.clone();
        next.apply_unchecked(change, base, now_ms)?;
        *self = next;
        Ok(())
    }

    fn apply_unchecked(
        &mut self,
        change: SceneChange,
        base: &Fixture,
        now_ms: u64,
    ) -> Result<(), SceneError> {
        match change {
            SceneChange::Caption { speaker, text } => self.set_caption(speaker, text),
            SceneChange::ClearCaption {} => {
                self.caption = Edit::Clear;
                Ok(())
            }
            SceneChange::PartyAdd { member } => self.add_member(member, base),
            SceneChange::PartySet {
                name,
                hit_points,
                maximum_hit_points,
                conditions,
            } => self.set_member(name, hit_points, maximum_hit_points, conditions, base),
            SceneChange::PartyRemove { name } => self.remove_member(name, base),
            SceneChange::PartyClear {} => {
                self.party = Edit::Clear;
                Ok(())
            }
            SceneChange::Suggested { choices } => self.set_suggested(choices),
            SceneChange::Group { name, choices } => self.set_group(name, choices, base),
            SceneChange::Unavailable { choices } => self.set_unavailable(choices),
            SceneChange::ClearChoices {} => {
                self.suggested = Edit::Clear;
                self.groups = Edit::Clear;
                self.unavailable = Edit::Clear;
                Ok(())
            }
            SceneChange::Image {
                slot,
                media,
                caption,
                rotation_degrees,
                slide,
                layout,
                plate,
            } => self.set_image(
                slot,
                ImageRef {
                    media,
                    caption,
                    rotation_degrees,
                    slide,
                    layout,
                    plate,
                },
                now_ms,
            ),
            SceneChange::ClearImage { slot } => {
                *self.image_slot_mut(slot) = Edit::Clear;
                Ok(())
            }
            SceneChange::Music {
                media,
                target,
                volume_percent,
                looping,
            } => self.start_music(media, target, volume_percent, looping),
            SceneChange::StopMusic {} => {
                self.music = Edit::Clear;
                Ok(())
            }
            SceneChange::Clues { notes } => self.set_clues(notes),
            SceneChange::ClearClues {} => {
                self.clues = Edit::Clear;
                Ok(())
            }
            SceneChange::Story { mark } => {
                mark.check()?;
                self.story = Edit::Set(mark);
                Ok(())
            }
            SceneChange::ClearStory {} => {
                self.story = Edit::Clear;
                Ok(())
            }
            SceneChange::Sound {
                media,
                target,
                volume_percent,
            } => self.add_cue(media, target, volume_percent),
            SceneChange::Camera {
                target,
                zoom_percent,
                duration_ms,
            } => self.move_camera(target, zoom_percent, duration_ms, now_ms),
            change @ (SceneChange::Narration { .. }
            | SceneChange::ClearNarration {}
            | SceneChange::SeatSet { .. }
            | SceneChange::SeatRemove { .. }
            | SceneChange::SeatsOpen { .. }
            | SceneChange::SeatsClear {}
            | SceneChange::Brand { .. }) => self.apply_ui(change, base),
            SceneChange::Screen { edit } => self.apply_screen(edit, base),
            SceneChange::Home { edit } => self.apply_home(edit, base),
            SceneChange::Reset {} => {
                *self = self.cleared();
                Ok(())
            }
        }
    }

    fn set_caption(&mut self, speaker: Option<String>, text: String) -> Result<(), SceneError> {
        checks::text("the caption", &text, MAXIMUM_CAPTION_CHARACTERS)?;
        if let Some(speaker) = &speaker {
            checks::labelled("the speaker", speaker)?;
        }
        self.caption = Edit::Set(Caption { speaker, text });
        Ok(())
    }

    fn add_member(&mut self, member: PartyMember, base: &Fixture) -> Result<(), SceneError> {
        checks::member(&member)?;
        let mut party = self.party_now(base);
        if party.iter().any(|existing| existing.name == member.name) {
            return Err(SceneError::DuplicateMember(member.name));
        }
        if party.len() >= MAXIMUM_PARTY_MEMBERS {
            return Err(SceneError::TooManyMembers);
        }
        party.push(member);
        self.party = Edit::Set(party);
        Ok(())
    }

    fn set_member(
        &mut self,
        name: String,
        hit_points: Option<u32>,
        maximum_hit_points: Option<u32>,
        conditions: Option<Vec<String>>,
        base: &Fixture,
    ) -> Result<(), SceneError> {
        let mut party = self.party_now(base);
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
        checks::member(member)?;
        self.party = Edit::Set(party);
        Ok(())
    }

    fn remove_member(&mut self, name: String, base: &Fixture) -> Result<(), SceneError> {
        let mut party = self.party_now(base);
        let before = party.len();
        party.retain(|member| member.name != name);
        if party.len() == before {
            return Err(SceneError::UnknownMember(name));
        }
        self.party = Edit::Set(party);
        Ok(())
    }

    fn set_suggested(&mut self, choices: Vec<Choice>) -> Result<(), SceneError> {
        checks::count(choices.len())?;
        choices.iter().try_for_each(checks::choice)?;
        self.suggested = Edit::Set(choices);
        Ok(())
    }

    fn set_group(
        &mut self,
        name: String,
        choices: Vec<Choice>,
        base: &Fixture,
    ) -> Result<(), SceneError> {
        checks::labelled("a group name", &name)?;
        checks::count(choices.len())?;
        choices.iter().try_for_each(checks::choice)?;
        let mut groups = base.phone.groups.clone();
        self.groups.apply_to_value(&mut groups);
        groups.retain(|group| group.name != name);
        if !choices.is_empty() {
            groups.push(ChoiceGroup { name, choices });
        }
        self.groups = Edit::Set(groups);
        Ok(())
    }

    fn set_unavailable(&mut self, choices: Vec<UnavailableChoice>) -> Result<(), SceneError> {
        checks::count(choices.len())?;
        for choice in &choices {
            checks::labelled("a choice label", &choice.label)?;
            checks::labelled("a reason", &choice.reason)?;
        }
        self.unavailable = Edit::Set(choices);
        Ok(())
    }

    /// Checks `image` for `slot` and shows it there; a slide starts now (`now_ms`).
    fn set_image(
        &mut self,
        slot: ImageSlot,
        image: ImageRef,
        now_ms: u64,
    ) -> Result<(), SceneError> {
        checks::labelled("a media name", &image.media)?;
        checks::rotation(image.rotation_degrees)?;
        if let Some(caption) = &image.caption {
            checks::labelled("a picture caption", caption)?;
        }
        let slide = match image.slide {
            None => None,
            Some(mut slide) => {
                if slot != ImageSlot::TvCard {
                    return Err(SceneError::SlideNeedsTheCard);
                }
                checks::slide(&slide)?;
                self.slide_count += 1;
                slide.serial = self.slide_count;
                slide.began_ms = now_ms;
                slide.elapsed_ms = 0;
                Some(slide)
            }
        };
        *self.image_slot_mut(slot) = Edit::Set(ImageRef { slide, ..image });
        Ok(())
    }

    fn start_music(
        &mut self,
        media: String,
        target: Target,
        volume_percent: u8,
        looping: bool,
    ) -> Result<(), SceneError> {
        checks::labelled("a media name", &media)?;
        checks::volume(volume_percent)?;
        self.music = Edit::Set(Track {
            media,
            target,
            volume_percent,
            looping,
        });
        Ok(())
    }

    fn set_clues(&mut self, notes: Vec<ClueNote>) -> Result<(), SceneError> {
        let board = ClueBoard { notes };
        board.check()?;
        self.clues = Edit::Set(board);
        Ok(())
    }

    fn add_cue(
        &mut self,
        media: String,
        target: Target,
        volume_percent: u8,
    ) -> Result<(), SceneError> {
        checks::labelled("a media name", &media)?;
        checks::volume(volume_percent)?;
        self.cue_count += 1;
        self.cues.push(Cue {
            id: self.cue_count,
            target,
            media,
            volume_percent,
        });
        if self.cues.len() > MAXIMUM_CUES_KEPT {
            self.cues.remove(0);
        }
        Ok(())
    }

    fn move_camera(
        &mut self,
        target: Option<String>,
        zoom_percent: u16,
        duration_ms: u64,
        now_ms: u64,
    ) -> Result<(), SceneError> {
        checks::camera(target.as_deref(), zoom_percent, duration_ms)?;
        self.camera_count += 1;
        self.camera = Edit::Set(Camera {
            target,
            zoom_percent,
            duration_ms,
            serial: self.camera_count,
            began_ms: now_ms,
            elapsed_ms: 0,
        });
        Ok(())
    }

    fn image_slot_mut(&mut self, slot: ImageSlot) -> &mut Edit<ImageRef> {
        match slot {
            ImageSlot::TvBackdrop => &mut self.backdrop,
            ImageSlot::TvCard => &mut self.card,
            ImageSlot::PhoneImage => &mut self.phone_image,
        }
    }
}
