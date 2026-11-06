//! Inspect reports and the verb set: what each report holds for the operator, that equal state
//! gives equal reports, that `describe` is read off the registry and that each report's field
//! table matches the report it names. What a seat or the host may not see is in
//! `inspect_redaction_tests`.

use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;

use crate::config::Settings;
use crate::conformance::{self, ConformanceCheck};
use crate::effects::TableEffect;
use crate::golden::{Leaves, World};
use crate::inspect::{
    Frame, Inspect, InspectAudience, InspectView, MachineStanding, MachineSupport, MachinesOf,
    Needs, OwnerQueueReport, PendingReport, ReportName, Verb, VerbAccess, WiredPorts, describe,
};
use crate::lobby_tests::{lobby, lobby_fixture};
use crate::machine::slot::{LoadError, MachineSlot};
use crate::machine::{
    Entry, HandOff, InspectReport, Machine, MachineError, MachineInput, MachineReadout,
    MachineSpec, ReadoutError, StateName, StepInput, StepResult,
};
use crate::owner_queue::OwnerQueue;
use crate::owner_queue_tests::{NullSink, seated_queue, settings, tap};
use crate::pipeline_fixtures_tests::{action, allows, window_of, world_of};
use crate::ports::{Audience, PortName, Preparation, Resolution};
use crate::registry::{FlagState, MachineFlags, RegistryBuilder};
use crate::table_state::{MachineHome, TableState};
use crate::table_state_tests::{apply, creature, element, hero, machine, scene, seat};
use crate::testkit::ScriptedRulesOwner;

pub(crate) const NOW_MS: u64 = 5_000;

pub(crate) fn view_of(audience: InspectAudience) -> InspectView {
    InspectView::new(audience, NOW_MS)
}

pub(crate) fn operator() -> InspectView {
    view_of(InspectAudience::Operator)
}

pub(crate) fn host() -> InspectView {
    view_of(InspectAudience::Host)
}

pub(crate) fn seat_view(number: u64) -> InspectView {
    view_of(InspectAudience::Seat { seat: seat(number) })
}

/// Everything a reader of the report could see: its JSON and its Debug text.
pub(crate) fn everything(report: &(impl Serialize + std::fmt::Debug)) -> String {
    format!(
        "{}\n{report:?}",
        serde_json::to_string(report).expect("a report serialises")
    )
}

pub(crate) fn json_of(report: &impl Serialize) -> Value {
    serde_json::to_value(report).expect("a report serialises")
}

/// A table of three seats in scene 10 with a goblin, and an ogre when `with_ogre` that nobody can
/// see. Both tables take the same number of commits, so their revisions agree.
pub(crate) fn table_of_three(with_ogre: bool) -> TableState {
    let mut table = TableState::seated(6, &[seat(1), seat(2), seat(3)]).expect("valid table");
    let mut setup = vec![TableEffect::SceneEntered { scene: scene(10) }];
    for number in 1..=3 {
        setup.push(TableEffect::CastEntered {
            entry: hero(
                &format!("hero{number}"),
                &format!("Hero{number}"),
                number,
                10,
                number as u32 * 20,
            ),
        });
    }
    setup.push(TableEffect::CastEntered {
        entry: creature("goblin", "Goblin", 10, 80),
    });
    apply(&mut table, 1, &setup).expect("the table is set");
    let hidden = if with_ogre {
        vec![
            TableEffect::CastEntered {
                entry: creature("ogre", crate::inspect_redaction_tests::HIDDEN_NAME, 10, 50),
            },
            TableEffect::CastChanged {
                cast: element("ogre"),
                change: crate::table_state::CastChange::Visibility {
                    visibility: crate::table_state::Visibility::Hidden,
                },
            },
        ]
    } else {
        Vec::new()
    };
    apply(&mut table, 2, &hidden).expect("the turn applies");
    table
}

/// A world whose registry holds the lobby, explore and the scene machine.
pub(crate) fn scene_world() -> World {
    let mut world = World::with_scene(&[("strike", Leaves::Effect)], 3, None).expect("builds");
    let rules = allows(ScriptedRulesOwner::default(), "strike");
    world.rules = rules;
    world
}

#[test]
fn the_operator_report_of_the_table_has_the_revision_status_scenes_seats_and_all_the_cast() {
    let table = table_of_three(true);

    let report = table.inspect(operator());

    assert_eq!(report.seats.len(), 3);
    assert_eq!(report.scenes.len(), 1);
    assert_eq!(report.focus_scene, Some(scene(10)));
    assert_eq!(report.seat_capacity, 6);
    assert_eq!(
        report.cast.len(),
        5,
        "three heroes, the goblin and the ogre"
    );
    assert_eq!(report.digest.shown(), Some(&table.digest()));
    assert_eq!(report.revision, table.snapshot().revision());
}

#[test]
fn equal_state_gives_equal_reports_for_every_audience() {
    let first = table_of_three(true);
    let second = table_of_three(true);

    for audience in [
        InspectAudience::Operator,
        InspectAudience::Host,
        InspectAudience::Seat { seat: seat(2) },
    ] {
        assert_eq!(
            first.inspect(view_of(audience)),
            second.inspect(view_of(audience))
        );
        assert_eq!(
            first.inspect(view_of(audience)),
            first.inspect(view_of(audience)),
            "asking twice changes nothing"
        );
    }
}

#[test]
fn the_settings_report_lists_every_setting_with_its_value_default_and_range() {
    let settings = Settings::default();

    let report = settings.inspect(seat_view(1));

    assert_eq!(report.settings.len(), crate::config::SettingId::ALL.len());
    for line in &report.settings {
        assert_eq!(line.value, line.default);
        assert!(line.minimum <= line.default && line.default <= line.maximum);
    }
}

#[test]
fn the_queue_report_counts_lanes_seats_and_what_is_in_progress() {
    let mut queue = seated_queue(2, &settings(4, 4, 1), Arc::new(NullSink));
    queue.admit(tap(1, 1), 100).expect("admitted");
    queue.admit(tap(2, 2), 100).expect("admitted");
    let served = queue.serve_next(200).expect("serves").expect("one waits");

    let report: OwnerQueueReport =
        queue.inspect(InspectView::new(InspectAudience::Operator, 1_200));

    let depths: Vec<usize> = report
        .lanes
        .shown()
        .expect("the operator sees the lanes")
        .iter()
        .map(|lane| lane.depth)
        .collect();
    assert_eq!(depths, [0, 0, 0, 1], "one player input is still waiting");
    assert_eq!(report.in_progress.len(), 1);
    assert_eq!(report.in_progress[0].turn_seq, served.turn_seq);
    assert_eq!(report.in_progress[0].age_ms, 1_000);
    assert_eq!(report.seats.len(), 2);
    assert_eq!(report.last_turn_seq.shown(), Some(&served.turn_seq));
}

/// The lobby, explore and the scene machine, with the scene machine switched off.
fn registry_with_scene_off() -> crate::registry::Registry {
    let scene_machine =
        crate::golden::SceneMachine::new(&[("strike", Leaves::Effect)]).expect("a valid spec");
    RegistryBuilder::new()
        .register(lobby())
        .register(crate::lobby_tests::explore_for_neighbour())
        .register(scene_machine)
        .with_flags(MachineFlags::default().with_flag(machine("scene"), FlagState::Off))
        .build(PortName::ALL)
        .expect("a startable registry")
}

#[test]
fn the_registry_report_lists_every_machine_with_its_spec_and_leaves_a_disabled_one_without() {
    let registry = registry_with_scene_off();

    let report = registry.inspect(operator());

    let scene_line = report
        .machines
        .iter()
        .find(|line| line.machine == machine("scene"))
        .expect("the scene machine is listed");
    assert_eq!(
        (scene_line.enablement, scene_line.reason),
        ("disabled", "flagged_off")
    );
    assert_eq!(scene_line.spec.shown(), Some(&None));
    let lobby_line = report
        .machines
        .iter()
        .find(|line| line.machine == machine("lobby"))
        .expect("lobby is listed");
    let spec = lobby_line
        .spec
        .shown()
        .and_then(Option::as_ref)
        .expect("an enabled machine has a spec line");
    assert_eq!(spec.schema_version, lobby().spec().schema_version());
    assert_eq!(spec.states, lobby().spec().states());
}

#[test]
fn the_ports_report_says_which_ports_are_wired_and_which_machines_need_them() {
    let world = scene_world();
    let wired = [PortName::Broker, PortName::Dice];

    let report = WiredPorts {
        registry: &world.registry,
        wired: &wired,
    }
    .inspect(host());

    let ports = report.ports.shown().expect("the host sees the wiring");
    assert_eq!(ports.len(), PortName::ALL.len());
    let rules = ports
        .iter()
        .find(|line| line.port == PortName::RulesOwner)
        .expect("listed");
    assert!(!rules.wired);
    assert_eq!(rules.required_by, [machine("explore"), machine("scene")]);
    let broker = ports
        .iter()
        .find(|line| line.port == PortName::Broker)
        .expect("listed");
    assert!(broker.wired && broker.required_by.is_empty());
}

/// A machine slot for the scene machine with a note the machine never decodes.
pub(crate) fn scene_slot_with_note(note: &str) -> MachineSlot {
    MachineSlot::from_parts(
        machine("scene"),
        1,
        serde_json::json!({"phase": "open", "counters": {"turns": 0, "ticks": 0, "resources": 0,
            "proposals": 0}, "note": note}),
    )
}

pub(crate) fn table_with_machines(note: &str) -> TableState {
    let mut table = table_of_three(false);
    apply(
        &mut table,
        3,
        &[
            TableEffect::ActiveMachineSet {
                home: MachineHome::Table,
                machine: machine("lobby"),
                slot: MachineSlot::from_parts(
                    machine("lobby"),
                    1,
                    serde_json::json!({"phase": "gathering", "ready": []}),
                ),
            },
            TableEffect::ActiveMachineSet {
                home: MachineHome::Scene { scene: scene(10) },
                machine: machine("scene"),
                slot: scene_slot_with_note(note),
            },
        ],
    )
    .expect("the machines start");
    table
}

#[test]
fn the_machines_report_reads_each_slot_through_its_machines_own_inspect() {
    let world = scene_world();
    let table = table_with_machines("a note");
    let snapshot = table.snapshot();

    let report = MachinesOf {
        registry: &world.registry,
        snapshot: &snapshot,
    }
    .inspect(operator());

    assert_eq!(report.machines.len(), 2);
    let scene_line = report
        .machines
        .iter()
        .find(|line| line.machine == machine("scene"))
        .expect("the scene machine runs");
    assert!(scene_line.focus && scene_line.frame == Frame::Active);
    match &scene_line.standing {
        MachineStanding::Reported {
            state,
            schema_version,
            payload,
        } => {
            assert_eq!(state.as_str(), "open");
            assert_eq!(*schema_version, 1);
            assert_eq!(
                payload.shown().expect("the operator sees it")["note"],
                "a note"
            );
        }
        other => panic!("expected a report, got {other:?}"),
    }
    let lobby_line = report
        .machines
        .iter()
        .find(|line| line.machine == machine("lobby"))
        .expect("the lobby is the table machine");
    assert!(
        !lobby_line.focus,
        "the scene's machine is the one the screen names"
    );
}

#[test]
fn a_machine_that_is_not_registered_is_reported_as_not_available_and_not_dropped() {
    let registry = RegistryBuilder::new()
        .register(lobby())
        .register(crate::lobby_tests::explore_for_neighbour())
        .build(PortName::ALL)
        .expect("a startable registry");
    let table = table_with_machines("a note");
    let snapshot = table.snapshot();

    let report = MachinesOf {
        registry: &registry,
        snapshot: &snapshot,
    }
    .inspect(operator());

    let missing = report
        .machines
        .iter()
        .find(|line| line.machine == machine("scene"))
        .expect("the scene slot is still listed");
    assert_eq!(missing.standing, MachineStanding::NotAvailable);
}

/// The lobby with no report: its `inspect` fails, as a machine that forgot it would.
struct NoReport(crate::lobby::LobbyMachine);

impl Machine for NoReport {
    fn spec(&self) -> &MachineSpec {
        self.0.spec()
    }

    fn enter(&self, table: &TableState, entry: &Entry) -> Result<MachineSlot, MachineError> {
        self.0.enter(table, entry)
    }

    fn accepts(&self, table: &TableState, input: &MachineInput, state: &MachineSlot) -> bool {
        self.0.accepts(table, input, state)
    }

    fn step(&self, state: &MachineSlot, input: &StepInput<'_>) -> Result<StepResult, MachineError> {
        self.0.step(state, input)
    }

    fn wants_hand_off(&self, state: &MachineSlot) -> Result<bool, MachineError> {
        self.0.wants_hand_off(state)
    }

    fn exit(&self, table: &TableState, state: &MachineSlot) -> Result<HandOff, MachineError> {
        self.0.exit(table, state)
    }

    fn state_name(&self, state: &MachineSlot) -> Result<StateName, MachineError> {
        self.0.state_name(state)
    }

    fn validate(&self, state: &MachineSlot) -> Result<(), MachineError> {
        self.0.validate(state)
    }

    fn project(
        &self,
        table: &TableState,
        state: &MachineSlot,
        audience: Audience,
    ) -> Result<MachineReadout, MachineError> {
        self.0.project(table, state, audience)
    }

    fn inspect(&self, _state: &MachineSlot) -> Result<InspectReport, MachineError> {
        Err(MachineError::Projection(ReadoutError::Blank))
    }

    fn migrate(&self, from: &MachineSlot) -> Result<MachineSlot, LoadError> {
        self.0.migrate(from)
    }
}

#[test]
fn a_machine_without_a_report_fails_the_conformance_suite_and_shows_as_failed_in_the_report() {
    let mut fixture = lobby_fixture();
    fixture.machine = Arc::new(NoReport(lobby()));

    let conformance = conformance::run(fixture);

    assert!(
        conformance
            .failed_checks()
            .contains(&ConformanceCheck::InspectReport),
        "findings: {:?}",
        conformance.findings()
    );
    let registry = RegistryBuilder::new()
        .register(NoReport(lobby()))
        .register(crate::lobby_tests::explore_for_neighbour())
        .build(PortName::ALL)
        .expect("a startable registry");
    let table = table_with_machines("a note");
    let snapshot = table.snapshot();
    let report = MachinesOf {
        registry: &registry,
        snapshot: &snapshot,
    }
    .inspect(operator());
    let line = report
        .machines
        .iter()
        .find(|line| line.machine == machine("lobby"))
        .expect("the lobby slot is listed");
    assert!(matches!(
        line.standing,
        MachineStanding::ReportFailed { .. }
    ));
}

#[test]
fn the_pending_report_lists_an_open_pending_with_its_offers_and_waiting_time() {
    let mut world = world_of(&["strike"], 2, strike_rules());
    world.dice = crate::golden::CountingDice::new([19]);
    world.clock.advance_ms(0);
    world.tap(1, "strike").expect("the strike runs");

    let report: PendingReport = world.table.pending().inspect(operator());

    assert_eq!(report.pendings.len(), 1);
    let line = &report.pendings[0];
    assert_eq!(line.opened_by, seat(1));
    assert_eq!(line.offers.len(), 1);
    assert_eq!(line.draws.shown().map(Vec::len), Some(1));
    assert!(line.continuation.is_shown());
}

pub(crate) fn strike_rules() -> ScriptedRulesOwner {
    use crate::pipeline_fixtures_tests::d20;
    ScriptedRulesOwner::default()
        .with_action(
            action("strike"),
            Preparation::Ready { draws: vec![d20()] },
            Resolution::applied(),
        )
        .with_resolutions(
            action("strike"),
            vec![Resolution::Await(crate::ports::Awaiting {
                kind: crate::ports::AwaitingKind::Choice,
                ..window_of(1, &["use_inspiration", "keep"])
            })],
        )
}

#[test]
fn the_pipeline_report_composes_every_module_and_is_the_same_twice() {
    let mut world = world_of(&["strike"], 2, strike_rules());
    world.dice = crate::golden::CountingDice::new([19]);
    world.tap(1, "strike").expect("the strike runs");

    let first = world.table.inspect(operator());
    let second = world.table.inspect(operator());

    assert_eq!(first, second);
    assert_eq!(first.facts.seats.len(), 2);
    assert_eq!(first.pending.pendings.len(), 1);
    assert_eq!(first.state_hash.shown(), Some(&world.table.state_hash()));
    assert_eq!(
        first.projections.len(),
        3,
        "the shared screen and two seats"
    );
    assert_eq!(first.results.remembered.shown(), Some(&1));
}

#[test]
fn describe_lists_every_verb_and_gives_every_enabled_machine_the_verbs_every_machine_needs() {
    let world = scene_world();

    let description = describe(&world.registry);

    assert_eq!(description.verbs.len(), Verb::ALL.len());
    assert_eq!(description.machines.len(), 3);
    for machine in &description.machines {
        let required: Vec<Verb> = machine
            .verbs
            .iter()
            .filter(|usage| usage.required)
            .map(|usage| usage.verb)
            .collect();
        assert_eq!(
            required,
            Verb::machine_required().collect::<Vec<_>>(),
            "{} lacks a required verb",
            machine.machine
        );
    }
    let lobby_line = description
        .machines
        .iter()
        .find(|line| line.machine == machine("lobby"))
        .expect("listed");
    let force = lobby_line
        .verbs
        .iter()
        .find(|usage| usage.verb == Verb::Force)
        .expect("lobby has force");
    assert_eq!(force.usage, "force lobby <gathering|started>");
    let dump = lobby_line
        .verbs
        .iter()
        .find(|usage| usage.verb == Verb::Dump)
        .expect("lobby has dump");
    assert_eq!(dump.usage, "dump lobby [--json]");
}

#[test]
fn describe_lists_a_disabled_machine_without_a_spec_or_verbs() {
    let registry = registry_with_scene_off();

    let description = describe(&registry);

    let scene_line = description
        .machines
        .iter()
        .find(|line| line.machine == machine("scene"))
        .expect("listed");
    assert!(!scene_line.enabled && scene_line.spec.is_none() && scene_line.verbs.is_empty());
}

#[test]
fn the_verb_table_names_every_verb_once_and_keeps_force_input_and_dump_machine_required() {
    let mut words: Vec<&str> = Verb::ALL.iter().map(|verb| verb.spec().words).collect();
    words.sort_unstable();
    words.dedup();
    assert_eq!(words.len(), Verb::ALL.len(), "two verbs share their words");
    for verb in Verb::ALL {
        assert_eq!(verb.spec().verb, *verb);
        assert!(!verb.spec().usage.is_empty() && !verb.spec().summary.is_empty());
    }
    assert_eq!(
        Verb::machine_required().collect::<Vec<_>>(),
        [Verb::Dump, Verb::Force, Verb::Input]
    );
    assert_eq!(Verb::Viable.spec().machines, MachineSupport::Meaningful);
}

#[test]
fn a_hosted_build_offers_no_verb_that_forces_state_loads_fixtures_or_reads_private_content() {
    let hosted: Vec<Verb> = Verb::ALL
        .iter()
        .copied()
        .filter(|verb| verb.spec().access.in_hosted_build())
        .collect();

    for operator_only in [
        Verb::Force,
        Verb::FixtureLoad,
        Verb::Dump,
        Verb::ProjectionShow,
        Verb::Trace,
        Verb::Replay,
        Verb::Input,
        Verb::Chaos,
    ] {
        assert_eq!(operator_only.spec().access, VerbAccess::Operator);
        assert!(!hosted.contains(&operator_only), "{operator_only:?}");
    }
    assert!(hosted.contains(&Verb::Describe) && hosted.contains(&Verb::Health));
}

/// Every report, for one audience, as the name the field table uses and its JSON.
fn all_reports(audience: InspectAudience) -> Vec<(ReportName, Value)> {
    let world = scene_world();
    let table = table_with_machines("a note");
    let snapshot = table.snapshot();
    let queue: OwnerQueue = seated_queue(2, &settings(4, 4, 1), Arc::new(NullSink));
    let mut pending_world = world_of(&["strike"], 2, strike_rules());
    pending_world.dice = crate::golden::CountingDice::new([19]);
    pending_world.tap(1, "strike").expect("the strike runs");
    let wired = [PortName::Broker];
    let at = view_of(audience);
    vec![
        (ReportName::OwnerQueue, json_of(&queue.inspect(at))),
        (
            ReportName::RegisteredMachines,
            json_of(&world.registry.inspect(at)),
        ),
        (
            ReportName::Ports,
            json_of(
                &WiredPorts {
                    registry: &world.registry,
                    wired: &wired,
                }
                .inspect(at),
            ),
        ),
        (
            ReportName::Machines,
            json_of(
                &MachinesOf {
                    registry: &world.registry,
                    snapshot: &snapshot,
                }
                .inspect(at),
            ),
        ),
        (ReportName::Table, json_of(&table.inspect(at))),
        (
            ReportName::Pending,
            json_of(&pending_world.table.pending().inspect(at)),
        ),
        (
            ReportName::Rulings,
            json_of(&pending_world.table.rulings().inspect(at)),
        ),
        (
            ReportName::Requests,
            json_of(&pending_world.table.requests().inspect(at)),
        ),
        (
            ReportName::Results,
            json_of(&pending_world.table.results().inspect(at)),
        ),
        (
            ReportName::Settings,
            json_of(&Settings::default().inspect(at)),
        ),
        (
            ReportName::Pipeline,
            json_of(&pending_world.table.inspect(at)),
        ),
    ]
}

#[test]
fn each_reports_field_table_matches_the_keys_the_report_writes_and_who_is_shown_each() {
    let for_operator = all_reports(InspectAudience::Operator);
    let for_host = all_reports(InspectAudience::Host);
    let for_seat = all_reports(InspectAudience::Seat { seat: seat(1) });
    assert_eq!(
        for_operator.len(),
        ReportName::ALL.len(),
        "a report is untested"
    );

    let visibility = |json: &Value, field: &str| {
        json[field]
            .get("visibility")
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    for (index, (name, operator_json)) in for_operator.iter().enumerate() {
        let mut expected: Vec<&str> = name.fields().iter().map(|field| field.name).collect();
        expected.sort_unstable();
        let mut written: Vec<String> = operator_json
            .as_object()
            .expect("a report is an object")
            .keys()
            .cloned()
            .collect();
        written.sort();
        assert_eq!(written, expected, "the field table of {name:?} drifted");
        for field in name.fields() {
            let (shown, redacted) = (Some("shown".to_owned()), Some("redacted".to_owned()));
            let (to_operator, to_host, to_seat) = match field.gate {
                None => (None, None, None),
                Some(Needs::Operator) => (shown.clone(), redacted.clone(), redacted.clone()),
                Some(Needs::Host | Needs::OwnSeat) => (shown.clone(), shown, redacted),
            };
            for (audience, json, wanted) in [
                ("operator", &for_operator[index].1, to_operator),
                ("host", &for_host[index].1, to_host),
                ("seat", &for_seat[index].1, to_seat),
            ] {
                assert_eq!(
                    visibility(json, field.name),
                    wanted,
                    "{name:?}.{} for the {audience}",
                    field.name
                );
            }
        }
    }
}

#[test]
fn a_redacted_field_names_who_could_see_it_and_a_shown_one_carries_its_value() {
    let table = table_of_three(false);

    let for_seat = json_of(&table.inspect(seat_view(1)));
    let for_operator = json_of(&table.inspect(operator()));

    assert_eq!(
        for_seat["digest"],
        serde_json::json!({"visibility": "redacted", "needs": "operator"})
    );
    assert_eq!(for_operator["digest"]["visibility"], "shown");
    assert_eq!(for_operator["digest"]["value"], table.digest());
}
