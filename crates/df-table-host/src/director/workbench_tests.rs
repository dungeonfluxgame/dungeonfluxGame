//! The Director on the host without a host: the manual clock, the stand-in view and the audit
//! lines, plus the checks that the Director's numbers are the dev camera verbs' numbers.

use std::sync::Arc;

use df_playground::camera::{
    clamped_zoom as page_clamped_zoom, zoom_ceiling_percent as page_ceiling,
};
use df_playground::limits::{MAXIMUM_CAMERA_MS, MAXIMUM_ZOOM_PERCENT, MINIMUM_ZOOM_PERCENT};
use df_playground::scene::SceneChange;
use df_table::director::StageDirection;
use df_table::director::error::DirectorError as DecisionError;
use df_table::director::reason::{self as director_reason, Reason, ReasonTable, WideReason};
use df_table::director::view::{clamped_zoom, zoom_ceiling_percent};
use df_table::element::ElementId;
use df_table::inspect::InspectAudience;
use df_table::testkit::ManualClock;

use super::request::{DirectorAct, DirectorError, StartKind, ViewEdit};
use super::workbench::{Decided, Workbench, scene_change};

fn manual(audience: InspectAudience) -> Workbench {
    Workbench::new(audience, Arc::new(ManualClock::default()))
}

fn id(name: &str) -> ElementId {
    ElementId::new(name).expect("an element name")
}

fn bench(audience: InspectAudience) -> Workbench {
    let mut bench = manual(audience);
    bench.refresh_stage(&[
        "finn".to_owned(),
        "sigrun".to_owned(),
        "guard".to_owned(),
        "lurker".to_owned(),
    ]);
    bench
}

fn focus(element: &str, reason: &str) -> DirectorAct {
    DirectorAct::Focus {
        element: id(element),
        reason: Some(Reason::from_key(reason).expect("a reason")),
        zoom_percent: None,
        over_ms: None,
        hold_ms: None,
    }
}

fn view(edit: ViewEdit) -> DirectorAct {
    DirectorAct::View { edit }
}

fn run(bench: &mut Workbench, act: DirectorAct) -> Decided {
    bench.act(act).expect("the verb is accepted")
}

#[test]
fn the_directors_numbers_are_the_dev_camera_verbs_numbers() {
    assert_eq!(director_reason::MINIMUM_ZOOM_PERCENT, MINIMUM_ZOOM_PERCENT);
    assert_eq!(director_reason::MAXIMUM_ZOOM_PERCENT, MAXIMUM_ZOOM_PERCENT);
    assert_eq!(director_reason::MAXIMUM_CAMERA_MS, MAXIMUM_CAMERA_MS);
    // The ceiling and the cut are the page's own functions, on a sweep of plates and stages.
    let widths = [
        None,
        Some(0),
        Some(1),
        Some(799),
        Some(1_920),
        Some(65_535),
        Some(u32::MAX),
    ];
    for plate in widths {
        for stage in [0, 1, 400, 1_920, 3_840, u32::MAX] {
            assert_eq!(
                zoom_ceiling_percent(plate, stage),
                page_ceiling(plate, stage),
                "plate {plate:?} on a stage of {stage} px"
            );
        }
    }
    for requested in [100, 150, 200, 210, 400, 10_000] {
        for ceiling in [100, 200, 210, 10_000] {
            assert_eq!(
                clamped_zoom(requested, ceiling),
                page_clamped_zoom(requested, ceiling)
            );
        }
    }
}

#[test]
fn every_move_the_director_can_make_is_a_camera_change_the_dev_verbs_accept() {
    let table = ReasonTable::default();
    for reason in Reason::ALL {
        let spec = table.spec(*reason);
        assert!(
            (MINIMUM_ZOOM_PERCENT..=MAXIMUM_ZOOM_PERCENT).contains(&spec.zoom_percent())
                && spec.over_ms() <= MAXIMUM_CAMERA_MS,
            "{reason}"
        );
    }
    let focus = StageDirection::Focus {
        element: ElementId::new("finn").expect("an id"),
        reason: Reason::Speaker,
        zoom_percent: 200,
        clamped_from: None,
        over_ms: 1_200,
        serial: 1,
        driven: true,
    };
    assert_eq!(
        scene_change(&focus),
        SceneChange::Camera {
            target: Some("finn".to_owned()),
            zoom_percent: 200,
            duration_ms: 1_200
        }
    );
    let wide = StageDirection::Wide {
        reason: WideReason::Timeout,
        over_ms: 900,
        serial: 2,
        driven: false,
    };
    assert_eq!(
        scene_change(&wide),
        SceneChange::Camera {
            target: None,
            zoom_percent: 100,
            duration_ms: 900
        }
    );
}

#[test]
fn a_focus_verb_is_one_driven_move_with_its_audit_line() {
    let mut bench = bench(InspectAudience::Operator);
    let decided = run(&mut bench, focus("finn", "speaker"));
    assert_eq!(decided.directions.len(), 1);
    assert_eq!(
        decided.lines.last().map(String::as_str),
        Some("CAMERA focus element=finn zoom=200 over=1200 reason=speaker serial=1 driven=true")
    );
}

#[test]
fn advance_moves_the_directors_own_clock_so_a_hold_runs_out_with_no_waiting() {
    let mut bench = bench(InspectAudience::Operator);
    run(&mut bench, focus("guard", "reveal"));
    let decided = run(&mut bench, DirectorAct::Advance { by_ms: 7_999 });
    assert!(decided.directions.is_empty());
    let decided = run(&mut bench, DirectorAct::Advance { by_ms: 1 });
    assert!(matches!(
        decided.directions.as_slice(),
        [StageDirection::Wide {
            reason: WideReason::Timeout,
            ..
        }]
    ));
    assert_eq!(
        decided.lines,
        ["CAMERA wide reason=timeout serial=2 over=1200"]
    );
    assert_eq!(
        bench
            .act(DirectorAct::Advance {
                by_ms: MAXIMUM_CAMERA_MS + 1
            })
            .err(),
        Some(DirectorError::InvalidAdvance {
            by_ms: MAXIMUM_CAMERA_MS + 1
        })
    );
}

#[test]
fn a_waiting_direction_starts_when_the_spacing_has_passed() {
    let mut bench = bench(InspectAudience::Operator);
    run(&mut bench, focus("finn", "speaker"));
    let decided = run(&mut bench, focus("sigrun", "speaker"));
    assert!(decided.directions.is_empty());
    assert!(bench.is_busy());
    run(&mut bench, DirectorAct::Advance { by_ms: 799 });
    let decided = run(&mut bench, DirectorAct::Advance { by_ms: 1 });
    assert!(matches!(
        decided.directions.as_slice(),
        [StageDirection::Focus { serial: 2, .. }]
    ));
}

#[test]
fn a_hidden_element_is_refused_and_a_focus_on_one_that_becomes_hidden_is_pulled_back() {
    let mut bench = bench(InspectAudience::Operator);
    run(
        &mut bench,
        view(ViewEdit::Hide {
            element: id("lurker"),
        }),
    );
    let decided = run(&mut bench, focus("lurker", "speaker"));
    assert!(decided.directions.is_empty());
    assert!(
        decided.lines.contains(
            &"CAMERA dropped reason=target_hidden direction=speaker element=lurker driven=true"
                .to_owned()
        ),
        "{:?}",
        decided.lines
    );
    run(&mut bench, focus("guard", "speaker"));
    let decided = run(
        &mut bench,
        view(ViewEdit::Hide {
            element: id("guard"),
        }),
    );
    assert!(matches!(
        decided.directions.as_slice(),
        [StageDirection::Wide {
            reason: WideReason::TargetHidden,
            ..
        }]
    ));
    // A hero is focusable whether or not the table can see it.
    run(
        &mut bench,
        view(ViewEdit::Hide {
            element: id("finn"),
        }),
    );
    run(
        &mut bench,
        view(ViewEdit::Hero {
            element: id("finn"),
        }),
    );
    run(&mut bench, DirectorAct::Advance { by_ms: 1_000 });
    let decided = run(&mut bench, focus("finn", "spotlight"));
    assert_eq!(decided.directions.len(), 1);
}

#[test]
fn a_hosted_build_names_nothing_that_is_hidden_in_its_lines_or_in_director_show() {
    let mut bench = bench(InspectAudience::Host);
    run(
        &mut bench,
        view(ViewEdit::Hide {
            element: id("lurker"),
        }),
    );
    let decided = run(&mut bench, focus("lurker", "speaker"));
    let all = decided.lines.join("\n");
    assert!(!all.contains("lurker"), "{all}");
    assert!(
        all.contains("CAMERA dropped reason=target_hidden direction=speaker"),
        "{all}"
    );
    let decided = run(&mut bench, focus("nobody", "speaker"));
    let all = decided.lines.join("\n");
    assert!(
        !all.contains("nobody") && all.contains("reason=target_hidden"),
        "an unknown name reads as hidden: {all}"
    );
    let show = bench.show();
    assert!(show.view.hidden.is_empty());
    let text = serde_json::to_string(&show).expect("serialises");
    assert!(
        !text.contains("lurker") && !text.contains("nobody"),
        "{text}"
    );
    let printed = super::format::format_show(&show);
    assert!(
        !printed.contains("lurker") && !printed.contains("nobody"),
        "{printed}"
    );
    // The operator's build, given the same calls, reads the names.
    let mut operator = bench_with_hidden();
    let decided = run(&mut operator, focus("lurker", "speaker"));
    assert!(decided.lines.join("\n").contains("element=lurker"));
    assert_eq!(operator.show().view.hidden, ["lurker"]);
}

fn bench_with_hidden() -> Workbench {
    let mut bench = bench(InspectAudience::Operator);
    run(
        &mut bench,
        view(ViewEdit::Hide {
            element: id("lurker"),
        }),
    );
    bench
}

#[test]
fn a_card_declines_every_focus_and_a_plate_sets_the_zoom_ceiling() {
    let mut bench = bench(InspectAudience::Operator);
    run(&mut bench, view(ViewEdit::Card));
    let decided = run(&mut bench, focus("finn", "hit"));
    assert!(decided.directions.is_empty());
    assert!(
        decided
            .lines
            .iter()
            .any(|line| line.contains("reason=no_target"))
    );
    run(
        &mut bench,
        view(ViewEdit::Plate {
            width_px: Some(800),
            stage_px: Some(400),
        }),
    );
    let mut request = focus("finn", "speaker");
    if let DirectorAct::Focus { zoom_percent, .. } = &mut request {
        *zoom_percent = Some(400);
    }
    let decided = run(&mut bench, request);
    assert!(matches!(
        decided.directions.as_slice(),
        [StageDirection::Focus {
            zoom_percent: 200,
            clamped_from: Some(400),
            ..
        }]
    ));
    assert!(
        decided
            .lines
            .contains(&"CAMERA clamped requested=400 allowed=200 serial=1".to_owned())
    );
    assert_eq!(bench.show().report.ceiling_percent, 200);
}

#[test]
fn a_result_card_holds_the_camera_still_for_everything_not_about_it() {
    let mut bench = bench(InspectAudience::Operator);
    run(
        &mut bench,
        view(ViewEdit::Beat {
            reveal: false,
            hold_ms: 3_000,
            element: Some(id("finn")),
        }),
    );
    let decided = run(&mut bench, focus("sigrun", "roll"));
    assert!(decided.directions.is_empty());
    let decided = run(&mut bench, DirectorAct::Advance { by_ms: 2_999 });
    assert!(decided.directions.is_empty());
    let decided = run(&mut bench, DirectorAct::Advance { by_ms: 1 });
    assert!(matches!(
        decided.directions.as_slice(),
        [StageDirection::Focus { .. }]
    ));
}

#[test]
fn pause_and_auto_leave_the_operators_own_direction_working() {
    let mut bench = bench(InspectAudience::Operator);
    run(&mut bench, DirectorAct::Pause);
    let decided = run(&mut bench, focus("finn", "speaker"));
    assert_eq!(
        decided.directions.len(),
        1,
        "a driven direction is not held"
    );
    run(&mut bench, DirectorAct::Resume);
    run(&mut bench, DirectorAct::Auto { on: false });
    assert!(!bench.show().report.auto_camera);
}

#[test]
fn end_scene_and_combat_return_the_camera_wide_and_wide_pulls_out_at_once() {
    let mut bench = bench(InspectAudience::Operator);
    run(&mut bench, focus("finn", "speaker"));
    let decided = run(
        &mut bench,
        DirectorAct::End {
            reason: Reason::Speaker,
            element: id("finn"),
        },
    );
    assert!(matches!(
        decided.directions.as_slice(),
        [StageDirection::Wide {
            reason: WideReason::ReasonEnded,
            ..
        }]
    ));
    run(&mut bench, DirectorAct::Advance { by_ms: 1_000 });
    run(&mut bench, focus("sigrun", "speaker"));
    let decided = run(
        &mut bench,
        DirectorAct::Start {
            what: StartKind::Combat,
        },
    );
    assert!(matches!(
        decided.directions.as_slice(),
        [StageDirection::Wide {
            reason: WideReason::CombatStart,
            ..
        }]
    ));
    run(&mut bench, DirectorAct::Advance { by_ms: 1_000 });
    run(&mut bench, focus("sigrun", "speaker"));
    let decided = run(&mut bench, DirectorAct::Wide { over_ms: Some(300) });
    assert_eq!(
        decided.lines.last().map(String::as_str),
        Some("CAMERA wide reason=operator serial=6 over=300 driven=true")
    );
    run(&mut bench, DirectorAct::Advance { by_ms: 1_000 });
    run(&mut bench, focus("sigrun", "speaker"));
    let decided = run(
        &mut bench,
        DirectorAct::Start {
            what: StartKind::Scene,
        },
    );
    assert!(matches!(
        decided.directions.as_slice(),
        [StageDirection::Wide {
            reason: WideReason::SceneStart,
            over_ms: 0,
            ..
        }]
    ));
}

#[test]
fn a_bad_name_reason_or_number_is_a_typed_refusal_that_says_what_is_allowed() {
    for body in [
        r#"{"verb":"focus","element":"Finn!"}"#,
        r#"{"verb":"focus","element":"finn","reason":"zoomies"}"#,
        r#"{"verb":"end","reason":"speaker","element":"Finn!"}"#,
    ] {
        assert!(
            serde_json::from_str::<DirectorAct>(body).is_err(),
            "{body} is not a verb"
        );
    }
    let mut bench = bench(InspectAudience::Operator);
    let mut request = focus("finn", "speaker");
    if let DirectorAct::Focus { over_ms, .. } = &mut request {
        *over_ms = Some(10);
    }
    let Err(error) = bench.act(request) else {
        panic!("a move too quick to follow is refused");
    };
    assert_eq!(
        error,
        DirectorError::Refused {
            cause: DecisionError::FocusMoveOutOfRange { over_ms: 10 }
        }
    );
    assert!(error.to_string().contains("600 to 2500"), "{error}");
    let mut request = focus("finn", "speaker");
    if let DirectorAct::Focus { zoom_percent, .. } = &mut request {
        *zoom_percent = Some(MAXIMUM_ZOOM_PERCENT + 1);
    }
    assert_eq!(
        bench.act(request).err(),
        Some(DirectorError::Refused {
            cause: DecisionError::ZoomOutOfRange {
                zoom_percent: MAXIMUM_ZOOM_PERCENT + 1
            }
        })
    );
}

#[test]
fn director_show_reads_the_same_log_the_lines_were_written_from() {
    let mut bench = bench(InspectAudience::Operator);
    let mut written = Vec::new();
    written.extend(run(&mut bench, focus("finn", "speaker")).lines);
    written.extend(run(&mut bench, focus("sigrun", "requested")).lines);
    run(&mut bench, DirectorAct::Advance { by_ms: 1_000 });
    let printed = super::format::format_show(&bench.show());
    for line in &written {
        assert!(printed.contains(line.as_str()), "{line}\n{printed}");
    }
    assert!(printed.contains("holding: finn for speaker"), "{printed}");
    assert!(printed.contains("auto camera on"), "{printed}");
}

fn beat_about_lurker() -> ViewEdit {
    ViewEdit::Beat {
        reveal: false,
        hold_ms: 3_000,
        element: Some(id("lurker")),
    }
}

/// Everything a shared reader of this workbench could be shown: the lines of every verb run, the
/// report as JSON and `director show` as printed.
fn shared_text(bench: &Workbench, lines: &[String]) -> String {
    let show = bench.show();
    format!(
        "{}\n{}\n{}",
        lines.join("\n"),
        serde_json::to_string(&show).expect("serialises"),
        super::format::format_show(&show)
    )
}

#[test]
fn a_hidden_id_never_reaches_a_shared_line_report_or_show_whatever_held_the_request_back() {
    let hide = || {
        view(ViewEdit::Hide {
            element: id("lurker"),
        })
    };
    // The Director sees the creature while it asks, and the table loses sight of it after.
    let scenarios: [(&str, Vec<DirectorAct>); 5] = [
        ("pause", vec![DirectorAct::Pause]),
        ("auto off", vec![DirectorAct::Auto { on: false }]),
        ("card", vec![view(ViewEdit::Card)]),
        (
            "end",
            vec![DirectorAct::End {
                reason: Reason::Speaker,
                element: id("lurker"),
            }],
        ),
        (
            "start scene",
            vec![DirectorAct::Start {
                what: StartKind::Scene,
            }],
        ),
    ];
    for (name, acts) in scenarios {
        let mut bench = bench(InspectAudience::Host);
        let mut lines = Vec::new();
        lines.extend(run(&mut bench, hide()).lines);
        lines.extend(run(&mut bench, view(beat_about_lurker())).lines);
        for act in acts {
            lines.extend(run(&mut bench, act).lines);
            lines.extend(run(&mut bench, focus("lurker", "speaker")).lines);
        }
        let text = shared_text(&bench, &lines);
        assert!(
            !text.contains("lurker"),
            "{name} named the hidden id:\n{text}"
        );
    }
}

#[test]
fn a_beat_about_a_hidden_creature_names_it_only_to_the_operator() {
    let mut shared = bench(InspectAudience::Host);
    run(&mut shared, view(beat_about_lurker()));
    run(
        &mut shared,
        view(ViewEdit::Hide {
            element: id("lurker"),
        }),
    );
    let beat = shared.show().view.beat.expect("a beat");
    assert!(
        !beat.contains("lurker") && beat.starts_with("result card until"),
        "{beat}"
    );
    let mut operator = bench(InspectAudience::Operator);
    run(&mut operator, view(beat_about_lurker()));
    let beat = operator.show().view.beat.expect("a beat");
    assert!(beat.contains("about lurker"), "{beat}");
    // A creature the table may see is named, hidden or not for the operator.
    run(
        &mut shared,
        view(ViewEdit::Unhide {
            element: id("lurker"),
        }),
    );
    let beat = shared.show().view.beat.expect("a beat");
    assert!(beat.contains("about lurker"), "{beat}");
}

#[test]
fn a_request_in_the_inbox_is_decided_by_the_host_tick_alone() {
    let mut bench = bench(InspectAudience::Operator);
    assert!(!bench.is_busy());
    let view = bench.view();
    let now = bench.director_mut().now_ms();
    let element = ElementId::new("finn").expect("a valid id");
    bench
        .director_mut()
        .request(
            now,
            &view,
            df_table::director::Request::focus(Reason::Speaker, element),
        )
        .expect("well formed");
    assert!(bench.is_busy(), "a queued request is work for the tick");
    let decided = bench.tick().expect("ticks");
    assert!(
        matches!(
            decided.directions.as_slice(),
            [StageDirection::Focus { .. }]
        ),
        "{decided:?}"
    );
}

#[test]
fn a_stage_name_that_is_not_an_element_name_is_reported_once_and_left_out() {
    let mut bench = manual(InspectAudience::Operator);
    let ids = ["finn".to_owned(), "Not A Name".to_owned()];
    let line = bench.refresh_stage(&ids).expect("a line");
    assert_eq!(
        line,
        "CAMERA stage_names_refused count=1 names=[\"Not A Name\"]"
    );
    assert_eq!(
        bench.refresh_stage(&ids),
        None,
        "the same names say nothing again"
    );
    assert_eq!(bench.refresh_stage(&["finn".to_owned()]), None);
    let mut shared = manual(InspectAudience::Host);
    assert_eq!(
        shared.refresh_stage(&ids).as_deref(),
        Some("CAMERA stage_names_refused count=1")
    );
}

#[test]
fn the_operators_camera_wide_is_a_cut_under_reduced_effects() {
    let mut bench = bench(InspectAudience::Operator);
    run(&mut bench, view(ViewEdit::Reduced { on: true }));
    run(&mut bench, focus("finn", "speaker"));
    let decided = run(&mut bench, DirectorAct::Wide { over_ms: None });
    assert!(
        decided
            .lines
            .iter()
            .any(|line| line == "CAMERA wide reason=operator serial=2 over=0 driven=true"),
        "{:?}",
        decided.lines
    );
    run(&mut bench, view(ViewEdit::Reduced { on: false }));
    run(&mut bench, focus("finn", "speaker"));
    let decided = run(&mut bench, DirectorAct::Wide { over_ms: None });
    assert!(
        decided
            .lines
            .iter()
            .any(|line| line.contains("reason=operator") && line.contains("over=1200")),
        "{:?}",
        decided.lines
    );
}

#[test]
fn a_camera_moved_elsewhere_makes_the_next_focus_a_move() {
    let mut bench = bench(InspectAudience::Operator);
    run(&mut bench, focus("finn", "speaker"));
    assert!(!bench.camera_moved_elsewhere(Some("finn")).expect("ok"));
    assert!(bench.camera_moved_elsewhere(None).expect("ok"));
    run(&mut bench, DirectorAct::Advance { by_ms: 1_000 });
    let decided = run(&mut bench, focus("finn", "speaker"));
    assert!(
        decided
            .lines
            .iter()
            .any(|line| line.starts_with("CAMERA focus element=finn")),
        "{:?}",
        decided.lines
    );
}
