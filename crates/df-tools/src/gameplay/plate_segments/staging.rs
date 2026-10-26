//! Staging facts for the storyteller and the NPC replies: where each person stands on the plate
//! on screen, how near they are to the camera, which way they face, and who they face or stand
//! next to. Neutral facts, phrased from closed values (a column, a depth band, a facing label),
//! so the narration and the replies agree with what the table sees.
//!
//! Positions are the person's head (or, with no head found, the loader anchor). Depth is read
//! from the head's size: a bigger head is nearer the camera.

use df_providers::HeadBox;

use super::{
    facing::Facing,
    geometry::{SegmentedPerson, box_height, centre},
};
use crate::gameplay::shot::Side;

/// A head this tall (a share of the plate's height) or taller is in the foreground; under
/// [`BACKGROUND_HEAD_HEIGHT`] it is in the background. Measured on a shop plate: two heroes seen
/// from behind 0.20 to 0.25, the cast across the room 0.07 to 0.12.
const FOREGROUND_HEAD_HEIGHT: f32 = 0.15;
const BACKGROUND_HEAD_HEIGHT: f32 = 0.06;

/// Who a person on the plate is.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum Who {
    /// A hero, the party member at this index (counted from 0).
    Hero(usize),
    /// A cast member, by the exact current actor id.
    Cast(String),
    /// A person nobody was matched to.
    Unknown,
}

/// A segmented person with the identity and facing the judges gave.
#[derive(Clone, Copy, Debug)]
pub(in crate::gameplay) struct AssignedPerson<'a> {
    pub(in crate::gameplay) who: &'a Who,
    pub(in crate::gameplay) person: &'a SegmentedPerson,
    pub(in crate::gameplay) facing: Option<Facing>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum Column {
    Left,
    Centre,
    Right,
}

impl Column {
    pub(in crate::gameplay) const fn label(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Centre => "centre",
            Self::Right => "right",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum Depth {
    Foreground,
    Midground,
    Background,
}

impl Depth {
    pub(in crate::gameplay) const fn label(self) -> &'static str {
        match self {
            Self::Foreground => "foreground",
            Self::Midground => "midground",
            Self::Background => "background",
        }
    }
}

/// What a person's facing points at.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum FacingTarget {
    /// The nearest person on the side they face (an index into [`PlateStaging::people`]).
    Person(usize),
    Camera,
    AwayFromCamera,
    /// They face a side with nobody on it.
    OpenSpace(Side),
    Unclear,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct StagedPerson {
    pub(in crate::gameplay) who: Who,
    /// The candidate number on the plate (for logs).
    pub(in crate::gameplay) candidate: usize,
    pub(in crate::gameplay) column: Column,
    /// `None` when no head was found to measure.
    pub(in crate::gameplay) depth: Option<Depth>,
    /// 0 for the person nearest the camera; `None` with no head.
    pub(in crate::gameplay) depth_rank: Option<usize>,
    pub(in crate::gameplay) facing: Option<Facing>,
    pub(in crate::gameplay) faces: FacingTarget,
    /// The nearest other person on the plate and the screen side they stand on.
    pub(in crate::gameplay) nearest: Option<(usize, Side)>,
}

/// Everyone staged on one plate, in left-to-right order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct PlateStaging {
    pub(in crate::gameplay) people: Vec<StagedPerson>,
}

fn position(person: &SegmentedPerson) -> (f32, f32) {
    person
        .head
        .as_ref()
        .map_or((person.anchor.x, person.anchor.y), centre)
}

fn head_height(person: &SegmentedPerson) -> Option<f32> {
    person.head.as_ref().map(|head: &HeadBox| box_height(head))
}

fn column_of(x: f32) -> Column {
    if x < 1.0 / 3.0 {
        Column::Left
    } else if x > 2.0 / 3.0 {
        Column::Right
    } else {
        Column::Centre
    }
}

fn depth_of(height: f32) -> Depth {
    if height >= FOREGROUND_HEAD_HEIGHT {
        Depth::Foreground
    } else if height >= BACKGROUND_HEAD_HEIGHT {
        Depth::Midground
    } else {
        Depth::Background
    }
}

/// Stages `people` on a plate of `aspect` (width over height). Distances are measured in equal
/// units across and down.
pub(in crate::gameplay) fn stage(people: &[AssignedPerson<'_>], aspect: f32) -> PlateStaging {
    let mut order: Vec<usize> = (0..people.len()).collect();
    order.sort_by(|a, b| {
        position(people[*a].person)
            .0
            .total_cmp(&position(people[*b].person).0)
    });
    let people: Vec<AssignedPerson<'_>> = order.iter().map(|index| people[*index]).collect();

    let distance = |a: usize, b: usize| {
        let ((ax, ay), (bx, by)) = (position(people[a].person), position(people[b].person));
        ((ax - bx) * aspect).hypot(ay - by)
    };
    let nearest_among = |from: usize, only: &dyn Fn(usize) -> bool| {
        (0..people.len())
            .filter(|other| *other != from && only(*other))
            .min_by(|a, b| distance(from, *a).total_cmp(&distance(from, *b)))
    };

    let mut by_size: Vec<usize> = (0..people.len())
        .filter(|index| head_height(people[*index].person).is_some())
        .collect();
    by_size.sort_by(|a, b| {
        head_height(people[*b].person)
            .unwrap_or(0.0)
            .total_cmp(&head_height(people[*a].person).unwrap_or(0.0))
    });

    let staged = (0..people.len())
        .map(|index| {
            let assigned = &people[index];
            let (x, _) = position(assigned.person);
            let side_of = |other: usize| {
                if position(people[other].person).0 < x {
                    Side::Left
                } else {
                    Side::Right
                }
            };
            let faces = match assigned.facing {
                Some(Facing::TowardCamera) => FacingTarget::Camera,
                Some(Facing::AwayFromCamera) => FacingTarget::AwayFromCamera,
                None => FacingTarget::Unclear,
                Some(side @ (Facing::Left | Facing::Right)) => {
                    let wanted = if side == Facing::Left {
                        Side::Left
                    } else {
                        Side::Right
                    };
                    nearest_among(index, &|other| side_of(other) == wanted)
                        .map_or(FacingTarget::OpenSpace(wanted), FacingTarget::Person)
                }
            };
            StagedPerson {
                who: assigned.who.clone(),
                candidate: assigned.person.candidate,
                column: column_of(x),
                depth: head_height(assigned.person).map(depth_of),
                depth_rank: by_size.iter().position(|other| *other == index),
                facing: assigned.facing,
                faces,
                nearest: nearest_among(index, &|_| true).map(|other| (other, side_of(other))),
            }
        })
        .collect();
    PlateStaging { people: staged }
}

fn side_name(side: Side) -> &'static str {
    match side {
        Side::Left => "left",
        Side::Right => "right",
    }
}

impl PlateStaging {
    /// The staged person for `who`, when they are on the plate.
    pub(in crate::gameplay) fn of(&self, who: &Who) -> Option<&StagedPerson> {
        (*who != Who::Unknown)
            .then(|| self.people.iter().find(|staged| staged.who == *who))
            .flatten()
    }

    /// How a sentence names the person at `index`: by name, or for someone nobody was matched
    /// to, by where they stand (a name is never guessed).
    fn label(&self, index: usize, name_of: &dyn Fn(&Who) -> String) -> String {
        let staged = &self.people[index];
        match staged.who {
            Who::Unknown => format!(
                "the unidentified person at the {} of the frame",
                staged.column.label()
            ),
            _ => name_of(&staged.who),
        }
    }

    /// How a sentence opens on the person at `index`: [`PlateStaging::label`], or for an
    /// unidentified person, which the sentence places itself, `An unidentified person`.
    fn subject(&self, index: usize, name_of: &dyn Fn(&Who) -> String) -> String {
        match self.people[index].who {
            Who::Unknown => "An unidentified person".to_owned(),
            _ => self.label(index, name_of),
        }
    }

    /// One neutral sentence about the person at `index` in [`PlateStaging::people`].
    pub(in crate::gameplay) fn fact(
        &self,
        index: usize,
        name_of: &dyn Fn(&Who) -> String,
    ) -> Option<String> {
        let staged = self.people.get(index)?;
        let mut sentence = format!(
            "{} is at the {} of the frame",
            self.subject(index, name_of),
            staged.column.label()
        );
        if let Some(depth) = staged.depth {
            sentence.push_str(&format!(", in the {}", depth.label()));
        }
        let nearest_is_faced = matches!(
            (staged.faces, staged.nearest),
            (FacingTarget::Person(faced), Some((nearest, _))) if faced == nearest
        );
        match staged.faces {
            FacingTarget::Person(faced) => {
                sentence.push_str(&format!(", facing {}", self.label(faced, name_of)));
                if nearest_is_faced {
                    sentence.push_str(" (the nearest person)");
                }
            }
            FacingTarget::Camera => sentence.push_str(", facing the camera"),
            FacingTarget::AwayFromCamera => sentence.push_str(", back to the camera"),
            FacingTarget::OpenSpace(side) => {
                sentence.push_str(&format!(", facing {} toward open space", side_name(side)));
            }
            FacingTarget::Unclear => {}
        }
        if let (Some((nearest, side)), false) = (staged.nearest, nearest_is_faced) {
            sentence.push_str(&format!(
                "; the nearest person is {}, to the screen {}",
                self.label(nearest, name_of),
                side_name(side)
            ));
        }
        sentence.push('.');
        Some(sentence)
    }

    /// One sentence per person, left to right across the plate.
    pub(in crate::gameplay) fn facts(&self, name_of: &dyn Fn(&Who) -> String) -> Vec<String> {
        (0..self.people.len())
            .filter_map(|index| self.fact(index, name_of))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::geometry::{people_from_segments, tests::instance};
    use super::*;

    /// Three people on a 200 by 100 plate: a tall-headed one on the left (near), a mid-sized
    /// one in the centre and a small one on the right (far).
    fn plate() -> Vec<SegmentedPerson> {
        people_from_segments(
            &[
                instance(0.95, 10, 10, 50, 100),
                instance(0.95, 80, 30, 120, 100),
                instance(0.95, 150, 40, 180, 100),
            ],
            &[
                instance(0.9, 15, 10, 45, 30),
                instance(0.9, 90, 30, 110, 40),
                instance(0.9, 158, 40, 172, 45),
            ],
        )
    }

    fn name(who: &Who) -> String {
        match who {
            Who::Hero(index) => format!("Hero {}", index + 1),
            Who::Cast(id) => id.clone(),
            Who::Unknown => "someone".to_owned(),
        }
    }

    fn assigned<'a>(
        whos: &'a [Who],
        people: &'a [SegmentedPerson],
        facings: &[Option<Facing>],
    ) -> Vec<AssignedPerson<'a>> {
        people
            .iter()
            .enumerate()
            .map(|(index, person)| AssignedPerson {
                who: &whos[index],
                person,
                facing: facings[index],
            })
            .collect()
    }

    #[test]
    fn people_are_staged_left_to_right_with_a_column_a_depth_and_a_rank() {
        let people = plate();
        let whos = [
            Who::Hero(0),
            Who::Cast("marek".to_owned()),
            Who::Cast("sigrun".to_owned()),
        ];
        let staging = stage(&assigned(&whos, &people, &[None, None, None]), 2.0);
        let columns: Vec<_> = staging.people.iter().map(|staged| staged.column).collect();
        assert_eq!(columns, [Column::Left, Column::Centre, Column::Right]);
        let depths: Vec<_> = staging.people.iter().map(|staged| staged.depth).collect();
        assert_eq!(
            depths,
            [
                Some(Depth::Foreground),
                Some(Depth::Midground),
                Some(Depth::Background)
            ]
        );
        let ranks: Vec<_> = staging
            .people
            .iter()
            .map(|staged| staged.depth_rank)
            .collect();
        assert_eq!(ranks, [Some(0), Some(1), Some(2)]);
    }

    #[test]
    fn a_person_facing_a_side_faces_the_nearest_person_on_that_side() {
        let people = plate();
        let whos = [
            Who::Hero(0),
            Who::Cast("marek".to_owned()),
            Who::Cast("sigrun".to_owned()),
        ];
        let staging = stage(
            &assigned(
                &whos,
                &people,
                &[
                    Some(Facing::Right),
                    Some(Facing::Right),
                    Some(Facing::Right),
                ],
            ),
            2.0,
        );
        assert_eq!(staging.people[0].faces, FacingTarget::Person(1));
        assert_eq!(staging.people[1].faces, FacingTarget::Person(2));
        // Nobody stands to the right of the rightmost person.
        assert_eq!(
            staging.people[2].faces,
            FacingTarget::OpenSpace(Side::Right)
        );
    }

    #[test]
    fn camera_away_and_unclear_facings_carry_over_and_the_nearest_person_is_named() {
        let people = plate();
        let whos = [Who::Hero(0), Who::Cast("marek".to_owned()), Who::Unknown];
        let staging = stage(
            &assigned(
                &whos,
                &people,
                &[
                    Some(Facing::AwayFromCamera),
                    Some(Facing::TowardCamera),
                    None,
                ],
            ),
            2.0,
        );
        assert_eq!(staging.people[0].faces, FacingTarget::AwayFromCamera);
        assert_eq!(staging.people[1].faces, FacingTarget::Camera);
        assert_eq!(staging.people[2].faces, FacingTarget::Unclear);
        assert_eq!(staging.people[0].nearest, Some((1, Side::Right)));
        assert_eq!(staging.people[2].nearest, Some((1, Side::Left)));
    }

    #[test]
    fn the_facts_read_as_neutral_sentences_the_storyteller_can_use() {
        let people = plate();
        let whos = [
            Who::Hero(0),
            Who::Cast("Marek".to_owned()),
            Who::Cast("Sigrun".to_owned()),
        ];
        let staging = stage(
            &assigned(
                &whos,
                &people,
                &[
                    Some(Facing::AwayFromCamera),
                    Some(Facing::Right),
                    Some(Facing::Left),
                ],
            ),
            2.0,
        );
        let facts = staging.facts(&name);
        assert_eq!(
            facts[0],
            "Hero 1 is at the left of the frame, in the foreground, back to the camera; \
             the nearest person is Marek, to the screen right."
        );
        assert_eq!(
            facts[1],
            "Marek is at the centre of the frame, in the midground, facing Sigrun (the nearest person)."
        );
        assert_eq!(
            facts[2],
            "Sigrun is at the right of the frame, in the background, facing Marek (the nearest person)."
        );
    }

    #[test]
    fn an_unidentified_person_is_described_by_where_they_stand_and_never_by_a_guessed_name() {
        let people = plate();
        let whos = [Who::Unknown, Who::Unknown, Who::Unknown];
        let staging = stage(&assigned(&whos, &people, &[None, None, None]), 2.0);
        assert_eq!(
            staging.facts(&name)[2],
            "An unidentified person is at the right of the frame, in the background; the nearest person is the unidentified person at the centre of the frame, to the screen left."
        );
        assert!(staging.of(&Who::Unknown).is_none());
    }

    #[test]
    fn a_person_with_no_head_has_a_position_but_no_depth() {
        let people = people_from_segments(
            &[
                instance(0.95, 10, 10, 50, 100),
                instance(0.95, 80, 30, 120, 100),
            ],
            &[instance(0.9, 90, 30, 110, 40)],
        );
        let whos = [Who::Hero(0), Who::Cast("marek".to_owned())];
        let staging = stage(&assigned(&whos, &people, &[None, None]), 2.0);
        assert_eq!(staging.people[0].depth, None);
        assert_eq!(staging.people[0].depth_rank, None);
        assert_eq!(staging.people[1].depth_rank, Some(0));
        assert!(staging.of(&Who::Hero(0)).is_some());
    }
}
