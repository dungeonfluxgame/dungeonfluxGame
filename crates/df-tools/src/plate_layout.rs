//! Where the speech bubble goes on a scene plate (PLAY-V09).
//!
//! The server says where the speaker's head is in the plate and which side of it is calm. The
//! display turns that into screen pixels and asks this pure function for the bubble's place:
//! beside the head on the given side (tail toward the head), never over the head box, never off
//! the screen, never under anything the display draws over the picture (the party strip, the
//! title, the choice chips, the narrator box). Pure geometry on pixels: the browser glue only
//! measures and applies.

/// An axis-aligned rectangle in pixels. `right` and `bottom` are exclusive edges.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Rect {
    pub(crate) left: f64,
    pub(crate) top: f64,
    pub(crate) right: f64,
    pub(crate) bottom: f64,
}

impl Rect {
    pub(crate) fn new(left: f64, top: f64, right: f64, bottom: f64) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub(crate) fn width(&self) -> f64 {
        self.right - self.left
    }

    pub(crate) fn height(&self) -> f64 {
        self.bottom - self.top
    }

    pub(crate) fn intersects(&self, other: &Self) -> bool {
        self.left < other.right
            && other.left < self.right
            && self.top < other.bottom
            && other.top < self.bottom
    }

    pub(crate) fn contains(&self, inner: &Self) -> bool {
        inner.left >= self.left
            && inner.right <= self.right
            && inner.top >= self.top
            && inner.bottom <= self.bottom
    }
}

/// Which side of the head the bubble stands on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Side {
    Left,
    Right,
}

impl Side {
    fn opposite(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

/// How the plate lies on the screen. The plate keeps its 16:9 shape and covers the screen
/// (`object-fit: cover`), centred; `head` and `head_box` are given as permille of the plate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PlateFrame {
    pub(crate) screen: Rect,
}

impl PlateFrame {
    /// The plate's rectangle on screen: the smallest 16:9 rectangle that covers the screen,
    /// centred on it.
    pub(crate) fn plate(&self) -> Rect {
        let (width, height) = (self.screen.width(), self.screen.height());
        let scale = (width / 16.0).max(height / 9.0);
        let (plate_width, plate_height) = (16.0 * scale, 9.0 * scale);
        let left = self.screen.left + (width - plate_width) / 2.0;
        let top = self.screen.top + (height - plate_height) / 2.0;
        Rect::new(left, top, left + plate_width, top + plate_height)
    }

    /// The head box on screen from the layout's centre and size in permille of the plate.
    pub(crate) fn head_box(&self, x: u32, y: u32, width: u32, height: u32) -> Rect {
        let plate = self.plate();
        let permille = |value: u32| f64::from(value.min(1000)) / 1000.0;
        let (cx, cy) = (
            plate.left + permille(x) * plate.width(),
            plate.top + permille(y) * plate.height(),
        );
        let (half_width, half_height) = (
            permille(width) * plate.width() / 2.0,
            permille(height) * plate.height() / 2.0,
        );
        Rect::new(
            cx - half_width,
            cy - half_height,
            cx + half_width,
            cy + half_height,
        )
    }
}

/// A bubble's place: its rectangle and the side of the head it stands on (its tail points to the
/// head from the opposite side).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Placement {
    pub(crate) rect: Rect,
    pub(crate) side: Side,
}

/// The gap between a bubble and the head box, as a fraction of the screen width.
const GAP_FRACTION: f64 = 0.012;
/// The screen's own margin the bubble keeps, as a fraction of the screen's short side.
const MARGIN_FRACTION: f64 = 0.02;
/// How far the bubble may slide up or down looking for room, in steps of this fraction of the
/// screen height, up to the limit.
const STEP_FRACTION: f64 = 0.025;
const MAXIMUM_STEPS: u32 = 22;

fn clear_of(rect: &Rect, head: &Rect, avoid: &[Rect]) -> bool {
    !rect.intersects(head) && !avoid.iter().any(|other| rect.intersects(other))
}

/// The place for a bubble of `size` (width, height) beside `head` on the `preferred` side, or on
/// the other side when that has no room. `tail_offset` is how far below the bubble's top its tail
/// starts, so the tail lines up with the head's centre. The bubble slides up or down, nearest the
/// head's centre first, until it clears every rectangle in `avoid` and the head box, and stays
/// inside `screen` (less its margin). `None` when no such place exists.
pub(crate) fn place_bubble(
    screen: Rect,
    head: Rect,
    preferred: Side,
    size: (f64, f64),
    tail_offset: f64,
    avoid: &[Rect],
) -> Option<Placement> {
    let (width, height) = size;
    if !(width > 0.0 && height > 0.0) {
        return None;
    }
    let margin = screen.width().min(screen.height()) * MARGIN_FRACTION;
    let inside = Rect::new(
        screen.left + margin,
        screen.top + margin,
        screen.right - margin,
        screen.bottom - margin,
    );
    let gap = screen.width() * GAP_FRACTION;
    let step = screen.height() * STEP_FRACTION;
    let centre = (head.top + head.bottom) / 2.0;
    for side in [preferred, preferred.opposite()] {
        let left = match side {
            Side::Left => head.left - gap - width,
            Side::Right => head.right + gap,
        };
        let ideal = centre - tail_offset;
        for index in 0..=2 * MAXIMUM_STEPS {
            // 0, +1, -1, +2, -2, ...: the nearest to the ideal height first.
            let shift =
                f64::from(index.div_ceil(2)) * step * if index % 2 == 1 { 1.0 } else { -1.0 };
            let top = ideal + shift;
            let rect = Rect::new(left, top, left + width, top + height);
            if inside.contains(&rect) && clear_of(&rect, &head, avoid) {
                return Some(Placement { rect, side });
            }
        }
    }
    None
}

/// A place above or below the head when neither side has room: centred on the head as far as
/// the screen allows, in the first of above, below that clears everything. The side reported is
/// the head's nearer screen half, for the tail.
pub(crate) fn place_bubble_stacked(
    screen: Rect,
    head: Rect,
    size: (f64, f64),
    avoid: &[Rect],
) -> Option<Placement> {
    let (width, height) = size;
    if !(width > 0.0 && height > 0.0) {
        return None;
    }
    let margin = screen.width().min(screen.height()) * MARGIN_FRACTION;
    let inside = Rect::new(
        screen.left + margin,
        screen.top + margin,
        screen.right - margin,
        screen.bottom - margin,
    );
    let gap = screen.width() * GAP_FRACTION;
    let centre = (head.left + head.right) / 2.0;
    let left = (centre - width / 2.0).clamp(inside.left, (inside.right - width).max(inside.left));
    let side = if centre < (screen.left + screen.right) / 2.0 {
        Side::Right
    } else {
        Side::Left
    };
    for top in [head.top - gap - height, head.bottom + gap] {
        let rect = Rect::new(left, top, left + width, top + height);
        if inside.contains(&rect) && clear_of(&rect, &head, avoid) {
            return Some(Placement { rect, side });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect {
        left: 0.0,
        top: 0.0,
        right: 1920.0,
        bottom: 1080.0,
    };

    /// What the display draws over the picture: the party strip at the top left, the title at the
    /// top right and the choice chips at the bottom centre.
    fn interface() -> Vec<Rect> {
        vec![
            Rect::new(0.0, 0.0, 1152.0, 194.0),
            Rect::new(1267.0, 0.0, 1920.0, 324.0),
            Rect::new(576.0, 950.0, 1344.0, 1080.0),
        ]
    }

    fn head_at(x: f64, y: f64) -> Rect {
        Rect::new(x - 115.0, y - 97.0, x + 115.0, y + 97.0)
    }

    fn assert_clear(placed: &Placement, head: &Rect, avoid: &[Rect]) {
        assert!(SCREEN.contains(&placed.rect), "off screen: {placed:?}");
        assert!(!placed.rect.intersects(head), "over the head: {placed:?}");
        for other in avoid {
            assert!(
                !placed.rect.intersects(other),
                "under the interface: {placed:?}"
            );
        }
    }

    #[test]
    fn the_bubble_stands_beside_the_head_on_the_given_side_for_every_anchor_the_server_sends() {
        let frame = PlateFrame { screen: SCREEN };
        for (x, y, side) in [(640, 370, Side::Left), (360, 370, Side::Right)] {
            let head = frame.head_box(x, y, 120, 180);
            let size = (480.0, 190.0);
            let placed = place_bubble(SCREEN, head, side, size, 50.0, &interface()).unwrap();
            assert_eq!(placed.side, side, "the preferred side has room");
            assert_clear(&placed, &head, &interface());
            let beside = match side {
                Side::Left => placed.rect.right <= head.left,
                Side::Right => placed.rect.left >= head.right,
            };
            assert!(beside, "{placed:?} beside {head:?}");
        }
    }

    #[test]
    fn a_bubble_never_covers_the_head_or_leaves_the_screen_or_sits_under_the_interface_anywhere() {
        let mut placed_count = 0;
        let avoid = interface();
        for column in 0..=20 {
            for row in 0..=20 {
                let head = head_at(f64::from(column) * 96.0, f64::from(row) * 54.0);
                for side in [Side::Left, Side::Right] {
                    for size in [(480.0, 190.0), (336.0, 260.0)] {
                        if let Some(placed) = place_bubble(SCREEN, head, side, size, 50.0, &avoid) {
                            assert_clear(&placed, &head, &avoid);
                            placed_count += 1;
                        }
                        if let Some(placed) = place_bubble_stacked(SCREEN, head, size, &avoid) {
                            assert_clear(&placed, &head, &avoid);
                            placed_count += 1;
                        }
                    }
                }
            }
        }
        assert!(placed_count > 1000, "{placed_count}");
    }

    #[test]
    fn the_other_side_is_used_when_the_preferred_one_has_no_room() {
        let head = head_at(200.0, 540.0);
        let placed = place_bubble(SCREEN, head, Side::Left, (480.0, 190.0), 50.0, &[]).unwrap();
        assert_eq!(placed.side, Side::Right);
        assert!(placed.rect.left >= head.right);
    }

    #[test]
    fn a_bubble_slides_out_from_under_the_interface_toward_free_space() {
        // A head under the title block: the bubble beside it must not be under the title.
        let head = head_at(1500.0, 200.0);
        let avoid = interface();
        let placed = place_bubble(SCREEN, head, Side::Left, (480.0, 190.0), 50.0, &avoid).unwrap();
        assert_clear(&placed, &head, &avoid);
    }

    #[test]
    fn nothing_fits_a_bubble_bigger_than_the_screen_or_an_empty_one() {
        let head = head_at(960.0, 500.0);
        assert!(place_bubble(SCREEN, head, Side::Left, (2400.0, 190.0), 50.0, &[]).is_none());
        assert!(place_bubble(SCREEN, head, Side::Left, (0.0, 190.0), 50.0, &[]).is_none());
        assert!(place_bubble_stacked(SCREEN, head, (2400.0, 190.0), &[]).is_none());
    }

    #[test]
    fn the_plate_is_centred_and_covers_a_screen_of_any_shape() {
        let wide = PlateFrame {
            screen: Rect::new(0.0, 0.0, 2000.0, 800.0),
        };
        let plate = wide.plate();
        assert!(plate.contains(&wide.screen) || plate.width() >= 2000.0);
        assert!((plate.width() / plate.height() - 16.0 / 9.0).abs() < 1e-9);
        let tall = PlateFrame {
            screen: Rect::new(0.0, 0.0, 800.0, 1000.0),
        };
        let plate = tall.plate();
        assert!(plate.height() >= 1000.0 && plate.width() > 800.0);
        assert!(((plate.left + plate.right) / 2.0 - 400.0).abs() < 1e-9);
    }

    #[test]
    fn sides_are_named_for_the_stylesheet() {
        assert_eq!((Side::Left.name(), Side::Right.name()), ("left", "right"));
    }

    #[test]
    fn the_head_box_follows_the_layout_in_permille_of_the_plate() {
        let frame = PlateFrame { screen: SCREEN };
        let head = frame.head_box(500, 500, 100, 200);
        assert!((head.left - 864.0).abs() < 1e-9 && (head.right - 1056.0).abs() < 1e-9);
        assert!((head.top - 432.0).abs() < 1e-9 && (head.bottom - 648.0).abs() < 1e-9);
    }
}
