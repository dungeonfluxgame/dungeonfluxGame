//! An approach: walking toward something on screen one step at a time, along one line of sight
//! (`planning/explore.md`, "Movement toward what is on screen"). Each step is a new plate that
//! continues the view before it from closer in: the place design of the step (the design route,
//! told the approach and checked by `design::check_approach`), its brief and sheets
//! (`cli::brief_design`, `cli::plate_inputs`) and a walk plate edited from the plate before it
//! (`persona::make_walk_plate` with that plate as its base), then shown and made the active place
//! as any move's place is. There is no second image path.
//!
//! How far the target is decides the steps ([`after_step`]): within reach, the step reaches it
//! ([`reaches`], [`arrives`]; a way, a direction, is never arrived at): its plate is the target
//! up close, the hero right beside it, made like any step (and made ahead like any, so going up
//! to it shows at once); across the square, one step brings
//! it within reach; down the road or on the horizon, a step may keep it as far (the land between
//! changes) or bring it one band closer, as the design judges the view. A target on the horizon
//! draws closer as long as the player keeps walking.
//!
//! A step made ahead (`ahead=true`) is designed and painted but shown only when the word `@take`
//! comes on its input; `@drop` or the end of the input leaves it in the library, never shown
//! ([`shown_after`]). Each moment is one `MOVE approach ..` line on the host
//! (`POST /dev/move/approach`).

use std::time::Instant;

use df_types::saturating_millis;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, BufReader, Lines};

use super::cli::{
    Briefed, MoveError, arrive, brief_design, design_text, plate_inputs, present_personas,
    resident_persona,
};
use super::routes::{
    ApproachNote, ApproachOutcome, DesignReply, DesignRequest, ExpectedResident, path as move_path,
};
use super::types::{
    ApproachAsk, Direction, Distance, MoveIntent, MoveKind, Pace, PartyMember, Placement,
    SceneDesign,
};
use crate::cast::request::CastList;
use crate::command::{CommandError, bad_choice};
use crate::dev::DevError;
use crate::dev_client::{self, post_json};
use crate::dev_http::DevReply;
use crate::imagegen::MAXIMUM_NAME_CHARS;
use crate::imagegen::model::plain_name;
use crate::persona::options::take_options;
use crate::persona::request::{PersonaReply, PersonaSummary};
use crate::persona::{Painting, make_walk_plate, show_walk_plate};
use crate::world::WalkNoted;
use df_content::world_bible::is_valid_id;

/// How far one step toward a target `before` away can leave it: within reach still for the step
/// that reaches it (its close-up), one band closer across the square, and as far or one band closer
/// down the road or on the horizon.
pub fn after_step(before: Distance) -> &'static [Distance] {
    match before {
        Distance::WithinReach => &[Distance::WithinReach],
        Distance::AcrossTheSquare => &[Distance::WithinReach],
        Distance::DownTheRoad => &[Distance::DownTheRoad, Distance::AcrossTheSquare],
        Distance::OnTheHorizon => &[Distance::OnTheHorizon, Distance::DownTheRoad],
    }
}

/// How far one running step toward a target `before` away can leave it: nearer than the nearest a
/// walking step ([`after_step`]) may bring it, so a run always covers more, never past within
/// reach.
pub fn after_run(before: Distance) -> &'static [Distance] {
    match before {
        Distance::WithinReach | Distance::AcrossTheSquare | Distance::DownTheRoad => {
            &[Distance::WithinReach]
        }
        Distance::OnTheHorizon => &[Distance::AcrossTheSquare],
    }
}

/// How far one step backing away from a target `before` away can leave it: as far, or one band
/// farther (the horizon stays the horizon).
pub fn after_away(before: Distance) -> &'static [Distance] {
    match before {
        Distance::WithinReach => &[Distance::WithinReach, Distance::AcrossTheSquare],
        Distance::AcrossTheSquare => &[Distance::AcrossTheSquare, Distance::DownTheRoad],
        Distance::DownTheRoad => &[Distance::DownTheRoad, Distance::OnTheHorizon],
        Distance::OnTheHorizon => &[Distance::OnTheHorizon],
    }
}

/// Whether a target `before` away is within reach, so a step toward it can go right up to it
/// ([`arrives`] says whether a step does).
pub fn within_reach(before: Distance) -> bool {
    before == Distance::WithinReach
}

/// Whether the step `ask` arrives at its target: a step (never the turn, step 0) toward a thing
/// within reach ([`reaches`]). A way (a [`Direction`]: walking along a wall, off to the left) is
/// open-ended and never arrived at; the walk goes on until the player names where to.
pub fn arrives(ask: &ApproachAsk) -> bool {
    ask.step > 0
        && !ask.away
        && within_reach(ask.distance)
        && Direction::from_key(&ask.target).is_none()
}

/// How far the step `ask` may leave its target: any distance for step 0, a turn to face it from
/// where the movers stand (a look that way), else [`after_step`], or [`after_run`] for a run.
pub fn allowed_after(ask: &ApproachAsk) -> &'static [Distance] {
    if ask.step == 0 {
        &Distance::ALL
    } else if ask.away {
        after_away(ask.distance)
    } else if ask.pace == Pace::Run {
        after_run(ask.distance)
    } else {
        after_step(ask.distance)
    }
}

/// The word on a step made ahead's input that settles it: shown (`@take`) or kept unshown
/// (`@drop`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AheadWord {
    Take,
    Drop,
}

impl AheadWord {
    /// The word a line of the input is, by the protocol's exact spelling.
    pub fn of_line(line: &str) -> Option<Self> {
        match line.trim() {
            "@take" => Some(Self::Take),
            "@drop" => Some(Self::Drop),
            _ => None,
        }
    }
}

/// Whether a step is shown: a step not made ahead is, at once; one made ahead only once `@take`
/// comes on `input`. `@drop`, the end of the input or input that cannot be read keeps it unshown;
/// other lines are passed over.
pub async fn shown_after<R: AsyncBufRead + Unpin>(ahead: bool, input: &mut Lines<R>) -> bool {
    if !ahead {
        return true;
    }
    while let Ok(Some(line)) = input.next_line().await {
        match AheadWord::of_line(&line) {
            Some(AheadWord::Take) => return true,
            Some(AheadWord::Drop) => return false,
            None => {}
        }
    }
    false
}

/// What `move approach` is asked: the step, the plate it continues (`from`, a library picture
/// without its extension), the plate it makes and whether it is made ahead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApproachOptions {
    pub ask: ApproachAsk,
    pub from: String,
    pub plate: String,
    pub ahead: bool,
}

const APPROACH_KEYS: [&str; 10] = [
    "target", "toward", "distance", "step", "from", "plate", "ahead", "pace", "away", "pose",
];

/// Reads `target=<id> toward=<words> distance=<band> step=<n> from=<plate> plate=<name>
/// [ahead=true|false] [pace=walk|run|sneak] [away=true|false] [pose=<words>]`, each once and
/// nothing after them.
pub(super) fn parse_options(words: &[String]) -> Result<ApproachOptions, CommandError> {
    let usage = || CommandError::Usage(super::cli::MOVEMENT_USAGE);
    let (given, rest) = take_options(words, &APPROACH_KEYS);
    if !rest.is_empty() {
        return Err(usage());
    }
    let value = |key: &str| {
        given
            .iter()
            .rev()
            .find(|(seen, _)| *seen == key)
            .map(|(_, value)| *value)
    };
    let plate_name = |key: &str| -> Result<String, CommandError> {
        let name = value(key).ok_or_else(usage)?;
        if plain_name(name) {
            Ok(name.to_owned())
        } else {
            Err(bad_choice(
                key,
                name,
                format!("1 to {MAXIMUM_NAME_CHARS} letters, digits, '-' or '_', with no extension"),
            ))
        }
    };
    let target = value("target").ok_or_else(usage)?;
    if !is_valid_id(target) {
        return Err(bad_choice(
            "target",
            target,
            "a thing, person or direction id",
        ));
    }
    let toward = value("toward").map(str::trim).unwrap_or_default();
    if toward.is_empty() {
        return Err(usage());
    }
    let distance = value("distance").ok_or_else(usage)?;
    let distance = Distance::from_key(distance).ok_or_else(|| {
        bad_choice(
            "distance",
            distance,
            "within_reach, across_the_square, down_the_road or on_the_horizon",
        )
    })?;
    let step = value("step").ok_or_else(usage)?;
    let step = step
        .parse::<u32>()
        .map_err(|_| bad_choice("step", step, "0 for a turn, else a step number from 1"))?;
    let ahead = match value("ahead") {
        None | Some("false") => false,
        Some("true") => true,
        Some(other) => return Err(bad_choice("ahead", other, "true or false")),
    };
    let away = match value("away") {
        None | Some("false") => false,
        Some("true") => true,
        Some(other) => return Err(bad_choice("away", other, "true or false")),
    };
    let pace = match value("pace") {
        None => Pace::Walk,
        Some(pace) => {
            Pace::from_key(pace).ok_or_else(|| bad_choice("pace", pace, "walk, run or sneak"))?
        }
    };
    Ok(ApproachOptions {
        ask: ApproachAsk {
            target: target.to_owned(),
            toward: toward.to_owned(),
            distance,
            step,
            pace,
            away,
            pose: value("pose")
                .map(str::trim)
                .filter(|pose| !pose.is_empty())
                .map(str::to_owned),
        },
        from: plate_name("from")?,
        plate: plate_name("plate")?,
        ahead,
    })
}

fn failed(report: &[String], key: &str, message: &str) -> MoveError {
    MoveError::Failed {
        report: report.join("\n"),
        key: key.to_owned(),
        message: message.to_owned(),
    }
}

/// One moment of the step on the host's log (`MOVE approach ..`); a note the host did not take is
/// a line of the report instead.
async fn note(
    port: u16,
    options: &ApproachOptions,
    (after, outcome): (Option<Distance>, ApproachOutcome),
    took_ms: Option<u64>,
    report: &mut Vec<String>,
) {
    let note = ApproachNote {
        target: options.ask.target.clone(),
        step: options.ask.step,
        before: options.ask.distance,
        after,
        plate: Some(options.plate.clone()),
        ahead: options.ahead,
        outcome,
        took_ms,
    };
    let noted = post_json::<_, DevReply<WalkNoted, crate::world::WorldError>>(
        port,
        move_path::MOVE_APPROACH,
        &note,
    )
    .await;
    match noted {
        Ok(DevReply::Done { .. }) => {}
        Ok(DevReply::Rejected { error, .. }) => {
            report.push(format!("approach note not recorded: {}", error.key()));
        }
        Err(error) => report.push(format!("approach note not recorded: {error}")),
    }
}

/// The party of a step: the walker alone, as the shown scene has them or by their stored persona.
/// The people of the place before stay there.
async fn walker(
    port: u16,
    character: &str,
    cast: &CastList,
) -> Result<Vec<PersonaSummary>, MoveError> {
    super::cli::party_of(port, &present_personas(cast), &[character.to_owned()])
        .await
        .map_err(|error| match error {
            DevError::Move(error) => error,
            error => failed(&[], "personas_unreadable", &error.to_string()),
        })
}

/// The design request of the step: the walker goes toward the target's words; a person walked
/// toward (someone the shown scene has, not the walker) is met at the step's place as a resident
/// with the same id.
fn step_request(
    character: &str,
    party: Vec<PartyMember>,
    ask: &ApproachAsk,
    cast: &CastList,
) -> DesignRequest {
    let person = cast
        .members
        .iter()
        .any(|member| member.name == ask.target && member.name != character);
    DesignRequest {
        intent: MoveIntent {
            kind: MoveKind::Walk,
            who: vec![character.to_owned()],
            to: ask.toward.clone(),
            line: format!("step {} toward {}", ask.step, ask.toward),
        },
        party,
        context: None,
        model: None,
        walk: None,
        residents: if person {
            vec![ExpectedResident {
                id: ask.target.clone(),
                who: ask.toward.clone(),
            }]
        } else {
            Vec::new()
        },
        approach: Some(ask.clone()),
    }
}

/// How far the step's design puts the target (checked by the design route).
fn distance_after(design: &SceneDesign, target: &str) -> Option<Distance> {
    super::types::distance_of(&design.distances, target)
}

/// One step of `character` toward the target of `options`: reached (its close-up is the caller's),
/// or designed, painted after the plate before it, and shown with its place made the active one,
/// unless it was made ahead and not taken. The report has the design's lines (things and
/// distances among them), the picture's words and one `approach ..` line per moment.
pub(super) async fn run(
    port: u16,
    character: &str,
    options: &ApproachOptions,
    cast: &CastList,
) -> Result<String, MoveError> {
    let ask = &options.ask;
    let mut report: Vec<String> = Vec::new();
    let started = Instant::now();
    let movers = walker(port, character, cast).await?;
    let party: Vec<PartyMember> = movers
        .iter()
        .map(|summary| PartyMember {
            id: summary.id.clone(),
            display_name: summary.display_name.clone(),
        })
        .collect();
    let request = step_request(character, party, ask, cast);
    let design = match post_json::<_, DesignReply>(port, move_path::MOVE_DESIGN, &request).await {
        Ok(DevReply::Done { value }) => value,
        Ok(DevReply::Rejected { error, message }) => {
            note(
                port,
                options,
                (None, ApproachOutcome::Failed),
                None,
                &mut report,
            )
            .await;
            return Err(failed(&report, error.key(), &message));
        }
        Err(error) => return Err(failed(&report, "route_unreachable", &error.to_string())),
    };
    let after = distance_after(&design, &ask.target);
    report.push(design_text(&design));
    let made = paint_step(port, options, &design, &movers, &mut report).await;
    let (mut made, mut present) = match made {
        Ok(made) => made,
        Err((key, message)) => {
            note(
                port,
                options,
                (after, ApproachOutcome::Failed),
                None,
                &mut report,
            )
            .await;
            return Err(failed(&report, &key, &message));
        }
    };
    // A pose plate is checked: the figure must take the stance. Painted otherwise, it is painted
    // once more under a name of its own, when the first painting left the time for it.
    let repainted;
    let options = match ask.pose.as_deref().filter(|_| ask.step == 0) {
        Some(pose) => {
            let shows = pose_shown(port, character, &options.plate, pose, &mut report).await;
            let again = ApproachOptions {
                plate: format!("{}-again", options.plate),
                ..options.clone()
            };
            let time_left = started.elapsed() <= POSE_REPAINT_WITHIN;
            let painted_again = if shows == Some(false) && time_left {
                paint_step(port, &again, &design, &movers, &mut report)
                    .await
                    .map_err(|(key, message)| {
                        report.push(format!("approach pose_repaint_failed {key}: {message}"));
                    })
                    .ok()
            } else {
                None
            };
            report.push(format!(
                "approach pose_check plate={} checked={} shows={} repainted={}{}",
                options.plate,
                if shows.is_some() { "yes" } else { "no" },
                match shows {
                    Some(true) => "yes",
                    Some(false) => "no",
                    None => "-",
                },
                u8::from(painted_again.is_some()),
                if shows == Some(false) && !time_left {
                    " (no time left to paint it again)"
                } else {
                    ""
                }
            ));
            match painted_again {
                Some((made_again, present_again)) => {
                    made = made_again;
                    present = present_again;
                    repainted = again;
                    &repainted
                }
                None => options,
            }
        }
        None => options,
    };
    let made_ms = saturating_millis(started.elapsed());
    note(
        port,
        options,
        (after, ApproachOutcome::Made),
        Some(made_ms),
        &mut report,
    )
    .await;
    let distance = format!(
        "{}->{}",
        ask.distance.key(),
        after.map_or("-", Distance::key)
    );
    report.push(format!(
        "approach made step={} target={} distance={distance} plate={} ahead={} made_ms={made_ms}",
        ask.step, ask.target, options.plate, options.ahead
    ));
    let waiting = Instant::now();
    let mut input = BufReader::new(tokio::io::stdin()).lines();
    if !shown_after(options.ahead, &mut input).await {
        note(
            port,
            options,
            (after, ApproachOutcome::Kept),
            None,
            &mut report,
        )
        .await;
        report.push(format!(
            "approach kept step={} plate={}: made ahead and not taken, in the library, not shown",
            ask.step, options.plate
        ));
        return Ok(report.join("\n"));
    }
    let waited_ms = saturating_millis(waiting.elapsed());
    let base = format!("{}.png", options.from);
    match show_walk_plate(port, made, &options.plate, Some(&base), &present).await {
        Ok(summary) => report.push(summary),
        Err((step, error)) => {
            note(
                port,
                options,
                (after, ApproachOutcome::Failed),
                None,
                &mut report,
            )
            .await;
            return Err(failed(&report, step.key(), &error.to_string()));
        }
    }
    note(
        port,
        options,
        (after, ApproachOutcome::Shown),
        Some(waited_ms),
        &mut report,
    )
    .await;
    // A stance taken where the hero stands (step 0 with a pose) is the same spot: its place keeps
    // the things of the place on screen, with their ids, looks and states (the gate shut stays
    // shut), never the design's own (host 55: a gate's state went onto the direction `ahead`).
    let same_spot = match ask.pose {
        Some(_) if ask.step == 0 => super::cli::things_here(port)
            .await
            .map(|things| SceneDesign {
                things,
                ..design.clone()
            }),
        _ => None,
    };
    if let Some(kept) = &same_spot {
        report.push(format!(
            "approach same_spot plate={} things_kept={}",
            options.plate,
            kept.things.len()
        ));
    }
    report.push(arrive(port, &options.plate, same_spot.as_ref().unwrap_or(&design)).await);
    let painted = if design.residents.is_empty() {
        Vec::new()
    } else {
        super::cli::painted_looks(port, character, &options.plate, &mut report).await
    };
    for resident in &design.residents {
        report.push(resident_persona(port, resident, &design.place, &painted).await);
    }
    report.push(format!(
        "approach shown step={} target={} distance={distance} plate={} waited_ms={waited_ms}",
        ask.step, ask.target, options.plate
    ));
    // Whether the next step toward it, from where this one leaves it, goes right up to it: the
    // runner's card and a look at it read this, never their own rule.
    if let Some(after) = after {
        let next = ApproachAsk {
            step: ask.step + 1,
            distance: after,
            ..ask.clone()
        };
        report.push(format!("approach next_reaches={}", arrives(&next)));
    }
    // The step that went up to it: the hero is beside the target now.
    if arrives(ask) {
        note(
            port,
            options,
            (after, ApproachOutcome::Reached),
            None,
            &mut report,
        )
        .await;
        report.push(format!(
            "approach reached target={} step={} plate={}",
            ask.target, ask.step, options.plate
        ));
    }
    Ok(report.join("\n"))
}

/// How long a pose plate may have taken, its check included, and still be painted once more when
/// the figure does not take the stance: the runner bounds the whole plate (`EXPLORE_POSE_WAIT`).
const POSE_REPAINT_WITHIN: std::time::Duration = std::time::Duration::from_secs(110);

/// Whether `who` as painted on `plate` takes `pose` (`persona perceive pose`, a closed vision
/// check); none when it could not be checked, with the report's line saying why.
async fn pose_shown(
    port: u16,
    who: &str,
    plate: &str,
    pose: &str,
    report: &mut Vec<String>,
) -> Option<bool> {
    use crate::persona::perceive::pose::{PoseCheckRequest, PoseChecked};
    use crate::persona::perceive::routes::path::PERSONA_PERCEIVE_POSE;
    let request = PoseCheckRequest {
        who: who.to_owned(),
        picture: format!("{plate}.png"),
        pose: pose.to_owned(),
        model: None,
    };
    let checked: Result<PersonaReply<PoseChecked>, _> =
        post_json(port, PERSONA_PERCEIVE_POSE, &request).await;
    match checked.map(dev_client::reply_value) {
        Ok(Ok(checked)) => {
            report.push(format!(
                "approach pose_seen plate={plate} shows={} ms={}: {}",
                checked.shows, checked.duration_ms, checked.because
            ));
            Some(checked.shows)
        }
        Ok(Err(error)) | Err(error) => {
            report.push(format!("approach pose_unchecked plate={plate}: {error}"));
            None
        }
    }
}

/// The step's plate, made after the plate before it and shown nowhere yet, and the ids of everyone
/// it names; the report gets the plate's words and what its room for pictures left out.
async fn paint_step(
    port: u16,
    options: &ApproachOptions,
    design: &SceneDesign,
    movers: &[PersonaSummary],
    report: &mut Vec<String>,
) -> Result<(crate::persona::MadePlate, Vec<String>), (String, String)> {
    // A stance taken where the hero stands (step 0 with a pose) is the picture's first
    // instruction, and each mover is doing exactly that: the plate before is the place only,
    // never their stance (host 56: "a sexy pose" came out mid-stride, as in the plate before).
    let pose = options
        .ask
        .pose
        .as_deref()
        .filter(|_| options.ask.step == 0);
    let posed = pose.map(|pose| SceneDesign {
        placements: design
            .placements
            .iter()
            .map(|placement| Placement {
                doing: pose.to_owned(),
                ..placement.clone()
            })
            .collect(),
        ..design.clone()
    });
    let design = posed.as_ref().unwrap_or(design);
    let Briefed {
        characters,
        shown,
        brief,
        picture,
    } = brief_design(port, design, movers).await?;
    if let Some(words) = picture {
        report.push(format!("picture: {words}"));
    }
    let (placed, sheets, lines) = plate_inputs(port, design, &characters, shown).await?;
    report.extend(lines);
    let present: Vec<String> = placed.iter().map(|request| request.name.clone()).collect();
    let prompt = match pose {
        Some(pose) => format!(
            "First and above all, the person stands {pose}: plainly that stance, still, never \
             walking or mid-stride. The picture given is the place only, the same view from the \
             same spot; the person's stance in it is not kept. {}",
            brief.prompt
        ),
        None => brief.prompt.clone(),
    };
    let painting = Painting {
        prompt: &prompt,
        avoid: &brief.avoid,
        sheets: &sheets,
    };
    let base = format!("{}.png", options.from);
    let made = make_walk_plate(port, &options.plate, &painting, placed, Some(&base))
        .await
        .map_err(|(step, error)| (step.key().to_owned(), error.to_string()))?;
    report.push(format!(
        "approach plate {}: {}",
        options.plate,
        made.found()
    ));
    Ok((made, present))
}
