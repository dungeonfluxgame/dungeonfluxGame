//! Where the loader and the speech bubble stand: the side of the head, and the wire boxes.
//!
//! The loader is the animation shown before a speech bubble (a hero's typing or talking loader,
//! an NPC's reply-pending loader). It anchors just above the person's crown, and the bubble that
//! follows stands on the side of the head that has open space, preferring the side the person
//! faces, and never over another person's head. All geometry on fractions of the plate.

use df_providers::HeadBox;

use super::{
    facing::Facing,
    geometry::{SegmentedPerson, box_height, box_width, centre},
};
use crate::gameplay::shot::Side;

/// The space a bubble needs beside a head, as fractions of the plate.
const BUBBLE_WIDTH: f32 = 0.20;
const BUBBLE_HEIGHT: f32 = 0.14;
/// The gap kept between the head and the bubble, and around other heads.
const GAP: f32 = 0.01;

/// Why the bubble went to the side it did (for the audit line).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum SideReason {
    /// Both sides were clear and the person faces one of them.
    FacesClearSide,
    /// Both sides were clear; the one with more open space, the middle of the frame on a tie.
    MoreOpenSpace,
    /// Only one side was clear.
    OnlyClearSide,
    /// Neither side was clear; the one with more open space. The display must find room above
    /// or below.
    NoClearSide,
}

impl SideReason {
    pub(in crate::gameplay) const fn label(self) -> &'static str {
        match self {
            Self::FacesClearSide => "faces_clear_side",
            Self::MoreOpenSpace => "more_open_space",
            Self::OnlyClearSide => "only_clear_side",
            Self::NoClearSide => "no_clear_side",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct SideChoice {
    pub(in crate::gameplay) side: Side,
    pub(in crate::gameplay) clear: bool,
    pub(in crate::gameplay) reason: SideReason,
}

/// The rectangle a bubble would fill beside `head` on `side`.
fn bubble_zone(head: &HeadBox, side: Side) -> HeadBox {
    let middle = centre(head).1;
    let (left, right) = match side {
        Side::Left => (head.left - GAP - BUBBLE_WIDTH, head.left - GAP),
        Side::Right => (head.right + GAP, head.right + GAP + BUBBLE_WIDTH),
    };
    HeadBox {
        left,
        top: middle - BUBBLE_HEIGHT / 2.0,
        right,
        bottom: middle + BUBBLE_HEIGHT / 2.0,
    }
}

fn touches(zone: &HeadBox, other: &HeadBox) -> bool {
    zone.left < other.right + GAP
        && other.left - GAP < zone.right
        && zone.top < other.bottom + GAP
        && other.top - GAP < zone.bottom
}

fn side_is_clear(head: &HeadBox, side: Side, others: &[HeadBox]) -> bool {
    let zone = bubble_zone(head, side);
    zone.left >= 0.0 && zone.right <= 1.0 && !others.iter().any(|other| touches(&zone, other))
}

/// How far the open space beside the head reaches before another head or the plate's edge.
fn open_space(head: &HeadBox, side: Side, others: &[HeadBox]) -> f32 {
    let band = bubble_zone(head, side);
    let edge = match side {
        Side::Left => head.left,
        Side::Right => 1.0 - head.right,
    };
    others
        .iter()
        .filter(|other| other.top < band.bottom && band.top < other.bottom)
        .filter_map(|other| match side {
            Side::Left => (other.right <= head.left).then_some(head.left - other.right),
            Side::Right => (other.left >= head.right).then_some(other.left - head.right),
        })
        .fold(edge, f32::min)
}

fn faced_side(facing: Option<Facing>) -> Option<Side> {
    match facing {
        Some(Facing::Left) => Some(Side::Left),
        Some(Facing::Right) => Some(Side::Right),
        Some(Facing::TowardCamera | Facing::AwayFromCamera) | None => None,
    }
}

/// The side of `head` the bubble stands on. `others` are the other people's heads. A side is
/// clear when the bubble fits beside the head inside the plate and touches no other head.
/// Among clear sides the one the person faces wins; otherwise the one with more open space; on
/// a tie, the side toward the middle of the frame (the rule the plates used before).
pub(in crate::gameplay) fn bubble_side(
    head: &HeadBox,
    facing: Option<Facing>,
    others: &[HeadBox],
) -> SideChoice {
    let toward_middle = if centre(head).0 >= 0.5 {
        Side::Left
    } else {
        Side::Right
    };
    let (left_clear, right_clear) = (
        side_is_clear(head, Side::Left, others),
        side_is_clear(head, Side::Right, others),
    );
    let by_space = || {
        let (left, right) = (
            open_space(head, Side::Left, others),
            open_space(head, Side::Right, others),
        );
        if (left - right).abs() < 1e-4 {
            toward_middle
        } else if left > right {
            Side::Left
        } else {
            Side::Right
        }
    };
    match (left_clear, right_clear) {
        (true, true) => match faced_side(facing) {
            Some(side) => SideChoice {
                side,
                clear: true,
                reason: SideReason::FacesClearSide,
            },
            None => SideChoice {
                side: by_space(),
                clear: true,
                reason: SideReason::MoreOpenSpace,
            },
        },
        (true, false) => SideChoice {
            side: Side::Left,
            clear: true,
            reason: SideReason::OnlyClearSide,
        },
        (false, true) => SideChoice {
            side: Side::Right,
            clear: true,
            reason: SideReason::OnlyClearSide,
        },
        (false, false) => SideChoice {
            side: by_space(),
            clear: false,
            reason: SideReason::NoClearSide,
        },
    }
}

/// A box on the wire: centre and size in permille of the plate (0 to 1000).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct PermilleBox {
    pub(in crate::gameplay) x: u16,
    pub(in crate::gameplay) y: u16,
    pub(in crate::gameplay) width: u16,
    pub(in crate::gameplay) height: u16,
}

fn permille(fraction: f32) -> u16 {
    (fraction.clamp(0.0, 1.0) * 1000.0).round() as u16
}

impl PermilleBox {
    pub(in crate::gameplay) fn of(found: &HeadBox) -> Self {
        let (x, y) = centre(found);
        Self {
            x: permille(x),
            y: permille(y),
            width: permille(box_width(found)).max(1),
            height: permille(box_height(found)).max(1),
        }
    }
}

/// What the wire layout carries for one person: the tight head box, the body box, the loader
/// anchor, which side the bubble stands on and which way the person faces.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct PersonLayout {
    pub(in crate::gameplay) head: PermilleBox,
    pub(in crate::gameplay) body: PermilleBox,
    pub(in crate::gameplay) anchor_x: u16,
    pub(in crate::gameplay) anchor_y: u16,
    pub(in crate::gameplay) side: SideChoice,
    pub(in crate::gameplay) facing: Option<Facing>,
}

/// The layout of `person` among `others` (the other people on the plate). `None` when no head
/// was paired: the caller keeps the head locator's box.
pub(in crate::gameplay) fn person_layout(
    person: &SegmentedPerson,
    facing: Option<Facing>,
    others: &[&SegmentedPerson],
) -> Option<PersonLayout> {
    let head = person.head.as_ref()?;
    let other_heads: Vec<HeadBox> = others.iter().filter_map(|other| other.head).collect();
    Some(PersonLayout {
        head: PermilleBox::of(head),
        body: PermilleBox::of(&person.body),
        anchor_x: permille(person.anchor.x),
        anchor_y: permille(person.anchor.y),
        side: bubble_side(head, facing, &other_heads),
        facing,
    })
}

#[cfg(test)]
mod tests {
    use super::super::geometry::{people_from_segments, tests::instance};
    use super::*;

    fn head(left: f32, top: f32, right: f32, bottom: f32) -> HeadBox {
        HeadBox {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn a_person_facing_a_clear_side_gets_the_bubble_there_even_away_from_the_middle() {
        // A head in the left half: the old rule put the bubble on the right (toward the middle).
        let found = head(0.40, 0.2, 0.46, 0.32);
        let facing_left = bubble_side(&found, Some(Facing::Left), &[]);
        assert_eq!(facing_left.side, Side::Left);
        assert_eq!(facing_left.reason, SideReason::FacesClearSide);
        let facing_right = bubble_side(&found, Some(Facing::Right), &[]);
        assert_eq!(facing_right.side, Side::Right);
    }

    #[test]
    fn facing_the_camera_or_unknown_takes_the_side_with_more_open_space() {
        let found = head(0.60, 0.2, 0.66, 0.32);
        // Another head near the right edge leaves more room on the left.
        let others = [head(0.95, 0.2, 1.0, 0.32)];
        for facing in [
            Some(Facing::TowardCamera),
            Some(Facing::AwayFromCamera),
            None,
        ] {
            let choice = bubble_side(&found, facing, &others);
            assert_eq!(choice.side, Side::Left, "{facing:?}");
        }
    }

    #[test]
    fn alone_in_the_frame_the_bubble_goes_to_the_roomier_side() {
        let right_half = bubble_side(&head(0.70, 0.2, 0.76, 0.32), None, &[]);
        assert_eq!(right_half.side, Side::Left);
        let left_half = bubble_side(&head(0.30, 0.2, 0.36, 0.32), None, &[]);
        assert_eq!(left_half.side, Side::Right);
        assert_eq!(left_half.reason, SideReason::MoreOpenSpace);
    }

    #[test]
    fn the_bubble_never_covers_another_head_even_on_the_side_the_person_faces() {
        let found = head(0.40, 0.2, 0.46, 0.32);
        // Another head stands right where a right-hand bubble would go.
        let others = [head(0.50, 0.22, 0.56, 0.34)];
        let choice = bubble_side(&found, Some(Facing::Right), &others);
        assert_eq!(choice.side, Side::Left);
        assert_eq!(choice.reason, SideReason::OnlyClearSide);
        assert!(choice.clear);
    }

    #[test]
    fn a_side_that_would_run_off_the_plate_is_not_clear() {
        let against_the_edge = head(0.05, 0.2, 0.11, 0.32);
        let choice = bubble_side(&against_the_edge, Some(Facing::Left), &[]);
        assert_eq!(choice.side, Side::Right);
        assert_eq!(choice.reason, SideReason::OnlyClearSide);
    }

    #[test]
    fn with_no_clear_side_the_wider_one_is_named_and_the_choice_says_it_is_not_clear() {
        // Heads on both sides within bubble reach; more room on the left.
        let found = head(0.50, 0.2, 0.56, 0.32);
        let others = [head(0.36, 0.2, 0.42, 0.32), head(0.62, 0.2, 0.68, 0.32)];
        let choice = bubble_side(&found, Some(Facing::Right), &others);
        assert!(!choice.clear);
        assert_eq!(choice.reason, SideReason::NoClearSide);
        assert_eq!(choice.side, Side::Left);
    }

    #[test]
    fn a_people_layout_carries_tight_boxes_in_permille_the_anchor_and_the_facing() {
        let people = people_from_segments(
            &[
                instance(0.95, 20, 20, 80, 100),
                instance(0.9, 120, 20, 180, 100),
            ],
            &[
                instance(0.9, 40, 20, 60, 40),
                instance(0.9, 135, 20, 160, 40),
            ],
        );
        let layout = person_layout(&people[0], Some(Facing::Right), &[&people[1]]).unwrap();
        // The head box (0.2..0.3 by 0.2..0.4) is centred at 0.25, 0.3 and is 0.1 by 0.2.
        assert_eq!(
            layout.head,
            PermilleBox {
                x: 250,
                y: 300,
                width: 100,
                height: 200
            }
        );
        assert_eq!(layout.body.width, 300);
        assert_eq!((layout.anchor_x, layout.anchor_y), (250, 200));
        assert_eq!(layout.side.side, Side::Right);
        assert_eq!(layout.facing, Some(Facing::Right));
    }

    #[test]
    fn a_person_without_a_head_has_no_layout() {
        let people = people_from_segments(&[instance(0.95, 20, 20, 80, 100)], &[]);
        assert!(person_layout(&people[0], None, &[]).is_none());
    }
}
