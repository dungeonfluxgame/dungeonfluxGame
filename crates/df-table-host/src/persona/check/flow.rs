//! One check, start to finish: the judge, the call, the roll, the card and the result.
//!
//! The judge says whether a roll is called. If it is, the dice are drawn from the host's recording
//! dice (real entropy in play) and resolved by `df-rules` before anything is said, so both lines
//! can be worded while nothing waits on the model and the table never hears a pause. Nothing shows
//! the dice yet. Then the dungeon master says the call and the run waits for it to be heard out
//! (the narrator's 3 s plus 70 ms a character), the roll card
//! goes on the TV, the dice tumble, and once the sum is on screen at the latest the dungeon master
//! reads the result. The card is held until the result line is heard out and two seconds more
//! (never under the page's six seconds) and then cleared. An attack that hits rolls its weapon's
//! damage at once from the same sheet entry (every die twice on a Critical Hit, SRD 5.2.1 page 16):
//! once the attack's card is cleared the damage card goes up, the dungeon master reads it, and it
//! is held and cleared the same way. With TTS off each line is shown as text only, and the audit
//! line says so.
//!
//! The waits go through a [`Pacer`], so the order and the times are tested on a virtual clock.
//!
//! Audit lines (times are milliseconds from the start of the run): `CHECK called at_ms=..
//! kind=.. skill=.. dc=.. mode=.. grounds=.. voice=..`, `CHECK rolled at_ms=.. drawn_ms=.. dice=..
//! total=.. outcome=.. natural=..`, `CHECK shown at_ms=.. id=.. revision=.. min_hold_ms=..`,
//! `CHECK result_read start_ms=.. end_ms=.. voice=..`, `CHECK cleared at_ms=.. hold_ms=..`, after a
//! hit `CHECK damage at_ms=.. dice=.. total=.. crit=yes|no` and, when no roll is called, `CHECK
//! skipped character=.. reason=..`.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use df_playground::fixture::RollView;
use df_playground::limits::MINIMUM_ROLL_HOLD_MS;
use df_playground::scene::SceneChange;
use df_rules::roll::{RollMode, RollOutcome, RollRequest, RollResult, RollStyle, Skill};
use serde_json::{Value, json};

use super::judge::judge_check;
use super::limits::{
    CARD_EXIT_MS, HEARD_OUT_BASE_MS, HOLD_AFTER_LINE_MS, READ_CHARACTERS_PER_SECOND,
    RESULT_LEAD_MS, RESULT_SLACK_MS,
};
use super::lines::{LineFacts, LineJob, LineMoment, ResultFacts, compose_line};
use super::numbers::{spoken_call, spoken_result, test_name};
use super::pending::{RollWait, pending_of};
use super::request::{
    CheckCall, CheckError, CheckJudged, CheckKind, CheckReason, CheckRouteError, CheckRun,
    CheckRunRequest, CheckTimes, CheckVerdict, Initiator, LineVoice, ModeGround, RolledCheck,
    RolledDamage, mode_key,
};
use super::rolling::{hit_damage, roll_request};
use super::routes::CheckRoutes;
use super::view::roll_view;
use crate::cast::routes::line_rest_ms;
use crate::persona::generate::load_persona;
use crate::persona::judge_route::judge_route;
use crate::persona::request::PersonaError;
use crate::persona::routes::{NarrateLine, Narrated, narrate_line};
use crate::speech::Voiced;
use crate::state::AppState;
use crate::text::TextRoute;

/// Gives each roll of the host its own id on the card.
static NEXT_ROLL: AtomicU64 = AtomicU64::new(0);

/// The clock of a run: the milliseconds since it began, and a wait until a later one.
pub(super) trait Pacer: Sync {
    fn elapsed_ms(&self) -> u64;

    /// Waits until `offset_ms` after the run began; returns at once when that has passed.
    fn sleep_until_ms(&self, offset_ms: u64) -> impl Future<Output = ()> + Send;
}

/// The clock a run has in the host: tokio's, from the moment it began.
pub(super) struct RunClock {
    began: tokio::time::Instant,
}

impl RunClock {
    pub(super) fn start() -> Self {
        Self {
            began: tokio::time::Instant::now(),
        }
    }
}

impl Pacer for RunClock {
    fn elapsed_ms(&self) -> u64 {
        u64::try_from(self.began.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn sleep_until_ms(&self, offset_ms: u64) -> impl Future<Output = ()> + Send {
        tokio::time::sleep_until(self.began + Duration::from_millis(offset_ms))
    }
}

fn failed(cause: impl Into<CheckError>) -> CheckRouteError {
    CheckRouteError::BadRequest {
        cause: cause.into(),
    }
}

fn voice_of(narrated: &Narrated) -> LineVoice {
    match narrated.voice {
        Some(Voiced::Played { .. }) => LineVoice::Played,
        Some(Voiced::Failed { .. }) => LineVoice::Failed,
        None => LineVoice::TextOnly,
    }
}

/// `CHECK read_len moment=.. chars=.. est_s=..`: how long a line is and how long a voice takes to
/// read it at the owner's pace, so a station that runs long shows it.
fn record_read_length(app: &AppState, moment: LineMoment, line: &str) {
    let characters = u64::try_from(line.chars().count()).unwrap_or(u64::MAX);
    let tenths = characters.saturating_mul(10) / READ_CHARACTERS_PER_SECOND;
    app.record_line(format!(
        "CHECK read_len moment={} chars={characters} est_s={}.{}",
        moment.key(),
        tenths / 10,
        tenths % 10
    ));
}

/// How long a spoken line takes to be heard out once it is cued.
fn heard_out_ms(text: &str) -> u64 {
    line_rest_ms(HEARD_OUT_BASE_MS, text.chars().count())
}

/// The place, the situation, what is in sight and the character's action, as the line judges are
/// told them.
fn scene_facts(request: &CheckRunRequest) -> Value {
    let judge = &request.judge;
    let station = judge.station.as_ref();
    json!({
        "place": judge.place.as_deref().or_else(|| station.map(|station| station.setting.as_str())),
        "situation": station.map(|station| station.situation.as_str()),
        "in_sight": judge.in_sight,
        "action": judge.action,
        "already_told_aloud": request.told,
    })
}

async fn cue(routes: &CheckRoutes, text: &str, speak: bool) -> Result<LineVoice, CheckRouteError> {
    let narrated = narrate_line(
        &routes.persona,
        &NarrateLine {
            text: text.to_owned(),
            speak,
        },
    )
    .await
    .map_err(failed)?;
    Ok(voice_of(&narrated))
}

async fn apply(app: &AppState, change: SceneChange) -> Result<u64, CheckRouteError> {
    crate::playground::apply_scene_changes(app, vec![change])
        .await
        .map(|applied| applied.revision)
        .map_err(|error| failed(PersonaError::from(error)))
}

fn call_fields(call: &CheckCall) -> (&'static str, &'static str, String) {
    (
        call.kind.key(),
        call.skill.map_or("-", Skill::key),
        call.dc_band
            .map_or_else(|| "-".to_owned(), |band| band.value().to_string()),
    )
}

/// The two lines of a roll, worded while the first is not yet spoken.
struct Lines {
    called: String,
    result: String,
}

/// What the lines are worded against: the route, the names to keep out and the call's facts,
/// which never depend on the dice.
struct Wording {
    route: Arc<TextRoute>,
    names: Vec<String>,
    called: LineFacts,
}

async fn prepare_wording(
    routes: &CheckRoutes,
    request: &CheckRunRequest,
    judged: &CheckJudged,
    call: &CheckCall,
    rolled: &RollRequest,
) -> Result<Wording, CheckRouteError> {
    let persona = &routes.persona;
    let own = load_persona(&persona.app, &request.judge.character)
        .await
        .map_err(failed)?;
    let route = judge_route(&persona.text, request.judge.model.as_deref())?;
    Ok(Wording {
        route,
        names: vec![own.display_name().to_owned()],
        called: call_facts(request, judged, call, rolled, LineMoment::Called, None),
    })
}

/// The facts a line of `call`'s roll `rolled` is worded from, at `moment`, with how it went once
/// the dice are drawn.
fn call_facts(
    request: &CheckRunRequest,
    judged: &CheckJudged,
    call: &CheckCall,
    rolled: &RollRequest,
    moment: LineMoment,
    result: Option<ResultFacts>,
) -> LineFacts {
    LineFacts {
        moment,
        test: test_name(rolled),
        scene: scene_facts(request),
        initiator: judged.verdict.initiator,
        mode: call.mode,
        grounds: call.grounds,
        difficulty: call.dc_band.map(|band| band.difficulty_name()),
        result,
    }
}

/// The facts of the damage a hit dealt, read after the attack's result.
fn damage_facts(
    request: &CheckRunRequest,
    judged: &CheckJudged,
    (rolled, result): &(RollRequest, RollResult),
) -> LineFacts {
    LineFacts {
        moment: LineMoment::Result,
        test: test_name(rolled),
        scene: scene_facts(request),
        initiator: judged.verdict.initiator,
        mode: RollMode::Normal,
        grounds: ModeGround::None,
        difficulty: None,
        result: Some(ResultFacts::of(result)),
    }
}

/// The dice of a roll: its result, the damage a hit dealt, the card and when they were drawn.
struct Drawn {
    result: RollResult,
    damage: Option<(RollRequest, RollResult)>,
    view: RollView,
    drawn_ms: u64,
}

/// Draws the dice of `roll` (and a hit's damage) and makes its card, named `id`.
fn draw<P: Pacer>(
    routes: &CheckRoutes,
    pacer: &P,
    (request, call): (&CheckRunRequest, &CheckCall),
    roll: &RollRequest,
    id: String,
) -> Result<Drawn, CheckRouteError> {
    let style = attack_style(call, &request.judge.sheet);
    let attack_part = RollStyle {
        great_weapon_fighting: false,
        ..style
    };
    let rolled = match call.kind {
        CheckKind::AttackRoll => routes.kit.roll_styled(roll, attack_part),
        CheckKind::Damage => routes.kit.roll_styled(roll, damage_part(style)),
        _ => routes.kit.roll(roll),
    };
    let result = rolled.map_err(failed)?.result;
    let damage = damage_of_hit(routes, call, &request.judge.sheet, &result)?;
    let drawn_ms = pacer.elapsed_ms();
    let mut view =
        roll_view(id, roll, &result, call.kind == CheckKind::Contested).map_err(failed)?;
    // A tool check is an ability check the card names by its tool: `Thieves' Tools (Dexterity)`.
    if let Some(title) = super::view::tool_title(call) {
        view.title = title;
    }
    Ok(Drawn {
        result,
        damage,
        view,
        drawn_ms,
    })
}

/// Words the result line of `drawn` (and a hit's damage line after it) with `job`, whose facts are
/// the result's; `damage` are the damage's facts.
async fn word_result(
    routes: &CheckRoutes,
    job: LineJob<'_>,
    damage: Option<&LineFacts>,
    roll: &RollRequest,
    drawn: &Drawn,
) -> Result<(String, Option<String>), CheckRouteError> {
    let worded = compose_line(&job, spoken_result(roll, &drawn.result)).await?;
    routes.remember(worded.flourish());
    let damage_line = match (&drawn.damage, damage) {
        (Some((rolled, dealt)), Some(facts)) => {
            let said_with_result = routes.said_before();
            let damage_job = LineJob {
                facts,
                said_before: &said_with_result,
                ..job
            };
            let worded = compose_line(&damage_job, spoken_result(rolled, dealt)).await?;
            routes.remember(worded.flourish());
            Some(worded.text())
        }
        _ => None,
    };
    Ok((worded.text(), damage_line))
}

/// What the run does once the card is on screen: waits for the sum, reads the result, holds the
/// card until it has been heard out and two seconds more.
struct Shown {
    shown_ms: u64,
    min_hold_ms: u64,
}

async fn read_and_hold<P: Pacer>(
    routes: &CheckRoutes,
    pacer: &P,
    shown: &Shown,
    line: &str,
    speak: bool,
) -> Result<(LineVoice, u64, u64), CheckRouteError> {
    let app = &routes.persona.app;
    pacer.sleep_until_ms(shown.shown_ms + RESULT_LEAD_MS).await;
    let voice = cue(routes, line, speak).await?;
    let start_ms = pacer.elapsed_ms();
    let end_ms = start_ms + heard_out_ms(line);
    app.record_line(format!(
        "CHECK result_read start_ms={start_ms} end_ms={end_ms} voice={}",
        voice.key()
    ));
    pacer.sleep_until_ms(end_ms).await;
    let hold_ms = MINIMUM_ROLL_HOLD_MS.max(shown.min_hold_ms);
    pacer
        .sleep_until_ms(shown.shown_ms + RESULT_LEAD_MS + hold_ms + CARD_EXIT_MS)
        .await;
    Ok((voice, start_ms, end_ms))
}

/// The seat that plays `character`, when a seat does: its player rolls the dice. A seat book that
/// cannot be read is recorded and the host rolls.
async fn rolling_seat(app: &AppState, character: &str) -> Option<(u64, u64)> {
    match crate::persona::hero_look::seated_hero(app, character).await {
        Ok(seated) => seated.map(|(seat, hero)| (seat, hero.id)),
        Err(error) => {
            app.record_line(format!(
                "CHECK seat_unavailable character={character} error={}",
                error.key()
            ));
            None
        }
    }
}

/// The dungeon master's own call when the judge called none for the test the scenario authored
/// (`request.judge.intent`) and set a difficulty for (`request.dc`): the scenario has decided there
/// are stakes, so the test is rolled anyway, as a narrator's obstacle, at that band, its mode only
/// what the sheet's own rules give (`judge::sheet_modes`). An ability check, a saving throw, or a
/// tool check with the tool the scenario names (`request.tool`, a catalog tool: its proficiency
/// and a skill used with it count as on any tool check). A weapon's test is never made up. None
/// when the judge called a roll, or nothing was authored.
pub(super) fn dm_called(judged: &CheckJudged, request: &CheckRunRequest) -> Option<CheckJudged> {
    if judged.verdict.call.is_some() {
        return None;
    }
    let (intent, dc_band) = (request.judge.intent.as_ref()?, request.dc?);
    let uses = match intent.kind {
        CheckKind::AbilityCheck | CheckKind::SavingThrow => None,
        CheckKind::ToolCheck => {
            let tool = request.tool.as_deref()?;
            df_rules::hero::Tool::from_key(tool)?;
            Some(tool.to_owned())
        }
        _ => return None,
    };
    let ability = intent.ability.or(intent.skill.map(Skill::ability))?;
    let rules = super::judge::sheet_modes(
        intent.kind,
        intent.skill,
        uses.as_deref(),
        &request.judge.sheet,
    );
    let (mode, grounds) = super::judge::combined_mode(RollMode::Normal, ModeGround::None, rules);
    Some(CheckJudged {
        verdict: CheckVerdict {
            initiator: Initiator::Narrator,
            reason: CheckReason::NarratorObstacle,
            call: Some(CheckCall {
                kind: intent.kind,
                ability: Some(ability),
                skill: intent.skill,
                dc_band: Some(dc_band),
                mode,
                grounds,
                uses,
                attack_mode: None,
                target_ac: None,
            }),
        },
        retried: judged.retried,
        duration_ms: judged.duration_ms,
        designed: judged.designed.clone(),
    })
}

/// Runs the check `request` asks for: judged, called, rolled, shown, read and cleared. When a seat
/// plays the character, the call is said and the table waits for the player to roll
/// (`pending`) before the dice are drawn; otherwise the dice are drawn at once and the result is
/// worded while the call is said.
pub(super) async fn run_check<P: Pacer>(
    routes: &CheckRoutes,
    pacer: &P,
    request: &CheckRunRequest,
) -> Result<CheckRun, CheckRouteError> {
    let app = &routes.persona.app;
    let judged = judge_check(app, &routes.persona.text, &request.judge).await?;
    let judged = match dm_called(&judged, request) {
        Some(called) => {
            let call = called.verdict.call.as_ref();
            app.record_line(format!(
                "CHECK called_by_dm character={} kind={} skill={} tool={} dc={} judge_reason={}",
                request.judge.character,
                call.map_or("-", |call| call.kind.key()),
                call.and_then(|call| call.skill).map_or("-", Skill::key),
                call.and_then(|call| call.uses.as_deref()).unwrap_or("-"),
                call.and_then(|call| call.dc_band)
                    .map_or(0, super::request::DcBand::value),
                judged.verdict.reason.key()
            ));
            called
        }
        None => judged,
    };
    let Some(call) = judged.verdict.call.clone() else {
        app.record_line(format!(
            "CHECK skipped character={} reason={}",
            request.judge.character,
            judged.verdict.reason.key()
        ));
        return Ok(CheckRun {
            judged,
            rolled: None,
        });
    };
    let roll = roll_request(&call, &request.judge.sheet, request.inspiration).map_err(failed)?;
    let serial = NEXT_ROLL.fetch_add(1, Ordering::Relaxed);
    let subject = request
        .judge
        .station
        .as_ref()
        .map_or(request.judge.character.as_str(), |station| {
            station.id.as_str()
        });
    let id = format!("check-{subject}-{serial}");
    let seated = rolling_seat(app, &request.judge.character).await;
    // A use of a limited feature with none left is refused before anything is said or rolled.
    if let Some((_, hero)) = seated {
        super::vitals::refusal_before(app, hero, &call)
            .await
            .map_err(failed)?;
    }
    let seat = seated.map(|(seat, _)| seat);
    let wording = prepare_wording(routes, request, &judged, &call, &roll).await?;
    let said_before = routes.said_before();
    let called_job = LineJob {
        app,
        route: &wording.route,
        names: &wording.names,
        facts: &wording.called,
        said_before: &said_before,
        budget: routes.line_budget,
    };
    // The call is worded first. The result is worded after it exists, told it as said, so it
    // neither repeats the call nor waits for the dice.
    let worded_call = compose_line(&called_job, spoken_call(&roll)).await?;
    routes.remember(worded_call.flourish());
    let said_with_call = routes.said_before();
    let say_call = async {
        let line = worded_call.text();
        let voice = cue(routes, &line, request.speak).await?;
        let at_ms = pacer.elapsed_ms();
        let (kind, skill, dc) = call_fields(&call);
        app.record_line(format!(
            "CHECK called at_ms={at_ms} kind={kind} skill={skill} dc={dc} mode={} grounds={} \
             voice={}",
            mode_key(call.mode),
            call.grounds.key(),
            voice.key()
        ));
        pacer.sleep_until_ms(at_ms + heard_out_ms(&line)).await;
        Ok::<_, CheckRouteError>((line, voice, at_ms))
    };
    let result_facts = |drawn: &Drawn| {
        (
            call_facts(
                request,
                &judged,
                &call,
                &roll,
                LineMoment::Result,
                Some(ResultFacts::of(&drawn.result)),
            ),
            drawn
                .damage
                .as_ref()
                .map(|dealt| damage_facts(request, &judged, dealt)),
        )
    };
    let ((called_line, called_voice, called_ms), drawn, (result_line, damage_line)) = match seat {
        None => {
            // The host rolls: the dice are drawn at once, and the result is worded while the
            // call is said.
            let drawn = draw(routes, pacer, (request, &call), &roll, id)?;
            let (result, damage) = result_facts(&drawn);
            let result_job = LineJob {
                facts: &result,
                said_before: &said_with_call,
                ..called_job
            };
            let (said, worded) = tokio::try_join!(
                say_call,
                word_result(routes, result_job, damage.as_ref(), &roll, &drawn)
            )?;
            (said, drawn, worded)
        }
        Some(seat) => {
            let said = say_call.await?;
            // The player rolls: the call waits on their seat until they do, with no deadline.
            // Nobody rolls in their place; a seat that no longer plays the hero ends the check.
            let pending = pending_of(id.clone(), &call, &roll);
            let character = request.judge.character.as_str();
            let waited = routes
                .pending
                .wait(app, (seat, pending), routes.roll_watch, || async move {
                    rolling_seat(app, character)
                        .await
                        .is_some_and(|(playing, _)| playing == seat)
                })
                .await;
            if waited == RollWait::Withdrawn {
                return Err(failed(CheckError::RollWithdrawn { seat }));
            }
            let drawn = draw(routes, pacer, (request, &call), &roll, id)?;
            let (result, damage) = result_facts(&drawn);
            let result_job = LineJob {
                facts: &result,
                said_before: &said_with_call,
                ..called_job
            };
            let worded = word_result(routes, result_job, damage.as_ref(), &roll, &drawn).await?;
            (said, drawn, worded)
        }
    };
    let Drawn {
        result,
        damage,
        view,
        drawn_ms,
    } = drawn;
    let lines = Lines {
        called: called_line,
        result: result_line,
    };
    record_read_length(app, LineMoment::Called, &lines.called);
    record_read_length(app, LineMoment::Result, &lines.result);

    let min_hold_ms = RESULT_SLACK_MS + heard_out_ms(&lines.result) + HOLD_AFTER_LINE_MS;
    let revision = apply(
        app,
        SceneChange::Roll {
            view: view.clone(),
            min_hold_ms,
        },
    )
    .await?;
    let shown = Shown {
        shown_ms: pacer.elapsed_ms(),
        min_hold_ms,
    };
    app.record_line(format!(
        "CHECK rolled at_ms={} drawn_ms={drawn_ms} dice={} total={} outcome={} natural={}",
        shown.shown_ms,
        view.dice.len(),
        view.total,
        result.outcome.key(),
        result.natural.key()
    ));
    app.record_line(format!(
        "CHECK shown at_ms={} id={} revision={revision} min_hold_ms={min_hold_ms}",
        shown.shown_ms, view.id
    ));

    let held = read_and_hold(routes, pacer, &shown, &lines.result, request.speak).await;
    // The card goes whatever came of the reading, and the reading's failure is the one answered.
    let cleared = apply(app, SceneChange::ClearRoll {}).await;
    let (result_voice, result_start_ms, result_end_ms) = held?;
    cleared?;
    let cleared_ms = pacer.elapsed_ms();
    let hold_ms = cleared_ms.saturating_sub(shown.shown_ms + RESULT_LEAD_MS);
    app.record_line(format!(
        "CHECK cleared at_ms={cleared_ms} hold_ms={hold_ms}"
    ));
    let damage = match (damage, damage_line) {
        (Some(dealt), Some(line)) => {
            Some(show_damage(routes, pacer, (&view.id, dealt), line, request.speak).await?)
        }
        _ => None,
    };
    // What the roll does to the hero's own vitals is taken once the table has seen it.
    if let Some((_, hero)) = seated {
        super::vitals::apply_result(app, hero, &call, &result).await;
    }
    Ok(CheckRun {
        judged,
        rolled: Some(RolledCheck {
            called_line: lines.called,
            result_line: lines.result,
            called_voice,
            result_voice,
            view,
            times: CheckTimes {
                drawn_ms,
                called_ms,
                shown_ms: shown.shown_ms,
                result_start_ms,
                result_end_ms,
                cleared_ms,
                hold_ms,
                min_hold_ms,
            },
            damage,
        }),
    })
}

/// The Fighting Style of the weapon `call` uses, as the sheet gives it (each roll takes its part).
fn attack_style(call: &CheckCall, sheet: &super::request::CheckSheet) -> RollStyle {
    RollStyle::of(
        call.uses
            .as_deref()
            .and_then(|id| sheet.attack(id))
            .and_then(|attack| attack.style),
    )
}

/// The part of `style` a damage roll takes: Great Weapon Fighting's floor, never Archery's bonus.
fn damage_part(style: RollStyle) -> RollStyle {
    RollStyle {
        archery: false,
        ..style
    }
}

/// The weapon's damage the attack `call` deals when `result` hits, drawn at once from the same
/// sheet entry; a Critical Hit rolls every die twice. None for any other roll or outcome.
fn damage_of_hit(
    routes: &CheckRoutes,
    call: &CheckCall,
    sheet: &super::request::CheckSheet,
    result: &RollResult,
) -> Result<Option<(RollRequest, RollResult)>, CheckRouteError> {
    if call.kind != CheckKind::AttackRoll {
        return Ok(None);
    }
    let critical = match result.outcome {
        RollOutcome::Hit => false,
        RollOutcome::CriticalHit => true,
        _ => return Ok(None),
    };
    let request = hit_damage(call, sheet, critical).map_err(failed)?;
    let style = damage_part(attack_style(call, sheet));
    let rolled = routes.kit.roll_styled(&request, style).map_err(failed)?;
    Ok(Some((request, rolled.result)))
}

/// Shows the damage card of the attack `attack_id`, reads `line` over it, holds it until the line
/// is heard out and two seconds more, and clears it.
async fn show_damage<P: Pacer>(
    routes: &CheckRoutes,
    pacer: &P,
    (attack_id, (rolled, dealt)): (&str, (RollRequest, RollResult)),
    line: String,
    speak: bool,
) -> Result<RolledDamage, CheckRouteError> {
    let app = &routes.persona.app;
    let critical = matches!(&rolled, RollRequest::Dice(dice) if dice.critical);
    let view = roll_view(format!("{attack_id}-damage"), &rolled, &dealt, false).map_err(failed)?;
    record_read_length(app, LineMoment::Result, &line);
    let min_hold_ms = RESULT_SLACK_MS + heard_out_ms(&line) + HOLD_AFTER_LINE_MS;
    apply(
        app,
        SceneChange::Roll {
            view: view.clone(),
            min_hold_ms,
        },
    )
    .await?;
    let shown = Shown {
        shown_ms: pacer.elapsed_ms(),
        min_hold_ms,
    };
    let faces: Vec<String> = view.dice.iter().map(|die| die.face.to_string()).collect();
    app.record_line(format!(
        "CHECK damage at_ms={} dice={} total={} crit={}",
        shown.shown_ms,
        faces.join("+"),
        view.total,
        if critical { "yes" } else { "no" }
    ));
    let held = read_and_hold(routes, pacer, &shown, &line, speak).await;
    let cleared = apply(app, SceneChange::ClearRoll {}).await;
    let (voice, ..) = held?;
    cleared?;
    Ok(RolledDamage {
        line,
        voice,
        view,
        critical,
    })
}
