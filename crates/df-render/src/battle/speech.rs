//! Who is speaking, on the battlefield. A spoken line (an enemy's taunt, a hero's typed line, a
//! battle cry) shows as a bubble anchored to the figure that says it, with its tail pointing at
//! the speaker's head. This module is the pure placement rule, in screen fractions of the framed
//! picture (0 to 1 across and down): the bubble is never off the screen, never over a plate, the
//! round strip, the dice card or the speaker themself, and its tail always runs from the bubble
//! to the speaker. The DOM bubble (`speech_bubble.rs`) draws what this decides.

/// How long a line may stay up when nothing ends it, in seconds. A line normally ends with its
/// voice; this only keeps a lost end call from leaving a bubble on screen.
pub const MAX_LINE_S: f32 = 20.0;
/// The camera's ease in on the speaker and back, in seconds.
pub const CAMERA_EASE_S: f32 = 0.7;
/// How much closer than the turn-focus shot the camera comes for a line (a dolly factor).
pub const CAMERA_DOLLY: f32 = 0.78;

/// The picture's usable area: a bubble never leaves it.
pub const SAFE: Rect = Rect {
    left: 0.02,
    top: 0.03,
    right: 0.98,
    bottom: 0.97,
};
/// Clear space kept between a bubble and anything it must not cover.
const MARGIN: f32 = 0.012;
/// How far the bubble sits from the speaker's head: the length of the tail.
const TAIL: f32 = 0.05;
/// Narrowest and widest a bubble is, as a share of the picture's width.
const MIN_WIDTH: f32 = 0.16;
const MAX_WIDTH: f32 = 0.3;

/// A rectangle in screen fractions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Rect {
    pub fn grown(&self, by: f32) -> Rect {
        Rect {
            left: self.left - by,
            top: self.top - by,
            right: self.right + by,
            bottom: self.bottom + by,
        }
    }

    pub fn contains(&self, point: (f32, f32)) -> bool {
        point.0 >= self.left
            && point.0 <= self.right
            && point.1 >= self.top
            && point.1 <= self.bottom
    }

    /// The area two rectangles share.
    pub fn overlap(&self, other: &Rect) -> f32 {
        let width = self.right.min(other.right) - self.left.max(other.left);
        let height = self.bottom.min(other.bottom) - self.top.max(other.top);
        if width > 0.0 && height > 0.0 {
            width * height
        } else {
            0.0
        }
    }

    #[cfg(test)]
    fn inside(&self, outer: &Rect) -> bool {
        self.left >= outer.left - 1e-4
            && self.top >= outer.top - 1e-4
            && self.right <= outer.right + 1e-4
            && self.bottom <= outer.bottom + 1e-4
    }
}

/// Which side of the bubble the tail leaves from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Side {
    Bottom,
    Top,
    Left,
    Right,
}

/// Where the bubble and its tail go.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub rect: Rect,
    /// The tail's two base points on the bubble's edge and its tip at the speaker.
    pub tail: [(f32, f32); 3],
    pub side: Side,
}

/// A guess at a bubble's size for a line before it has been measured: a width that grows with the
/// text up to a cap, and as many lines as the text needs. `aspect` is the picture's width over
/// its height.
pub fn estimate_size(text: &str, aspect: f32) -> (f32, f32) {
    let characters = text.chars().count().max(1) as f32;
    let width = (0.05 + characters * 0.0065).clamp(MIN_WIDTH, MAX_WIDTH);
    // About 0.0105 of the picture's width per character at the display's type size.
    let per_line = (width / 0.0105).max(8.0);
    let lines = (characters / per_line).ceil().max(1.0);
    let height = (0.05 + lines * 0.043) * (16.0 / 9.0) / aspect.max(0.1);
    (width, height)
}

/// Where a speaker is on the picture: the top of the head, the mouth, and how far the body
/// reaches to either side of the head (all in screen fractions).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Speaker {
    pub head: (f32, f32),
    pub mouth: (f32, f32),
    pub reach: f32,
}

/// Places a bubble of `size` for `speaker`. `avoid` is everything the bubble must stay off (the
/// plates, the round strip, the dice card, the speaker's own body). The first position that is on
/// screen, clear of everything and not over the speaker wins: above the head (the tail runs down to
/// it), then beside the head (the tail runs across to the body's edge), then below; if none is clear,
/// the one that covers the least wins. The tail always runs from the bubble's edge to the speaker.
pub fn place(speaker: Speaker, size: (f32, f32), avoid: &[Rect]) -> Placement {
    let (w, h) = size;
    let anchor = speaker.head;
    let clamp_x = |x: f32| x.clamp(SAFE.left, (SAFE.right - w).max(SAFE.left));
    let clamp_y = |y: f32| y.clamp(SAFE.top, (SAFE.bottom - h).max(SAFE.top));
    // Every position worth trying, most natural first; each leaning left, centred or right (or
    // shifted up and down beside the speaker) so a crowded corner still finds a gap.
    let mut candidates: Vec<(f32, f32, Side)> = Vec::new();
    let leans = [0.5, 0.2, 0.8, 0.05, 0.95];
    let shifts = [0.0, -0.08, 0.08, -0.16, 0.16, -0.26, 0.26, -0.38, 0.38];
    let beside = speaker.reach + TAIL;
    for lean in leans {
        candidates.push((anchor.0 - w * lean, anchor.1 - TAIL - h, Side::Bottom));
    }
    for shift in shifts {
        candidates.push((
            speaker.mouth.0 + beside,
            speaker.mouth.1 - h * 0.5 + shift,
            Side::Left,
        ));
        candidates.push((
            speaker.mouth.0 - beside - w,
            speaker.mouth.1 - h * 0.5 + shift,
            Side::Right,
        ));
    }
    for lean in leans {
        candidates.push((anchor.0 - w * lean, anchor.1 + TAIL, Side::Top));
    }
    let faces_speaker = |rect: &Rect, side: Side| match side {
        Side::Bottom => anchor.1 >= rect.bottom - 1e-4,
        Side::Top => anchor.1 <= rect.top + 1e-4,
        Side::Left => speaker.mouth.0 <= rect.left + 1e-4,
        Side::Right => speaker.mouth.0 >= rect.right - 1e-4,
    };
    let cost = |rect: &Rect| -> f32 {
        let covered: f32 = avoid
            .iter()
            .map(|other| rect.overlap(&other.grown(MARGIN)))
            .sum();
        let over_speaker = if rect.grown(0.004).contains(anchor) {
            1.0
        } else {
            0.0
        };
        covered + over_speaker
    };
    let mut best: Option<(f32, Rect, Side)> = None;
    for (left, top, side) in candidates {
        let (left, top) = (clamp_x(left), clamp_y(top));
        let rect = Rect {
            left,
            top,
            right: left + w,
            bottom: top + h,
        };
        // A position the screen edge pushed to the wrong side of the speaker cannot carry a tail
        // to it.
        if !faces_speaker(&rect, side) {
            continue;
        }
        let c = cost(&rect);
        if best.as_ref().is_none_or(|(least, _, _)| c < *least - 1e-6) {
            best = Some((c, rect, side));
        }
        if c == 0.0 {
            break;
        }
    }
    let (rect, side) = match best {
        Some((_, rect, side)) => (rect, side),
        None => {
            // No position keeps its tail to the speaker (a bubble wider than the room): above the
            // head, clamped, with the tail to wherever the head is.
            let left = clamp_x(anchor.0 - w * 0.5);
            let top = clamp_y(anchor.1 - TAIL - h);
            (
                Rect {
                    left,
                    top,
                    right: left + w,
                    bottom: top + h,
                },
                Side::Bottom,
            )
        }
    };
    Placement {
        rect,
        tail: tail_for(&rect, side, speaker),
        side,
    }
}

/// The tail from the bubble's edge to the speaker: a short triangle whose base sits on the edge
/// facing the speaker, kept off the corners. Above or below the head it ends at the head; beside
/// it, at the edge of the body at the mouth's height.
fn tail_for(rect: &Rect, side: Side, speaker: Speaker) -> [(f32, f32); 3] {
    let half = 0.012;
    match side {
        Side::Bottom | Side::Top => {
            let edge = if side == Side::Bottom {
                rect.bottom
            } else {
                rect.top
            };
            let at = speaker.head.0.clamp(rect.left + 0.03, rect.right - 0.03);
            [(at - half, edge), (at + half, edge), speaker.head]
        }
        Side::Left | Side::Right => {
            let edge = if side == Side::Left {
                rect.left
            } else {
                rect.right
            };
            let at = speaker.mouth.1.clamp(rect.top + 0.03, rect.bottom - 0.03);
            let toward = if side == Side::Left { 1.0 } else { -1.0 };
            // The tip stops at the body's edge: a tail that crossed the figure would hide it.
            let tip = (
                speaker.mouth.0 - toward * speaker.reach * 0.45,
                speaker.mouth.1,
            );
            [(edge, at - half * 1.6), (edge, at + half * 1.6), tip]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASPECT: f32 = 16.0 / 9.0;

    fn plates() -> Vec<Rect> {
        // Two party plates left, three enemy plates right, the round strip across the top.
        vec![
            Rect {
                left: 0.02,
                top: 0.04,
                right: 0.22,
                bottom: 0.27,
            },
            Rect {
                left: 0.78,
                top: 0.11,
                right: 0.98,
                bottom: 0.46,
            },
            Rect {
                left: 0.34,
                top: 0.02,
                right: 0.66,
                bottom: 0.23,
            },
        ]
    }

    fn dice_card() -> Rect {
        Rect {
            left: 0.25,
            top: 0.78,
            right: 0.75,
            bottom: 0.96,
        }
    }

    fn body_below(anchor: (f32, f32)) -> Rect {
        Rect {
            left: anchor.0 - 0.04,
            top: anchor.1,
            right: anchor.0 + 0.04,
            bottom: (anchor.1 + 0.3).min(1.0),
        }
    }

    fn speaker_at(anchor: (f32, f32)) -> Speaker {
        Speaker {
            head: anchor,
            mouth: (anchor.0, anchor.1 + 0.03),
            reach: 0.045,
        }
    }

    fn check(anchor: (f32, f32), avoid: &[Rect]) -> Placement {
        let size = estimate_size(
            "Hand over the packet and nobody has to bleed tonight.",
            ASPECT,
        );
        let placed = place(speaker_at(anchor), size, avoid);
        assert!(
            placed.rect.inside(&SAFE),
            "{anchor:?}: off screen {:?}",
            placed.rect
        );
        for other in avoid {
            assert_eq!(
                placed.rect.overlap(other),
                0.0,
                "{anchor:?}: covers {other:?} with {:?}",
                placed.rect
            );
        }
        assert!(
            !placed.rect.contains(anchor),
            "{anchor:?}: covers the speaker"
        );
        placed
    }

    #[test]
    fn a_bubble_is_on_screen_clear_of_plates_strip_and_dice_card_and_off_the_speaker() {
        let mut avoid = plates();
        avoid.push(dice_card());
        for anchor in [
            (0.5, 0.5),
            (0.3, 0.45),
            (0.62, 0.38),
            (0.5, 0.33),
            (0.12, 0.55),
            (0.88, 0.6),
            (0.5, 0.7),
            (0.95, 0.9),
            (0.05, 0.4),
        ] {
            let mut obstacles = avoid.clone();
            obstacles.push(body_below(anchor));
            check(anchor, &obstacles);
        }
    }

    #[test]
    fn the_tail_runs_from_the_bubble_edge_to_the_speaker() {
        let mut avoid = plates();
        avoid.push(dice_card());
        for anchor in [
            (0.5, 0.5),
            (0.3, 0.45),
            (0.62, 0.38),
            (0.88, 0.6),
            (0.12, 0.55),
        ] {
            let mut obstacles = avoid.clone();
            obstacles.push(body_below(anchor));
            let placed = check(anchor, &obstacles);
            let speaker = speaker_at(anchor);
            let tip = placed.tail[2];
            match placed.side {
                Side::Bottom | Side::Top => assert_eq!(tip, speaker.head, "the tip is on the head"),
                Side::Left | Side::Right => {
                    assert_eq!(tip.1, speaker.mouth.1);
                    assert!((tip.0 - speaker.mouth.0).abs() <= speaker.reach, "{tip:?}");
                }
            }
            for base in &placed.tail[..2] {
                let on_edge = match placed.side {
                    Side::Bottom => (base.1 - placed.rect.bottom).abs() < 1e-5,
                    Side::Top => (base.1 - placed.rect.top).abs() < 1e-5,
                    Side::Left => (base.0 - placed.rect.left).abs() < 1e-5,
                    Side::Right => (base.0 - placed.rect.right).abs() < 1e-5,
                };
                assert!(
                    on_edge,
                    "{anchor:?}: base {base:?} off the {:?} edge of {:?}",
                    placed.side, placed.rect
                );
            }
            // The tip is on the side of the bubble the tail leaves from.
            match placed.side {
                Side::Bottom => assert!(tip.1 >= placed.rect.bottom),
                Side::Top => assert!(tip.1 <= placed.rect.top),
                Side::Left => assert!(tip.0 <= placed.rect.left),
                Side::Right => assert!(tip.0 >= placed.rect.right),
            }
        }
    }

    #[test]
    fn a_speaker_near_the_top_gets_a_bubble_below_the_strip_not_over_it() {
        let mut avoid = plates();
        let anchor = (0.5, 0.3);
        avoid.push(body_below(anchor));
        let placed = check(anchor, &avoid);
        assert!(placed.rect.top > 0.23, "{:?}", placed.rect);
    }

    #[test]
    fn a_bubble_that_cannot_avoid_everything_still_stays_on_screen_and_off_the_speaker() {
        // Obstacles everywhere: it must still be on screen and not on the speaker.
        let everything = [Rect {
            left: 0.0,
            top: 0.0,
            right: 1.0,
            bottom: 1.0,
        }];
        let anchor = (0.5, 0.5);
        let size = estimate_size("Short.", ASPECT);
        let placed = place(speaker_at(anchor), size, &everything);
        assert!(placed.rect.inside(&SAFE));
        assert!(!placed.rect.contains(anchor));
    }

    #[test]
    fn longer_lines_get_wider_then_taller_bubbles_within_the_caps() {
        let (short_w, short_h) = estimate_size("No.", ASPECT);
        let (long_w, long_h) = estimate_size(&"word ".repeat(40), ASPECT);
        assert!(short_w >= MIN_WIDTH && long_w <= MAX_WIDTH);
        assert!(long_w >= short_w && long_h > short_h);
    }

    #[test]
    fn the_timing_constants_are_a_readable_ease_and_a_bounded_line() {
        assert!((0.4..=1.2).contains(&CAMERA_EASE_S));
        // Closer, not inside the figure.
        const { assert!(CAMERA_DOLLY < 1.0 && CAMERA_DOLLY > 0.5) };
        const { assert!(MAX_LINE_S >= 10.0) };
    }
}
