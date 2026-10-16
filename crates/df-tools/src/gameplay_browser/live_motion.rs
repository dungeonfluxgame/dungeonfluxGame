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
        self, ActorSlot, Card, CombatSnapshot, CombatStage, GridCell, StageActor, TableKind,
        TableLine, TableRole, Transitions,
    },
    stage_conductor::{
        ClipOp, ClipSignal, Command, Conductor, IntentKind, IntentMark, LineId, ScriptLine,
        ViewPlan,
    },
    stage_director::{StageState, ViewDiff},
    stage_machines::{ActorState, BattleState, ClipState, LineState, SceneState, reading_time_ms},
    voice_line::{VoiceLine, display_lines},
};
use std::{
    collections::{HashMap, VecDeque},
    rc::Weak,
    time::Duration,
};

const SEEN_MEMORY: usize = 64;
const LOG_LINES: usize = 3;
/// The land cue when no clip plays (a clip rest lands on its own manifest time).
const DICE_LAND: Duration = Duration::from_millis(900);
const ROLL_RESULT: Duration = Duration::from_millis(1_050);
/// The hit or miss lands just after the die does.
const RESULT_AFTER_LAND: Duration = Duration::from_millis(150);
const LUNGE_AT: Duration = Duration::from_millis(700);
const TURN_CHIME_AT: Duration = Duration::from_millis(1_900);
const BANNER_SOUND_DELAY: Duration = Duration::from_millis(150);
/// The slot of a line whose speaker has no figure on the flat stage; its bubble is the
/// battlefield's, or none.
const OFF_STAGE_SLOT: usize = usize::MAX;
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
        /// The finished fight's Dungeon Master closing line, installed when the banner
        /// rises: the ending is never skipped, so neither is its line.
        closing: Vec<ScriptLine<Detail>>,
    },
    /// The title card of the story's beginning or the closing card of a fight.
    Card(Box<Card>),
    Heal,
    /// A hero typing or talking: a loader on that hero.
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
    fight_over: bool,
    turn_changed: bool,
    action_line: String,
    description: Option<RollDescriptionOwned>,
    /// The attack's name and the rules' damage type, for the named damage text.
    attack_label: Option<String>,
    damage_type: String,
    /// The attack presentation follows the resolved style and weapon id, not the figure's art.
    attack_style: df_render::battle::AttackStyle,
    /// The server's d20 numbers for the visible die: the kept face, every draw (a pair for
    /// advantage or disadvantage), and the roll mode. The browser settles only on `attack_die`.
    attack_die: u32,
    faces: Vec<u32>,
    roll_mode: i32,
    /// The server's voice family for the attacker and the target, for effort, pain and fall
    /// sounds on the flat stage. `None` stays silent: no voice is ever invented.
    attack_voice: Option<df_render::battle::VoiceSet>,
    target_voice: Option<df_render::battle::VoiceSet>,
    /// The first outcome of this attacker's turn within the view: its turn begins here.
    turn_start: bool,
    turn_only: bool,
    /// Where the attacker stood before this turn and where the server put it: shown as a walk
    /// before the turn's first attack.
    walk: Option<(GridCell, GridCell)>,
    /// The outcome's cries and narration as stage script lines. They ride the blow's cues
    /// and are installed after its dice have settled: a skipped blow stays silent.
    beat: Vec<ScriptLine<Detail>>,
    /// The server's beat sequence of this outcome: the one clock every layer follows.
    sequence: u64,
    /// The target's hit points after this outcome, from the outcome itself (never from the
    /// newest view, which may already carry later blows).
    target_hp_after: u32,
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
    fight_line: bool,
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
            fight_line: false,
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
    /// The narration the scene last put on the stage, so only new narration is spoken again.
    narration: String,
    party_hit_points: Vec<u32>,
    /// Where each combatant stood in the last view, to tell the walk a new view carries.
    cells: HashMap<String, GridCell>,
    /// Lines the battlefield is showing as a bubble over their speaker.
    battlefield_lines: Vec<LineId>,
    /// Recent fight lines, retained briefly so late actual voice callbacks are recorded.
    fight_lines: VecDeque<LineId>,
    /// Actual clip callbacks can arrive just before the opened line is passed to the adapter.
    pending_fight_voice: VecDeque<(LineId, ClipState, u64)>,
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
    /// The newest beat sequence the server has resolved, from the accepted view.
    latest_seq: u64,
    /// The sequence last reported to the server as presented, and when (performance ms).
    reported_seq: u64,
    reported_at_ms: u64,
    battlefield_progress: Option<BattlefieldProgress>,
}

/// What the battlefield last reported about its loading, kept to notice a change.
type BattlefieldProgress = (Option<u32>, Option<(u32, u32, bool)>, bool);

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
                narration: String::new(),
                party_hit_points: Vec::new(),
                cells: HashMap::new(),
                battlefield_lines: Vec::new(),
                fight_lines: VecDeque::new(),
                pending_fight_voice: VecDeque::new(),
                seen: VecDeque::new(),
                generation: 0,
                roll_tick: false,
                overlay: None,
                frame: None,
                frame_armed: false,
                timer: None,
                timer_armed: false,
                log: VecDeque::new(),
                latest_seq: 0,
                reported_seq: 0,
                reported_at_ms: 0,
                battlefield_progress: None,
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

/// The outcomes the view adds, identified by the server's monotonic beat sequence.
#[cfg(test)]
fn fresh_outcomes(
    outcomes: &[rpc::CombatOutcome],
    after_sequence: u64,
) -> Vec<&rpc::CombatOutcome> {
    outcomes
        .iter()
        .filter(|outcome| outcome.sequence > after_sequence)
        .collect()
}

fn restore_pending_move_origin(
    cells: &mut HashMap<String, GridCell>,
    previous: &HashMap<String, GridCell>,
    movement: &rpc::CombatOutcome,
) -> Option<GridCell> {
    let from = *previous.get(&movement.actor_name)?;
    cells.insert(movement.actor_name.clone(), from);
    Some(from)
}

fn roll_detail(
    outcome: &rpc::CombatOutcome,
    lines: &[TableLine],
    transitions: Transitions,
    voices: &HashMap<String, df_render::battle::VoiceSet>,
    fight_over: bool,
) -> Option<RollDetail> {
    let turn_only = outcome.kind == rpc::OutcomeKind::Move as i32;
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
        target: if turn_only {
            outcome.actor_name.clone()
        } else {
            outcome.target_name.clone()
        },
        hit: outcome.hit,
        critical: outcome.critical,
        grazed: outcome.grazed,
        damage: outcome.damage,
        target_down: !turn_only && (outcome.knocked_out || outcome.target_hit_points == 0),
        fight_over,
        turn_changed: transitions.turn_changed,
        action_line,
        description,
        // A miss that still dealt damage is the weapon's mastery: "Greatsword \u{b7} 3 slashing, graze".
        attack_label: Some(df_render::battle::float_text::attack_name(&outcome.weapon))
            .filter(|name| !name.is_empty())
            .or_else(|| (outcome.grazed && !outcome.hit).then(|| "Graze".to_owned())),
        damage_type: if outcome.grazed && !outcome.hit {
            format!("{}, graze", outcome.damage_type)
        } else {
            outcome.damage_type.clone()
        },
        attack_style: df_render::battle::AttackStyle::for_weapon(
            outcome.attack_style == 2,
            &outcome.weapon,
        ),
        attack_die: outcome.attack_die,
        faces: outcome.faces.clone(),
        roll_mode: outcome.roll_mode,
        attack_voice: voices.get(&outcome.actor_name).copied(),
        target_voice: voices.get(&outcome.target_name).copied(),
        turn_start: false,
        turn_only,
        walk: None,
        beat: Vec::new(),
        sequence: outcome.sequence,
        target_hp_after: outcome.target_hit_points,
    })
}

/// The Dungeon Master line of a fight script: the narrator's box, never a bubble.
fn narrator_script(text: &str, voice: VoiceLine) -> Option<ScriptLine<Detail>> {
    Some(ScriptLine {
        voiceable: true,
        read_ms: reading_time_ms(text),
        slot: None,
        settled: false,
        detail: Detail::Line(LineDetail {
            speaker: NARRATOR_NAME.to_owned(),
            role: TableRole::Narrator,
            kind: TableKind::Speech,
            text: text.to_owned(),
            fight_line: true,
            voice: Some(voice),
        }),
    })
}

/// An outcome's cries and Dungeon Master lines as stage script lines: the attacker's cry,
/// the victim's answer, then the narration. Slots resolve by figure name so a drifted
/// hero index can never misattribute a bubble; a named speaker with no figure on the
/// flat stage speaks through the battlefield, which finds the figure by name.
fn fight_scripts(lines: &[rpc::FightLine], cast: &[StageActor]) -> Vec<ScriptLine<Detail>> {
    lines
        .iter()
        .filter_map(|line| {
            let voice = crate::voice_line::fight_voice_line(line)?;
            // A cry is spoken, never typed: heroes cry through a speech bubble, with the
            // narrator's box reserved for the Dungeon Master.
            let (hero, speaker) = match rpc::TableRole::try_from(line.role).ok()? {
                rpc::TableRole::Hero => (
                    Some(usize::try_from(line.hero_index).ok()?),
                    line.speaker.clone(),
                ),
                rpc::TableRole::Npc => (None, line.speaker.clone()),
                _ => return narrator_script(&line.text, voice),
            };
            let named = cast
                .iter()
                .find(|actor| actor.name == speaker)
                .and_then(|actor| super::live_stage::slot_index(actor.slot));
            let slot = named
                .or_else(|| {
                    hero.and_then(|index| super::live_stage::slot_index(ActorSlot::Hero(index)))
                })
                .or(Some(OFF_STAGE_SLOT));
            Some(ScriptLine {
                voiceable: true,
                read_ms: reading_time_ms(&line.text),
                slot,
                settled: false,
                detail: Detail::Line(LineDetail {
                    speaker,
                    role: TableRole::Npc,
                    kind: TableKind::Speech,
                    text: line.text.clone(),
                    fight_line: true,
                    voice: Some(voice),
                }),
            })
        })
        .collect()
}

/// The heroes the table says are typing or talking (an ephemeral signal of the phones).
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
fn npc_slot(cast: &[StageActor], actor_id: &str, name: &str, anywhere: bool) -> Option<usize> {
    if !actor_id.is_empty() {
        return cast
            .iter()
            .find(|actor| actor.actor_id == actor_id && matches!(actor.slot, ActorSlot::Npc(_)))
            .and_then(|actor| super::live_stage::slot_index(actor.slot));
    }
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
            fight_line: false,
            voice,
        }),
    }
}

/// The narration a view adds to the scene on stage: nothing when it is the narration already
/// told, the appended part when the told text grew, otherwise the whole new text.
fn added_narration(told: &str, now: &str) -> Option<String> {
    let now = now.trim();
    if now.is_empty() || now == told.trim() {
        return None;
    }
    let appended = now
        .strip_prefix(told.trim())
        .map(str::trim)
        .filter(|rest| !rest.is_empty() && !told.trim().is_empty());
    Some(appended.unwrap_or(now).to_owned())
}

/// A table line as a line of the stage script, or `None` when no figure carries it.
fn script_line(
    line: &TableLine,
    cast: &[StageActor],
    voices: &[VoiceLine],
    settled: bool,
    fighting: bool,
) -> Option<ScriptLine<Detail>> {
    let slot = match line.role {
        TableRole::Hero(index) => super::live_stage::slot_index(ActorSlot::Hero(index)),
        TableRole::Npc => npc_slot(cast, &line.actor_id, &line.speaker, line.scripted),
        TableRole::Narrator => None,
    };
    // In a fight a speaker the flat stage has no figure for (a sixth hero, a fourth enemy) still
    // speaks: its line goes to the battlefield, which finds the figure by name.
    let slot = slot.or_else(|| {
        (fighting && line.kind == TableKind::Speech && line.role != TableRole::Narrator)
            .then_some(OFF_STAGE_SLOT)
    });
    if slot.is_none() && line.role != TableRole::Narrator {
        return None;
    }
    let voice = match (line.kind, line.role) {
        (TableKind::Speech, TableRole::Npc) => voices
            .iter()
            .find(|voice| {
                voice.text.trim() == line.text.trim()
                    && if line.actor_id.is_empty() {
                        voice.speaker() == Some(line.speaker.as_str())
                    } else {
                        voice.actor_id() == Some(line.actor_id.as_str())
                    }
            })
            .cloned(),
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
    let stale_combat_view = journey.combat.as_ref().is_some_and(|combat| {
        with_motion(|motion| {
            let restart = motion.previous.is_none() || motion.generation != generation || hidden;
            let entered =
                presentation::transitions(motion.previous.as_ref(), &snapshot).entered_combat;
            !restart
                && !entered
                && presentation::is_stale_combat_sequence(motion.latest_seq, combat.latest_sequence)
        })
        .unwrap_or(false)
    });
    if stale_combat_view {
        return Ok(());
    }
    let fighting = snapshot.stage == CombatStage::Fighting;
    let lines = presentation::table_lines(&view);
    let cast = presentation::scene_cast(&view);
    let key = format!("{}|{}", journey.phase, scene.title);
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
    let standing = presentation::standing_cells(journey);
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
        let presented = effective_presented(motion);
        motion.conductor.observe_presented(presented);
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
                Some(line) => script.extend(script_line(line, &cast, &voices, true, fighting)),
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
                script.extend(script_line(line, &cast, &voices, false, fighting));
            }
        } else {
            // What the story adds to a scene already on stage (a look's result) is narrated once;
            // a reply to table talk never replays the narration already told.
            if !in_fight && let Some(added) = added_narration(&motion.narration, &narration) {
                script.push(narration_line(&added, &voices, false));
            }
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
                    .filter_map(|line| script_line(line, &cast, &voices, false, fighting)),
            );
        }
        let healed = !transitions.new_roll
            && motion.party_hit_points.len() == hit_points.len()
            && hit_points
                .iter()
                .zip(&motion.party_hit_points)
                .zip(&journey.party)
                .any(|((now, before), member)| member.character_ready && now > before);
        // Every outcome the view adds is staged in order: a tactical fight resolves several turns
        // between two views. At a fight's start the server's already-resolved opening turns are
        // held and staged after the entrance; otherwise the newest is the view's own blow.
        let before_cells = std::mem::replace(&mut motion.cells, standing.clone());
        if !restart
            && let Some(movement) = journey
                .combat
                .as_ref()
                .and_then(presentation::pending_combat_move)
            && let Some(from) =
                restore_pending_move_origin(&mut motion.cells, &before_cells, movement)
        {
            super::live_battlefield::hold_move(&movement.actor_name, from);
        }
        // The server's voice family per combatant, for effort, pain and fall sounds on the
        // flat stage. The 3D battlefield reads the same wire field for its own tokens.
        let mut voices: HashMap<String, df_render::battle::VoiceSet> = HashMap::new();
        for member in &journey.party {
            if let Some(voice) = super::live_battlefield::voice_set(&member.voice_set) {
                voices.insert(member.name.clone(), voice);
            }
        }
        if let Some(combat) = journey.combat.as_ref() {
            for member in &combat.participants {
                if let Some(voice) = super::live_battlefield::voice_set(&member.voice_set) {
                    voices.insert(member.name.clone(), voice);
                }
            }
        }
        let mut staged: Vec<RollDetail> = if !restart {
            let shown = journey.combat.as_ref().map_or_else(Vec::new, |combat| {
                presentation::fresh_combat_beats(
                    combat,
                    if transitions.entered_combat {
                        0
                    } else {
                        motion.latest_seq
                    },
                )
            });
            let marks = presentation::turn_marks(&shown, &before_cells, &standing);
            let finishing_sequence = journey
                .combat
                .as_ref()
                .filter(|_| matches!(snapshot.stage, CombatStage::Finished { .. }))
                .and_then(|combat| combat.outcomes.last().map(|outcome| outcome.sequence));
            shown
                .into_iter()
                .zip(marks)
                .filter_map(|(outcome, mark)| {
                    let mut detail = roll_detail(
                        outcome,
                        &lines,
                        transitions,
                        &voices,
                        finishing_sequence == Some(outcome.sequence),
                    )?;
                    detail.turn_start = mark.turn_start;
                    detail.walk = mark.walk.filter(|_| !restart);
                    if detail.turn_only {
                        detail.walk = outcome
                            .path
                            .first()
                            .zip(outcome.path.last())
                            .map(|(from, to)| ((from.col, from.row), (to.col, to.row)));
                    }
                    // A reload stages the current view silently: past beats keep their
                    // history, but their cries are not replayed.
                    if !restart {
                        detail.beat = fight_scripts(&outcome.lines, &cast);
                    }
                    Some(detail)
                })
                .collect()
        } else {
            Vec::new()
        };
        let roll = if transitions.entered_combat {
            None
        } else {
            staged.pop()
        };
        let (before, after) = if transitions.entered_combat {
            (Vec::new(), staged)
        } else {
            (staged, Vec::new())
        };
        // The battlefield keeps each mover where it stood, and the label on the attacker whose
        // beats are playing, until the cue stream reaches that attacker's turn.
        if restart {
            super::live_battlefield::drop_pending();
            // The coherent catch-up jump: every layer lands on the newest state at once
            // (stage cut to idle, dice down), never one layer jumped and another replayed.
            super::live_battlefield::jump_to_latest();
        } else {
            let turns = before.len() + usize::from(roll.is_some()) + after.len();
            if turns > 0 {
                // Everyone who stands elsewhere than in the last view waits where they were: an
                // attacker walks as its turn is shown, anyone else once the attacks are over.
                for (name, from) in &before_cells {
                    if standing.get(name).is_some_and(|now| now != from) {
                        super::live_battlefield::hold_move(name, *from);
                    }
                }
            }
            super::live_battlefield::accept_cells(&standing);
            super::live_battlefield::accept_cues(turns);
        }
        let memorable = roll
            .as_ref()
            .is_some_and(|roll| roll.critical || roll.target_down);
        let sequence = roll.as_ref().map_or(0, |roll| roll.sequence);
        let diff = ViewDiff {
            scene_changed,
            enters_combat: transitions.entered_combat
                || (restart && snapshot.stage == CombatStage::Fighting),
            turn_changed: transitions.turn_changed,
            roll: roll
                .clone()
                .filter(|roll| !roll.turn_only)
                .map(|roll| Detail::Roll(Box::new(roll))),
            turn: roll
                .clone()
                .filter(|roll| roll.turn_only)
                .map(|roll| Detail::Roll(Box::new(roll))),
            impact: roll
                .clone()
                .filter(|roll| !roll.turn_only)
                .map(|roll| Detail::Roll(Box::new(roll))),
            defeat: roll
                .filter(|roll| roll.target_down)
                .map(|roll| Detail::Roll(Box::new(roll))),
            heal: healed.then_some(Detail::Heal),
            title: transitions
                .story_begun
                .then(|| Detail::Card(Box::new(presentation::title_card(scene, &narration)))),
            banner: transitions.finished.map(|victory| Detail::Banner {
                victory,
                closing: journey
                    .combat
                    .as_ref()
                    .map(|combat| fight_scripts(&combat.closing_lines, &cast))
                    .unwrap_or_default(),
            }),
            // A won fight goes straight from the battlefield to the victory card: a closing card
            // would name the next scene before the audience has read the victory. Only a defeat
            // gets its closing card.
            closing: transitions
                .finished
                .filter(|victory| !victory)
                .map(|victory| {
                    Detail::Card(Box::new(presentation::closing_card(
                        journey, scene, &narration, victory,
                    )))
                }),
            blow_only: false,
            memorable,
            sequence,
        };
        let plan = ViewPlan {
            diff,
            script,
            cast: cast_slots(&cast),
        };
        let chime = transitions.turn_changed && !transitions.new_roll;
        if let Some(combat) = journey.combat.as_ref() {
            motion.latest_seq = if transitions.entered_combat {
                combat.latest_sequence
            } else {
                motion.latest_seq.max(combat.latest_sequence)
            };
        }
        motion.previous = Some(snapshot.clone());
        motion.scene_key = Some(key.clone());
        motion.narration.clone_from(&narration);
        motion.party_hit_points = hit_points.clone();
        motion.generation = generation;
        motion.thinking_for = thinking.clone();
        if let Some(overlay) = motion.overlay.take() {
            overlay.remove();
        }
        motion.conductor.set_sound(sound_on);
        if transitions.entered_combat {
            live_sfx::reset_ending_sting();
            // The clips fetch once and are reused: a roll never waits on the network.
            super::live_dice::preload();
        }
        motion.conductor.set_battlefield_expected(gpu_expected());
        motion
            .conductor
            .set_battlefield_busy(super::live_battlefield::is_busy());
        let blow_plan = |roll: &RollDetail| ViewPlan {
            diff: ViewDiff {
                roll: (!roll.turn_only).then(|| Detail::Roll(Box::new(roll.clone()))),
                turn: roll.turn_only.then(|| Detail::Roll(Box::new(roll.clone()))),
                impact: (!roll.turn_only).then(|| Detail::Roll(Box::new(roll.clone()))),
                defeat: roll
                    .target_down
                    .then(|| Detail::Roll(Box::new(roll.clone()))),
                blow_only: true,
                memorable: roll.critical || roll.target_down,
                sequence: roll.sequence,
                ..ViewDiff::default()
            },
            script: Vec::new(),
            cast: plan.cast.clone(),
        };
        let mut commands = Vec::new();
        if restart {
            commands.extend(motion.conductor.restart(now, plan));
        } else {
            for earlier in &before {
                commands.extend(motion.conductor.on_view(now, blow_plan(earlier)));
            }
            let later: Vec<ViewPlan<Detail>> = after.iter().map(blow_plan).collect();
            commands.extend(motion.conductor.on_view(now, plan));
            for turn in later {
                commands.extend(motion.conductor.on_view(now, turn));
            }
        }
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
fn show_thinking(
    client: &Rc<RefCell<Client>>,
    speaker: Option<&str>,
    awaiting: Option<usize>,
) -> Result<(), JsValue> {
    let state = client.borrow();
    let Some(stage) = state.live.tv.as_ref() else {
        return Ok(());
    };
    // A reply waiting for its voice clip keeps the thinking mark until the clip starts.
    let slot = awaiting
        .map(super::live_stage::slot_of_index)
        .filter(|slot| matches!(slot, ActorSlot::Npc(_)))
        .or_else(|| {
            speaker.and_then(|name| {
                stage
                    .figures()
                    .slot_named(name)
                    .filter(|slot| matches!(slot, ActorSlot::Npc(_)))
            })
        });
    stage.figures().set_thinking(slot)
}

/// The sequence whose result this display has presented: strike acknowledgements from the
/// battlefield, promoted to the newest resolved beat once nothing is playing anywhere (which
/// covers beats the sequencer never cues, such as a death save shown by the stage alone).
fn effective_presented(motion: &Motion) -> u64 {
    let presented = motion.conductor.presented_sequence();
    if motion.conductor.director_idle()
        && !super::live_battlefield::is_busy()
        && !super::live_battlefield::has_pending()
    {
        presented.max(motion.latest_seq)
    } else {
        presented
    }
}

/// The battlefield's impact event confirms that the strike result is visible now.
pub(super) fn strike_landed(sequence: u64) {
    let observed = with_motion(|motion| {
        motion.conductor.observe_presented(sequence);
    });
    if observed.is_some() {
        // The impact event is the battlefield's exact visible sequence. The conductor cursor
        // can still wait on an earlier cue, so it cannot decide which health change this card shows.
        super::live_battlefield::set_presented(sequence, "impact");
    }
    ensure_frame();
}

thread_local! {
    /// Which report keep-alive loop is current; an older one ends itself.
    static REPORT_LOOP: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// This page's display instance, chosen once per page load: two televisions on
    /// one room (and one television across a reload) report as different instances,
    /// so the server can let the slowest fresh one govern and drop a silent one.
    static DISPLAY_INSTANCE: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// This page's display instance (never zero, which the server reads as legacy).
pub(super) fn display_instance() -> u64 {
    DISPLAY_INSTANCE.with(|cell| {
        if cell.get() == 0 {
            let seed = (js_sys::Math::random() * 9_007_199_254_740_991.0) as u64;
            cell.set(seed.max(1) ^ (now_ms() << 17) | 1);
        }
        cell.get()
    })
}

/// Sends one presentation report; returns the server's released sequence after it.
async fn send_report(client: &Rc<RefCell<Client>>, sequence: u64) -> Option<u64> {
    let (sessions, credential) = {
        let state = client.borrow();
        (state.sessions.clone()?, state.credential.clone())
    };
    let mut sessions = sessions;
    let mut call = request(
        rpc::PresentedRequest {
            sequence,
            display_instance: display_instance(),
            layout_readback_json: String::new(),
        },
        &credential,
    )
    .ok()?;
    call.set_timeout(Duration::from_millis(2_000));
    let outcome = sessions.report_presented(call).await;
    let root = client.borrow().root.clone();
    let _marked = root.set_attribute(
        "data-presented-report",
        if outcome.is_ok() { "sent" } else { "failed" },
    );
    outcome
        .ok()
        .map(|response| response.into_inner().released_sequence)
}

/// Tells the server the last beat this display finished presenting, so it can release the
/// next actor (its timer is only the bound for a missing display). Best effort like the
/// speech-intent signal: a failed report is marked on the root and never blocks the stage.
/// The report repeats on its own timer until the server's released sequence reaches the
/// newest resolved beat: the stage stops ticking at idle, and a single report could leave
/// an older in-window report governing the release forever.
fn report_presented(client: &Rc<RefCell<Client>>) {
    if client.borrow().role != "display" {
        return;
    }
    let now = now_ms();
    let due = with_motion(|motion| {
        let presented = effective_presented(motion);
        if presented == 0 {
            return None;
        }
        let changed = presented != motion.reported_seq;
        let stale = now.saturating_sub(motion.reported_at_ms) >= 1_000;
        if !changed && !stale {
            return None;
        }
        motion.reported_seq = presented;
        motion.reported_at_ms = now;
        Some(presented)
    })
    .flatten();
    let Some(sequence) = due else {
        return;
    };
    let id = REPORT_LOOP.with(|current| {
        current.set(current.get() + 1);
        current.get()
    });
    let weak = Rc::downgrade(client);
    wasm_bindgen_futures::spawn_local(async move {
        let mut sequence = sequence;
        loop {
            let Some(client) = weak.upgrade() else {
                return;
            };
            let released = send_report(&client, sequence).await;
            let (presented, latest) =
                with_motion(|motion| (effective_presented(motion), motion.latest_seq))
                    .unwrap_or((sequence, sequence));
            if released.is_some_and(|released| released >= latest) && presented >= latest {
                return;
            }
            gloo_timers::future::TimeoutFuture::new(1_600).await;
            if REPORT_LOOP.with(std::cell::Cell::get) != id {
                return;
            }
            sequence = with_motion(|motion| {
                let presented = effective_presented(motion);
                motion.reported_seq = presented;
                motion.reported_at_ms = now_ms();
                presented
            })
            .unwrap_or(sequence);
            if sequence == 0 {
                return;
            }
        }
    });
}

/// Publishes the machines' states on the stage element, so a browser check asserts on states
/// instead of sleeping, and shows the thinking mark when the lines before the reply are done.
fn publish_state(client: &Rc<RefCell<Client>>) -> Result<(), JsValue> {
    let Some((
        scene,
        line,
        pending,
        waiting_lines,
        thinking,
        speaker,
        battle,
        awaiting,
        presented,
        latest,
        beat_log,
    )) = with_motion(|motion| {
        let presented = effective_presented(motion);
        motion.conductor.observe_presented(presented);
        super::live_battlefield::set_presented(presented, "director");
        let beat_log = motion
            .conductor
            .beat_log()
            .iter()
            .map(|record| match record.skip_reason {
                Some(reason) => format!(
                    "{}:skipped:{reason}:{}:{}:wait={}",
                    record.sequence, record.start_ms, record.end_ms, record.wait_ms
                ),
                None => format!(
                    "{}:{}:{}:{}:wait={}",
                    record.sequence, record.kind, record.start_ms, record.end_ms, record.wait_ms
                ),
            })
            .collect::<Vec<_>>()
            .join(",");
        (
            motion.conductor.scene_state(),
            motion.conductor.focus_line(),
            motion.conductor.has_pending(),
            motion.conductor.lines_waiting(),
            motion.thinking_for.clone(),
            motion.conductor.active_speaker(),
            motion.conductor.battle_state(),
            motion.conductor.awaiting_clip_slot(),
            presented,
            motion.latest_seq,
            beat_log,
        )
    })
    else {
        return Ok(());
    };
    report_presented(client);
    let battlefield_beat_log = super::live_battlefield::beat_log();
    let card_log = super::live_battlefield::card_log();
    let beat_log = merge_battlefield_beat_log(&beat_log, &battlefield_beat_log);
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
            // The one-clock debug record: checks assert sync from these numbers.
            root.set_attribute("data-presented-seq", &presented.to_string())?;
            root.set_attribute("data-display-instance", &display_instance().to_string())?;
            root.set_attribute("data-latest-seq", &latest.to_string())?;
            root.set_attribute("data-beat-log", &beat_log)?;
            root.set_attribute("data-battlefield-beat-log", &battlefield_beat_log)?;
            root.set_attribute("data-card-log", &card_log)?;
            root.set_attribute(
                "data-battlefield-live",
                if battlefield_live() { "true" } else { "false" },
            )?;
            super::live_tv::refresh_timing_visibility(root);
            // Whose words the stage shows follows the line being heard, not the raw view.
            stage.set_speaker(match speaker {
                None => "",
                Some(None) => "narrator",
                Some(Some(_)) => "character",
            })?;
        }
    }
    show_thinking(
        client,
        thinking.as_deref().filter(|_| !waiting_lines),
        awaiting,
    )
}

fn merge_battlefield_beat_log(conductor: &str, battlefield: &str) -> String {
    let details: HashMap<&str, &str> = battlefield
        .split(',')
        .filter_map(|record| record.split_once(':'))
        .collect();
    conductor
        .split(',')
        .map(|record| {
            let Some((sequence, phase)) = record.split_once(':') else {
                return record.to_owned();
            };
            if phase.starts_with("rolling:")
                && let Some(detail) = details.get(sequence)
            {
                return format!("{record}:battlefield={detail}");
            }
            record.to_owned()
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn redraw(client: &Rc<RefCell<Client>>) -> Result<(), JsValue> {
    let drawn = redraw_current(client);
    live_voice::observe(client);
    drawn
}

fn apply(client: &Rc<RefCell<Client>>, commands: Vec<Command<Detail>>) -> Result<(), JsValue> {
    let mut first_error = None;
    let mut pending_battle = None;
    for command in commands {
        let command = match command {
            Command::Battle(state) => {
                pending_battle = Some(state);
                continue;
            }
            command => command,
        };
        let entered = matches!(&command, Command::Enter { .. });
        let outcome = match command {
            Command::Skip { detail } => {
                battlefield_cue_named("skipped", detail.as_ref());
                Ok(())
            }
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
            } => {
                let outcome = enter(client, state, detail.as_ref(), short);
                // The beat's own lines start after its dice settle: a blow's cries at impact,
                // the closing line when the banner rises. Beats that never start
                // never install their lines.
                let beat = match detail.as_ref() {
                    Some(Detail::Roll(roll)) if fight_line_stage(state, roll.turn_only) => {
                        roll.beat.clone()
                    }
                    Some(Detail::Banner { closing, .. }) if state == StageState::Aftermath => {
                        closing.clone()
                    }
                    _ => Vec::new(),
                };
                if beat.is_empty() {
                    outcome
                } else {
                    let more = with_motion(|motion| motion.conductor.install_beat(now_ms(), beat))
                        .unwrap_or_default();
                    outcome.and(apply(client, more))
                }
            }
            // Rest is a beat boundary too: the dice come down here as on any later cue, so
            // a die never stands stale while the game waits for a player.
            Command::Rest => set_stage(client, "").and(super::live_dice::clear(client)),
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
            Command::Battle(state) => {
                pending_battle = Some(state);
                Ok(())
            }
            Command::ClipState { id, state } => set_clip_state(client, id, state),
        };
        if let Err(error) = outcome {
            first_error.get_or_insert(error);
        }
        if entered
            && let Some(state) = pending_battle.take()
            && let Err(error) = set_battle_state(client, state)
        {
            first_error.get_or_insert(error);
        }
    }
    if let Some(state) = pending_battle
        && let Err(error) = set_battle_state(client, state)
    {
        first_error.get_or_insert(error);
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
        StageState::Turn => "turn",
        StageState::Impact => "impact",
        StageState::Defeat => "defeat",
        StageState::Healing => "healing",
        StageState::Aftermath => "aftermath",
        StageState::ClosingCard => "closing",
    }
}

fn fight_line_stage(state: StageState, turn_only: bool) -> bool {
    state == StageState::Impact || (state == StageState::Turn && turn_only)
}

fn enter(
    client: &Rc<RefCell<Client>>,
    state: StageState,
    detail: Option<&Detail>,
    short: bool,
) -> Result<(), JsValue> {
    battlefield_cue(state, detail);
    set_stage(client, state_name(state))?;
    // The dice live only for their beat: the roll shows them, the impact keeps them while the
    // hit lands, and every later cue takes them down — including the rest between blows, so
    // the dice never stand stale over the walkers of a move-only turn.
    if !matches!(state, StageState::Rolling | StageState::Impact) {
        super::live_dice::clear(client)?;
    }
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
            // The fall plays out; the dice came down with the cue above.
            // The fallen combatant's cry on the flat stage.
            voice_event(
                &roll.target,
                roll.target_voice,
                df_render::battle::SoundKind::Down,
            );
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
        (StageState::Aftermath, Some(Detail::Banner { victory, .. })) => {
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
    battlefield_cue_named(state_name(state), detail);
}

fn battlefield_cue_named(state: &str, detail: Option<&Detail>) {
    let summary = match detail {
        Some(Detail::Roll(roll)) => Some(super::live_battlefield::RollSummary {
            sequence: roll.sequence,
            attacker: roll.attacker.clone(),
            target: roll.target.clone(),
            hit: roll.hit,
            critical: roll.critical,
            grazed: roll.grazed,
            damage: roll.damage,
            target_down: roll.target_down,
            fight_over: roll.fight_over,
            attack_label: roll.attack_label.clone(),
            damage_type: roll.damage_type.clone(),
            attack_style: roll.attack_style,
            turn_start: roll.turn_start,
            walk: roll.walk,
            target_hp_after: roll.target_hp_after,
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
    super::live_battlefield::on_cue(state, summary.as_ref());
}

/// Effort, pain and fall sounds on the flat stage, from the server's voice family for the
/// named combatant. The 3D battlefield plays the same clips from its own tokens at the same
/// beats, so this stays quiet while it is live: one blow, one voice. The picker varies the
/// clip without repeating the last; an unknown combatant stays silent.
fn voice_event(
    name: &str,
    voice: Option<df_render::battle::VoiceSet>,
    kind: df_render::battle::SoundKind,
) {
    if battlefield_live() {
        return;
    }
    let Some(voice) = voice else {
        return;
    };
    live_sfx::play_event(
        &df_render::battle::SoundEvent {
            kind,
            voice,
            surface: df_render::battle::Surface::default(),
            token: df_render::battle::TokenId(name.to_owned()),
            rate: 1.0,
        },
        super::live_battlefield::content_rating(),
    );
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
    // The clip plays now and the server's number lands at its manifest rest time; catch-up
    // shortens the stage, not the die animation. If clips are unavailable, show_roll uses
    // a shipped resting image and never waits.
    let rest_ms = if let Some(play) =
        crate::dice_roll::play_of(roll.attack_die, &roll.faces, roll.roll_mode)
    {
        super::live_dice::show_roll(client, play, short)?
    } else {
        super::live_dice::clear(client)?;
        0
    };
    // The attacker's effort on the flat stage; the battlefield voices its own tokens.
    voice_event(
        &roll.attacker,
        roll.attack_voice,
        df_render::battle::SoundKind::Effort,
    );
    live_sfx::cancel_unstarted();
    live_sfx::play(Cue::DiceRattle, Duration::ZERO);
    // The land and the result follow the clip's own rest time from the manifest, not the
    // old fixed pin; with no clip the card resolves on the previous schedule.
    let land_at = if rest_ms > 0 {
        Duration::from_millis(rest_ms)
    } else {
        DICE_LAND
    };
    live_sfx::play(Cue::DiceLand, land_at);
    live_sfx::play(
        roll_cue(roll),
        if rest_ms > 0 {
            land_at + RESULT_AFTER_LAND
        } else {
            ROLL_RESULT
        },
    );
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
    if short || battlefield_live() {
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
    // The target's pain on the flat stage; the battlefield voices its own tokens.
    if roll.hit || roll.grazed {
        voice_event(
            &roll.target,
            roll.target_voice,
            df_render::battle::SoundKind::Pain,
        );
    }
    let figures = stage.figures();
    if let Some(target) = figures.slot_named(&roll.target) {
        if roll.hit || roll.grazed {
            figures.fx(target, Fx::Hit, Duration::ZERO, 1)?;
            // PLAY-H01: the shake is sized by the share of the bar the blow took.
            figures.health_blow(target, roll.damage, roll.critical, false)?;
            let (text, kind) = if roll.critical {
                (format!("-{}!", roll.damage), FloatKind::Critical)
            } else {
                (format!("-{}", roll.damage), FloatKind::Damage)
            };
            // The battlefield names the attack and its type in its own floating text.
            if !battlefield_live() {
                figures.float_text(target, &text, kind, Duration::ZERO)?;
            }
        } else if let Some(attacker) = figures.slot_named(&roll.attacker) {
            let direction = super::live_stage::Figures::direction(attacker, target);
            figures.fx(target, Fx::Dodge, Duration::ZERO, direction)?;
            if !battlefield_live() {
                figures.float_text(target, "Miss", FloatKind::Miss, Duration::ZERO)?;
            }
        }
    }
    if !battlefield_live()
        && let Some(attacker) = figures.slot_named(&roll.attacker)
    {
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

/// A line's voice clip changed state: a clip that failed or was skipped shows as such on the
/// bubble or box, so a line read without its voice never looks spoken.
fn set_clip_state(
    client: &Rc<RefCell<Client>>,
    id: LineId,
    state: ClipState,
) -> Result<(), JsValue> {
    let root = client.borrow().root.clone();
    if let Some(element) = root.query_selector(&format!("[data-line-id=\"{id}\"]"))? {
        element.set_attribute("data-clip-state", state.name())?;
    }
    match client.borrow().live.tv.as_ref() {
        Some(stage) => stage.root().set_attribute("data-clip-state", state.name()),
        None => Ok(()),
    }
}

fn set_battle_state(client: &Rc<RefCell<Client>>, state: BattleState) -> Result<(), JsValue> {
    if matches!(state, BattleState::TurnEnd | BattleState::TurnStart) {
        // The previous turn's beats are over: let the battlefield show the server's active
        // combatant (its turn label and marker) now, not at the next roll.
        super::live_battlefield::resync();
    }
    if matches!(state, BattleState::Exiting | BattleState::Absent) {
        // Nothing from the battle survives its exit: the combat bubbles and call-outs close
        // with it, and the battlefield fades out under the transition.
        super::live_battlefield::drop_pending();
        if state == BattleState::Exiting {
            super::live_battlefield::fade_out();
        }
        if let Some(stage) = client.borrow().live.tv.as_ref() {
            stage.figures().clear_bubbles()?;
        }
    }
    match client.borrow().live.tv.as_ref() {
        Some(stage) => {
            stage
                .root()
                .set_attribute("data-battle-state", state.name())?;
            stage.root().set_attribute(
                "data-battlefield-live",
                if battlefield_live() { "true" } else { "false" },
            )?;
            super::live_tv::refresh_timing_visibility(stage.root());
            Ok(())
        }
        None => Ok(()),
    }
}

/// Whether the 3D battlefield is showing (or about to show) the fight, so the flat combat
/// screen, its dice card and its call-out bubbles must not.
pub(super) fn battlefield_live() -> bool {
    with_motion(|motion| motion.conductor.battlefield_live()).unwrap_or(false)
}

fn set_scene_state(client: &Rc<RefCell<Client>>, state: SceneState) -> Result<(), JsValue> {
    match client.borrow().live.tv.as_ref() {
        Some(stage) => {
            if state == SceneState::Entering {
                stage.clear_narration()?;
            }
            // PLAY-V12: every line of the scene has been told, so whatever the narration did, the
            // plate may be cut to.
            if state == SceneState::AwaitingChoice {
                stage.narration_over()?;
            }
            stage.root().set_attribute("data-scene-state", state.name())
        }
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
    mirror_splash_caption(state, line, slot);
    if battlefield_speech(id, state, line) {
        if line.fight_line {
            record_fight_line(id, state, line, slot);
        }
        return Ok(());
    }
    let shown = {
        let state_ref = client.borrow();
        let Some(stage) = state_ref.live.tv.as_ref() else {
            return Ok(());
        };
        match slot {
            None => stage.narrator_line(id, state, &line.text)?,
            Some(position) => {
                let kind = match (line.kind, line.role) {
                    (TableKind::Action, _) => BubbleKind::Action,
                    (TableKind::Speech, TableRole::Hero(_)) => BubbleKind::Typed,
                    (TableKind::Speech, _) => BubbleKind::Speech,
                };
                stage.character_line(
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
    if shown && line.fight_line {
        record_fight_line(id, state, line, slot);
    }
    Ok(())
}

fn record_fight_line(id: LineId, state: LineState, line: &LineDetail, slot: Option<usize>) {
    let pending = with_motion(|motion| {
        if matches!(state, LineState::Opening | LineState::Lingering)
            && !motion.fight_lines.contains(&id)
        {
            motion.fight_lines.push_back(id);
            if motion.fight_lines.len() > 128 {
                motion.fight_lines.pop_front();
            }
        }
        if motion.fight_lines.contains(&id) {
            take_pending_fight_voice(&mut motion.pending_fight_voice, id)
        } else {
            Vec::new()
        }
    })
    .unwrap_or_default();
    super::presentation_timing::line(
        id,
        state,
        slot,
        line.text.chars().count() as u32,
        reading_time_ms(&line.text),
        "fight",
    );
    for (state, at_ms) in pending {
        super::presentation_timing::voice_at(id, state, at_ms);
    }
}

fn take_pending_fight_voice(
    pending: &mut VecDeque<(LineId, ClipState, u64)>,
    id: LineId,
) -> Vec<(ClipState, u64)> {
    let mut matching = Vec::new();
    let mut retained = VecDeque::with_capacity(pending.len());
    while let Some((pending_id, state, at_ms)) = pending.pop_front() {
        if pending_id == id {
            matching.push((state, at_ms));
        } else {
            retained.push_back((pending_id, state, at_ms));
        }
    }
    *pending = retained;
    matching
}

/// While the fight's loading splash holds, the line being spoken is read on it too, in the
/// story stage's own words: the Dungeon Master label for the narrator's box, the speaker's name
/// otherwise. The newest opened line replaces the last; a closed line clears it, and the splash
/// going takes it with the card. A no-op without a splash, so story scenes never see it.
fn mirror_splash_caption(state: LineState, line: &LineDetail, slot: Option<usize>) {
    match state {
        LineState::Opening | LineState::Speaking | LineState::Lingering => {
            let label = match slot {
                None => NARRATOR_NAME,
                Some(_) => line.speaker.as_str(),
            };
            super::live_battlefield::splash_caption(label, &line.text, true);
        }
        LineState::Closing | LineState::Done => {
            super::live_battlefield::splash_caption("", "", false);
        }
        _ => {}
    }
}

/// A line spoken by a figure during a fight goes to the battlefield: its bubble opens over the
/// speaker with the camera eased in when the voice starts (the line opens), and closes when the
/// voice ends (the line lingers). The narrator's lines stay in the narrator's box. Returns whether
/// the battlefield carries this line, so nothing else draws it; a speaker the battlefield does not
/// show falls back to the flat bubble.
fn battlefield_speech(id: LineId, state: LineState, line: &LineDetail) -> bool {
    if line.kind != TableKind::Speech || line.role == TableRole::Narrator {
        return false;
    }
    let carried = with_motion(|motion| motion.battlefield_lines.contains(&id)).unwrap_or(false);
    match state {
        LineState::Opening if !carried => {
            let opened = super::live_battlefield::speak(&line.speaker, &line.text, id);
            if opened {
                with_motion(|motion| motion.battlefield_lines.push(id));
            }
            opened
        }
        LineState::Lingering | LineState::Closing | LineState::Done if carried => {
            super::live_battlefield::end_speech(id);
            if state == LineState::Done {
                with_motion(|motion| motion.battlefield_lines.retain(|line| *line != id));
            }
            true
        }
        _ => carried,
    }
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
    let clip_state = timing_clip_state(signal);
    let voice_observation = clip_state.and_then(|state| {
        let at_ms = web_sys::window()?.performance()?.now();
        (at_ms.is_finite() && at_ms >= 0.0).then_some((state, at_ms as u64))
    });
    if clip_state.is_some() && voice_observation.is_none() {
        super::presentation_timing::report_loss(1);
    }
    let voice_observation = with_motion(|motion| {
        if motion.fight_lines.contains(&id) {
            voice_observation
        } else {
            if let Some((state, at_ms)) = voice_observation {
                if motion.pending_fight_voice.len() == 128 {
                    motion.pending_fight_voice.pop_front();
                    super::presentation_timing::report_loss(1);
                }
                motion.pending_fight_voice.push_back((id, state, at_ms));
            }
            None
        }
    })
    .flatten();
    if let Some((state, at_ms)) = voice_observation {
        super::presentation_timing::voice_at(id, state, at_ms);
    }
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

fn timing_clip_state(signal: ClipSignal) -> Option<ClipState> {
    match signal {
        ClipSignal::Started { .. } => Some(ClipState::Playing),
        ClipSignal::Finished => Some(ClipState::Ended),
        ClipSignal::Error => Some(ClipState::Failed),
        ClipSignal::Loaded => None,
    }
}

/// Sound was unlocked, muted or unmuted: later lines ask for a clip only while it is on.
pub(super) fn sound_changed(on: bool) {
    with_motion(|motion| motion.conductor.set_sound(on));
}

/// Media readiness wakes the same owner used by the conductor, even after a final idle line.
pub(super) fn wake_plate_clock() {
    ensure_frame();
}

pub(super) fn clock_needs_work(
    conductor_busy: bool,
    stage: Option<&super::live_tv::TvStage>,
) -> bool {
    conductor_busy || stage.is_some_and(|stage| stage.plate_needs_clock())
}

fn clock_busy(motion: &Motion) -> bool {
    if motion.conductor.is_busy() {
        return true;
    }
    motion
        .client
        .upgrade()
        .is_some_and(|client| clock_needs_work(false, client.borrow().live.tv.as_ref()))
}

/// One animation frame and one fallback timer serve conductor and decoded-plate work.
/// Both stop re-arming once those owners are idle.
fn ensure_frame() {
    arm_frame();
    arm_timer();
}

fn arm_frame() {
    let armed = with_motion(|motion| {
        if motion.frame_armed || !clock_busy(motion) {
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
        if motion.timer_armed || !clock_busy(motion) {
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
        motion
            .conductor
            .set_battlefield_busy(super::live_battlefield::is_busy());
        let mut commands = motion.conductor.on_tick(now);
        // A fight's entrance waits for the battlefield; ask how far it is.
        if motion.conductor.wants_battlefield() {
            let entrance = super::live_battlefield::entrance();
            if motion.conductor.battle_state() == BattleState::Entering {
                let progress = super::live_battlefield::entrance_progress();
                if motion.battlefield_progress != Some(progress) {
                    motion.conductor.on_battlefield_progress(now);
                    motion.battlefield_progress = Some(progress);
                }
            }
            commands.extend(motion.conductor.on_battlefield(now, entrance));
            if entrance == crate::stage_conductor::Battlefield::Pending
                && motion.conductor.battlefield_failed()
            {
                super::live_battlefield::fail_entrance();
            }
        } else {
            motion.battlefield_progress = None;
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
    // The phone's share of the one-clock debug record: the sequence the server has
    // released to act past, and the newest it has resolved.
    if let Some(combat) = journey.combat.as_ref() {
        let root = client.borrow().root.clone();
        root.set_attribute("data-presented-seq", &combat.released_sequence.to_string())?;
        root.set_attribute("data-latest-seq", &combat.latest_sequence.to_string())?;
    }
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
        let transitions = presentation::transitions(motion.previous.as_ref(), &snapshot);
        motion.previous = Some(snapshot.clone());
        for line in &lines {
            if remember(&mut motion.seen, line.sequence) && !hidden {
                if motion.log.len() >= LOG_LINES {
                    motion.log.pop_front();
                }
                // The server may number an action before the lines that followed it and
                // deliver it after them: the log keeps the server's order.
                let position = motion
                    .log
                    .partition_point(|shown| shown.sequence < line.sequence);
                motion.log.insert(position, line.clone());
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

#[cfg(test)]
mod tests {
    use super::{
        Detail, fight_line_stage, fight_scripts, fresh_outcomes, merge_battlefield_beat_log,
        restore_pending_move_origin, take_pending_fight_voice, timing_clip_state,
    };
    use std::collections::HashMap;

    #[test]
    fn canonical_background_actor_id_selects_its_bubble_slot_without_focus_fallback() {
        use df_protocol::common as rpc;
        let view = rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Display(
                rpc::DisplayGameplayView {
                    journey: Some(rpc::JourneyView::default()),
                    scene: Some(rpc::GameplayScene {
                        cast: vec![
                            rpc::SceneActor {
                                id: "courier".into(),
                                name: "Courier".into(),
                                speaking: true,
                                ..Default::default()
                            },
                            rpc::SceneActor {
                                id: "bandit".into(),
                                name: "Bandit".into(),
                                ..Default::default()
                            },
                        ],
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        let cast = crate::presentation::scene_cast(&view);
        assert_eq!(
            super::npc_slot(&cast, "bandit", "Courier", true),
            super::super::live_stage::slot_index(crate::presentation::ActorSlot::Npc(1))
        );
        assert!(super::npc_slot(&cast, "departed", "Courier", true).is_none());
    }

    /// SRD 5.2.1 p. 177 (Attack [Action]); this test covers display ordering only.
    #[test]
    fn pending_move_keeps_its_origin_for_the_next_view_attack() {
        let movement = df_protocol::common::CombatOutcome {
            sequence: 1,
            actor_name: "Weasel 1".to_owned(),
            kind: df_protocol::common::OutcomeKind::Move as i32,
            ..Default::default()
        };
        let previous = HashMap::from([("Weasel 1".to_owned(), (3, 1))]);
        let mut cells = HashMap::from([("Weasel 1".to_owned(), (3, 3))]);

        assert_eq!(
            restore_pending_move_origin(&mut cells, &previous, &movement),
            Some((3, 1))
        );
        assert_eq!(cells["Weasel 1"], (3, 1));
    }

    #[test]
    fn fresh_outcomes_uses_sequence_and_keeps_every_new_beat_in_order() {
        let outcomes: Vec<_> = (1..=7)
            .map(|sequence| df_protocol::common::CombatOutcome {
                sequence,
                ..Default::default()
            })
            .collect();

        let fresh = fresh_outcomes(&outcomes, 0);

        assert_eq!(
            fresh
                .iter()
                .map(|outcome| outcome.sequence)
                .collect::<Vec<_>>(),
            (1..=7).collect::<Vec<_>>()
        );
        assert_eq!(fresh_outcomes(&outcomes, 6)[0].sequence, 7);
    }

    /// SRD 5.2.1 p. 177 (Attack [Action]); this test covers display diagnostics only.
    #[test]
    fn beat_log_attaches_battlefield_parts_to_the_matching_rolling_record() {
        assert_eq!(
            merge_battlefield_beat_log(
                "4:rolling:0:1200:wait=0,4:impact:1200:2700:wait=0,6:rolling:2700:3900:wait=0",
                "4:Weasel 2:heading=320:walk=440:roll=1500:approach=300:strike=500:settle=250",
            ),
            "4:rolling:0:1200:wait=0:battlefield=4:Weasel 2:heading=320:walk=440:roll=1500:approach=300:strike=500:settle=250,4:impact:1200:2700:wait=0,6:rolling:2700:3900:wait=0"
        );
    }

    #[test]
    fn fight_narration_and_cries_are_staged_as_voiced_fight_lines() {
        use crate::stage_conductor::ClipSignal;
        use df_protocol::common as rpc;

        let lines = [
            rpc::FightLine {
                role: rpc::TableRole::Narrator as i32,
                text: "The rat rushes from the shadows.".to_owned(),
                ..Default::default()
            },
            rpc::FightLine {
                role: rpc::TableRole::Hero as i32,
                hero_index: 0,
                speaker: "Brakka".to_owned(),
                text: "Hold the line!".to_owned(),
            },
        ];
        let scripts = fight_scripts(&lines, &[]);

        assert_eq!(scripts.len(), 2);
        assert!(!fight_line_stage(
            crate::stage_director::StageState::Rolling,
            false
        ));
        assert!(fight_line_stage(
            crate::stage_director::StageState::Impact,
            false
        ));
        assert!(fight_line_stage(
            crate::stage_director::StageState::Turn,
            true
        ));
        assert!(scripts.iter().all(|script| script.voiceable));
        assert!(scripts.iter().all(|script| match &script.detail {
            Detail::Line(line) => line.fight_line && line.voice.is_some(),
            _ => false,
        }));
        assert_eq!(
            timing_clip_state(ClipSignal::Started {
                duration_ms: Some(1_100),
            }),
            Some(crate::stage_machines::ClipState::Playing)
        );
        assert_eq!(
            timing_clip_state(ClipSignal::Finished),
            Some(crate::stage_machines::ClipState::Ended)
        );
    }

    #[test]
    fn fight_voice_callbacks_before_the_line_opens_are_retained_for_timing() {
        let mut pending = std::collections::VecDeque::from([
            (41, crate::stage_machines::ClipState::Playing, 10),
            (42, crate::stage_machines::ClipState::Failed, 20),
            (41, crate::stage_machines::ClipState::Ended, 30),
        ]);

        assert_eq!(
            take_pending_fight_voice(&mut pending, 41),
            [
                (crate::stage_machines::ClipState::Playing, 10),
                (crate::stage_machines::ClipState::Ended, 30)
            ]
        );
        assert_eq!(
            pending,
            std::collections::VecDeque::from([(42, crate::stage_machines::ClipState::Failed, 20,)])
        );
    }
}
