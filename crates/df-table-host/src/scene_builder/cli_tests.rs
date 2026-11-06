use crate::command::{Command, CommandError, parse, split_words};

use super::cli::{BUFFER_USAGE, BUILD_USAGE, PLATES_USAGE, SHOW_USAGE, SceneBuilderCommand};
use super::fixtures::name;
use super::format::{format_buffer, format_build, format_plates, format_run, format_show};
use super::request::{BuildRequest, DevMoment, DevStage, RunRequest, SlowStage};
use super::workbench::Workbench;
use df_table::plate::timing::PlateTiming;

fn command(line: &str) -> Result<SceneBuilderCommand, CommandError> {
    let words = split_words(line).expect("the line splits");
    match parse(&words)? {
        Command::SceneBuilder(command) => Ok(command),
        other => panic!("'{line}' parsed as {other:?}"),
    }
}

#[test]
fn scene_build_takes_a_place_and_the_two_dev_tweaks() {
    assert_eq!(
        command("scene build \"Harbor market\"").expect("parses"),
        SceneBuilderCommand::Build(BuildRequest {
            place: name("Harbor market"),
            fail: None,
            slow: None,
        })
    );
    assert_eq!(
        command("scene build quay fail=plate slow=background:12").expect("parses"),
        SceneBuilderCommand::Build(BuildRequest {
            place: name("quay"),
            fail: Some(DevStage::Plate),
            slow: Some(SlowStage {
                stage: DevStage::Background,
                seconds: 12,
            }),
        })
    );
}

#[test]
fn the_other_verbs_have_their_own_shapes() {
    assert_eq!(
        command("scene show").expect("parses"),
        SceneBuilderCommand::Show
    );
    assert_eq!(
        command("plates").expect("parses"),
        SceneBuilderCommand::Plates
    );
    assert_eq!(
        command("plates run 60").expect("parses"),
        SceneBuilderCommand::PlatesRun(RunRequest {
            seconds: 60,
            quiet: false,
            moment: DevMoment::LineBoundary,
        })
    );
    assert_eq!(
        command("plates run 30 quiet moment=dice_or_result_card").expect("parses"),
        SceneBuilderCommand::PlatesRun(RunRequest {
            seconds: 30,
            quiet: true,
            moment: DevMoment::DiceOrResultCard,
        })
    );
    assert_eq!(
        command("buffer show").expect("parses"),
        SceneBuilderCommand::BufferShow
    );
}

#[test]
fn a_mistake_gets_the_usage_line_or_names_what_was_wrong() {
    let usage = |line: &str| match command(line) {
        Err(CommandError::Usage(text)) => text,
        other => panic!("'{line}' gave {other:?}"),
    };
    assert_eq!(usage("scene build"), BUILD_USAGE);
    assert_eq!(usage("scene build quay surprise=1"), BUILD_USAGE);
    assert_eq!(usage("scene show now"), SHOW_USAGE);
    assert_eq!(usage("plates run"), PLATES_USAGE);
    assert_eq!(usage("plates sometimes"), PLATES_USAGE);
    assert_eq!(usage("buffer"), BUFFER_USAGE);
    assert_eq!(usage("buffer show now"), BUFFER_USAGE);
    assert!(matches!(
        command("scene build quay fail=ready"),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        command("scene build quay slow=plate"),
        Err(CommandError::BadPair(_))
    ));
    assert!(matches!(
        command("scene build quay slow=plate:soon"),
        Err(CommandError::BadCount { .. })
    ));
    assert!(matches!(
        command("scene build quay fail=plate fail=locate"),
        Err(CommandError::DuplicateKey(_))
    ));
    assert!(matches!(
        command("plates run 5 moment=never"),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        command("plates run five"),
        Err(CommandError::BadCount { .. })
    ));
}

#[test]
fn the_playground_scene_verbs_still_parse_as_before() {
    let words = split_words("scene reset").expect("splits");
    assert!(matches!(parse(&words), Ok(Command::Scene(_))));
    let words = split_words("scene caption hello").expect("splits");
    assert!(matches!(parse(&words), Ok(Command::Scene(_))));
}

#[test]
fn help_lists_the_scene_builder_verbs() {
    let help = crate::command::usage_text();
    for usage in [BUILD_USAGE, SHOW_USAGE, PLATES_USAGE, BUFFER_USAGE] {
        assert!(help.contains(usage), "help is missing {usage}");
    }
}

#[test]
fn every_list_in_an_answer_is_numbered_from_one() {
    let mut bench = Workbench::new(PlateTiming::default(), None).expect("a workbench");
    let build = bench
        .build(&BuildRequest {
            place: name("Harbor market"),
            fail: None,
            slow: None,
        })
        .expect("builds");
    let text = format_build(&build);
    assert!(
        text.lines()
            .next()
            .expect("a head")
            .starts_with("scene 1 (generation ")
    );
    let numbered: Vec<&str> = text.lines().skip(1).collect();
    assert!(!numbered.is_empty());
    for (index, line) in numbered.iter().enumerate() {
        assert!(line.starts_with(&format!("{}: ", index + 1)), "{line}");
    }
    for text in [
        format_show(&bench.show()),
        format_plates(&bench.plates()),
        format_buffer(&bench.buffer_show()),
        format_run(
            &bench
                .run(&RunRequest {
                    seconds: 20,
                    quiet: false,
                    moment: DevMoment::LineBoundary,
                })
                .expect("runs"),
        ),
    ] {
        let lines: Vec<&str> = text.lines().filter(|line| line.contains(": ")).collect();
        assert!(lines.iter().any(|line| line.starts_with("1: ")), "{text}");
    }
}

#[test]
fn the_plates_answer_states_the_estimate_where_a_stage_is_unpriced() {
    let mut bench = Workbench::new(PlateTiming::default(), None).expect("a workbench");
    bench
        .build(&BuildRequest {
            place: name("Harbor market"),
            fail: None,
            slow: None,
        })
        .expect("builds");
    let text = format_plates(&bench.plates());
    assert!(
        text.contains("interval 15 s: at most 240 plates an hour"),
        "{text}"
    );
    assert!(
        text.contains("stage plate (unpriced): 240 calls an hour"),
        "{text}"
    );
    assert!(
        text.contains(
            "stage locate ($0.01 a call): 960 to 2160 calls an hour, $9.60 to $21.60 an hour"
        ),
        "{text}"
    );
    assert!(
        text.contains("estimate: the priced stages come to $9.60 to $21.60 an hour"),
        "{text}"
    );
}
