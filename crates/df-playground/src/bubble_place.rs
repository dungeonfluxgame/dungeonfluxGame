//! Where an anchored speech or thought bubble goes so it avoids the screen's chrome and the other
//! bubbles.
//!
//! The page measures the stage, every bubble and the chrome that must stay clear (logo, title bar,
//! pills, status chip, caption bars, markers), hands them here in stage units, and applies what
//! comes back. Nothing here reads a page, so native tests cover every row. Everything is in one
//! unit (the stage's layout pixels, origin at its top left), so the camera's zoom is the caller's
//! to apply to the anchors; a bubble keeps one size on the screen, so its measured size is already
//! screen space.
//!
//! A bubble may never cover a face: every named character's face is a keep-out that must stay
//! clean, and when nothing fits cleanly a covered face weighs far more than any other overlap.
//! A bubble tries, in order: above its head (tail down) centred on it and slid along the row; then
//! beside its face, on the side with more room first (tail pointing sideways at the face; speech
//! only, a thought keeps its dot trail on the top or bottom edge); then below its chin (tail up).
//! The first candidate fully inside the stage that touches no keep-out, no face and no bubble
//! placed before it wins; when none does, the one with the least weighted overlap. The same input
//! always gives the same output.

use serde::{Deserialize, Serialize};

use crate::camera::Framing;

/// How far along a candidate's box the tail's tip moves between tries, as a fraction of its width.
const SLIDE_STEP: f32 = 0.125;
/// The tip stays this far inside the box's ends, as a fraction of its width.
const TIP_INSET: f32 = 0.1;
/// Two edges closer than this are not overlapping (rounding noise of a measured box).
const TOUCH: f32 = 0.01;
/// How much worse a covered face is than the same area of other overlap, when nothing fits.
const FACE_WEIGHT: f32 = 1000.0;
/// Where down a box beside a face its tail's tip may sit (fractions of its height), best first.
const BESIDE_TIPS: [f32; 5] = [0.5, 0.3, 0.7, 0.15, 0.85];

/// A rectangle in stage units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn new(left: f32, top: f32, width: f32, height: f32) -> Self {
        Self {
            left,
            top,
            width,
            height,
        }
    }

    fn right(&self) -> f32 {
        self.left + self.width
    }

    fn bottom(&self) -> f32 {
        self.top + self.height
    }

    /// The rectangle grown by `by` on every side.
    fn grown(&self, by: f32) -> Self {
        Self::new(
            self.left - by,
            self.top - by,
            self.width + 2.0 * by,
            self.height + 2.0 * by,
        )
    }

    /// The area both rectangles cover.
    pub fn overlap_area(&self, other: &Self) -> f32 {
        let across = (self.right().min(other.right()) - self.left.max(other.left)).max(0.0);
        let down = (self.bottom().min(other.bottom()) - self.top.max(other.top)).max(0.0);
        across * down
    }
}

/// Which side of its head a bubble is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BubbleSide {
    /// Above the head, the tail pointing down at it.
    AboveHead,
    /// Below the chin, the tail pointing up at it.
    BelowHead,
    /// Left of the face, the tail on the box's right edge pointing at it.
    LeftOfFace,
    /// Right of the face, the tail on the box's left edge pointing at it.
    RightOfFace,
}

impl BubbleSide {
    /// The serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::AboveHead => "above_head",
            Self::BelowHead => "below_head",
            Self::LeftOfFace => "left_of_face",
            Self::RightOfFace => "right_of_face",
        }
    }

    /// Whether the tail is on a side edge, its tip measured down the box rather than across.
    pub fn is_beside(self) -> bool {
        matches!(self, Self::LeftOfFace | Self::RightOfFace)
    }
}

/// The side and tail position a bubble had when it was last placed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Choice {
    pub side: BubbleSide,
    /// Where the tail's tip is on the box: across (0 left, 1 right) above or below a head, down
    /// (0 top, 1 bottom) beside a face.
    pub tip_fraction: f32,
}

/// A bubble to place.
#[derive(Debug, Clone, PartialEq)]
pub struct BubbleRequest {
    /// The character it speaks for; the tie-break of the order.
    pub id: String,
    /// The point its tail's tip must touch: just over the character's head.
    pub anchor_x: f32,
    pub anchor_y: f32,
    /// The measured box, tail included.
    pub width: f32,
    pub height: f32,
    /// Where along the box its tail would be if nothing were in the way (0 to 1), such as a lean
    /// a scene author asked for; it is tried first.
    pub preferred_tip: f32,
    /// Where it was last placed. It stays there while that still fits.
    pub previous: Option<Choice>,
    /// The speaker's face, which the tail points at from beside or below.
    pub face: Rect,
    /// Whether it may go beside the face (a speech bubble; a thought keeps its dot trail upright).
    pub beside: bool,
}

/// The clearances a placement keeps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Clearance {
    /// The least distance from a bubble to the stage's edge.
    pub margin: f32,
    /// The least distance from a bubble to the chrome and to another bubble.
    pub gap: f32,
    /// How far under the head point a tail pointing up stops, so a bubble below one head does not
    /// touch a bubble above a head at the same height.
    pub drop: f32,
}

/// Where one bubble goes.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub id: String,
    pub side: BubbleSide,
    /// The box's top left corner.
    pub left: f32,
    pub top: f32,
    /// Where the tail's tip is along the box, so it still points at the head.
    pub tip_fraction: f32,
    /// What it still overlaps (chrome, bubbles, outside the stage); 0 when it fits cleanly.
    pub overlap_area: f32,
}

impl Placement {
    /// The side and tip, as the next placement's `previous`.
    pub fn choice(&self) -> Choice {
        Choice {
            side: self.side,
            tip_fraction: self.tip_fraction,
        }
    }
}

#[derive(Clone)]
struct Candidate {
    side: BubbleSide,
    left: f32,
    top: f32,
    tip_fraction: f32,
}

impl Candidate {
    fn rect(&self, request: &BubbleRequest) -> Rect {
        Rect::new(self.left, self.top, request.width, request.height)
    }
}

/// The candidate that puts the tail's tip at `tip_fraction` of the box on `side`, the box moved
/// back inside the stage when it would stick out (the tip then follows the head).
fn candidate(
    request: &BubbleRequest,
    stage: &Rect,
    clearance: Clearance,
    side: BubbleSide,
    tip_fraction: f32,
) -> Candidate {
    let wanted = request.anchor_x - tip_fraction * request.width;
    let least = stage.left + clearance.margin;
    let most = stage.right() - clearance.margin - request.width;
    let left = if most < least {
        least
    } else {
        wanted.clamp(least, most)
    };
    let top = match side {
        BubbleSide::AboveHead => request.anchor_y - request.height,
        BubbleSide::BelowHead => request.face.bottom() + clearance.drop,
        BubbleSide::LeftOfFace | BubbleSide::RightOfFace => {
            return beside(request, stage, clearance, side, tip_fraction);
        }
    };
    let tip = if request.width > 0.0 {
        ((request.anchor_x - left) / request.width).clamp(0.0, 1.0)
    } else {
        0.5
    };
    Candidate {
        side,
        left,
        top,
        tip_fraction: tip,
    }
}

/// The candidate beside the face on `side` with its tail's tip `tip_fraction` down the box, at
/// the face's middle; the box moved back inside the stage when it would stick out.
fn beside(
    request: &BubbleRequest,
    stage: &Rect,
    clearance: Clearance,
    side: BubbleSide,
    tip_fraction: f32,
) -> Candidate {
    let face = &request.face;
    let left = match side {
        BubbleSide::LeftOfFace => face.left - clearance.drop - request.width,
        _ => face.right() + clearance.drop,
    };
    let middle = face.top + face.height / 2.0;
    let least = stage.top + clearance.margin;
    let most = stage.bottom() - clearance.margin - request.height;
    let wanted = middle - tip_fraction * request.height;
    let top = if most < least {
        least
    } else {
        wanted.clamp(least, most)
    };
    let tip = if request.height > 0.0 {
        ((middle - top) / request.height).clamp(TIP_INSET, 1.0 - TIP_INSET)
    } else {
        0.5
    };
    Candidate {
        side,
        left,
        top,
        tip_fraction: tip,
    }
}

/// The tail positions to try: the preferred one, centred, then alternately left and right.
fn tip_fractions(preferred: f32) -> Vec<f32> {
    let mut fractions = vec![preferred.clamp(0.0, 1.0), 0.5];
    let mut steps = 1.0_f32;
    while 0.5 - steps * SLIDE_STEP >= TIP_INSET - TOUCH {
        fractions.push(0.5 - steps * SLIDE_STEP);
        fractions.push(0.5 + steps * SLIDE_STEP);
        steps += 1.0;
    }
    fractions.push(TIP_INSET);
    fractions.push(1.0 - TIP_INSET);
    fractions.push(0.0);
    fractions.push(1.0);
    fractions
}

/// What a box overlaps: the part outside the stage's inner area, the chrome and the placed boxes.
struct Overlap {
    outside: f32,
    others: f32,
    faces: f32,
}

impl Overlap {
    fn total(&self) -> f32 {
        self.outside + self.others + FACE_WEIGHT * self.faces
    }

    fn clean(&self) -> bool {
        self.outside <= TOUCH && self.others <= TOUCH && self.faces <= TOUCH
    }
}

fn overlap_of(
    rect: &Rect,
    stage: &Rect,
    clearance: Clearance,
    keep_out: &[Rect],
    faces: &[Rect],
    placed: &[Rect],
) -> Overlap {
    let inner = Rect::new(
        stage.left + clearance.margin,
        stage.top + clearance.margin,
        (stage.width - 2.0 * clearance.margin).max(0.0),
        (stage.height - 2.0 * clearance.margin).max(0.0),
    );
    let outside = (rect.width * rect.height - rect.overlap_area(&inner)).max(0.0);
    let grown = rect.grown(clearance.gap / 2.0);
    let others = keep_out
        .iter()
        .chain(placed)
        .map(|other| grown.overlap_area(&other.grown(clearance.gap / 2.0)))
        .sum();
    // A face is checked against the box itself: a tail's tip may touch the face's edge.
    let faces = faces.iter().map(|face| rect.overlap_area(face)).sum();
    Overlap {
        outside,
        others,
        faces,
    }
}

/// The most candidates the search for a clean arrangement judges before it gives up.
const SEARCH_BUDGET: u32 = 20_000;

/// Everything a search needs besides the bubbles.
struct Scene<'a> {
    stage: &'a Rect,
    keep_out: &'a [Rect],
    faces: &'a [Rect],
    clearance: Clearance,
}

/// The candidates of a bubble in the order they are tried: where it was last placed, then the
/// preferred tail, centred, and slid along the row, above the head and then below it. A slide that
/// the stage's edge cut short repeats an earlier try and is left out.
fn candidates(request: &BubbleRequest, scene: &Scene<'_>) -> Vec<Candidate> {
    let mut list: Vec<Candidate> = Vec::new();
    let mut add = |option: Candidate| {
        let repeats = list.iter().any(|seen| {
            seen.side == option.side
                && (seen.left - option.left).abs() < TOUCH
                && (seen.top - option.top).abs() < TOUCH
        });
        if !repeats {
            list.push(option);
        }
    };
    if let Some(previous) = request.previous {
        add(candidate(
            request,
            scene.stage,
            scene.clearance,
            previous.side,
            previous.tip_fraction,
        ));
    }
    let fractions = tip_fractions(request.preferred_tip);
    for fraction in &fractions {
        add(candidate(
            request,
            scene.stage,
            scene.clearance,
            BubbleSide::AboveHead,
            *fraction,
        ));
    }
    if request.beside {
        // The side with more room first.
        let room_left = request.face.left - scene.stage.left;
        let room_right = scene.stage.right() - request.face.right();
        let sides = if room_right >= room_left {
            [BubbleSide::RightOfFace, BubbleSide::LeftOfFace]
        } else {
            [BubbleSide::LeftOfFace, BubbleSide::RightOfFace]
        };
        for side in sides {
            for fraction in BESIDE_TIPS {
                add(candidate(
                    request,
                    scene.stage,
                    scene.clearance,
                    side,
                    fraction,
                ));
            }
        }
    }
    for fraction in &fractions {
        add(candidate(
            request,
            scene.stage,
            scene.clearance,
            BubbleSide::BelowHead,
            *fraction,
        ));
    }
    list
}

fn judge(
    request: &BubbleRequest,
    option: &Candidate,
    scene: &Scene<'_>,
    placed: &[Rect],
) -> Overlap {
    overlap_of(
        &option.rect(request),
        scene.stage,
        scene.clearance,
        scene.keep_out,
        scene.faces,
        placed,
    )
}

/// Looks for candidates, one per bubble, that all fit cleanly: each bubble takes its first
/// candidate that fits with those before it, and a bubble with none sends the one before it on to
/// its next candidate. Returns the index chosen for each bubble, or `None` when no arrangement
/// was found within the budget.
fn search(
    order: &[&BubbleRequest],
    lists: &[Vec<Candidate>],
    scene: &Scene<'_>,
    budget: &mut u32,
) -> Option<Vec<usize>> {
    let mut chosen: Vec<usize> = Vec::with_capacity(order.len());
    let mut placed: Vec<Rect> = Vec::with_capacity(order.len());
    let mut next = 0;
    while chosen.len() < order.len() {
        let slot = chosen.len();
        let mut found = None;
        for (index, option) in lists[slot].iter().enumerate().skip(next) {
            if *budget == 0 {
                return None;
            }
            *budget -= 1;
            if judge(order[slot], option, scene, &placed).clean() {
                found = Some(index);
                break;
            }
        }
        match found {
            Some(index) => {
                placed.push(lists[slot][index].rect(order[slot]));
                chosen.push(index);
                next = 0;
            }
            None => {
                let back = chosen.pop()?;
                placed.pop();
                next = back + 1;
            }
        }
    }
    Some(chosen)
}

/// Places every bubble of `requests` inside `stage`, clear of `keep_out` and of each other. The
/// bubbles are taken left to right by anchor (ties by id), a bubble earlier in that order having
/// the first pick, and the result comes back in that order. When the first picks leave a later
/// bubble nowhere to go, earlier bubbles give way. When no arrangement is clean, each bubble in
/// turn takes the candidate with the least overlap.
pub fn place_bubbles(
    stage: &Rect,
    requests: &[BubbleRequest],
    keep_out: &[Rect],
    faces: &[Rect],
    clearance: Clearance,
) -> Vec<Placement> {
    let mut order: Vec<&BubbleRequest> = requests.iter().collect();
    order.sort_by(|a, b| {
        a.anchor_x
            .total_cmp(&b.anchor_x)
            .then_with(|| a.id.cmp(&b.id))
    });
    let scene = Scene {
        stage,
        keep_out,
        faces,
        clearance,
    };
    let lists: Vec<Vec<Candidate>> = order
        .iter()
        .map(|request| candidates(request, &scene))
        .collect();
    let mut budget = SEARCH_BUDGET;
    let clean = search(&order, &lists, &scene, &mut budget);
    let mut placed: Vec<Rect> = Vec::new();
    let mut result = Vec::with_capacity(order.len());
    for (slot, request) in order.iter().enumerate() {
        let (option, overlap) = match &clean {
            Some(indices) => {
                let option = &lists[slot][indices[slot]];
                (option.clone(), judge(request, option, &scene, &placed))
            }
            None => least_overlap(request, &lists[slot], &scene, &placed),
        };
        placed.push(option.rect(request));
        result.push(Placement {
            id: request.id.clone(),
            side: option.side,
            left: option.left,
            top: option.top,
            tip_fraction: option.tip_fraction,
            overlap_area: overlap.total(),
        });
    }
    result
}

/// The first candidate that fits, or else the one with the least overlap.
fn least_overlap(
    request: &BubbleRequest,
    list: &[Candidate],
    scene: &Scene<'_>,
    placed: &[Rect],
) -> (Candidate, Overlap) {
    let mut best: Option<(&Candidate, Overlap)> = None;
    for option in list {
        let overlap = judge(request, option, scene, placed);
        if overlap.clean() {
            return (option.clone(), overlap);
        }
        if best
            .as_ref()
            .is_none_or(|(_, least)| overlap.total() < least.total())
        {
            best = Some((option, overlap));
        }
    }
    match best {
        Some((option, overlap)) => (option.clone(), overlap),
        None => {
            let option = candidate(
                request,
                scene.stage,
                scene.clearance,
                BubbleSide::AboveHead,
                0.5,
            );
            let overlap = judge(request, &option, scene, placed);
            (option, overlap)
        }
    }
}

/// Where the point at (`tip_x_permille`, `tip_y_permille`) of the picture lands on the stage when
/// the camera is framed as `framing`, in the stage's units; `width` and `height` are the stage's.
/// The world layer is drawn from its top left corner, scaled and then moved.
pub fn anchor_on_stage(tip: (u16, u16), framing: &Framing, width: f32, height: f32) -> (f32, f32) {
    let across = framing.x_percent / 100.0 + framing.scale * f32::from(tip.0) / 1000.0;
    let down = framing.y_percent / 100.0 + framing.scale * f32::from(tip.1) / 1000.0;
    (across * width, down * height)
}

/// The class list of a bubble moved to `side`: its other classes, and the one that draws the tail
/// on that side.
pub fn class_for_side(classes: &str, side: BubbleSide) -> String {
    let mut kept: Vec<&str> = classes
        .split_whitespace()
        .filter(|class| {
            !matches!(
                *class,
                "rp-below" | "rp-above" | "rp-tail-left" | "rp-tail-right"
            )
        })
        .collect();
    kept.push(match side {
        BubbleSide::AboveHead => "rp-below",
        BubbleSide::BelowHead => "rp-above",
        BubbleSide::LeftOfFace => "rp-tail-right",
        BubbleSide::RightOfFace => "rp-tail-left",
    });
    kept.join(" ")
}

/// Where a bubble's box is in the world layer, from the placement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldBox {
    /// The point the tail's tip touches, in thousandths of the screen.
    pub tip_x_permille: u16,
    pub tip_y_permille: u16,
    pub width_permille: u16,
    /// How far under the head point a tail pointing up stops, in screen pixels.
    pub drop_px: f32,
    /// How far from the box's ends the tail's root stays, in percent of its width.
    pub root_inset_percent: f32,
    /// The speaker's face in thousandths of the screen: left, top, right, bottom.
    pub face_permille: (u16, u16, u16, u16),
    /// The box's own height in layout pixels (the page scales a bubble about its tip).
    pub height_px: f32,
}

/// The style of a bubble placed on `side` with its tail's tip at `tip_fraction` of its width. The
/// box is positioned so that the tip stays on the head point whatever the camera's zoom: the page
/// scales a bubble about its tip, so only the box's place relative to the tip matters.
pub fn placed_style(world: &WorldBox, side: BubbleSide, tip_fraction: f32) -> String {
    if side.is_beside() {
        return beside_style(world, side, tip_fraction);
    }
    let tip_percent = tip_fraction * 100.0;
    let root_percent =
        tip_percent.clamp(world.root_inset_percent, 100.0 - world.root_inset_percent);
    let width_percent = f32::from(world.width_permille) / 10.0;
    let left_percent = f32::from(world.tip_x_permille) / 10.0 - tip_fraction * width_percent;
    let tip_y_percent = f32::from(world.tip_y_permille) / 10.0;
    let vertical = match side {
        BubbleSide::AboveHead => format!(
            "bottom:{:.3}%;top:auto;margin-top:0;",
            100.0 - tip_y_percent
        ),
        BubbleSide::BelowHead | BubbleSide::LeftOfFace | BubbleSide::RightOfFace => format!(
            "top:{:.3}%;bottom:auto;margin-top:calc({:.2}px / var(--cam-scale, 1));",
            f32::from(world.face_permille.3) / 10.0,
            world.drop_px
        ),
    };
    format!(
        "left:{left_percent:.3}%;{vertical}margin-left:0;width:{width_percent:.3}%;\
         --tail-root:{root_percent:.2}%;--tail-tip:{tip_percent:.2}%;"
    )
}

/// The style of a bubble beside the face: its tail's tip at the face's edge, `tip_fraction` down
/// the box. The page scales the bubble about that tip, so it stays on the face at any zoom.
fn beside_style(world: &WorldBox, side: BubbleSide, tip_fraction: f32) -> String {
    let (face_left, face_top, face_right, face_bottom) = world.face_permille;
    let width_percent = f32::from(world.width_permille) / 10.0;
    let middle_percent = (f32::from(face_top) + f32::from(face_bottom)) / 20.0;
    let (left_percent, gap_sign) = match side {
        BubbleSide::LeftOfFace => (f32::from(face_left) / 10.0 - width_percent, -1.0),
        _ => (f32::from(face_right) / 10.0, 1.0),
    };
    let tip_percent = tip_fraction * 100.0;
    let root_percent =
        tip_percent.clamp(world.root_inset_percent, 100.0 - world.root_inset_percent);
    format!(
        "left:{left_percent:.3}%;top:{middle_percent:.3}%;bottom:auto;width:{width_percent:.3}%;\
         margin-top:{:.2}px;margin-left:calc({:.2}px / var(--cam-scale, 1));\
         --tail-root-y:{root_percent:.2}%;--tail-tip-y:{tip_percent:.2}%;",
        -tip_fraction * world.height_px,
        gap_sign * world.drop_px
    )
}
