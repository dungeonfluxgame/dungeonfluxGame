//! The creation machine: the lobby's hand-off brings the seats, each seat moves through drafting,
//! reviewing and ready, a ready seat reopens until the start, and the start hands one sheet per
//! hero to play. The rules are a fake: a build is complete once it names a background, and a
//! design-only class gets one mark.
use std::collections::BTreeMap;

use serde_json::json;

use crate::creation::{
    BuildDraft, BuildIssue, BuildRules, CreationError, CreationTable, CreationTransition,
    Derivation, DesignMark, HeroStart, SeatCreationState, SheetDocument,
};
use crate::effects::TableState;
use crate::identity::{SceneId, SeatId, TableId, TableScope};
use crate::lobby::LobbyMachine;
use crate::machine::slot::MachineSlot;
use crate::machine::{Entry, Machine, MachineInput, StepInput};
use crate::ports::ActionId;
use crate::testkit::correlation;

struct FakeRules;

impl BuildRules for FakeRules {
    fn derive(&self, hero: &HeroStart, draft: &BuildDraft) -> Derivation {
        let Some(background) = draft.picks().get("background") else {
            return Derivation::Incomplete {
                issues: vec![BuildIssue {
                    slot: Some("background".to_owned()),
                    sentence: "No background was picked.".to_owned(),
                }],
            };
        };
        let class = draft.picks().get("class").cloned().unwrap_or_default();
        let marks = if class == "fighter" {
            Vec::new()
        } else {
            vec![DesignMark {
                subject: class.clone(),
                reason: "not_in_play_yet".to_owned(),
                note: "Class features are not in play yet.".to_owned(),
            }]
        };
        Derivation::Complete {
            sheet: SheetDocument::new(json!({
                "id": format!("hero-{}", hero.hero),
                "class": class,
                "background": background,
            })),
            marks,
        }
    }

    fn surprise(
        &self,
        draft: &BuildDraft,
        pick: &mut dyn FnMut(usize) -> usize,
    ) -> BTreeMap<String, String> {
        let backgrounds = ["acolyte", "soldier"];
        let mut picks = draft.picks().clone();
        picks.insert(
            "background".to_owned(),
            backgrounds[pick(backgrounds.len()) % backgrounds.len()].to_owned(),
        );
        picks
    }

    fn fill(
        &self,
        draft: &BuildDraft,
        pick: &mut dyn FnMut(usize) -> usize,
    ) -> BTreeMap<String, String> {
        if draft.picks().contains_key("background") {
            return draft.picks().clone();
        }
        self.surprise(draft, pick)
    }
}

fn seat(number: u64) -> SeatId {
    SeatId::new(number)
}

fn hero(id: u64, name: &str, class: &str) -> HeroStart {
    HeroStart {
        hero: id,
        name: name.to_owned(),
        species: "human".to_owned(),
        class: class.to_owned(),
        portrait: Some(format!("hero-{id}.png")),
        public_story: vec!["A painter from the river town.".to_owned()],
    }
}

/// The lobby's hand-off after seats 1 and 2 said they are ready and seat 1 started the game.
fn lobby_hand_off() -> Entry {
    let lobby = LobbyMachine::new(SceneId::new(7)).expect("the lobby spec is valid");
    let mut table = TableState::seated(4, &[seat(1), seat(2)]).expect("a valid table");
    let mut slot = lobby.enter(&table, &Entry::empty()).expect("enters");
    for (who, action) in [(1, "ready"), (2, "ready"), (1, "start")] {
        let input = StepInput {
            correlation: correlation(who),
            now_ms: 0,
            table: &table,
            input: MachineInput::Intent {
                seat: seat(who),
                action: ActionId::new(action).expect("valid action"),
            },
            decision: None,
        };
        let result = lobby.step(&slot, &input).expect("steps");
        let next: MachineSlot = result.state_after(&slot).clone();
        table
            .apply(
                &TableScope::of_table(TableId::new(1)),
                correlation(who),
                &result.table_effects,
            )
            .expect("the effects apply");
        slot = next;
    }
    assert!(lobby.wants_hand_off(&slot).expect("reads"));
    Entry::from_hand_off(&lobby.exit(&table, &slot).expect("hands off"))
}

fn line(transition: &CreationTransition) -> String {
    let state = |state: Option<SeatCreationState>| state.map_or("none", SeatCreationState::key);
    format!(
        "seat={} {} {}>{} issues={} marks={}",
        transition.seat,
        transition.cause.key(),
        state(transition.from),
        state(transition.to),
        transition.issues,
        transition.marks
    )
}

#[test]
fn the_lobby_hands_its_seats_to_creation_each_drafting_without_a_hero() {
    let creation = CreationTable::enter(&lobby_hand_off()).expect("enters");
    let public = creation.public_seats();
    assert_eq!(
        public
            .iter()
            .map(|seat| (seat.seat, seat.name.clone(), seat.state))
            .collect::<Vec<_>>(),
        [
            (seat(1), None, SeatCreationState::Drafting),
            (seat(2), None, SeatCreationState::Drafting)
        ]
    );
    assert_eq!(
        CreationTable::enter(&Entry::empty()),
        Err(CreationError::NoSeats)
    );
}

#[test]
fn a_seat_without_a_hero_cannot_draft() {
    let mut creation = CreationTable::enter(&lobby_hand_off()).expect("enters");
    assert_eq!(
        creation.choose(seat(1), "background", "soldier", &FakeRules),
        Err(CreationError::NoHero { seat: seat(1) })
    );
    assert_eq!(
        creation.choose(seat(9), "background", "soldier", &FakeRules),
        Err(CreationError::UnknownSeat { seat: seat(9) })
    );
}

#[test]
fn the_build_starts_from_the_species_and_class_the_look_studio_chose() {
    let mut creation = CreationTable::enter(&lobby_hand_off()).expect("enters");
    creation
        .begin(seat(1), hero(11, "Mara", "wizard"), &FakeRules)
        .expect("begins");
    let draft = creation
        .seat(seat(1))
        .expect("seated")
        .draft
        .picks()
        .clone();
    assert_eq!(
        draft,
        BTreeMap::from([
            ("class".to_owned(), "wizard".to_owned()),
            ("species".to_owned(), "human".to_owned())
        ])
    );
}

#[test]
fn a_ready_seat_changes_nothing_until_it_reopens_and_reopens_only_before_the_start() {
    let mut creation = CreationTable::enter(&lobby_hand_off()).expect("enters");
    creation
        .begin(seat(1), hero(11, "Mara", "fighter"), &FakeRules)
        .expect("begins");
    assert_eq!(
        creation.submit(seat(1)),
        Err(CreationError::BuildIncomplete {
            seat: seat(1),
            issues: 1
        })
    );
    creation
        .choose(seat(1), "background", "soldier", &FakeRules)
        .expect("chooses");
    creation.submit(seat(1)).expect("submits");
    assert_eq!(
        creation.choose(seat(1), "background", "sage", &FakeRules),
        Err(CreationError::SeatReady { seat: seat(1) })
    );
    assert_eq!(
        creation.reopen(seat(2)),
        Err(CreationError::NotReady { seat: seat(2) })
    );
    creation.leave(seat(2));
    creation.start().expect("starts");
    assert_eq!(creation.reopen(seat(1)), Err(CreationError::Started));
}

#[test]
fn a_golden_creation_session_ends_with_one_sheet_per_hero_and_a_hand_off_to_play() {
    let mut creation = CreationTable::enter(&lobby_hand_off()).expect("enters");
    let mut log = Vec::new();
    let mut record = |result: Result<CreationTransition, CreationError>| match result {
        Ok(transition) => log.push(line(&transition)),
        Err(error) => log.push(format!("declined: {error}")),
    };
    record(creation.begin(seat(1), hero(11, "Mara", "fighter"), &FakeRules));
    record(creation.begin(seat(2), hero(12, "Odo", "wizard"), &FakeRules));
    record(creation.choose(seat(1), "background", "soldier", &FakeRules));
    record(creation.surprise(seat(2), &FakeRules, &mut |_| 0));
    record(creation.submit(seat(1)));
    record(creation.start().map(|_| unreachable_transition()));
    record(creation.reopen(seat(1)));
    record(creation.clear(seat(1), "background", &FakeRules));
    record(creation.choose(seat(1), "background", "criminal", &FakeRules));
    record(creation.submit(seat(1)));
    record(creation.submit(seat(2)));
    assert_eq!(
        log,
        [
            "seat=1 begin drafting>drafting issues=1 marks=0",
            "seat=2 begin drafting>drafting issues=1 marks=0",
            "seat=1 choose drafting>reviewing issues=0 marks=0",
            "seat=2 surprise drafting>reviewing issues=0 marks=1",
            "seat=1 submit reviewing>ready issues=0 marks=0",
            "declined: every seat must be ready first; seat 2 is not",
            "seat=1 reopen ready>reviewing issues=0 marks=0",
            "seat=1 choose reviewing>drafting issues=1 marks=0",
            "seat=1 choose drafting>reviewing issues=0 marks=0",
            "seat=1 submit reviewing>ready issues=0 marks=0",
            "seat=2 submit reviewing>ready issues=0 marks=1",
        ]
    );
    let outcome = creation.start().expect("every seat is ready");
    assert_eq!(
        outcome.hand_off.machine_target().map(|id| id.as_str()),
        Some("explore")
    );
    assert_eq!(
        outcome.hand_off.carry().len(),
        2,
        "both seats are carried to play"
    );
    let sheets: Vec<_> = outcome
        .heroes
        .iter()
        .map(|output| {
            (
                output.seat,
                output.hero.name.as_str(),
                output.sheet.as_value()["background"].clone(),
                output.marks.len(),
            )
        })
        .collect();
    assert_eq!(
        sheets,
        [
            (seat(1), "Mara", json!("criminal"), 0),
            (seat(2), "Odo", json!("acolyte"), 1)
        ]
    );
    assert_eq!(
        outcome.heroes[0].hero.portrait.as_deref(),
        Some("hero-11.png")
    );
    assert_eq!(creation.start(), Err(CreationError::Started));
}

/// `start` answers no transition; the golden log records only its decline.
fn unreachable_transition() -> CreationTransition {
    panic!("the start was expected to decline")
}

#[test]
fn a_seat_that_joins_later_is_admitted_and_the_table_reads_back_from_its_record() {
    let mut creation = CreationTable::enter(&lobby_hand_off()).expect("enters");
    creation.admit(seat(3)).expect("admits");
    creation
        .begin(seat(3), hero(13, "Pell", "rogue"), &FakeRules)
        .expect("begins");
    let transition = creation
        .fill(seat(3), &FakeRules, &mut |_| 1)
        .expect("fills");
    assert_eq!(transition.to, Some(SeatCreationState::Reviewing));
    let written = serde_json::to_string(&creation).expect("the record writes");
    let read: CreationTable = serde_json::from_str(&written).expect("the record reads");
    assert_eq!(read, creation);
    creation.admit(seat(3)).expect("admits again");
    assert_eq!(
        creation.seat(seat(3)).map(|seat| seat.state),
        Some(SeatCreationState::Reviewing),
        "a seat already in creation is left as it is"
    );
}

#[test]
fn a_build_belongs_to_its_hero_across_a_change_of_hero_a_leave_and_the_next_start() {
    let mut creation = CreationTable::enter(&lobby_hand_off()).expect("enters");
    creation.leave(seat(2));
    let (_, fresh) = creation
        .take_up(seat(1), hero(6, "Wren", "ranger"), &FakeRules)
        .expect("takes up")
        .expect("a new hero changes the seat");
    assert!(fresh);
    creation
        .choose(seat(1), "background", "sage", &FakeRules)
        .expect("chooses");
    creation.submit(seat(1)).expect("submits");
    creation.start().expect("starts");
    assert!(creation.reopen_for_next());
    assert!(
        creation.seat(seat(1)).is_none(),
        "the next start begins anew"
    );
    // The seat plays another hero now: a new build, not Wren's.
    let (moved, fresh) = creation
        .take_up(seat(1), hero(7, "Bran", "fighter"), &FakeRules)
        .expect("takes up")
        .expect("a new hero changes the seat");
    assert!(fresh);
    assert_eq!(moved.to, Some(SeatCreationState::Drafting));
    assert_eq!(
        creation
            .build_of(6)
            .and_then(|build| build.draft.picks().get("background").cloned()),
        Some("sage".to_owned()),
        "Wren's finished build is kept for her"
    );
    // Wren comes back at another seat: her build as she left it, ready, not made again.
    creation.leave(seat(1));
    let (back, fresh) = creation
        .take_up(seat(3), hero(6, "Wren", "ranger"), &FakeRules)
        .expect("takes up")
        .expect("a returning hero changes the seat");
    assert!(!fresh);
    assert_eq!(back.to, Some(SeatCreationState::Ready));
    assert_eq!(
        creation.take_up(seat(3), hero(6, "Wren", "ranger"), &FakeRules),
        Ok(None)
    );
    // Bran's build was a draft only: it is not kept when his seat leaves.
    assert!(creation.build_of(7).is_none());
}
