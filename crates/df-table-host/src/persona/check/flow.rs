//! One check, start to finish: the judge, the call, the roll, the card and the result.
//!
//! The judge says whether a roll is called. If it is, the dice are drawn from the host's recording
//! dice (real entropy in play) and resolved by `df-rules` before anything is said, so both lines
//! can be worded while nothing waits on the model and the table never hears a pause. Nothing shows
//! the dice yet. Then the dungeon master says the call and the run waits for it to be heard out
//! (the narrator's 3 s plus 70 ms a character), the roll card
//! goes on the TV, the dice tumble, and once the sum is on screen at the latest the dungeon master
//! reads the result. The card is held until the result line is heard out and two seconds more
//! (never under the page's six seconds) and then cleared. With TTS off each line is shown as text
//! only, and the audit line says so.
//!
//! The waits go through a [`Pacer`], so the order and the times are tested on a virtual clock.
//!
//! Audit lines (times are milliseconds from the start of the run): `CHECK called at_ms=..
//! kind=.. skill=.. dc=.. voice=..`, `CHECK rolled at_ms=.. drawn_ms=.. dice=.. total=.. outcome=..
//! natural=..`, `CHECK shown at_ms=.. id=.. revision=.. min_hold_ms=..`, `CHECK result_read
//! start_ms=.. end_ms=.. voice=..`, `CHECK cleared at_ms=.. hold_ms=..` and, when no roll is
//! called, `CHECK skipped character=.. reason=..`.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use df_playground::limits::MINIMUM_ROLL_HOLD_MS;
use df_playground::scene::SceneChange;
use df_rules::roll::{RollRequest, RollResult, Skill};
use serde_json::{Value, json};

use super::judge::judge_check;
use super::limits::{
    CARD_EXIT_MS, HEARD_OUT_BASE_MS, HOLD_AFTER_LINE_MS, READ_CHARACTERS_PER_SECOND,
    RESULT_LEAD_MS, RESULT_SLACK_MS,
};
use super::lines::{LineFacts, LineJob, LineMoment, ResultFacts, compose_line};
use super::numbers::{spoken_call, spoken_result, test_name};
use super::request::{
    CheckCall, CheckError, CheckJudged, CheckKind, CheckRouteError, CheckRun, CheckRunRequest,
    CheckTimes, LineVoice, RolledCheck,
};
use super::rolling::roll_request;
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

/// What the lines are worded against: the route, the names to keep out and the facts of each.
struct Wording {
    route: Arc<TextRoute>,
    names: Vec<String>,
    called: LineFacts,
    result: LineFacts,
}

async fn prepare_wording(
    routes: &CheckRoutes,
    request: &CheckRunRequest,
    judged: &CheckJudged,
    call: &CheckCall,
    roll: (&RollRequest, &RollResult),
) -> Result<Wording, CheckRouteError> {
    let persona = &routes.persona;
    let own = load_persona(&persona.app, &request.judge.character)
        .await
        .map_err(failed)?;
    let route = judge_route(&persona.text, request.judge.model.as_deref())?;
    let (rolled, result) = roll;
    let facts = |moment, how_it_went| LineFacts {
        moment,
        test: test_name(rolled),
        scene: scene_facts(request),
        initiator: judged.verdict.initiator,
        mode: call.mode,
        grounds: call.grounds,
        difficulty: call.dc_band.map(|band| band.difficulty_name()),
        result: how_it_went,
    };
    Ok(Wording {
        route,
        names: vec![own.display_name().to_owned()],
        called: facts(LineMoment::Called, None),
        result: facts(LineMoment::Result, Some(ResultFacts::of(result))),
    })
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
    lines: &Lines,
    speak: bool,
) -> Result<(LineVoice, u64, u64), CheckRouteError> {
    let app = &routes.persona.app;
    pacer.sleep_until_ms(shown.shown_ms + RESULT_LEAD_MS).await;
    let voice = cue(routes, &lines.result, speak).await?;
    let start_ms = pacer.elapsed_ms();
    let end_ms = start_ms + heard_out_ms(&lines.result);
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

/// Runs the check `request` asks for: judged, called, rolled, shown, read and cleared.
pub(super) async fn run_check<P: Pacer>(
    routes: &CheckRoutes,
    pacer: &P,
    request: &CheckRunRequest,
) -> Result<CheckRun, CheckRouteError> {
    let app = &routes.persona.app;
    let judged = judge_check(app, &routes.persona.text, &request.judge).await?;
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
    let rolled = routes.kit.roll(&roll).map_err(failed)?;
    let result = rolled.result;
    let drawn_ms = pacer.elapsed_ms();
    let serial = NEXT_ROLL.fetch_add(1, Ordering::Relaxed);
    let subject = request
        .judge
        .station
        .as_ref()
        .map_or(request.judge.character.as_str(), |station| {
            station.id.as_str()
        });
    let view = roll_view(
        format!("check-{subject}-{serial}"),
        &roll,
        &result,
        call.kind == CheckKind::Contested,
    )
    .map_err(failed)?;
    let wording = prepare_wording(routes, request, &judged, &call, (&roll, &result)).await?;
    let said_before = routes.said_before();
    let called_job = LineJob {
        app,
        route: &wording.route,
        names: &wording.names,
        facts: &wording.called,
        said_before: &said_before,
        budget: routes.line_budget,
    };
    // The call is worded first. The result is worded after it exists, told it as said, while the
    // call is cued and heard out, so it neither repeats the call nor waits for the dice.
    let worded_call = compose_line(&called_job, spoken_call(&roll)).await?;
    routes.remember(worded_call.flourish());
    let said_with_call = routes.said_before();
    let result_job = LineJob {
        facts: &wording.result,
        said_before: &said_with_call,
        ..called_job
    };
    let say_call = async {
        let line = worded_call.text();
        let voice = cue(routes, &line, request.speak).await?;
        let at_ms = pacer.elapsed_ms();
        let (kind, skill, dc) = call_fields(&call);
        app.record_line(format!(
            "CHECK called at_ms={at_ms} kind={kind} skill={skill} dc={dc} voice={}",
            voice.key()
        ));
        pacer.sleep_until_ms(at_ms + heard_out_ms(&line)).await;
        Ok::<_, CheckRouteError>((line, voice, at_ms))
    };
    let word_result = async {
        let worded = compose_line(&result_job, spoken_result(&roll, &result)).await?;
        routes.remember(worded.flourish());
        Ok::<_, CheckRouteError>(worded.text())
    };
    let ((called_line, called_voice, called_ms), result_line) =
        tokio::try_join!(say_call, word_result)?;
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

    let held = read_and_hold(routes, pacer, &shown, &lines, request.speak).await;
    // The card goes whatever came of the reading, and the reading's failure is the one answered.
    let cleared = apply(app, SceneChange::ClearRoll {}).await;
    let (result_voice, result_start_ms, result_end_ms) = held?;
    cleared?;
    let cleared_ms = pacer.elapsed_ms();
    let hold_ms = cleared_ms.saturating_sub(shown.shown_ms + RESULT_LEAD_MS);
    app.record_line(format!(
        "CHECK cleared at_ms={cleared_ms} hold_ms={hold_ms}"
    ));
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
        }),
    })
}
