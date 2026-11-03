//! What each live change to the scene means: the one place that turns a `SceneChange` into a new
//! overlay, or refuses it with the reason.
//!
//! A change is all or nothing: it is made on a copy, and a refusal leaves the overlay as it was.
//! The range checks are in `checks`, shared with the scenario files. Each kind of change has its
//! own small method here.

use crate::checks;
use crate::fixture::{Camera, UnavailableChoice};
use crate::fixture::{Caption, Choice, ChoiceGroup, Fixture, ImageRef, PartyMember, Slide};
use crate::limits::{MAXIMUM_CAPTION_CHARACTERS, MAXIMUM_CUES_KEPT, MAXIMUM_PARTY_MEMBERS};
use crate::scene::{
    CaptionOverride, Cue, ImageSlot, Override, SceneChange, SceneError, SceneOverlay, Target, Track,
};

impl SceneOverlay {
    /// The party as drawn: the overlay's, or the fixture's.
    pub(crate) fn party_or<'a>(&'a self, base: &'a Fixture) -> &'a [PartyMember] {
        self.party.as_deref().unwrap_or(&base.tv.party)
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
                self.caption = Some(CaptionOverride::Clear);
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
                self.party = Some(Vec::new());
                Ok(())
            }
            SceneChange::Suggested { choices } => self.set_suggested(choices),
            SceneChange::Group { name, choices } => self.set_group(name, choices, base),
            SceneChange::Unavailable { choices } => self.set_unavailable(choices),
            SceneChange::ClearChoices {} => {
                self.suggested = Some(Vec::new());
                self.groups = Some(Vec::new());
                self.unavailable = Some(Vec::new());
                Ok(())
            }
            SceneChange::Image {
                slot,
                media,
                caption,
                rotation_degrees,
                slide,
            } => self.set_image(slot, media, caption, rotation_degrees, slide, now_ms),
            SceneChange::ClearImage { slot } => {
                *self.image_slot_mut(slot) = Some(Override::Clear);
                Ok(())
            }
            SceneChange::Music {
                media,
                target,
                volume,
                looping,
            } => self.start_music(media, target, volume, looping),
            SceneChange::StopMusic {} => {
                self.music = Some(Override::Clear);
                Ok(())
            }
            SceneChange::Sound {
                media,
                target,
                volume,
            } => self.add_cue(media, target, volume),
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
            SceneChange::Screen { edit } => self.screens.apply(edit, base),
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
        self.caption = Some(CaptionOverride::Set(Caption { speaker, text }));
        Ok(())
    }

    fn add_member(&mut self, member: PartyMember, base: &Fixture) -> Result<(), SceneError> {
        checks::member(&member)?;
        let mut party = self.party_or(base).to_vec();
        if party.iter().any(|existing| existing.name == member.name) {
            return Err(SceneError::DuplicateMember(member.name));
        }
        if party.len() >= MAXIMUM_PARTY_MEMBERS {
            return Err(SceneError::TooManyMembers);
        }
        party.push(member);
        self.party = Some(party);
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
        checks::member(member)?;
        self.party = Some(party);
        Ok(())
    }

    fn remove_member(&mut self, name: String, base: &Fixture) -> Result<(), SceneError> {
        let mut party = self.party_or(base).to_vec();
        let before = party.len();
        party.retain(|member| member.name != name);
        if party.len() == before {
            return Err(SceneError::UnknownMember(name));
        }
        self.party = Some(party);
        Ok(())
    }

    fn set_suggested(&mut self, choices: Vec<Choice>) -> Result<(), SceneError> {
        checks::count(choices.len())?;
        choices.iter().try_for_each(checks::choice)?;
        self.suggested = Some(choices);
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
        let mut groups = self
            .groups
            .clone()
            .unwrap_or_else(|| base.phone.groups.clone());
        groups.retain(|group| group.name != name);
        if !choices.is_empty() {
            groups.push(ChoiceGroup { name, choices });
        }
        self.groups = Some(groups);
        Ok(())
    }

    fn set_unavailable(&mut self, choices: Vec<UnavailableChoice>) -> Result<(), SceneError> {
        checks::count(choices.len())?;
        for choice in &choices {
            checks::labelled("a choice label", &choice.label)?;
            checks::labelled("a reason", &choice.reason)?;
        }
        self.unavailable = Some(choices);
        Ok(())
    }

    fn set_image(
        &mut self,
        slot: ImageSlot,
        media: String,
        caption: Option<String>,
        rotation_degrees: u16,
        slide: Option<Slide>,
        now_ms: u64,
    ) -> Result<(), SceneError> {
        checks::labelled("a media name", &media)?;
        checks::rotation(rotation_degrees)?;
        if let Some(caption) = &caption {
            checks::labelled("a picture caption", caption)?;
        }
        let slide = match slide {
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
        *self.image_slot_mut(slot) = Some(Override::Set(ImageRef {
            media,
            caption,
            rotation_degrees,
            slide,
        }));
        Ok(())
    }

    fn start_music(
        &mut self,
        media: String,
        target: Target,
        volume: u8,
        looping: bool,
    ) -> Result<(), SceneError> {
        checks::labelled("a media name", &media)?;
        checks::volume(volume)?;
        self.music = Some(Override::Set(Track {
            media,
            target,
            volume,
            looping,
        }));
        Ok(())
    }

    fn add_cue(&mut self, media: String, target: Target, volume: u8) -> Result<(), SceneError> {
        checks::labelled("a media name", &media)?;
        checks::volume(volume)?;
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
        Ok(())
    }

    fn move_camera(
        &mut self,
        target: Option<String>,
        zoom_percent: u16,
        duration_ms: u32,
        now_ms: u64,
    ) -> Result<(), SceneError> {
        checks::camera(target.as_deref(), zoom_percent, duration_ms)?;
        self.camera_count += 1;
        self.camera = Some(Camera {
            target,
            zoom_percent,
            duration_ms,
            serial: self.camera_count,
            began_ms: now_ms,
            elapsed_ms: 0,
        });
        Ok(())
    }

    fn image_slot_mut(&mut self, slot: ImageSlot) -> &mut Option<Override<ImageRef>> {
        match slot {
            ImageSlot::TvBackdrop => &mut self.backdrop,
            ImageSlot::TvCard => &mut self.card,
            ImageSlot::PhoneImage => &mut self.phone_image,
        }
    }
}
