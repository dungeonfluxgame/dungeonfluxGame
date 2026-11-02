use crate::layout::{hit_point_permille, stage_percent, stage_size_percent};

#[test]
fn the_middle_of_the_canvas_is_the_middle_of_the_stage() {
    assert_eq!(stage_percent(50.0, 28.125), (50.0, 50.0));
    assert_eq!(stage_percent(0.0, 0.0), (0.0, 0.0));
    assert_eq!(stage_percent(100.0, 56.25), (100.0, 100.0));
}

#[test]
fn a_point_outside_the_canvas_or_not_a_number_is_held_on_the_stage() {
    assert_eq!(stage_percent(150.0, -5.0), (100.0, 0.0));
    assert_eq!(stage_percent(f32::NAN, f32::INFINITY), (0.0, 0.0));
}

#[test]
fn a_size_is_a_share_of_the_stage_and_never_negative_or_over_all_of_it() {
    assert_eq!(stage_size_percent(10.0, 5.625), (10.0, 10.0));
    assert_eq!(stage_size_percent(-1.0, f32::NAN), (0.0, 0.0));
    assert_eq!(stage_size_percent(500.0, 500.0), (100.0, 100.0));
}

#[test]
fn a_health_bar_fills_in_thousandths_and_is_safe_at_the_edges() {
    assert_eq!(hit_point_permille(6, 12), 500);
    assert_eq!(hit_point_permille(0, 12), 0);
    assert_eq!(hit_point_permille(12, 12), 1_000);
    assert_eq!(hit_point_permille(30, 12), 1_000);
    assert_eq!(hit_point_permille(5, 0), 0);
    assert_eq!(hit_point_permille(u32::MAX, u32::MAX), 1_000);
}
