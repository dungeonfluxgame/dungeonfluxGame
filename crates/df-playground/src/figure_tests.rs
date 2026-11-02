use crate::figure::{FigureLook, figure_style, first_turn, label_style, label_turn, next_turn};
use crate::fixture::{ElementShape, Fixture, StageElement, StagePivot};

fn pip() -> StageElement {
    StageElement {
        id: "pip".to_owned(),
        kind: "character".to_owned(),
        label: "Pip".to_owned(),
        x: 50.0,
        y: 28.125,
        color: Some("#4f8cff".to_owned()),
        shape: ElementShape::Circle,
        width_units: 10.0,
        height_units: 10.0,
        fields: serde_json::Map::new(),
        rotation_degrees: 0,
        pivot: StagePivot::default(),
    }
}

fn turned(rotation_degrees: u16) -> StageElement {
    StageElement {
        rotation_degrees,
        ..pip()
    }
}

fn transform_part(style: &str) -> &str {
    &style[style.find("transform:").expect("has a transform")..]
}

#[test]
fn an_unturned_figure_keeps_its_place_and_size_and_is_centred_with_a_zero_turn() {
    let style = figure_style(&pip(), 0);
    assert_eq!(
        style,
        "left:50.00%;top:50.00%;width:10.00%;height:17.78%;z-index:500;background:#4f8cff;\
         transform:translate(-50%,-50%) rotate(0deg);transform-origin:50% 50%;"
    );
}

#[test]
fn the_turn_is_written_as_degrees_for_zero_ninety_and_359() {
    for degrees in [0_u16, 90, 359] {
        let style = figure_style(&turned(degrees), first_turn(&turned(degrees)));
        assert_eq!(
            transform_part(&style),
            format!(
                "transform:translate(-50%,-50%) rotate({degrees}deg);transform-origin:50% 50%;"
            )
        );
    }
}

#[test]
fn a_custom_pivot_sets_the_transform_origin_in_percent() {
    let hinge = StageElement {
        pivot: StagePivot {
            x_percent: 0,
            y_percent: 50,
        },
        ..turned(90)
    };
    let style = figure_style(&hinge, 90);
    assert!(
        style.ends_with("rotate(90deg);transform-origin:0% 50%;"),
        "{style}"
    );
    let corner = StageElement {
        pivot: StagePivot {
            x_percent: 100,
            y_percent: 100,
        },
        ..turned(1)
    };
    assert!(figure_style(&corner, 1).ends_with("transform-origin:100% 100%;"));
}

#[test]
fn a_pivot_past_the_figure_is_reported_in_a_fixture_and_not_drawn_clamped() {
    let text = fixture_with_element(
        r#"{"id":"d","kind":"door","label":"D","x":1,"y":1,"color":null,"shape":"rect","width_units":2,"height_units":2,"rotation_degrees":0,
  "pivot":{"x_percent":255,"y_percent":0}}"#,
    );
    let error = Fixture::parse(&text).expect_err("refused");
    assert!(
        error.message.contains("tv.elements[0].pivot.x_percent")
            && error
                .message
                .contains("0 to 100 percent of the figure, not 255"),
        "{}",
        error.message
    );
    assert_eq!(error.line, 5, "the line of the value");
    let edge = StageElement {
        pivot: StagePivot {
            x_percent: 100,
            y_percent: 100,
        },
        ..pip()
    };
    assert!(figure_style(&edge, 0).ends_with("transform-origin:100% 100%;"));
}

#[test]
fn the_transform_is_numbers_and_fixed_words_only_and_no_text_of_the_element_reaches_a_style() {
    let hostile = StageElement {
        id: "id\"><script>".to_owned(),
        kind: "k;background:url(x)".to_owned(),
        label: "</style><b>".to_owned(),
        color: Some("red;background:url(x)".to_owned()),
        ..turned(300)
    };
    let style = figure_style(&hostile, first_turn(&hostile));
    for text in ["script", "url", "style", "<", ">", "\"", "red"] {
        assert!(!style.contains(text), "{text} in {style}");
    }
    assert!(
        !style.contains("background"),
        "an unchecked colour is dropped"
    );
    assert!(
        transform_part(&style).chars().all(|character| matches!(
            character,
            '0'..='9' | 'a'..='z' | '%' | '(' | ')' | ',' | ':' | ';' | '-' | ' '
        )),
        "{style}"
    );
}

#[test]
fn a_first_turn_is_the_rotation_and_a_rotation_past_a_full_turn_is_reported_in_a_fixture() {
    assert_eq!(first_turn(&turned(0)), 0);
    assert_eq!(first_turn(&turned(359)), 359);
    let text = fixture_with_element(
        r#"{"id":"d","kind":"door","label":"D","x":1,"y":1,"color":null,"shape":"rect","width_units":2,"height_units":2,
  "rotation_degrees":400,"pivot":{"x_percent":50,"y_percent":50}}"#,
    );
    let error = Fixture::parse(&text).expect_err("refused");
    assert!(
        error.message.contains("tv.elements[0].rotation_degrees")
            && error.message.contains("0 to 359 degrees, not 400"),
        "{}",
        error.message
    );
    assert_eq!(error.line, 5);
}

#[test]
fn a_figure_goes_the_short_way_round_to_its_new_rotation() {
    assert_eq!(next_turn(350, 10), 370, "20 degrees on, not 340 back");
    assert_eq!(next_turn(10, 350), -10, "20 degrees back, not 340 on");
    assert_eq!(next_turn(0, 90), 90);
    assert_eq!(next_turn(90, 0), 0);
    assert_eq!(next_turn(0, 180), 180, "a half turn goes clockwise");
    assert_eq!(next_turn(180, 0), 360);
}

#[test]
fn a_turn_already_drawn_past_a_full_turn_is_kept_and_a_repeat_does_not_spin() {
    assert_eq!(
        next_turn(370, 10),
        370,
        "the same rotation draws the same turn"
    );
    assert_eq!(next_turn(-10, 350), -10);
    assert_eq!(next_turn(370, 30), 390);
    assert_eq!(next_turn(370, 350), 350, "back 20 from 370 is 350");
}

#[test]
fn going_round_and_round_keeps_every_step_under_a_half_turn() {
    let mut drawn = first_turn(&turned(0));
    for step in 1..=200_u16 {
        let rotation = (step * 37) % 360;
        let next = next_turn(drawn, rotation);
        assert!((next - drawn).abs() <= 180, "{drawn} to {next}");
        assert_eq!(next.rem_euclid(360), i64::from(rotation));
        drawn = next;
    }
}

#[test]
fn the_label_turns_back_by_the_figures_turn_so_its_text_stays_upright() {
    assert_eq!(label_style(0), "transform:translateX(-50%) rotate(0deg);");
    assert_eq!(
        label_style(90),
        "transform:translateX(-50%) rotate(-90deg);"
    );
    assert_eq!(
        label_style(-10),
        "transform:translateX(-50%) rotate(10deg);"
    );
    assert_eq!(
        label_style(359),
        "transform:translateX(-50%) rotate(-359deg);"
    );
}

fn fixture_with_element(element: &str) -> String {
    format!(
        "{{\n  \"name\": \"F\",\n  \"tv\": {{\"place\": \"P\", \"status\": {{\"kind\": \"ok\"}}, \"elements\": [\n{element}\n  ]}},\n  \"phone\": {{\"hero\": \"H\", \"status\": {{\"kind\": \"ok\"}}}}\n}}"
    )
}

#[test]
fn the_exact_half_turn_goes_clockwise_from_a_negative_turn_too() {
    assert_eq!(next_turn(-180, 0), 0);
    assert_eq!(next_turn(-180, 180), -180, "already there");
    assert_eq!(next_turn(-90, 90), 90);
    assert_eq!(next_turn(90, 270), 270);
    assert_eq!(
        next_turn(-270, 0),
        -360,
        "90 back is the short way from -270 to a full turn off"
    );
}

#[test]
fn the_turn_wraps_through_zero_in_both_directions_without_a_long_spin() {
    assert_eq!(next_turn(359, 0), 360);
    assert_eq!(next_turn(0, 359), -1);
    assert_eq!(next_turn(1, 359), -1);
    assert_eq!(next_turn(359, 1), 361);
}

#[test]
fn a_figure_that_has_turned_many_times_still_takes_the_short_way_and_ends_at_its_rotation() {
    let mut drawn = first_turn(&turned(0));
    for step in 1..=10_000_u32 {
        let rotation = u16::try_from((step * 7919) % 360).expect("under 360");
        let next = next_turn(drawn, rotation);
        assert!((next - drawn).abs() <= 180, "{drawn} to {next}");
        assert_eq!(next.rem_euclid(360), i64::from(rotation));
        drawn = next;
    }
    // Drift is only whole turns, so the picture is the rotation, and asking for it again is still.
    let rotation = u16::try_from(drawn.rem_euclid(360)).expect("under 360");
    assert_eq!(next_turn(drawn, rotation), drawn);
}

#[test]
fn a_figure_drawn_again_from_nothing_starts_at_its_rotation_with_no_glide_to_take() {
    // A node despawned and spawned again between frames is new, so no earlier turn applies.
    assert_eq!(first_turn(&turned(350)), 350);
    assert_eq!(
        next_turn(10, 350),
        -10,
        "while a figure that was drawn goes the short way"
    );
}

#[test]
fn the_label_turn_cancels_the_figures_turn_at_every_turn() {
    for turn in -720..=720_i64 {
        assert_eq!(turn + label_turn(turn), 0);
        assert!(label_style(turn).contains(&format!("rotate({}deg)", label_turn(turn))));
    }
}

#[test]
fn a_figure_look_matches_only_the_element_it_was_made_from() {
    let item = turned(30);
    let look = FigureLook::of(&item, 30);
    assert!(look.matches(&item, 30));
    assert!(!look.matches(&item, 31), "another turn");
    for changed in [
        StageElement {
            x: 51.0,
            ..item.clone()
        },
        StageElement {
            y: 29.0,
            ..item.clone()
        },
        StageElement {
            width_units: 11.0,
            ..item.clone()
        },
        StageElement {
            height_units: 11.0,
            ..item.clone()
        },
        StageElement {
            shape: ElementShape::Rect,
            ..item.clone()
        },
        StageElement {
            color: None,
            ..item.clone()
        },
        StageElement {
            pivot: StagePivot {
                x_percent: 0,
                y_percent: 50,
            },
            ..item.clone()
        },
    ] {
        assert!(!look.matches(&changed, 30), "{changed:?}");
    }
    let relabelled = StageElement {
        label: "Other".to_owned(),
        ..item.clone()
    };
    assert!(
        look.matches(&relabelled, 30),
        "a label is not part of the style"
    );
}

#[test]
fn the_page_reads_what_the_host_serves_with_and_without_rotation_and_pivot() {
    let full = r##"{"id":"d","kind":"door","label":"Door","x":10.5,"y":20.0,"color":"#aabbcc","shape":"rect",
        "width_units":4.0,"height_units":8.0,"fields":{"portrait":"a.png"},
        "rotation_degrees":90,"pivot":{"x_percent":0,"y_percent":50}}"##;
    let element: StageElement = serde_json::from_str(full).expect("reads");
    assert_eq!(element.rotation_degrees, 90);
    assert_eq!(
        element.pivot,
        StagePivot {
            x_percent: 0,
            y_percent: 50
        }
    );
    assert_eq!(element.portrait(), Some("a.png"));
    let bare = r#"{"id":"d","kind":"door","label":"Door","x":1,"y":2,"color":null,"shape":"circle",
        "width_units":4.0,"height_units":4.0}"#;
    let element: StageElement = serde_json::from_str(bare).expect("reads");
    assert_eq!(element.rotation_degrees, 0);
    assert_eq!(element.pivot, StagePivot::default());
    assert_eq!(element.shape, ElementShape::Circle);
}
