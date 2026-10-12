//! Applies the presentation director's commands to the display, and the phone's small share
//! of motion.
//!
//! For the display, an accepted view does not redraw at once. `presentation` works out what
//! changed, `stage_director` orders it (a killing blow is seen before the scene moves on, a
//! scene change leaves, shows the wordmark and enters, one voice at a time) and this module
//! turns each command into DOM attributes, CSS animations and sound cues. The server stays
//! the authority and the newest accepted view is always what a `Render` command draws.
//! Nothing here sends a request or delays an input; the phone never waits for the display.
//!
//! Time comes from `requestAnimationFrame`, which runs only while the director is busy. A
//! hidden tab, a first view or a new connection restarts at the current view with nothing
//! queued, so stale cues are skipped, not replayed.
use super::live_sfx::{self, Cue};
use super::live_stage::{BubbleKind, FloatKind, Fx};
use super::*;
use crate::{
    presentation::{
        self, ActorSlot, Card, CombatSnapshot, CombatStage, TableKind, TableLine, TableRole,
        Transitions,
    },
    stage_director::{Command, CueKind, StageDirector, StageState, ViewDiff},
};
use std::{collections::VecDeque, rc::Weak, time::Duration};

const SEEN_MEMORY: usize = 64;
const LOG_LINES: usize = 3;
const MINIMUM_LINE_HOLD_MS: u32 = 2_500;
const MAXIMUM_LINE_HOLD_MS: u32 = 7_000;
const UNVOICED_NARRATION_CAP_MS: u32 = 4_000;
const DICE_LAND: Duration = Duration::from_millis(900);
const ROLL_RESULT: Duration = Duration::from_millis(1_050);
const LUNGE_AT: Duration = Duration::from_millis(700);
const TURN_CHIME_AT: Duration = Duration::from_millis(1_900);
const BANNER_SOUND_DELAY: Duration = Duration::from_millis(150);
const NARRATOR_NAME: &str = "Dungeon Master";

/// What a cue carries; the director treats it as opaque.
#[derive(Clone)]
enum Detail {
    Roll(Box<RollDetail>),
    Line(LineDetail),
    Banner {
        victory: bool,
    },
    /// The title card of the story's beginning or the closing card of a fight.
    Card(Box<Card>),
    Heal,
}

#[derive(Clone)]
struct RollDetail {
    attacker: String,
    target: String,
    hit: bool,
    critical: bool,
    grazed: bool,
    damage: u32,
    target_down: bool,
    turn_changed: bool,
    action_line: String,
    description: Option<RollDescriptionOwned>,
}

#[derive(Clone)]
struct RollDescriptionOwned {
    label: String,
    value: String,
    explanation: String,
    source: String,
    verdict: &'static str,
    tone: &'static str,
}

#[derive(Clone)]
struct LineDetail {
    speaker: String,
    role: TableRole,
    kind: TableKind,
    text: String,
    /// Said at the table rather than scripted for the scene: it belongs on the figure named
    /// as its speaker and nowhere else, so a reply that outlives its beat is not shown on a
    /// stranger.
    conversational: bool,
}

impl LineDetail {
    fn of(line: &TableLine) -> Self {
        Self {
            speaker: line.speaker.clone(),
            role: line.role,
            kind: line.kind,
            text: line.text.clone(),
            conversational: !line.scripted,
        }
    }
}

struct Motion {
    client: Weak<RefCell<Client>>,
    director: StageDirector<Detail>,
    previous: Option<CombatSnapshot>,
    scene_key: Option<String>,
    party_hit_points: Vec<u32>,
    seen: VecDeque<u64>,
    generation: u64,
    roll_tick: bool,
    overlay: Option<Element>,
    frame: Option<Closure<dyn FnMut(f64)>>,
    frame_armed: bool,
    log: VecDeque<TableLine>,
}

thread_local! {static MOTION:RefCell<Option<Motion>>=const{RefCell::new(None)};}

fn with_motion<R>(operate: impl FnOnce(&mut Motion) -> R) -> Option<R> {
    MOTION.with(|slot| slot.borrow_mut().as_mut().map(operate))
}

fn now_ms() -> u64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map_or(0, |performance| performance.now() as u64)
}

fn hold_for(text: &str) -> u32 {
    let characters = text.chars().count() as u32;
    (1_800 + 35 * characters).clamp(MINIMUM_LINE_HOLD_MS, MAXIMUM_LINE_HOLD_MS)
}

fn journey_scene(view: &rpc::ViewMessage) -> Option<(&rpc::JourneyView, &rpc::GameplayScene)> {
    match &view.audience {
        Some(rpc::view_message::Audience::Display(display)) => {
            Some((display.journey.as_ref()?, display.scene.as_ref()?))
        }
        Some(rpc::view_message::Audience::Player(player)) => {
            Some((player.journey.as_ref()?, player.scene.as_ref()?))
        }
        None => None,
    }
}

fn ensure_motion(client: &Rc<RefCell<Client>>) {
    MOTION.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            *slot = Some(Motion {
                client: Rc::downgrade(client),
                director: StageDirector::new(),
                previous: None,
                scene_key: None,
                party_hit_points: Vec::new(),
                seen: VecDeque::new(),
                generation: 0,
                roll_tick: false,
                overlay: None,
                frame: None,
                frame_armed: false,
                log: VecDeque::new(),
            });
        }
    });
}

fn remember(seen: &mut VecDeque<u64>, sequence: u64) -> bool {
    if seen.contains(&sequence) {
        return false;
    }
    if seen.len() >= SEEN_MEMORY {
        seen.pop_front();
    }
    seen.push_back(sequence);
    true
}

fn roll_detail(
    journey: &rpc::JourneyView,
    lines: &[TableLine],
    transitions: Transitions,
) -> Option<RollDetail> {
    let outcome = journey.combat.as_ref()?.outcomes.last()?;
    let description = live_combat::describe(outcome).map(|description| RollDescriptionOwned {
        label: description.label,
        value: description.value,
        explanation: description.explanation,
        source: description.source,
        verdict: description.verdict,
        tone: description.tone,
    });
    // The server's own line for this attacker; the roll's phrase only while it has none yet.
    let action_line = lines
        .iter()
        .rev()
        .find(|line| line.kind == TableKind::Action && line.speaker == outcome.actor_name)
        .map_or_else(
            || presentation::action_text(outcome),
            |line| line.text.clone(),
        );
    Some(RollDetail {
        attacker: outcome.actor_name.clone(),
        target: outcome.target_name.clone(),
        hit: outcome.hit,
        critical: outcome.critical,
        grazed: outcome.grazed,
        damage: outcome.damage,
        target_down: outcome.knocked_out || outcome.target_hit_points == 0,
        turn_changed: transitions.turn_changed,
        action_line,
        description,
    })
}

/// Called for every newly accepted display view instead of an immediate redraw.
pub(super) fn on_view(client: &Rc<RefCell<Client>>) -> Result<(), JsValue> {
    ensure_motion(client);
    let (view, generation, hidden) = {
        let state = client.borrow();
        let Some(view) = state
            .views
            .as_ref()
            .and_then(|views| views.current().cloned())
        else {
            return Ok(());
        };
        (view, state.generation, state.document.hidden())
    };
    let Some((journey, scene)) = journey_scene(&view) else {
        return redraw(client);
    };
    let snapshot = presentation::snapshot(journey, scene);
    let lines = presentation::table_lines(&view);
    let key = format!("{}|{}|{}", journey.phase, scene.title, scene.scene_asset);
    let hit_points: Vec<u32> = journey
        .party
        .iter()
        .map(|member| member.hit_points)
        .collect();
    let narration = match &view.audience {
        Some(rpc::view_message::Audience::Display(display)) => display.narration.clone(),
        _ => String::new(),
    };
    let voiced = live_voice::effects_gate().is_some();
    let now = now_ms();
    let planned = with_motion(|motion| {
        // The server restarts its table-log sequence on a new watch connection.
        if motion.generation != generation {
            motion.seen.clear();
        }
        let restart = motion.previous.is_none() || motion.generation != generation || hidden;
        let transitions = presentation::transitions(motion.previous.as_ref(), &snapshot);
        let scene_changed = motion
            .scene_key
            .as_deref()
            .is_some_and(|known| known != key)
            || transitions.entered_combat
            || transitions.finished.is_some();
        let fresh: Vec<&TableLine> = lines
            .iter()
            .filter(|line| remember(&mut motion.seen, line.sequence))
            .collect();
        let speech = fresh
            .iter()
            .rev()
            .find(|line| {
                line.scripted && line.kind == TableKind::Speech && line.role == TableRole::Npc
            })
            .map(|line| LineDetail::of(line));
        // After a reload the line to leave on screen is the speaker's last answer to a hero
        // when that is the newest thing said, otherwise the scene's own line.
        let reply_now = lines
            .last()
            .filter(|line| line.kind == TableKind::Speech && line.role == TableRole::Npc)
            .map(LineDetail::of);
        // A hero's typed line and what a hero did outside a fight are shown at once: they are
        // table talk, so no cue waits for them and they never hold an outcome back.
        let in_fight = transitions.new_roll || snapshot.stage != CombatStage::Absent;
        let immediate: Vec<LineDetail> = if restart || scene_changed {
            Vec::new()
        } else {
            fresh
                .iter()
                .filter(|line| match line.kind {
                    TableKind::Speech => {
                        matches!(line.role, TableRole::Hero(_))
                            || (line.role == TableRole::Npc && !line.scripted)
                    }
                    TableKind::Action => !in_fight && line.role != TableRole::Narrator,
                })
                .map(|line| LineDetail::of(line))
                .collect()
        };
        let healed = !transitions.new_roll
            && motion.party_hit_points.len() == hit_points.len()
            && hit_points
                .iter()
                .zip(&motion.party_hit_points)
                .zip(&journey.party)
                .any(|((now, before), member)| member.character_ready && now > before);
        let roll = if transitions.new_roll {
            roll_detail(journey, &lines, transitions)
        } else {
            None
        };
        let diff = ViewDiff {
            scene_changed,
            roll: roll.clone().map(|roll| Detail::Roll(Box::new(roll))),
            impact: roll.clone().map(|roll| Detail::Roll(Box::new(roll))),
            defeat: roll
                .filter(|roll| roll.target_down)
                .map(|roll| Detail::Roll(Box::new(roll))),
            heal: healed.then_some(Detail::Heal),
            title: transitions
                .story_begun
                .then(|| Detail::Card(Box::new(presentation::title_card(scene, &narration)))),
            banner: transitions
                .finished
                .map(|victory| Detail::Banner { victory }),
            closing: transitions.finished.map(|victory| {
                Detail::Card(Box::new(presentation::closing_card(
                    journey, scene, &narration, victory,
                )))
            }),
            narration: (scene_changed && !narration.is_empty()).then(|| {
                let hold = if voiced {
                    hold_for(&narration)
                } else {
                    hold_for(&narration).min(UNVOICED_NARRATION_CAP_MS)
                };
                (
                    Detail::Line(LineDetail {
                        speaker: NARRATOR_NAME.to_owned(),
                        role: TableRole::Narrator,
                        kind: TableKind::Speech,
                        text: narration.clone(),
                        conversational: false,
                    }),
                    hold,
                )
            }),
            speech: speech
                .clone()
                .map(|line| (Detail::Line(line.clone()), hold_for(&line.text))),
            voiced,
        };
        let chime = transitions.turn_changed && !transitions.new_roll;
        motion.previous = Some(snapshot.clone());
        motion.scene_key = Some(key.clone());
        motion.party_hit_points = hit_points.clone();
        motion.generation = generation;
        if let Some(overlay) = motion.overlay.take() {
            overlay.remove();
        }
        let commands = if restart {
            motion.director.restart()
        } else {
            motion.director.on_view(now, diff)
        };
        (
            commands,
            restart.then_some(reply_now.or(speech)).flatten(),
            chime,
            immediate,
        )
    });
    let Some((commands, seed, chime, immediate)) = planned else {
        return Ok(());
    };
    if chime {
        live_sfx::play(Cue::TurnChime, Duration::ZERO);
    }
    let applied = apply(client, commands);
    let thinking = show_thinking(client, &view);
    if let Some(line) = seed {
        // A reload shows the current line silently, attributed to its speaker, and leaves the
        // character's words on screen rather than the narration.
        set_speaker(client, "character")?;
        show_line(client, &line, false)?;
    }
    for line in &immediate {
        show_line(client, line, false)?;
    }
    ensure_frame();
    applied.and(thinking)
}

/// Shows the thinking mark at the speaker the server says is writing a reply, if that speaker
/// stands in this scene, and removes it otherwise.
fn show_thinking(client: &Rc<RefCell<Client>>, view: &rpc::ViewMessage) -> Result<(), JsValue> {
    let state = client.borrow();
    let Some(stage) = state.live.tv.as_ref() else {
        return Ok(());
    };
    let slot = presentation::replying_speaker(view).and_then(|name| {
        stage
            .figures()
            .slot_named(&name)
            .filter(|slot| matches!(slot, ActorSlot::Npc(_)))
    });
    stage.figures().set_thinking(slot)
}

fn redraw(client: &Rc<RefCell<Client>>) -> Result<(), JsValue> {
    let drawn = redraw_current(client);
    live_voice::observe(client);
    drawn
}

fn apply(client: &Rc<RefCell<Client>>, commands: Vec<Command<Detail>>) -> Result<(), JsValue> {
    let mut first_error = None;
    for command in commands {
        let outcome = match command {
            Command::Render => {
                if let Some(Some(overlay)) = with_motion(|motion| motion.overlay.take()) {
                    overlay.remove();
                }
                redraw(client)
            }
            Command::Enter {
                state,
                detail,
                short,
            } => enter(client, state, detail.as_ref(), short),
            Command::Rest => set_stage(client, ""),
        };
        if let Err(error) = outcome {
            first_error.get_or_insert(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn set_stage(client: &Rc<RefCell<Client>>, name: &str) -> Result<(), JsValue> {
    match client.borrow().live.tv.as_ref() {
        Some(stage) => stage.set_stage_state(name),
        None => Ok(()),
    }
}

fn state_name(state: StageState) -> &'static str {
    match state {
        StageState::Idle => "",
        StageState::LeavingScene => "leaving",
        StageState::Interstitial => "interstitial",
        StageState::TitleCard => "title",
        StageState::EnteringScene => "entering",
        StageState::Narrating => "narrating",
        StageState::Speaking => "speaking",
        StageState::Rolling => "rolling",
        StageState::Impact => "impact",
        StageState::Defeat => "defeat",
        StageState::Healing => "healing",
        StageState::Aftermath => "aftermath",
        StageState::ClosingCard => "closing",
    }
}

fn enter(
    client: &Rc<RefCell<Client>>,
    state: StageState,
    detail: Option<&Detail>,
    short: bool,
) -> Result<(), JsValue> {
    battlefield_cue(state, detail);
    set_stage(client, state_name(state))?;
    match (state, detail) {
        (StageState::LeavingScene, _) => {
            // Nothing said in the beat being left follows into the next one.
            if let Some(stage) = client.borrow().live.tv.as_ref() {
                stage.figures().clear_bubbles()?;
            }
            set_speaker(client, "")?;
            live_sfx::cancel_unstarted();
            live_sfx::play(Cue::SceneTransition, Duration::ZERO);
        }
        (StageState::EnteringScene, _) => {
            if let Some(stage) = client.borrow().live.tv.as_ref() {
                stage.begin_entering()?;
            }
        }
        (StageState::Rolling, Some(Detail::Roll(roll))) => start_roll(client, roll, short)?,
        (StageState::Impact, Some(Detail::Roll(roll))) => impact(client, roll)?,
        (StageState::Defeat, Some(Detail::Roll(roll))) => {
            if let Some(stage) = client.borrow().live.tv.as_ref()
                && let Some(slot) = stage.figures().slot_named(&roll.target)
            {
                stage.figures().force_down(slot)?;
            }
        }
        (StageState::Narrating, _) => set_speaker(client, "narrator")?,
        (StageState::Speaking, Some(Detail::Line(line))) => {
            set_speaker(client, "character")?;
            show_line(client, line, true)?;
        }
        (StageState::TitleCard | StageState::ClosingCard, Some(Detail::Card(card))) => {
            let state = client.borrow();
            if let Some(stage) = state.live.tv.as_ref() {
                stage.show_card(&state.document, card)?;
            }
        }
        (StageState::Aftermath, Some(Detail::Banner { victory })) => {
            live_sfx::play(
                if *victory {
                    Cue::VictoryFanfare
                } else {
                    Cue::DefeatToll
                },
                BANNER_SOUND_DELAY,
            );
        }
        _ => {}
    }
    Ok(())
}

/// PLAY-F02 hook: the 3D battlefield follows this same cue stream (roll, then impact), so the dice
/// sound, the approach and strike, and the damage number stay in step. Copies what it needs.
fn battlefield_cue(state: StageState, detail: Option<&Detail>) {
    let summary = match detail {
        Some(Detail::Roll(roll)) => Some(super::live_battlefield::RollSummary {
            attacker: roll.attacker.clone(),
            target: roll.target.clone(),
            hit: roll.hit,
            critical: roll.critical,
            grazed: roll.grazed,
            damage: roll.damage,
            target_down: roll.target_down,
            card: roll
                .description
                .as_ref()
                .map(|description| df_render::battle::RollCard {
                    value: description.value.clone(),
                    label: description.label.clone(),
                    explanation: description.explanation.clone(),
                    verdict: description.verdict.to_owned(),
                    tone: verdict_name(roll).to_owned(),
                }),
        }),
        _ => None,
    };
    super::live_battlefield::on_cue(state_name(state), summary.as_ref());
}

fn roll_cue(roll: &RollDetail) -> Cue {
    if roll.hit && roll.critical {
        Cue::Critical
    } else if roll.hit || roll.grazed {
        Cue::Hit
    } else {
        Cue::Miss
    }
}

fn verdict_name(roll: &RollDetail) -> &'static str {
    if roll.hit && roll.critical {
        "critical"
    } else if roll.hit {
        "hit"
    } else if roll.grazed {
        "graze"
    } else {
        "miss"
    }
}

fn start_roll(client: &Rc<RefCell<Client>>, roll: &RollDetail, short: bool) -> Result<(), JsValue> {
    let tick = with_motion(|motion| {
        motion.roll_tick = !motion.roll_tick;
        motion.roll_tick
    })
    .unwrap_or(false);
    mark_roll(client, tick, verdict_name(roll))?;
    live_sfx::cancel_unstarted();
    live_sfx::play(Cue::DiceRattle, Duration::ZERO);
    live_sfx::play(Cue::DiceLand, DICE_LAND);
    live_sfx::play(roll_cue(roll), ROLL_RESULT);
    if roll.turn_changed {
        live_sfx::play(Cue::TurnChime, TURN_CHIME_AT);
    }
    if let Some(stage) = client.borrow().live.tv.as_ref() {
        let figures = stage.figures();
        if let (Some(attacker), Some(target)) = (
            figures.slot_named(&roll.attacker),
            figures.slot_named(&roll.target),
        ) {
            let direction = super::live_stage::Figures::direction(attacker, target);
            figures.fx(attacker, Fx::Lunge, LUNGE_AT, direction)?;
        }
    }
    if short {
        return Ok(());
    }
    show_final_roll(client, roll)
}

/// The final blow's roll is drawn over the combat screen the new view will replace.
fn show_final_roll(client: &Rc<RefCell<Client>>, roll: &RollDetail) -> Result<(), JsValue> {
    let finishing = client.borrow().views.as_ref().is_some_and(|views| {
        views
            .current()
            .and_then(journey_scene)
            .is_some_and(|(journey, scene)| {
                matches!(
                    presentation::snapshot(journey, scene).stage,
                    presentation::CombatStage::Finished { .. }
                )
            })
    });
    let (Some(description), true) = (roll.description.as_ref(), finishing) else {
        return Ok(());
    };
    let (document, stage_root) = {
        let state = client.borrow();
        let Some(stage) = state.live.tv.as_ref() else {
            return Ok(());
        };
        (state.document.clone(), stage.root().clone())
    };
    let overlay = node(&document, &stage_root, "div", "tv-final-roll", "")?;
    overlay.set_attribute("data-verdict", verdict_name(roll))?;
    let panel = node(&document, &overlay, "div", "tv-final-panel", "")?;
    let die = node(&document, &panel, "div", "tv-final-die", "")?;
    node(
        &document,
        &die,
        "span",
        "tv-final-number",
        &description.value,
    )?;
    let words = node(&document, &panel, "div", "tv-final-words", "")?;
    node(&document, &words, "h2", "", &description.label)?;
    node(
        &document,
        &words,
        "p",
        "tv-final-expl",
        &description.explanation,
    )?;
    let ribbon = node(
        &document,
        &words,
        "p",
        "tv-final-ribbon",
        description.verdict,
    )?;
    ribbon.set_attribute("data-tone", description.tone)?;
    node(
        &document,
        &words,
        "p",
        "tv-final-source",
        &description.source,
    )?;
    with_motion(|motion| {
        if let Some(previous) = motion.overlay.replace(overlay) {
            previous.remove();
        }
    });
    Ok(())
}

fn mark_roll(client: &Rc<RefCell<Client>>, tick: bool, verdict: &str) -> Result<(), JsValue> {
    let host = client.borrow().root.query_selector(".live-screen")?;
    if let Some(host) = host {
        host.set_attribute("data-roll-tick", if tick { "a" } else { "b" })?;
        host.set_attribute("data-roll-verdict", verdict)?;
    }
    Ok(())
}

fn impact(client: &Rc<RefCell<Client>>, roll: &RollDetail) -> Result<(), JsValue> {
    let state = client.borrow();
    let Some(stage) = state.live.tv.as_ref() else {
        return Ok(());
    };
    let figures = stage.figures();
    if let Some(target) = figures.slot_named(&roll.target) {
        if roll.hit || roll.grazed {
            figures.fx(target, Fx::Hit, Duration::ZERO, 1)?;
            let (text, kind) = if roll.critical {
                (format!("-{}!", roll.damage), FloatKind::Critical)
            } else {
                (format!("-{}", roll.damage), FloatKind::Damage)
            };
            figures.float_text(target, &text, kind, Duration::ZERO)?;
        } else if let Some(attacker) = figures.slot_named(&roll.attacker) {
            let direction = super::live_stage::Figures::direction(attacker, target);
            figures.fx(target, Fx::Dodge, Duration::ZERO, direction)?;
            figures.float_text(target, "Miss", FloatKind::Miss, Duration::ZERO)?;
        }
    }
    if let Some(attacker) = figures.slot_named(&roll.attacker) {
        figures.say(
            attacker,
            BubbleKind::Action,
            &roll.attacker,
            &roll.action_line,
            Duration::ZERO,
            false,
        )?;
    }
    Ok(())
}

/// A character's line at the figure that speaks it, with a soft pop when it appears: a
/// non-player line at the character named, a hero's line at that hero.
fn show_line(client: &Rc<RefCell<Client>>, line: &LineDetail, sound: bool) -> Result<(), JsValue> {
    let state = client.borrow();
    let Some(stage) = state.live.tv.as_ref() else {
        return Ok(());
    };
    let slot = match line.role {
        TableRole::Hero(index) => Some(ActorSlot::Hero(index)),
        TableRole::Npc if line.conversational => stage
            .figures()
            .slot_named(&line.speaker)
            .filter(|slot| matches!(slot, ActorSlot::Npc(_))),
        TableRole::Npc => stage.figures().npc_slot_for(&line.speaker),
        TableRole::Narrator => None,
    };
    let Some(slot) = slot else {
        return Ok(());
    };
    if line.role == TableRole::Npc && line.kind == TableKind::Speech {
        // The speaker's answer takes the place of the hero's words it answers.
        stage.figures().clear_typed();
        stage.set_speaker("character")?;
    }
    let kind = match (line.kind, line.role) {
        (TableKind::Action, _) => BubbleKind::Action,
        (TableKind::Speech, TableRole::Hero(_)) => BubbleKind::Typed,
        (TableKind::Speech, _) => BubbleKind::Speech,
    };
    stage.figures().say(
        slot,
        kind,
        &line.speaker,
        &line.text,
        Duration::ZERO,
        line.role == TableRole::Npc && line.kind == TableKind::Speech,
    )?;
    if sound {
        live_sfx::play(Cue::BubblePop, Duration::ZERO);
    }
    Ok(())
}

/// Which speaker the stage's words belong to: `narrator` (the Dungeon Master's caption),
/// `character` (that character's bubble) or empty before anyone has spoken in the scene.
fn set_speaker(client: &Rc<RefCell<Client>>, speaker: &str) -> Result<(), JsValue> {
    match client.borrow().live.tv.as_ref() {
        Some(stage) => stage.set_speaker(speaker),
        None => Ok(()),
    }
}

/// The audio of a line began: `speaker` is the character's name, or `None` for the narrator.
/// The display follows the voice, so the Dungeon Master's caption leaves and the character's
/// bubble appears (or the other way round) as the clip starts, whatever the timed sequence had
/// planned. The director orders the state; showing the bubble is idempotent, so a line the
/// director had no cue for still appears.
pub(super) fn voice_started(speaker: Option<&str>, text: &str) {
    let now = now_ms();
    let kind = if speaker.is_some() {
        CueKind::Speak
    } else {
        CueKind::Narrate
    };
    let planned = with_motion(|motion| {
        (
            motion.client.upgrade(),
            motion.director.on_clip_started(now, kind),
        )
    });
    let Some((Some(client), commands)) = planned else {
        return;
    };
    // Errors of a redraw are already reported by `redraw_current`.
    let _applied = apply(&client, commands);
    let shown = match speaker {
        Some(name) => set_speaker(&client, "character").and_then(|()| {
            show_line(
                &client,
                &LineDetail {
                    speaker: name.to_owned(),
                    role: TableRole::Npc,
                    kind: TableKind::Speech,
                    text: text.to_owned(),
                    conversational: false,
                },
                false,
            )
        }),
        None => set_speaker(&client, "narrator"),
    };
    // A refused attribute write leaves the previous caption; the next view corrects it.
    let _shown = shown;
    ensure_frame();
}

/// A voice clip ended: the line the director waits for may finish early. `character` is
/// whether it was a character's line rather than the narration.
pub(super) fn voice_over(character: bool) {
    let now = now_ms();
    let commands = with_motion(|motion| {
        let kind = if character {
            CueKind::Speak
        } else {
            CueKind::Narrate
        };
        (
            motion.client.upgrade(),
            motion.director.on_cue_finished(now, kind),
        )
    });
    if let Some((Some(client), commands)) = commands {
        // Errors of a redraw are already reported by `redraw_current`.
        let _applied = apply(&client, commands);
        ensure_frame();
    }
}

/// Arms one animation frame while the director is busy; it stops re-arming when idle.
fn ensure_frame() {
    let armed = with_motion(|motion| {
        if motion.frame_armed || motion.director.state() == StageState::Idle {
            return false;
        }
        if motion.frame.is_none() {
            let callback = Closure::wrap(Box::new(frame) as Box<dyn FnMut(f64)>);
            motion.frame = Some(callback);
        }
        motion.frame_armed = true;
        true
    });
    if armed != Some(true) {
        return;
    }
    let requested = with_motion(|motion| {
        let (Some(window), Some(callback)) = (web_sys::window(), motion.frame.as_ref()) else {
            return false;
        };
        window
            .request_animation_frame(callback.as_ref().unchecked_ref())
            .is_ok()
    });
    if requested != Some(true) {
        // Without frames the director cannot advance, so it restarts at the current view.
        let _cleared = with_motion(|motion| {
            motion.frame_armed = false;
            motion.director.restart()
        });
    }
}

fn frame(timestamp: f64) {
    let commands = with_motion(|motion| {
        motion.frame_armed = false;
        (
            motion.client.upgrade(),
            motion.director.on_tick(timestamp as u64),
        )
    });
    if let Some((Some(client), commands)) = commands {
        // Errors of a redraw are already reported by `redraw_current`.
        let _applied = apply(&client, commands);
    }
    ensure_frame();
}

/// Called for every newly accepted phone view: the dice tumble and the table log.
pub(super) fn on_player_view(client: &Rc<RefCell<Client>>) -> Result<(), JsValue> {
    ensure_motion(client);
    let Some(view) = client
        .borrow()
        .views
        .as_ref()
        .and_then(|views| views.current().cloned())
    else {
        return Ok(());
    };
    let Some((journey, scene)) = journey_scene(&view) else {
        return Ok(());
    };
    let snapshot = presentation::snapshot(journey, scene);
    let lines = presentation::table_lines(&view);
    let (hidden, generation) = {
        let state = client.borrow();
        (state.document.hidden(), state.generation)
    };
    // The server restarts its table-log sequence on a new watch connection.
    with_motion(|motion| {
        if motion.generation != generation {
            motion.generation = generation;
            motion.seen.clear();
        }
    });
    let key = format!("{}|{}", journey.phase, scene.title);
    let scene_changed = with_motion(|motion| {
        let changed = motion
            .scene_key
            .as_deref()
            .is_some_and(|known| known != key);
        motion.scene_key = Some(key.clone());
        changed
    })
    .unwrap_or(false);
    if scene_changed && !hidden {
        // A quick fade of the phone's content: the flag trick restarts the animation.
        let view = client.borrow().root.query_selector(".view")?;
        if let Some(view) = view {
            let next = if view.get_attribute("data-swap").as_deref() == Some("a") {
                "b"
            } else {
                "a"
            };
            view.set_attribute("data-swap", next)?;
        }
    }
    let rolled = with_motion(|motion| {
        let first = motion.previous.is_none();
        let transitions = presentation::transitions(motion.previous.as_ref(), &snapshot);
        motion.previous = Some(snapshot.clone());
        for line in &lines {
            if remember(&mut motion.seen, line.sequence)
                && !hidden
                && (!first || line.kind == TableKind::Speech)
            {
                if motion.log.len() >= LOG_LINES {
                    motion.log.pop_front();
                }
                motion.log.push_back(line.clone());
            }
        }
        if transitions.new_roll && !hidden {
            motion.roll_tick = !motion.roll_tick;
            Some(motion.roll_tick)
        } else {
            None
        }
    })
    .flatten();
    if let Some(tick) = rolled {
        let newest = journey
            .combat
            .as_ref()
            .and_then(|combat| combat.outcomes.last())
            .map(|outcome| {
                if outcome.hit && outcome.critical {
                    "critical"
                } else if outcome.hit {
                    "hit"
                } else if outcome.grazed {
                    "graze"
                } else {
                    "miss"
                }
            });
        if let Some(verdict) = newest {
            mark_roll(client, tick, verdict)?;
        }
    }
    render_log(client, &journey.party)
}

/// The compact "table log": the latest lines with their speaker above the phone's choices.
fn render_log(client: &Rc<RefCell<Client>>, party: &[rpc::PartyMemberView]) -> Result<(), JsValue> {
    let (document, root) = {
        let state = client.borrow();
        (state.document.clone(), state.root.clone())
    };
    if let Some(previous) = root.query_selector(".table-log")? {
        previous.remove();
    }
    let log: Vec<TableLine> =
        with_motion(|motion| motion.log.iter().cloned().collect()).unwrap_or_default();
    if log.is_empty() {
        return Ok(());
    }
    let anchor = match root.query_selector(".df-exploration .exploration-interaction")? {
        Some(anchor) => Some(anchor),
        None => root.query_selector(".combat-lower .combat-action-rail")?,
    };
    let Some(anchor) = anchor else {
        return Ok(());
    };
    let Some(parent) = anchor.parent_node() else {
        return Ok(());
    };
    let strip = document.create_element("div")?;
    strip.set_class_name("table-log");
    strip.set_attribute("role", "log")?;
    strip.set_attribute("aria-label", "What has happened at the table")?;
    for line in &log {
        let row = node(&document, &strip, "p", "table-line", "")?;
        row.set_attribute(
            "data-kind",
            if line.kind == TableKind::Speech {
                "speech"
            } else {
                "action"
            },
        )?;
        let chip = node(&document, &row, "span", "table-chip", "")?;
        match line.role {
            TableRole::Hero(index) if party.get(index).is_some() => {
                chip.set_attribute(
                    "style",
                    &format!(
                        "background-image:url({}/portrait-hero-{})",
                        live_tv::GENERATED,
                        index + 1
                    ),
                )?;
            }
            TableRole::Narrator => chip.set_text_content(Some("DM")),
            _ => chip.set_text_content(Some(
                &line
                    .speaker
                    .chars()
                    .next()
                    .map(|first| first.to_uppercase().collect::<String>())
                    .unwrap_or_default(),
            )),
        }
        node(&document, &row, "b", "table-name", &line.speaker)?;
        node(&document, &row, "span", "table-text", &line.short_text())?;
    }
    parent.insert_before(&strip, Some(&anchor))?;
    Ok(())
}
