//! The placement of anchored bubbles: the order of the tries, the clearances and the stability.

use crate::bubble_place::*;
use crate::camera::Framing;

const CLEARANCE: Clearance = Clearance {
    margin: 4.0,
    gap: 4.0,
    drop: 4.0,
};

fn stage() -> Rect {
    Rect::new(0.0, 0.0, 1000.0, 560.0)
}

fn bubble(id: &str, anchor_x: f32, anchor_y: f32) -> BubbleRequest {
    BubbleRequest {
        id: id.to_owned(),
        anchor_x,
        anchor_y,
        width: 200.0,
        height: 80.0,
        preferred_tip: 0.5,
        previous: None,
        // A face 60 wide and 70 tall under the head point, as for a character 200 tall.
        face: Rect::new(anchor_x - 30.0, anchor_y, 60.0, 70.0),
        beside: true,
    }
}

fn box_of(placement: &Placement) -> Rect {
    Rect::new(placement.left, placement.top, 200.0, 80.0)
}

fn place(requests: &[BubbleRequest], keep_out: &[Rect]) -> Vec<Placement> {
    place_bubbles(&stage(), requests, keep_out, &faces_of(requests), CLEARANCE)
}

/// The faces of `requests`: every speaker's own.
fn faces_of(requests: &[BubbleRequest]) -> Vec<Rect> {
    requests.iter().map(|request| request.face).collect()
}

#[test]
fn a_bubble_with_room_sits_above_its_head_centred_with_its_tail_down() {
    let placed = place(&[bubble("warrior", 500.0, 300.0)], &[]);
    let only = &placed[0];
    assert_eq!(only.side, BubbleSide::AboveHead);
    assert_eq!((only.left, only.top), (400.0, 220.0));
    assert!((only.tip_fraction - 0.5).abs() < 1e-6);
    assert_eq!(only.overlap_area, 0.0);
}

#[test]
fn a_preferred_lean_is_tried_first_and_given_up_when_it_collides() {
    let mut leaning = bubble("a", 500.0, 300.0);
    leaning.preferred_tip = 0.8;
    let placed = place(&[leaning.clone()], &[]);
    assert!((placed[0].tip_fraction - 0.8).abs() < 1e-6);
    let chrome = Rect::new(300.0, 200.0, 110.0, 100.0);
    let moved = place(&[leaning], &[chrome]);
    assert_eq!(moved[0].overlap_area, 0.0);
}

#[test]
fn chrome_over_the_head_slides_the_bubble_along_the_row_keeping_the_tail_on_the_head() {
    // A title bar covers the left half of the row above the head.
    let title = Rect::new(0.0, 200.0, 440.0, 100.0);
    let placed = place(&[bubble("warrior", 500.0, 300.0)], &[title]);
    let only = &placed[0];
    assert_eq!(only.side, BubbleSide::AboveHead);
    assert_eq!(only.top, 220.0);
    assert!(only.left >= 444.0, "left was {}", only.left);
    // The tip is still on the head: left + tip * width is the anchor.
    assert!((only.left + only.tip_fraction * 200.0 - 500.0).abs() < 0.01);
    assert!(only.tip_fraction >= 0.1 - 1e-6 && only.tip_fraction < 0.5);
}

#[test]
fn a_bar_across_the_whole_row_sends_the_bubble_below_the_head_with_its_tail_up() {
    let bar = Rect::new(0.0, 100.0, 1000.0, 200.0);
    let mut request = bubble("warrior", 500.0, 300.0);
    request.beside = false;
    let placed = place(&[request], &[bar]);
    let only = &placed[0];
    assert_eq!(only.side, BubbleSide::BelowHead);
    // Under the chin (the face is 70 tall), not under the head.
    assert_eq!(only.top, 374.0);
    assert!((only.tip_fraction - 0.5).abs() < 1e-6);
}

#[test]
fn a_bubble_near_the_left_edge_stays_inside_the_stage_and_its_tail_still_reaches_the_head() {
    let placed = place(&[bubble("elf", 20.0, 300.0)], &[]);
    let only = &placed[0];
    assert_eq!(only.left, 4.0);
    assert!((only.left + only.tip_fraction * 200.0 - 20.0).abs() < 0.01);
    assert_eq!(only.overlap_area, 0.0);
}

#[test]
fn a_head_too_high_for_a_bubble_above_it_gets_one_below() {
    let mut request = bubble("wizard", 500.0, 40.0);
    request.beside = false;
    let placed = place(&[request], &[]);
    assert_eq!(placed[0].side, BubbleSide::BelowHead);
    assert_eq!(placed[0].overlap_area, 0.0);
}

#[test]
fn two_close_heads_do_not_get_overlapping_bubbles() {
    let placed = place(
        &[
            bubble("a_wizard", 520.0, 300.0),
            bubble("b_warrior", 480.0, 300.0),
        ],
        &[],
    );
    assert_eq!(placed.len(), 2);
    // Left to right by anchor: the warrior (480) is placed first.
    assert_eq!(placed[0].id, "b_warrior");
    assert_eq!(placed[1].id, "a_wizard");
    assert_eq!(box_of(&placed[0]).overlap_area(&box_of(&placed[1])), 0.0);
    assert!(placed.iter().all(|one| one.overlap_area == 0.0));
}

#[test]
fn ties_in_anchor_are_ordered_by_name() {
    let placed = place(
        &[bubble("zed", 500.0, 300.0), bubble("amy", 500.0, 300.0)],
        &[],
    );
    assert_eq!(placed[0].id, "amy");
    assert_eq!(placed[1].id, "zed");
}

#[test]
fn the_result_does_not_depend_on_the_order_the_bubbles_are_given_in() {
    let requests = [
        bubble("a", 300.0, 300.0),
        bubble("b", 400.0, 300.0),
        bubble("c", 500.0, 300.0),
    ];
    let mut reversed = requests.clone();
    reversed.reverse();
    assert_eq!(place(&requests, &[]), place(&reversed, &[]));
}

#[test]
fn the_same_input_gives_the_same_output() {
    let requests = [bubble("a", 300.0, 200.0), bubble("b", 340.0, 210.0)];
    let chrome = [Rect::new(250.0, 100.0, 120.0, 60.0)];
    assert_eq!(place(&requests, &chrome), place(&requests, &chrome));
}

#[test]
fn when_nothing_fits_the_candidate_with_the_least_overlap_wins() {
    // The whole stage is chrome, but a thin strip at the top of it is only half covered.
    let wall = Rect::new(0.0, 0.0, 1000.0, 560.0);
    let placed = place(&[bubble("a", 500.0, 300.0)], &[wall]);
    assert!(placed[0].overlap_area > 0.0);
    // Above the head the whole box is inside the wall; below it is too, but a box
    // that sticks out of the stage is worse, so the choice is one that lies inside.
    let inner = Rect::new(4.0, 4.0, 992.0, 552.0);
    assert_eq!(box_of(&placed[0]).overlap_area(&inner), 200.0 * 80.0);
}

#[test]
fn a_bubble_keeps_its_place_while_it_still_fits() {
    let mut request = bubble("a", 500.0, 300.0);
    request.previous = Some(Choice {
        side: BubbleSide::BelowHead,
        tip_fraction: 0.3,
    });
    let placed = place(&[request], &[]);
    assert_eq!(placed[0].side, BubbleSide::BelowHead);
    assert!((placed[0].tip_fraction - 0.3).abs() < 1e-6);
    // The kept choice can be fed back and stays.
    let mut again = bubble("a", 500.0, 300.0);
    again.previous = Some(placed[0].choice());
    assert_eq!(place(&[again], &[])[0], placed[0]);
}

#[test]
fn a_bubble_leaves_its_previous_place_when_chrome_arrives_there() {
    let mut request = bubble("a", 500.0, 300.0);
    request.beside = false;
    request.previous = Some(Choice {
        side: BubbleSide::AboveHead,
        tip_fraction: 0.5,
    });
    let chrome = Rect::new(380.0, 200.0, 240.0, 60.0);
    let placed = place(&[request], &[chrome]);
    assert_eq!(placed[0].side, BubbleSide::BelowHead);
    assert_eq!(box_of(&placed[0]).overlap_area(&chrome), 0.0);
    assert_eq!(placed[0].overlap_area, 0.0);
}

#[test]
fn a_clearance_gap_is_kept_from_chrome() {
    // The chrome ends 2 units from where the centred bubble starts: closer than the gap.
    let chrome = Rect::new(300.0, 220.0, 98.0, 80.0);
    let placed = place(&[bubble("a", 500.0, 300.0)], &[chrome]);
    assert_ne!(placed[0].left, 400.0);
    assert_eq!(placed[0].overlap_area, 0.0);
}

#[test]
fn no_bubbles_place_nothing() {
    assert!(place(&[], &[Rect::new(0.0, 0.0, 10.0, 10.0)]).is_empty());
}

#[test]
fn a_stage_narrower_than_a_bubble_does_not_panic() {
    let narrow = Rect::new(0.0, 0.0, 100.0, 560.0);
    let request = bubble("a", 50.0, 300.0);
    let placed = place_bubbles(
        &narrow,
        std::slice::from_ref(&request),
        &[],
        &[request.face],
        CLEARANCE,
    );
    assert_eq!(placed.len(), 1);
}

#[test]
fn a_side_has_a_stable_name() {
    assert_eq!(BubbleSide::AboveHead.key(), "above_head");
    assert_eq!(BubbleSide::BelowHead.key(), "below_head");
    assert_eq!(
        serde_json::to_string(&BubbleSide::BelowHead).unwrap(),
        "\"below_head\""
    );
}

#[test]
fn a_point_of_the_picture_lands_where_the_camera_puts_it() {
    let whole = Framing::WHOLE_STAGE;
    assert_eq!(
        anchor_on_stage((250, 500), &whole, 1000.0, 560.0),
        (250.0, 280.0)
    );
    // Zoomed to 2x with the world moved a quarter of the stage left and a tenth up.
    let zoomed = Framing {
        scale: 2.0,
        x_percent: -25.0,
        y_percent: -10.0,
    };
    let (x, y) = anchor_on_stage((250, 500), &zoomed, 1000.0, 560.0);
    assert!(
        (x - 250.0).abs() < 1e-3 && (y - 504.0).abs() < 1e-3,
        "{x} {y}"
    );
}

#[test]
fn a_bubble_changes_only_its_side_class() {
    assert_eq!(
        class_for_side(
            "rp-bubble rp-tone-npc rp-below rp-anchored",
            BubbleSide::BelowHead
        ),
        "rp-bubble rp-tone-npc rp-anchored rp-above"
    );
    assert_eq!(
        class_for_side("rp-bubble rp-above", BubbleSide::AboveHead),
        "rp-bubble rp-below"
    );
}

fn world() -> WorldBox {
    WorldBox {
        tip_x_permille: 400,
        tip_y_permille: 300,
        width_permille: 200,
        drop_px: 4.0,
        root_inset_percent: 12.0,
        face_permille: (370, 300, 430, 370),
        height_px: 80.0,
    }
}

#[test]
fn the_style_of_a_bubble_above_keeps_its_tip_on_the_head_point() {
    let style = placed_style(&world(), BubbleSide::AboveHead, 0.25);
    // left = 40% - 0.25 * 20% = 35%; the box's bottom edge is the tip, at 100% - 30%.
    assert_eq!(
        style,
        concat!(
            "left:35.000%;bottom:70.000%;top:auto;margin-top:0;margin-left:0;width:20.000%;",
            "--tail-root:25.00%;--tail-tip:25.00%;"
        )
    );
}

#[test]
fn the_style_of_a_bubble_below_hangs_from_the_chin_and_the_root_keeps_inside_the_box() {
    let style = placed_style(&world(), BubbleSide::BelowHead, 0.0);
    assert_eq!(
        style,
        concat!(
            "left:40.000%;top:37.000%;bottom:auto;",
            "margin-top:calc(4.00px / var(--cam-scale, 1));margin-left:0;width:20.000%;",
            "--tail-root:12.00%;--tail-tip:0.00%;"
        )
    );
}

#[test]
fn an_earlier_bubble_gives_way_when_a_later_one_would_have_nowhere_to_go() {
    // Greedy, the left bubble takes the room over both heads and the right one is boxed in.
    let requests = [bubble("left", 440.0, 200.0), bubble("right", 560.0, 200.0)];
    let wall = [Rect::new(0.0, 0.0, 1000.0, 120.0)];
    let placed = place(&requests, &wall);
    assert!(
        placed.iter().all(|one| one.overlap_area == 0.0),
        "{placed:?}"
    );
    assert_eq!(box_of(&placed[0]).overlap_area(&box_of(&placed[1])), 0.0);
}

#[test]
fn a_bubble_never_covers_any_face_even_when_the_row_above_the_head_is_blocked() {
    // The whole row above the head is chrome, so the bubble must go beside the face, not on it.
    let blocked = Rect::new(0.0, 0.0, 1000.0, 300.0);
    let request = bubble("warrior", 500.0, 300.0);
    let placed = place(std::slice::from_ref(&request), &[blocked]);
    let only = &placed[0];
    assert!(
        matches!(only.side, BubbleSide::RightOfFace | BubbleSide::LeftOfFace),
        "{only:?}"
    );
    assert_eq!(box_of(only).overlap_area(&request.face), 0.0);
    assert_eq!(only.overlap_area, 0.0);
}

#[test]
fn a_bubble_keeps_off_another_characters_face_too() {
    let speaker = bubble("a", 500.0, 300.0);
    let neighbour = bubble("b", 420.0, 230.0);
    let placed = place_bubbles(
        &stage(),
        std::slice::from_ref(&speaker),
        &[],
        &[speaker.face, neighbour.face],
        CLEARANCE,
    );
    assert_eq!(box_of(&placed[0]).overlap_area(&neighbour.face), 0.0);
    assert_eq!(box_of(&placed[0]).overlap_area(&speaker.face), 0.0);
}

#[test]
fn a_thought_is_never_put_beside_a_face() {
    let blocked = Rect::new(0.0, 0.0, 1000.0, 300.0);
    let mut thought = bubble("elf", 500.0, 300.0);
    thought.beside = false;
    let placed = place(&[thought], &[blocked]);
    assert!(!placed[0].side.is_beside(), "{:?}", placed[0]);
    // TODO(DEV-12): cover the fallback when every side covers some face (least weighted overlap).
}

#[test]
fn the_style_of_a_bubble_beside_a_face_puts_its_tip_on_the_faces_edge() {
    let right = placed_style(&world(), BubbleSide::RightOfFace, 0.5);
    // Left edge at the face's right edge (43%), tip down the middle of the face (33.5%).
    assert!(right.contains("left:43.000%"), "{right}");
    assert!(right.contains("top:33.500%"), "{right}");
    assert!(right.contains("--tail-tip-y:50.00%"), "{right}");
    let left = placed_style(&world(), BubbleSide::LeftOfFace, 0.5);
    // The box ends at the face's left edge (37%): 37% - its 20% width.
    assert!(left.contains("left:17.000%"), "{left}");
    assert_eq!(
        class_for_side("rp-bubble rp-below", BubbleSide::RightOfFace),
        "rp-bubble rp-tail-left"
    );
    assert_eq!(
        class_for_side("rp-bubble rp-tail-left", BubbleSide::AboveHead),
        "rp-bubble rp-below"
    );
}
