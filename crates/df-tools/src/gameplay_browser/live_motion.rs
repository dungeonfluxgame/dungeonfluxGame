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
        self, ActorSlot, Card, CombatSnapshot, CombatStage, StageActor, TableKind, TableLine,
        TableRole, Transitions,
    },
    stage_conductor::{
        ClipOp, ClipSignal, Command, Conductor, IntentKind, IntentMark, LineId, ScriptLine,
        ViewPlan,
    },
    stage_director::{StageState, ViewDiff},
    stage_machines::{ActorState, BattleState, LineState, SceneState, reading_time_ms},
    voice_line::{VoiceLine, display_lines},
};
use std::{collections::VecDeque, rc::Weak, time::Duration};

const SEEN_MEMORY: usize = 64;
const LOG_LINES: usize = 3;
const DICE_LAND: Duration = Duration::from_millis(900);
const ROLL_RESULT: Duration = Duration::from_millis(1_050);
const LUNGE_AT: Duration = Duration::from_millis(700);
const TURN_CHIME_AT: Duration = Duration::from_millis(1_900);
const BANNER_SOUND_DELAY: Duration = Duration::from_millis(150);
const NARRATOR_NAME: &str = "Dungeon Master";
/// The fallback tick of a page whose animation frames are throttled.
const TIMER_TICK_MS: i32 = 100;

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
    /// A hero typing or holding to talk: a loader on that hero.
    Intent {
        kind: IntentKind,
        hero: String,
    },
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
    /// The clip the server voices this line with, when it is voiced at all.
    voice: Option<VoiceLine>,
}

impl LineDetail {
    fn of(line: &TableLine) -> Self {
        Self {
            speaker: line.speaker.clone(),
            role: line.role,
            kind: line.kind,
            text: line.text.clone(),
            voice: None,
        }
    }
}

struct Motion {
    client: Weak<RefCell<Client>>,
    conductor: Conductor<Detail>,
    /// The speaker the server says is writing a reply; shown once the lines before it are.
    thinking_for: Option<String>,
    previous: Option<CombatSnapshot>,
    scene_key: Option<String>,
    party_hit_points: Vec<u32>,
    seen: VecDeque<u64>,
    generation: u64,
    roll_tick: bool,
    overlay: Option<Element>,
    frame: Option<Closure<dyn FnMut(f64)>>,
    frame_armed: bool,
    /// A page whose frames are throttled (a background tab) still advances: a timer ticks the
    /// conductor too, so a watchdog never waits for a frame that does not come.
    timer: Option<Closure<dyn FnMut()>>,
    timer_armed: bool,
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
                conductor: Conductor::new(),
                thinking_for: None,
                previous: None,
                scene_key: None,
                party_hit_points: Vec::new(),
                seen: VecDeque::new(),
                generation: 0,
                roll_tick: false,
                overlay: None,
                frame: None,
                frame_armed: false,
                timer: None,
                timer_armed: false,
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

/// The heroes the table says are typing or holding to talk (an ephemeral signal of the phones).
fn intent_marks(journey: &rpc::JourneyView) -> Vec<IntentMark<Detail>> {
    journey
        .composing
        .iter()
        .filter_map(|mark| {
            let hero = usize::try_from(mark.hero_index).ok()?;
            let name = journey.party.get(hero)?.name.clone();
            let kind = match rpc::ComposeKind::try_from(mark.kind).ok()? {
                rpc::ComposeKind::Typing => IntentKind::Typing,
                rpc::ComposeKind::Speaking => IntentKind::Speaking,
                rpc::ComposeKind::Submitted => IntentKind::Submitted,
                rpc::ComposeKind::Stopped | rpc::ComposeKind::Unspecified => return None,
            };
            Some(IntentMark {
                slot: super::live_stage::slot_index(ActorSlot::Hero(hero))?,
                kind,
                detail: Detail::Intent { kind, hero: name },
            })
        })
        .collect()
}

/// The stage slot indices of the cast the view shows.
fn cast_slots(cast: &[StageActor]) -> Vec<usize> {
    cast.iter()
        .filter_map(|actor| super::live_stage::slot_index(actor.slot))
        .collect()
}

/// The non-player figure that carries a line: the one named like its speaker; for a scripted
/// line also the one speaking, else the first non-player figure present.
fn npc_slot(cast: &[StageActor], name: &str, anywhere: bool) -> Option<usize> {
    let npc = |actor: &&StageActor| matches!(actor.slot, ActorSlot::Npc(_));
    let named = cast.iter().filter(npc).find(|actor| actor.name == name);
    let chosen = if anywhere {
        named
            .or_else(|| cast.iter().filter(npc).find(|actor| actor.speaking))
            .or_else(|| cast.iter().find(npc))
    } else {
        named
    };
    chosen.and_then(|actor| super::live_stage::slot_index(actor.slot))
}

/// The server's voice line for `text` said by `speaker` (`None` for the narrator).
fn voice_for(voices: &[VoiceLine], speaker: Option<&str>, text: &str) -> Option<VoiceLine> {
    voices
        .iter()
        .find(|voice| voice.speaker() == speaker && voice.text.trim() == text.trim())
        .cloned()
}

fn narration_line(narration: &str, voices: &[VoiceLine], settled: bool) -> ScriptLine<Detail> {
    let voice = voice_for(voices, None, narration);
    ScriptLine {
        voiceable: voice.is_some(),
        read_ms: reading_time_ms(narration),
        slot: None,
        settled,
        detail: Detail::Line(LineDetail {
            speaker: NARRATOR_NAME.to_owned(),
            role: TableRole::Narrator,
            kind: TableKind::Speech,
            text: narration.to_owned(),
            voice,
        }),
    }
}

/// A table line as a line of the stage script, or `None` when no figure carries it.
fn script_line(
    line: &TableLine,
    cast: &[StageActor],
    voices: &[VoiceLine],
    settled: bool,
) -> Option<ScriptLine<Detail>> {
    let slot = match line.role {
        TableRole::Hero(index) => super::live_stage::slot_index(ActorSlot::Hero(index)),
        TableRole::Npc => npc_slot(cast, &line.speaker, line.scripted),
        TableRole::Narrator => None,
    };
    if slot.is_none() && line.role != TableRole::Narrator {
        return None;
    }
    let voice = match (line.kind, line.role) {
        (TableKind::Speech, TableRole::Npc) => voice_for(voices, Some(&line.speaker), &line.text),
        (TableKind::Speech, TableRole::Narrator) => voice_for(voices, None, &line.text),
        _ => None,
    };
    let mut detail = LineDetail::of(line);
    if line.role == TableRole::Narrator {
        detail.speaker = NARRATOR_NAME.to_owned();
    }
    detail.voice = voice.clone();
    Some(ScriptLine {
        voiceable: voice.is_some(),
        read_ms: reading_time_ms(&line.text),
        slot,
        settled,
        detail: Detail::Line(detail),
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
    let cast = presentation::scene_cast(&view);
    let key = format!("{}|{}|{}", journey.phase, scene.title, scene.scene_asset);
    let hit_points: Vec<u32> = journey
        .party
        .iter()
        .map(|member| member.hit_points)
        .collect();
    let (narration, voices) = match &view.audience {
        Some(rpc::view_message::Audience::Display(display)) => {
            (display.narration.clone(), display_lines(display))
        }
        _ => (String::new(), Vec::new()),
    };
    let intents = intent_marks(journey);
    let sound_on = live_voice::effects_gate().is_some();
    let thinking = presentation::replying_speaker(&view);
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
        let scripted = fresh
            .iter()
            .rev()
            .find(|line| line.scripted && line.kind == TableKind::Speech)
            .copied();
        // After a reload the line to leave on screen is the speaker's last answer to a hero
        // when that is the newest thing said, otherwise the scene's own line.
        let reply_now = lines
            .last()
            .filter(|line| line.kind == TableKind::Speech && line.role == TableRole::Npc);
        // A hero's typed line and what a hero did outside a fight go to the stage in order;
        // they are table talk, so they never hold an outcome back.
        let in_fight = transitions.new_roll || snapshot.stage != CombatStage::Absent;
        let mut script: Vec<ScriptLine<Detail>> = Vec::new();
        if restart {
            // A reload shows the current line silently, attributed to its speaker; with no
            // speaker line, the narration stands in the box.
            let settled = reply_now.or(scripted.filter(|line| line.role == TableRole::Npc));
            match settled {
                Some(line) => script.extend(script_line(line, &cast, &voices, true)),
                None if !narration.is_empty() => {
                    script.push(narration_line(&narration, &voices, true));
                }
                None => {}
            }
        } else if scene_changed {
            if !narration.is_empty() {
                script.push(narration_line(&narration, &voices, false));
            }
            if let Some(line) = scripted {
                script.extend(script_line(line, &cast, &voices, false));
            }
        } else {
            script.extend(
                fresh
                    .iter()
                    .filter(|line| match line.kind {
                        TableKind::Speech => {
                            matches!(line.role, TableRole::Hero(_))
                                || (line.role == TableRole::Npc && !line.scripted)
                        }
                        TableKind::Action => !in_fight && line.role != TableRole::Narrator,
                    })
                    .filter_map(|line| script_line(line, &cast, &voices, false)),
            );
        }
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
            enters_combat: transitions.entered_combat,
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
        };
        let plan = ViewPlan {
            diff,
            script,
            cast: cast_slots(&cast),
        };
        let chime = transitions.turn_changed && !transitions.new_roll;
        motion.previous = Some(snapshot.clone());
        motion.scene_key = Some(key.clone());
        motion.party_hit_points = hit_points.clone();
        motion.generation = generation;
        motion.thinking_for = thinking.clone();
        if let Some(overlay) = motion.overlay.take() {
            overlay.remove();
        }
        motion.conductor.set_sound(sound_on);
        motion.conductor.set_battlefield_expected(gpu_expected());
        let mut commands = if restart {
            motion.conductor.restart(now, plan)
        } else {
            motion.conductor.on_view(now, plan)
        };
        commands.extend(motion.conductor.on_intents(now, intents));
        (commands, chime)
    });
    let Some((commands, chime)) = planned else {
        return Ok(());
    };
    if chime {
        live_sfx::play(Cue::TurnChime, Duration::ZERO);
    }
    let applied = apply(client, commands);
    let published = publish_state(client);
    ensure_frame();
    applied.and(published)
}

/// Shows the thinking mark at the speaker the server says is writing a reply, once every line
/// before the reply has been shown, if that speaker stands in this scene; removes it otherwise.
fn show_thinking(client: &Rc<RefCell<Client>>, speaker: Option<&str>) -> Result<(), JsValue> {
    let state = client.borrow();
    let Some(stage) = state.live.tv.as_ref() else {
        return Ok(());
    };
    let slot = speaker.and_then(|name| {
        stage
            .figures()
            .slot_named(name)
            .filter(|slot| matches!(slot, ActorSlot::Npc(_)))
    });
    stage.figures().set_thinking(slot)
}

/// Publishes the machines' states on the stage element, so a browser check asserts on states
/// instead of sleeping, and shows the thinking mark when the lines before the reply are done.
fn publish_state(client: &Rc<RefCell<Client>>) -> Result<(), JsValue> {
    let Some((scene, line, pending, waiting_lines, thinking, speaker, battle)) =
        with_motion(|motion| {
            (
                motion.conductor.scene_state(),
                motion.conductor.focus_line(),
                motion.conductor.has_pending(),
                motion.conductor.lines_waiting(),
                motion.thinking_for.clone(),
                motion.conductor.active_speaker(),
                motion.conductor.battle_state(),
            )
        })
    else {
        return Ok(());
    };
    {
        let state = client.borrow();
        if let Some(stage) = state.live.tv.as_ref() {
            let root = stage.root();
            root.set_attribute("data-scene-state", scene.name())?;
            match line {
                Some((id, line_state)) => {
                    root.set_attribute("data-line-state", line_state.name())?;
                    root.set_attribute("data-line-id", &id.to_string())?;
                }
                None => {
                    root.set_attribute("data-line-state", "idle")?;
                    root.remove_attribute("data-line-id")?;
                }
            }
            root.set_attribute("data-stage-pending", if pending { "true" } else { "false" })?;
            root.set_attribute("data-battle-state", battle.name())?;
            // Whose words the stage shows follows the line being heard, not the raw view.
            stage.set_speaker(match speaker {
                None => "",
                Some(None) => "narrator",
                Some(Some(_)) => "character",
            })?;
        }
    }
    show_thinking(client, thinking.as_deref().filter(|_| !waiting_lines))
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
            Command::Scene(state) => set_scene_state(client, state),
            Command::Line {
                id,
                state,
                detail,
                slot,
            } => line_command(client, id, state, &detail, slot),
            Command::Clip { id, op, detail } => {
                clip_command(id, op, &detail);
                Ok(())
            }
            Command::Actor { slot, state } => actor_command(client, slot, state),
            Command::Battle(state) => set_battle_state(client, state),
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
            // The scene's exit: its bubbles are already closing through their machines.
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

/// Whether the browser offers WebGPU, so the 3D battlefield is expected to show a fight and the
/// flat combat view must not appear before it.
fn gpu_expected() -> bool {
    web_sys::window().is_some_and(|window| {
        js_sys::Reflect::get(window.navigator().as_ref(), &JsValue::from_str("gpu"))
            .is_ok_and(|gpu| gpu.is_object())
    })
}

fn set_battle_state(client: &Rc<RefCell<Client>>, state: BattleState) -> Result<(), JsValue> {
    match client.borrow().live.tv.as_ref() {
        Some(stage) => stage
            .root()
            .set_attribute("data-battle-state", state.name()),
        None => Ok(()),
    }
}

fn set_scene_state(client: &Rc<RefCell<Client>>, state: SceneState) -> Result<(), JsValue> {
    match client.borrow().live.tv.as_ref() {
        Some(stage) => stage.root().set_attribute("data-scene-state", state.name()),
        None => Ok(()),
    }
}

/// Applies one line machine transition to the display: the narrator's box, or the bubble at
/// the figure that speaks it. A character's bubble pops softly as its line opens.
fn line_command(
    client: &Rc<RefCell<Client>>,
    id: LineId,
    state: LineState,
    detail: &Detail,
    slot: Option<usize>,
) -> Result<(), JsValue> {
    if let (Detail::Intent { kind, hero }, Some(position)) = (detail, slot) {
        return match client.borrow().live.tv.as_ref() {
            Some(stage) => stage.figures().set_intent(position, id, state, *kind, hero),
            None => Ok(()),
        };
    }
    let Detail::Line(line) = detail else {
        return Ok(());
    };
    let shown = {
        let state_ref = client.borrow();
        let Some(stage) = state_ref.live.tv.as_ref() else {
            return Ok(());
        };
        match slot {
            None => stage.narrator_line(state, &line.text)?,
            Some(position) => {
                let kind = match (line.kind, line.role) {
                    (TableKind::Action, _) => BubbleKind::Action,
                    (TableKind::Speech, TableRole::Hero(_)) => BubbleKind::Typed,
                    (TableKind::Speech, _) => BubbleKind::Speech,
                };
                stage.figures().set_line(
                    position,
                    id,
                    state,
                    &super::live_stage::LineBubble {
                        kind,
                        speaker: &line.speaker,
                        text: &line.text,
                        voiced: line.voice.is_some() && line.role == TableRole::Npc,
                    },
                )?
            }
        }
    };
    if shown && state == LineState::Opening && slot.is_some() {
        live_sfx::play(Cue::BubblePop, Duration::ZERO);
    }
    Ok(())
}

fn clip_command(id: LineId, op: ClipOp, detail: &Detail) {
    let Detail::Line(line) = detail else {
        return;
    };
    let Some(voice) = line.voice.as_ref() else {
        return;
    };
    match op {
        ClipOp::Request => live_voice::request(id, voice),
        ClipOp::Play => live_voice::play(id),
        ClipOp::Cancel => live_voice::cancel(id),
    }
}

fn actor_command(
    client: &Rc<RefCell<Client>>,
    slot: usize,
    state: ActorState,
) -> Result<(), JsValue> {
    match client.borrow().live.tv.as_ref() {
        Some(stage) => stage.figures().set_actor_state(slot, state),
        None => Ok(()),
    }
}

/// The voice player reports about the clip of line `id`; the line machine follows it.
pub(super) fn clip_signal(id: LineId, signal: ClipSignal) {
    let now = now_ms();
    let planned = with_motion(|motion| {
        (
            motion.client.upgrade(),
            motion.conductor.on_clip(now, id, signal),
        )
    });
    let Some((Some(client), commands)) = planned else {
        return;
    };
    // Errors of a redraw are already reported by `redraw_current`.
    let _applied = apply(&client, commands);
    let _published = publish_state(&client);
    ensure_frame();
}

/// Sound was unlocked, muted or unmuted: later lines ask for a clip only while it is on.
pub(super) fn sound_changed(on: bool) {
    with_motion(|motion| motion.conductor.set_sound(on));
}

/// Arms one animation frame and one fallback timer while the conductor is busy; both stop
/// re-arming when idle.
fn ensure_frame() {
    arm_frame();
    arm_timer();
}

fn arm_frame() {
    let armed = with_motion(|motion| {
        if motion.frame_armed || !motion.conductor.is_busy() {
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
        // Without frames the timer still advances the conductor.
        let _cleared = with_motion(|motion| motion.frame_armed = false);
    }
}

fn arm_timer() {
    let armed = with_motion(|motion| {
        if motion.timer_armed || !motion.conductor.is_busy() {
            return false;
        }
        if motion.timer.is_none() {
            let callback = Closure::wrap(Box::new(timer_tick) as Box<dyn FnMut()>);
            motion.timer = Some(callback);
        }
        motion.timer_armed = true;
        true
    });
    if armed != Some(true) {
        return;
    }
    let scheduled = with_motion(|motion| {
        let (Some(window), Some(callback)) = (web_sys::window(), motion.timer.as_ref()) else {
            return false;
        };
        window
            .set_timeout_with_callback_and_timeout_and_arguments_0(
                callback.as_ref().unchecked_ref(),
                TIMER_TICK_MS,
            )
            .is_ok()
    });
    if scheduled != Some(true) {
        let _cleared = with_motion(|motion| motion.timer_armed = false);
    }
}

fn timer_tick() {
    let _cleared = with_motion(|motion| motion.timer_armed = false);
    tick(now_ms());
    ensure_frame();
}

fn frame(timestamp: f64) {
    let _cleared = with_motion(|motion| motion.frame_armed = false);
    tick(timestamp as u64);
    ensure_frame();
}

/// Advances the conductor to `now` and applies what it commands.
fn tick(now: u64) {
    let commands = with_motion(|motion| {
        let mut commands = motion.conductor.on_tick(now);
        // A fight's entrance waits for the battlefield; ask how far it is.
        if motion.conductor.battle_state() == BattleState::Entering {
            commands.extend(
                motion
                    .conductor
                    .on_battlefield(now, super::live_battlefield::entrance()),
            );
        }
        (motion.client.upgrade(), commands)
    });
    if let Some((Some(client), commands)) = commands {
        // Errors of a redraw are already reported by `redraw_current`.
        let _applied = apply(&client, commands);
        let _published = publish_state(&client);
    }
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
