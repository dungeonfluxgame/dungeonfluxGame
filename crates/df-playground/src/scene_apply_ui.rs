//! What the live changes to the narration box, the seat strip and the logo mean.
//!
//! Like the other changes they are made on a copy and checked with the rules a scene file is
//! checked with (`overlay_checks`), so the command line, the overlay and a fixture refuse a value in
//! the same words. Each change starts from what is drawn now: the overlay's own value, or else the
//! fixture's.

use crate::checks;
use crate::fixture::{Fixture, PartyMember};
use crate::overlay::{NarrationBox, PartyLook, SeatStrip};
use crate::overlay_checks::{first_error, narration, seat_strip};
use crate::scene::{Override, SceneChange, SceneError, SceneOverlay};

impl SceneOverlay {
    /// The narration box as drawn: the overlay's, or the fixture's.
    fn narration_now(&self, base: &Fixture) -> Option<NarrationBox> {
        match &self.narration {
            Some(Override::Set(narration)) => Some(narration.clone()),
            Some(Override::Clear) => None,
            None => base.tv.narration.clone(),
        }
    }

    /// The seat strip as drawn: the overlay's, or the fixture's, or an empty one.
    fn seats_now(&self, base: &Fixture) -> SeatStrip {
        self.seats
            .clone()
            .or_else(|| base.tv.seats.clone())
            .unwrap_or_default()
    }

    pub(crate) fn apply_ui(
        &mut self,
        change: SceneChange,
        base: &Fixture,
    ) -> Result<(), SceneError> {
        match change {
            SceneChange::Narration {
                text,
                speaker,
                badge,
                continue_label,
                voice,
                placement,
                progress,
            } => {
                let mut next = match (self.narration_now(base), text.clone()) {
                    (Some(current), _) => current,
                    (None, Some(text)) => NarrationBox {
                        speaker: None,
                        text,
                        continue_label: None,
                        badge: None,
                        voice: true,
                        progress: None,
                        placement: Default::default(),
                    },
                    (None, None) => return Err(SceneError::NoNarrationYet),
                };
                if let Some(text) = text {
                    next.text = text;
                }
                speaker.apply_to(&mut next.speaker);
                badge.apply_to(&mut next.badge);
                continue_label.apply_to(&mut next.continue_label);
                progress.apply_to(&mut next.progress);
                if let Some(voice) = voice {
                    next.voice = voice;
                }
                if let Some(placement) = placement {
                    next.placement = placement;
                }
                first_error(narration(&[], &next))?;
                self.narration = Some(Override::Set(next));
                Ok(())
            }
            SceneChange::ClearNarration {} => {
                self.narration = Some(Override::Clear);
                Ok(())
            }
            SceneChange::SeatSet {
                name,
                class,
                level,
                portrait,
                glyph,
                accent,
                hit_points,
                maximum_hit_points,
                conditions,
            } => {
                checks::labelled("a hero's name", &name)?;
                let mut strip = self.seats_now(base);
                if !strip.looks.iter().any(|look| look.name == name) {
                    strip.looks.push(PartyLook {
                        name: name.clone(),
                        class: None,
                        level: None,
                        portrait: None,
                        glyph: None,
                        accent: None,
                    });
                }
                if let Some(look) = strip.looks.iter_mut().find(|look| look.name == name) {
                    class.apply_to(&mut look.class);
                    level.apply_to(&mut look.level);
                    portrait.apply_to(&mut look.portrait);
                    glyph.apply_to(&mut look.glyph);
                    accent.apply_to(&mut look.accent);
                }
                first_error(seat_strip(&[], &strip))?;
                let mut party = self.party_or(base).to_vec();
                let changes_member =
                    hit_points.is_some() || maximum_hit_points.is_some() || conditions.is_some();
                let position = party.iter().position(|member| member.name == name);
                if position.is_none() || changes_member {
                    let index = match position {
                        Some(index) => index,
                        None => {
                            let hit_points = hit_points
                                .ok_or_else(|| SceneError::NeedsHitPoints(name.clone()))?;
                            party.push(PartyMember {
                                name: name.clone(),
                                hit_points,
                                maximum_hit_points: maximum_hit_points.unwrap_or(hit_points),
                                conditions: Vec::new(),
                                color: None,
                            });
                            party.len() - 1
                        }
                    };
                    let member = &mut party[index];
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
                }
                if party.len() > crate::limits::MAXIMUM_PARTY_MEMBERS {
                    return Err(SceneError::TooManyMembers);
                }
                self.seats = Some(strip);
                self.party = Some(party);
                Ok(())
            }
            SceneChange::SeatRemove { name } => {
                let mut strip = self.seats_now(base);
                let mut party = self.party_or(base).to_vec();
                let (looks, members) = (strip.looks.len(), party.len());
                strip.looks.retain(|look| look.name != name);
                party.retain(|member| member.name != name);
                if strip.looks.len() == looks && party.len() == members {
                    return Err(SceneError::UnknownMember(name));
                }
                self.seats = Some(strip);
                self.party = Some(party);
                Ok(())
            }
            SceneChange::SeatsOpen { open } => {
                let mut strip = self.seats_now(base);
                strip.open = open;
                self.seats = Some(strip);
                Ok(())
            }
            SceneChange::SeatsClear {} => {
                self.seats = Some(SeatStrip::default());
                Ok(())
            }
            SceneChange::Brand { on } => {
                self.brand = Some(on);
                Ok(())
            }
            // The dispatcher in `scene_apply` sends only the changes above.
            _ => Err(SceneError::Empty("a change to the overlay components")),
        }
    }
}
