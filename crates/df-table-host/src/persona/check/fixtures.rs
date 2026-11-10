//! Helpers the check tests share: the warrior's sheet, judge answers, a host showing a plain scene
//! over a scripted text backend, dice that roll the faces a test names and a clock that does not
//! wait.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex, PoisonError};

use df_ai::text::{TextOutcome, TextRequest};
use df_ai::text_fixture::ScriptedBackend;
use df_content::persona::Persona;
use df_rules::roll::{Ability, AttackKind, DiceExpression, DieSize, ModifierLabel, Skill};
use serde_json::{Value, json};

use super::flow::Pacer;
use super::request::{
    CheckJudgeRequest, CheckKind, CheckSheet, CheckStation, Initiator, SheetAttack, SheetFeature,
    SheetHealing,
};
use super::routes::CheckRoutes;
use crate::adapters::SeatRoster;
use crate::adapters::{ActionBook, DfRulesOwner, EntropySource, RecordedDice, RulesCheckKit};
use crate::persona::routes::PersonaRoutes;
use crate::speech::fixtures::silent_speech;
use crate::state::AppState;
use crate::test_support::{Running, STARTER_FILES, start};
use crate::text::{BackendChoice, TextRuntime};

/// The id of the character the tests roll for.
pub(super) const WARRIOR: &str = "gauntlet_warrior";

/// The name the warrior goes by, which a line of the dungeon master must not say.
pub(super) const WARRIOR_NAME: &str = "Corwen";

/// The warrior's sheet: Strength 16, proficient in Athletics, Strength and Constitution saves, a
/// Champion's critical range of 19, a longsword, a longbow and a Second Wind.
pub(super) fn sheet() -> CheckSheet {
    CheckSheet {
        id: WARRIOR.to_owned(),
        proficiency_bonus: 2,
        ability_modifiers: BTreeMap::from([
            (Ability::Strength, 3),
            (Ability::Dexterity, 1),
            (Ability::Constitution, 2),
            (Ability::Intelligence, 0),
            (Ability::Wisdom, 1),
            (Ability::Charisma, -1),
        ]),
        saving_throw_proficiencies: vec![Ability::Strength, Ability::Constitution],
        skill_proficiencies: vec![
            Skill::Athletics,
            Skill::Intimidation,
            Skill::Perception,
            Skill::Survival,
        ],
        skill_expertise: Vec::new(),
        initiative_proficient: true,
        critical_from: 19,
        attacks: vec![
            SheetAttack {
                id: "longsword".to_owned(),
                name: "Longsword".to_owned(),
                kind: AttackKind::Melee,
                ability: Ability::Strength,
                damage: DiceExpression {
                    count: 1,
                    die: DieSize::D8,
                    modifier: 3,
                },
            },
            SheetAttack {
                id: "longbow".to_owned(),
                name: "Longbow".to_owned(),
                kind: AttackKind::Ranged,
                ability: Ability::Dexterity,
                damage: DiceExpression {
                    count: 1,
                    die: DieSize::D8,
                    modifier: 1,
                },
            },
        ],
        healing: vec![SheetHealing {
            id: "second_wind".to_owned(),
            dice: DiceExpression {
                count: 1,
                die: DieSize::D10,
                modifier: 3,
            },
            bonus_label: ModifierLabel::ClassLevel,
        }],
        equipment: vec!["chain mail".to_owned(), "shield".to_owned()],
        features: vec![SheetFeature {
            name: "champion_remarkable_athlete".to_owned(),
            text: "Advantage on Initiative rolls and on Strength (Athletics) checks.".to_owned(),
        }],
    }
}

pub(super) fn athletics_station() -> CheckStation {
    CheckStation {
        id: "athletics".to_owned(),
        title: "Athletics (Strength)".to_owned(),
        kind: CheckKind::AbilityCheck,
        ability: Some(Ability::Strength),
        skill: Some(Skill::Athletics),
        setting: "A sheer grey limestone cliff face with ledges and cracks.".to_owned(),
        situation: "The only way on is up a sheer cliff face, and he sets his hands to the rock."
            .to_owned(),
    }
}

/// A judge request at `moment` for the warrior at the athletics station.
pub(super) fn judge_request(moment: Initiator, action: Option<&str>) -> CheckJudgeRequest {
    CheckJudgeRequest {
        character: WARRIOR.to_owned(),
        sheet: sheet(),
        moment,
        action: action.map(str::to_owned),
        station: Some(athletics_station()),
        place: None,
        in_sight: Vec::new(),
        checked: Vec::new(),
        intent: None,
        model: None,
    }
}

/// `base` with the keys of `changes` set over it.
pub(super) fn with(mut base: Value, changes: Value) -> Value {
    if let (Some(fields), Some(changes)) = (base.as_object_mut(), changes.as_object()) {
        for (key, value) in changes {
            fields.insert(key.clone(), value.clone());
        }
    }
    base
}

/// A judge answer that calls an Athletics check at difficulty 15 at a player moment.
pub(super) fn athletics_answer() -> Value {
    json!({
        "check_required": true, "initiator": "player", "kind": "ability_check",
        "ability": "strength", "skill": "athletics", "dc_band": 15, "mode": "normal",
        "grounds": "none", "uses": null, "reason": "uncertain_with_consequence"
    })
}

/// A judge answer that calls no roll because the action is trivial.
pub(super) fn trivial_answer() -> Value {
    json!({
        "check_required": false, "initiator": "player", "kind": null, "ability": null,
        "skill": null, "dc_band": null, "mode": "normal", "grounds": "none", "uses": null,
        "reason": "trivial_no_check"
    })
}

/// A flourish answer in the wry tone, which fits a call and every outcome but a natural 20.
pub(super) fn line_answer(before: &str, after: &str) -> Value {
    toned(before, after, "wry")
}

/// A flourish answer in `tone`.
pub(super) fn toned(before: &str, after: &str, tone: &str) -> Value {
    json!({"tone": tone, "before": before, "after": after})
}

/// A host showing a plain scene, a text runtime over a script and what the script was asked.
pub(super) struct Table {
    pub(super) running: Running,
    pub(super) text: Arc<TextRuntime>,
    pub(super) calls: Arc<AtomicUsize>,
    pub(super) asked: Arc<Mutex<Vec<TextRequest>>>,
}

impl Table {
    pub(super) fn app(&self) -> &Arc<AppState> {
        &self.running.state
    }

    /// The facts of the `n`th question the script was asked (from 0).
    pub(super) fn facts_of(&self, n: usize) -> Value {
        self.asked.lock().unwrap_or_else(PoisonError::into_inner)[n]
            .facts
            .clone()
    }

    /// The facts of every question the script was asked under `output_name`, in order.
    pub(super) fn facts_under(&self, output_name: &str) -> Vec<Value> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|ask| ask.output_name == output_name)
            .map(|ask| ask.facts.clone())
            .collect()
    }

    /// Every audit line so far that starts with `start`.
    pub(super) fn lines_starting_with(&self, start: &str) -> Vec<String> {
        self.running
            .state
            .log()
            .after_sequence(0, usize::MAX)
            .into_iter()
            .map(|line| line.text)
            .filter(|text| text.starts_with(start))
            .collect()
    }
}

/// A table whose text backend answers with `answers`, in order. The warrior's persona is stored.
pub(super) async fn table(answers: Vec<Value>) -> Table {
    let running = start(&STARTER_FILES).await;
    running.write_asset("media/plate.png", "plate.png");
    running.write_asset(
        "fixtures/one.json",
        concat!(
            r#"{"name": "One", "tv": {"place": "Here", "status": {"kind": "ok"}, "elements": [], "#,
            r#""backdrop": {"media": "plate.png", "caption": null}}, "#,
            r#""phone": {"hero": "Pip", "status": {"kind": "ok"}}}"#
        ),
    );
    let persona: Persona = serde_json::from_value(json!({
        "schema": 1, "id": WARRIOR, "display_name": WARRIOR_NAME,
        "job": "A sellsword and veteran of border wars"
    }))
    .expect("the warrior's persona reads");
    let directory = running.playground_path("assets/personas");
    std::fs::create_dir_all(&directory).expect("makes the persona folder");
    crate::persona::write_persona(&directory, &persona).expect("stores the persona");
    let calls = Arc::new(AtomicUsize::new(0));
    let asked = Arc::new(Mutex::new(Vec::new()));
    let script = ScriptedBackend::new(
        answers.into_iter().map(TextOutcome::Answered),
        Arc::clone(&calls),
    )
    .recording(Arc::clone(&asked));
    let recorder = Arc::clone(&running.state);
    let text = Arc::new(TextRuntime::new(
        BackendChoice::Scripted(Arc::new(script), None),
        Arc::new(move |line| recorder.record_line(line)),
    ));
    Table {
        running,
        text,
        calls,
        asked,
    }
}

/// Entropy that makes the dice show the faces it is given, in order, and runs dry after them.
#[derive(Debug)]
pub(super) struct Faces(VecDeque<u64>);

impl EntropySource for Faces {
    fn next_u64(&mut self) -> u64 {
        self.0.pop_front().map_or(u64::MAX, |face| face - 1)
    }
}

/// Recording dice that roll `faces`, in order.
pub(super) fn dice_showing(faces: &[u64]) -> Arc<RecordedDice> {
    Arc::new(RecordedDice::new(Box::new(Faces(
        faces.iter().copied().collect(),
    ))))
}

/// The check routes of `table` over `dice`: the persona routes of the host, speech off.
pub(super) fn routes_over(table: &Table, dice: Arc<RecordedDice>) -> CheckRoutes {
    let persona = Arc::new(PersonaRoutes {
        app: Arc::clone(table.app()),
        text: Arc::clone(&table.text),
        speech: silent_speech(table.app()),
    });
    let rules = Arc::new(DfRulesOwner::new(
        SeatRoster::default(),
        ActionBook::default(),
    ));
    CheckRoutes::new(
        persona,
        RulesCheckKit::new(rules, dice),
        std::time::Duration::from_secs(3_600),
    )
}

/// What was on the shared screen when the run waited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Waited {
    pub(super) until_ms: u64,
    pub(super) card_up: bool,
    pub(super) narration: Option<String>,
}

/// A clock that never waits: it jumps to the time asked, and notes what the screen showed.
pub(super) struct VirtualClock {
    app: Arc<AppState>,
    now_ms: Mutex<u64>,
    waits: Mutex<Vec<Waited>>,
}

impl VirtualClock {
    pub(super) fn over(app: &Arc<AppState>) -> Self {
        Self {
            app: Arc::clone(app),
            now_ms: Mutex::new(0),
            waits: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn waits(&self) -> Vec<Waited> {
        self.waits
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Pacer for VirtualClock {
    fn elapsed_ms(&self) -> u64 {
        *self.now_ms.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn sleep_until_ms(&self, offset_ms: u64) -> impl std::future::Future<Output = ()> + Send {
        let scene = self.app.playground().snapshot().scene;
        self.waits
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Waited {
                until_ms: offset_ms,
                card_up: scene.roll.set_value().is_some(),
                narration: scene
                    .narration
                    .set_value()
                    .map(|narration| narration.text.clone()),
            });
        let mut now = self.now_ms.lock().unwrap_or_else(PoisonError::into_inner);
        *now = (*now).max(offset_ms);
        std::future::ready(())
    }
}

/// The check routes of `table` over `dice` with the speech service over `port`.
pub(super) fn routes_speaking(
    table: &Table,
    dice: Arc<RecordedDice>,
    port: &Arc<crate::speech::fixtures::FakePort>,
) -> CheckRoutes {
    let mut routes = routes_over(table, dice);
    let persona = PersonaRoutes {
        app: Arc::clone(table.app()),
        text: Arc::clone(&table.text),
        speech: crate::speech::fixtures::speech_over(table.app(), port),
    };
    routes.persona = Arc::new(persona);
    routes
}
