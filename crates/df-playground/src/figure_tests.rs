use crate::figure::{figure_style, first_turn, label_style, next_turn};
use crate::fixture::{ElementShape, StageElement, StagePivot};

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
fn a_pivot_past_the_figure_is_held_on_its_edge() {
    let far = StageElement {
        pivot: StagePivot {
            x_percent: 255,
            y_percent: 101,
        },
        ..pip()
    };
    assert!(figure_style(&far, 0).ends_with("transform-origin:100% 100%;"));
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
fn a_first_turn_past_a_full_turn_is_the_same_picture_as_the_remainder() {
    assert_eq!(first_turn(&turned(359)), 359);
    assert_eq!(first_turn(&turned(360)), 0);
    assert_eq!(first_turn(&turned(725)), 5);
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
