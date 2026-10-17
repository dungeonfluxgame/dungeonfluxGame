//! Presentation-only reading of accepted views: what changed since the last view, and which
//! short "table lines" the room can follow. Nothing here decides a game outcome or sends
//! anything; every input is a field the server already put in the audience's own view, so
//! the browser can animate and voice a change without owning any rule.
use df_protocol::common as rpc;
use df_render::art_contract;

/// Longest text a compact table line shows, in characters.
const LINE_LIMIT: usize = 90;
/// The fixed class of the two playable heroes; the server offers no other.
pub(crate) const HERO_ROLE: &str = "Dwarf Fighter";
const GENERATED: &str = "/assets/ui/generated";

/// How the encounter looks in one view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CombatStage {
    Absent,
    Fighting,
    Finished { victory: bool },
}

/// The parts of a view whose change the display presents with motion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CombatSnapshot {
    pub(crate) stage: CombatStage,
    phase: i32,
    rolls: Vec<u64>,
    active: String,
}

/// What changed between two consecutive views.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Transitions {
    pub(crate) entered_combat: bool,
    /// The newest roll of the window is new since the previous view.
    pub(crate) new_roll: bool,
    pub(crate) turn_changed: bool,
    /// The fight ended in this view: `Some(true)` for a victory.
    pub(crate) finished: Option<bool>,
    /// The story's first scene replaced the room: its title card is shown.
    pub(crate) story_begun: bool,
}

fn fnv(parts: &[&[u8]]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for byte in *part {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

fn fingerprint(outcome: &rpc::CombatOutcome) -> u64 {
    let numbers = [
        outcome.attack_die.to_le_bytes().to_vec(),
        outcome.attack_modifier.to_le_bytes().to_vec(),
        outcome.attack_total.to_le_bytes().to_vec(),
        outcome.target_armor_class.to_le_bytes().to_vec(),
        outcome.damage.to_le_bytes().to_vec(),
        outcome.target_hit_points.to_le_bytes().to_vec(),
        vec![
            u8::from(outcome.hit),
            u8::from(outcome.critical),
            u8::from(outcome.knocked_out),
            u8::from(outcome.grazed),
        ],
    ];
    let mut parts: Vec<&[u8]> = vec![
        outcome.actor_name.as_bytes(),
        outcome.target_name.as_bytes(),
    ];
    parts.extend(numbers.iter().map(Vec::as_slice));
    fnv(&parts)
}

/// "Victory" when every server-projected enemy is at zero hit points.
/// The aftermath screen uses this same rule for its heading.
pub(crate) fn is_victory(journey: &rpc::JourneyView) -> bool {
    let Some(combat) = journey.combat.as_ref() else {
        return false;
    };
    let mut enemies = combat
        .participants
        .iter()
        .filter(|participant| participant.side == 2);
    let Some(first_enemy) = enemies.next() else {
        return false;
    };
    first_enemy.hit_points == 0 && enemies.all(|enemy| enemy.hit_points == 0)
}

/// The display's type scale for the aftermath's closing paragraph, by its length in
/// characters: the whole paragraph always shows (never an ellipsis), so longer text is
/// set smaller (`live_endings.css` reads `data-narrative`). The bounds fit a 1080p
/// ten-foot layout with the party row below the story.
pub(crate) fn narrative_tier(narration: &str) -> &'static str {
    match narration.chars().count() {
        0..=450 => "short",
        451..=900 => "full",
        901..=1600 => "long",
        _ => "epic",
    }
}

pub(crate) fn snapshot(journey: &rpc::JourneyView, scene: &rpc::GameplayScene) -> CombatSnapshot {
    let Some(combat) = journey.combat.as_ref() else {
        return CombatSnapshot {
            stage: CombatStage::Absent,
            phase: journey.phase,
            rolls: Vec::new(),
            active: String::new(),
        };
    };
    let inn = scene.destination == rpc::HarborDestination::HarborInn as i32;
    let stage = match rpc::JourneyPhase::try_from(journey.phase) {
        Ok(rpc::JourneyPhase::Combat) => CombatStage::Fighting,
        Ok(rpc::JourneyPhase::Complete) if combat.finished && !inn => CombatStage::Finished {
            victory: is_victory(journey),
        },
        _ => CombatStage::Absent,
    };
    CombatSnapshot {
        stage,
        phase: journey.phase,
        rolls: combat.outcomes.iter().map(fingerprint).collect(),
        active: if combat.finished {
            String::new()
        } else {
            combat.active_actor.clone()
        },
    }
}

/// The outcomes a view carries that the previous view did not, oldest first. A tactical fight can
/// resolve several turns (an enemy's move and attack, then another's) between two views, so more
/// than the newest outcome may be new; the display stages each one, in order.
pub(crate) fn new_outcomes<'a>(
    previous: Option<&CombatSnapshot>,
    journey: &'a rpc::JourneyView,
) -> Vec<&'a rpc::CombatOutcome> {
    let Some(combat) = journey.combat.as_ref() else {
        return Vec::new();
    };
    combat
        .outcomes
        .iter()
        .filter(|outcome| {
            previous.is_none_or(|before| !before.rolls.contains(&fingerprint(outcome)))
        })
        .collect()
}

/// A grid cell as (column, row).
pub(crate) type GridCell = (u32, u32);

/// A combat view cannot replace a newer accepted combat sequence. The caller handles fight
/// restarts and reconnects, which establish a new sequence scope.
pub(crate) const fn is_stale_combat_sequence(current: u64, incoming: u64) -> bool {
    incoming < current
}

/// Merge a move directly before its actor's attack into that attack. A later attack after another
/// actor's turn must not pull the move out of server order.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn fresh_combat_beats(combat: &rpc::CombatView, after: u64) -> Vec<&rpc::CombatOutcome> {
    let pending_move = pending_combat_move(combat).map(|movement| movement.sequence);
    let mut outcomes: Vec<_> = combat
        .outcomes
        .iter()
        .chain(combat.move_outcomes.iter().filter(|movement| {
            pending_move != Some(movement.sequence)
                && !combat
                    .outcomes
                    .iter()
                    .chain(combat.move_outcomes.iter())
                    .filter(|outcome| outcome.sequence > movement.sequence)
                    .min_by_key(|outcome| outcome.sequence)
                    .is_some_and(|next| {
                        next.actor_name == movement.actor_name
                            && next.kind != rpc::OutcomeKind::Move as i32
                    })
        }))
        .filter(|outcome| outcome.sequence > after)
        .collect();
    outcomes.sort_by_key(|outcome| outcome.sequence);
    outcomes
}

/// A final movement outcome for the actor still taking its turn may be followed by its attack
/// in the next view; leave its cue and origin available for that combined turn.
pub(crate) fn pending_combat_move(combat: &rpc::CombatView) -> Option<&rpc::CombatOutcome> {
    combat.move_outcomes.iter().find(|movement| {
        movement.sequence == combat.latest_sequence
            && movement.actor_name == combat.active_actor
            && !combat.outcomes.iter().any(|attack| {
                attack.sequence > movement.sequence && attack.actor_name == movement.actor_name
            })
    })
}

/// A skipped blow still applies its server-provided position and exact health result.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn skipped_blow_tokens(
    tokens: &[df_render::battle::Token],
    attacker: &str,
    target: &str,
    destination: Option<df_render::battle::Cell>,
    hit_points_after: u32,
) -> Vec<df_render::battle::Token> {
    tokens
        .iter()
        .cloned()
        .map(|mut token| {
            if token.name == attacker
                && let Some(cell) = destination
            {
                token.cell = cell;
            }
            if token.name == target
                && let Some(points) = &mut token.hit_points
            {
                points.current = hit_points_after;
            }
            token
        })
        .collect()
}

/// Where each combatant of the view stands, by name.
// Read by the browser display; a native build only runs its tests.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn standing_cells(
    journey: &rpc::JourneyView,
) -> std::collections::HashMap<String, GridCell> {
    journey
        .combat
        .iter()
        .flat_map(|combat| &combat.participants)
        .filter_map(|member| {
            let cell = member.cell.as_ref()?;
            Some((member.name.clone(), (cell.col, cell.row)))
        })
        .collect()
}

/// How one outcome of a view fits its attacker's turn.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) struct TurnMark {
    /// The first outcome of a run by the same attacker: its turn begins here.
    pub(crate) turn_start: bool,
    /// Where the attacker stood before, and where the server put it, on its first outcome of the
    /// view when those differ: a walk shown before that outcome's attack.
    pub(crate) walk: Option<(GridCell, GridCell)>,
}

/// Splits the outcomes a view adds into the turns they belong to, in the server's order (nothing
/// is reordered): a turn is a run of outcomes by one attacker, and it is preceded by the walk that
/// took the attacker from where the previous view left it to where this view says it stands.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn turn_marks(
    outcomes: &[&rpc::CombatOutcome],
    before: &std::collections::HashMap<String, GridCell>,
    now: &std::collections::HashMap<String, GridCell>,
) -> Vec<TurnMark> {
    let mut walked: Vec<&str> = Vec::new();
    let mut last: Option<&str> = None;
    outcomes
        .iter()
        .map(|outcome| {
            let attacker = outcome.actor_name.as_str();
            let turn_start = last != Some(attacker);
            last = Some(attacker);
            let walk = if walked.contains(&attacker) {
                None
            } else {
                walked.push(attacker);
                before
                    .get(attacker)
                    .zip(now.get(attacker))
                    .filter(|(from, to)| from != to)
                    .map(|(from, to)| (*from, *to))
            };
            TurnMark { turn_start, walk }
        })
        .collect()
}

/// The changes the display should present. With no previous view (first view, reconnect,
/// a tab that was hidden) nothing is presented: obsolete cues are skipped, not replayed.
pub(crate) fn transitions(previous: Option<&CombatSnapshot>, next: &CombatSnapshot) -> Transitions {
    let Some(previous) = previous else {
        return Transitions::default();
    };
    let fighting = next.stage == CombatStage::Fighting;
    Transitions {
        entered_combat: fighting && previous.stage != CombatStage::Fighting,
        new_roll: !next.rolls.is_empty() && previous.rolls != next.rolls,
        turn_changed: fighting
            && previous.stage == CombatStage::Fighting
            && previous.active != next.active,
        finished: match (previous.stage, next.stage) {
            (CombatStage::Fighting, CombatStage::Finished { victory }) => Some(victory),
            _ => None,
        },
        story_begun: previous.phase == rpc::JourneyPhase::Room as i32
            && next.phase == rpc::JourneyPhase::Opening as i32,
    }
}

/// The longest sub-line a card or the lobby lede carries.
const LEDE_CHARACTERS: usize = 120;

/// The first sentence of `text`, ended by a full stop, a question mark or an exclamation mark.
/// Generated narration can open with a long sentence, so one that would overrun a line is cut
/// at its last clause break within the limit.
pub(crate) fn first_sentence(text: &str) -> String {
    let end = text
        .char_indices()
        .find(|&(index, character)| {
            matches!(character, '.' | '!' | '?')
                && text[index + character.len_utf8()..].starts_with(' ')
        })
        .map_or(text.len(), |(index, character)| {
            index + character.len_utf8()
        });
    let sentence = &text[..end];
    if sentence.chars().count() <= LEDE_CHARACTERS {
        return sentence.to_owned();
    }
    let clause_end = sentence
        .char_indices()
        .filter(|&(index, character)| {
            let characters = sentence[..index].chars().count();
            matches!(character, ',' | ';' | ':') && (40..=LEDE_CHARACTERS).contains(&characters)
        })
        .map(|(index, _)| index)
        .next_back();
    match clause_end {
        Some(index) => format!("{}.", sentence[..index].trim_end()),
        None => sentence.to_owned(),
    }
}

/// Which card the display shows over the dimmed painting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CardKind {
    /// The adventure's title when the story begins.
    Title,
    /// A fight won.
    Victory,
    /// A fight lost.
    Defeat,
    /// The adventure's end: its title, "The End", the party and the outcome.
    End,
}

/// One hero named on the closing card.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CardHero {
    pub(crate) name: String,
    pub(crate) role: String,
    /// Served path of the hero's small portrait.
    pub(crate) portrait: String,
}

/// The text of a title or closing card. Only words the accepted view already carries (the
/// scene's title and place, the first sentence of its narration, the heroes' names) and the
/// fixed kicker of the card appear on it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Card {
    pub(crate) kind: CardKind,
    pub(crate) kicker: String,
    pub(crate) headline: String,
    pub(crate) subline: String,
    pub(crate) place: String,
    pub(crate) heroes: Vec<CardHero>,
}

/// The card shown when the story begins: the opening scene's title and place.
pub(crate) fn title_card(scene: &rpc::GameplayScene, _narration: &str) -> Card {
    Card {
        kind: CardKind::Title,
        kicker: "A tale begins".to_owned(),
        headline: scene.title.clone(),
        subline: String::new(),
        place: scene.place.clone(),
        heroes: Vec::new(),
    }
}

/// The card shown when a fight is won or lost: it names the heroes who played.
pub(crate) fn closing_card(
    journey: &rpc::JourneyView,
    scene: &rpc::GameplayScene,
    narration: &str,
    victory: bool,
) -> Card {
    Card {
        kind: if victory {
            CardKind::Victory
        } else {
            CardKind::Defeat
        },
        kicker: if victory {
            "The tale continues"
        } else {
            "The heroes fall"
        }
        .to_owned(),
        headline: scene.title.clone(),
        subline: first_sentence(narration),
        place: scene.place.clone(),
        heroes: journey
            .party
            .iter()
            .enumerate()
            .take(2)
            .filter(|(_, member)| member.character_ready)
            .map(|(index, member)| CardHero {
                name: member.name.clone(),
                role: HERO_ROLE.to_owned(),
                portrait: format!("{GENERATED}/figure-hero-{}-documentary-photo", index + 1),
            })
            .collect(),
    }
}

/// The adventure's end card: the errand's title, "The End", each hero's state and the outcome.
pub(crate) fn end_card(journey: &rpc::JourneyView, adventure: &str, outcome: &str) -> Card {
    Card {
        kind: CardKind::End,
        kicker: adventure.to_owned(),
        headline: "The End".to_owned(),
        subline: first_sentence(outcome),
        place: String::new(),
        heroes: journey
            .party
            .iter()
            .enumerate()
            .take(2)
            .filter(|(_, member)| member.character_ready)
            .map(|(index, member)| CardHero {
                name: member.name.clone(),
                role: if member.unconscious || member.hit_points == 0 {
                    "down".to_owned()
                } else {
                    format!("{} / {} HP", member.hit_points, member.maximum_hit_points)
                },
                portrait: format!("{GENERATED}/figure-hero-{}-documentary-photo", index + 1),
            })
            .collect(),
    }
}

/// Lighting family of a scene painting, chosen from its asset name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SceneFamily {
    /// Lantern-lit rooms: figures are graded warmer.
    Interior,
    /// Open air and water at night: figures are graded cooler.
    Exterior,
}

impl SceneFamily {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Interior => "interior",
            Self::Exterior => "exterior",
        }
    }
}

/// Whether the scene is indoors, as the server declared it from the place's setting.
pub(crate) fn scene_family(scene: &rpc::GameplayScene) -> SceneFamily {
    if scene.interior {
        SceneFamily::Interior
    } else {
        SceneFamily::Exterior
    }
}

/// Which standing place a character takes on the stage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ActorSlot {
    /// The party member at this position, standing at the left or right edge.
    Hero(usize),
    /// A non-player character of the scene's cast, by position in the server's cast list.
    Npc(usize),
}

/// How many non-player figures the stage can hold; a longer cast list shows its first ones.
pub(crate) const MAX_NPCS: usize = 3;

/// One character standing in the scene.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StageActor {
    pub(crate) actor_id: String,
    pub(crate) slot: ActorSlot,
    pub(crate) name: String,
    pub(crate) role: String,
    /// Served path of the transparent cut-out; empty when the server named none.
    pub(crate) sprite: String,
    /// Served path of the small portrait; the 2:3 portrait name derives from it.
    pub(crate) portrait: String,
    pub(crate) hit_points: Option<(u32, u32)>,
    /// Speaking now, or acting on this turn.
    pub(crate) speaking: bool,
    pub(crate) down: bool,
    // PLAY-L01: a server-generated expression variant of this character's cut-out, loaded from
    // `/gameplay/imagery/<key>`; `None` shows the authored cut-out.
    pub(crate) live_sprite: Option<String>,
}

/// A local served asset path, or `None` for anything else. Only these reach CSS or `src`.
/// A generated-art path goes through the art contract: only chrome and the reviewed photograph
/// that answers it are served, so a painted stem, a role token or an identity key with no
/// photograph answers `None` and the caller's neutral fallback stands.
pub(crate) fn asset_url(path: &str) -> Option<String> {
    let path = path.trim_start_matches('/');
    let local = path.starts_with("assets/")
        && path
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'/' | b'-' | b'.' | b'_'));
    if !local {
        return None;
    }
    match path.strip_prefix("assets/ui/generated/") {
        // Sound and dice live in subdirectories; they are chrome and keep their own paths.
        Some(generated) if !generated.contains('/') => {
            art_contract::fetch_target(generated).map(|target| format!("{GENERATED}/{target}"))
        }
        _ => Some(format!("/{path}")),
    }
}

/// The picture of a place for a role, by the art contract's order: the live image already made
/// for it (the room photograph or the scene plate, per the role), then a reviewed photograph the
/// scene's art names. `None` means nothing is ready: the role's neutral stands and the server,
/// not the display, asks for the live image to be made.
pub(crate) fn place_picture(
    role: art_contract::Role,
    scene_asset: &str,
    live_backdrop: &str,
    live_plate: &str,
) -> Option<String> {
    let plan = art_contract::plan(role, scene_asset);
    plan.live
        .iter()
        .find_map(|source| match source {
            art_contract::LiveSource::RoomPhoto => imagery_url(live_backdrop),
            art_contract::LiveSource::ScenePlate => imagery_url(live_plate),
            art_contract::LiveSource::CanonicalCharacter => None,
        })
        .or_else(|| plan.reviewed.map(|photo| format!("{GENERATED}/{photo}")))
}

/// Whether the scene art a view names is a reference the art contract knows: nothing, a role
/// token, a live image path, a reviewed photograph, an identity key, chrome, or a served local
/// asset (the reviewed harbour plate and the concept art among them). It answers whether the
/// reference can be drawn, never whether the view may be shown: a view with art the display
/// cannot draw is still shown, on the neutral stage, and the display never refuses a whole view
/// because its base scene art is a token or is not ready yet. Only a deleted painting (a
/// depiction stem that is neither reviewed nor a key) answers `false`, so the caller can log it.
pub(crate) fn scene_art_admitted(path: &str) -> bool {
    if asset_url(path).is_some() {
        return true;
    }
    !matches!(
        art_contract::classify(path),
        art_contract::Reference::Painted(_)
    )
}

// PLAY-L01: a server-generated image path: `/gameplay/imagery/` and a 64-digit lowercase
// hexadecimal key, nothing else. Only these reach CSS or `src` besides local assets.
pub(crate) fn imagery_url(path: &str) -> Option<String> {
    let key = path.strip_prefix("/gameplay/imagery/")?;
    let plain = key.len() == 64 && key.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'));
    plain.then(|| path.to_owned())
}

/// PLAY-V09: what the server says about the scene's plate (one composed picture of the scene).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum PlateState {
    /// No plate is planned for this scene: it looks as it always did.
    #[default]
    None,
    Pending,
    Ready,
    Failed,
    /// PLAY-V12: imagery is on but this story scene will not get a plate: it holds its
    /// establishing picture.
    Unavailable,
}

impl PlateState {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Pending => "pending",
            Self::Ready => "ready",
            Self::Failed => "failed",
            Self::Unavailable => "unavailable",
        }
    }
}

impl PlateState {
    /// The state a stage flag names (the inverse of [`Self::name`]); anything else is none.
    pub(crate) fn from_name(name: &str) -> Self {
        match name {
            "pending" => Self::Pending,
            "ready" => Self::Ready,
            "failed" => Self::Failed,
            "unavailable" => Self::Unavailable,
            _ => Self::None,
        }
    }
}

/// PLAY-V12: which picture a story scene shows on the shared display.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Shot {
    /// The scene's cut-out figures stand in the painting: imagery is off, no plate is planned
    /// for the scene, or its plate failed for good.
    Stage,
    /// The place picture alone, with the narrator over it, until the plate can be cut to. A
    /// scene never shows its cut-outs while its plate is on its way.
    Establishing,
    /// The composed plate.
    Plate,
}

impl Shot {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Stage => "stage",
            Self::Establishing => "establishing",
            Self::Plate => "plate",
        }
    }
}

/// The picture a story scene shows: the plate only once its picture has loaded and the
/// narration of the scene's entry is over; the establishing shot until then.
pub(crate) fn shot_of(plate: PlateState, picture_loaded: bool, narration_over: bool) -> Shot {
    match plate {
        PlateState::None | PlateState::Failed => Shot::Stage,
        PlateState::Pending | PlateState::Unavailable => Shot::Establishing,
        PlateState::Ready if picture_loaded && narration_over => Shot::Plate,
        PlateState::Ready => Shot::Establishing,
    }
}

/// Where one hero's head is in the plate (permille of the plate), and the side of it a bubble
/// stands on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PlateHero {
    /// The party member, counted from 0.
    pub(crate) index: usize,
    pub(crate) bubble_side: crate::plate_layout::Side,
    pub(crate) head_x: u32,
    pub(crate) head_y: u32,
    pub(crate) head_width: u32,
    pub(crate) head_height: u32,
}

/// Where the speaker's head is in the plate (permille of the plate) and which side of it is calm,
/// and where the heroes' heads are when the server found them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PlateActor {
    pub(crate) actor_id: String,
    pub(crate) bubble_side: crate::plate_layout::Side,
    pub(crate) head_x: u32,
    pub(crate) head_y: u32,
    pub(crate) head_width: u32,
    pub(crate) head_height: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PlateGeometry {
    pub(crate) bubble_side: crate::plate_layout::Side,
    pub(crate) head_x: u32,
    pub(crate) head_y: u32,
    pub(crate) head_width: u32,
    pub(crate) head_height: u32,
    pub(crate) heroes: Vec<PlateHero>,
    pub(crate) actors: Vec<PlateActor>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct PlateView {
    pub(crate) state: PlateState,
    /// Server-owned work for the current visible scene, including a replacement while the old
    /// plate remains on screen. Successor prefetches never set this flag.
    pub(crate) update_pending: bool,
    /// A well-formed imagery path, present only when the plate is ready.
    pub(crate) url: Option<String>,
    pub(crate) layout: Option<PlateGeometry>,
}

/// The plate a scene carries. A ready plate without a well-formed path is a failed one: the
/// scene keeps its cut-outs.
pub(crate) fn plate_of(scene: &rpc::GameplayScene) -> PlateView {
    let state = match rpc::PlateState::try_from(scene.plate_state) {
        Ok(rpc::PlateState::Pending) => PlateState::Pending,
        Ok(rpc::PlateState::Ready) => PlateState::Ready,
        Ok(rpc::PlateState::Failed) => PlateState::Failed,
        Ok(rpc::PlateState::Unavailable) => PlateState::Unavailable,
        Ok(rpc::PlateState::Unspecified) | Err(_) => PlateState::None,
    };
    let url = imagery_url(&scene.live_plate);
    let side_of = |side: i32| match rpc::PlateSide::try_from(side) {
        Ok(rpc::PlateSide::Left) => Some(crate::plate_layout::Side::Left),
        Ok(rpc::PlateSide::Right) => Some(crate::plate_layout::Side::Right),
        Ok(rpc::PlateSide::Unspecified) | Err(_) => None,
    };
    let layout = scene.plate_layout.as_ref().and_then(|layout| {
        Some(PlateGeometry {
            actors: layout
                .actors
                .iter()
                .filter_map(|head| {
                    if !scene.cast.iter().any(|actor| actor.id == head.actor_id)
                        || head.head_x_permille > 1000
                        || head.head_y_permille > 1000
                        || !(1..=400).contains(&head.head_width_permille)
                        || !(1..=500).contains(&head.head_height_permille)
                    {
                        return None;
                    }
                    Some(PlateActor {
                        actor_id: head.actor_id.clone(),
                        bubble_side: side_of(head.bubble_side)?,
                        head_x: head.head_x_permille,
                        head_y: head.head_y_permille,
                        head_width: head.head_width_permille,
                        head_height: head.head_height_permille,
                    })
                })
                .collect(),
            bubble_side: side_of(layout.bubble_side)?,
            head_x: layout.head_x_permille,
            head_y: layout.head_y_permille,
            head_width: layout.head_width_permille,
            head_height: layout.head_height_permille,
            // A hero entry with no usable side is left out: that hero's bubble stands by their card.
            heroes: layout
                .heroes
                .iter()
                .filter_map(|hero| {
                    Some(PlateHero {
                        index: usize::try_from(hero.hero_index).ok()?,
                        bubble_side: side_of(hero.bubble_side)?,
                        head_x: hero.head_x_permille,
                        head_y: hero.head_y_permille,
                        head_width: hero.head_width_permille,
                        head_height: hero.head_height_permille,
                    })
                })
                .collect(),
        })
    });
    match (state, url) {
        (PlateState::Ready, None) => PlateView {
            state: PlateState::Failed,
            update_pending: scene.plate_update_pending,
            ..PlateView::default()
        },
        (PlateState::Ready, Some(url)) => PlateView {
            state,
            url: Some(url),
            layout,
            update_pending: scene.plate_update_pending,
        },
        (state, _) => PlateView {
            state,
            update_pending: scene.plate_update_pending,
            ..PlateView::default()
        },
    }
}

fn hit_points(member: &rpc::PartyMemberView) -> Option<(u32, u32)> {
    (member.maximum_hit_points > 0).then_some((member.hit_points, member.maximum_hit_points))
}

fn is_down(member: &rpc::PartyMemberView) -> bool {
    member.character_ready
        && (member.unconscious || (member.maximum_hit_points > 0 && member.hit_points == 0))
}

/// The characters standing in the scene this view shows: both heroes once their characters
/// are ready, and the scene's cast exactly as the server sends it.
///
/// SERVER CAST SWITCH POINT: this function and [`cast_of`] are the one place the stage's cast
/// is derived. The cast is the server's `GameplayScene.cast` (sprite, portrait and who is
/// speaking); only the fight's hit points and turn marker are joined from the combat
/// participants of the same view, by name.
pub(crate) fn scene_cast(view: &rpc::ViewMessage) -> Vec<StageActor> {
    let (journey, scene) = match &view.audience {
        Some(rpc::view_message::Audience::Display(display)) => {
            (display.journey.as_ref(), display.scene.as_ref())
        }
        Some(rpc::view_message::Audience::Player(player)) => {
            (player.journey.as_ref(), player.scene.as_ref())
        }
        None => (None, None),
    };
    match (journey, scene) {
        (Some(journey), Some(scene)) => cast_of(journey, scene),
        _ => Vec::new(),
    }
}

/// [`scene_cast`] for a view already split into its journey and scene.
fn cast_of(journey: &rpc::JourneyView, scene: &rpc::GameplayScene) -> Vec<StageActor> {
    let fighting = matches!(
        rpc::JourneyPhase::try_from(journey.phase),
        Ok(rpc::JourneyPhase::Combat)
    ) && journey
        .combat
        .as_ref()
        .is_some_and(|combat| !combat.finished);
    let mut cast: Vec<StageActor> = journey
        .party
        .iter()
        .enumerate()
        .take(2)
        .filter(|(_, member)| member.character_ready)
        .map(|(index, member)| StageActor {
            slot: ActorSlot::Hero(index),
            actor_id: String::new(),
            name: member.name.clone(),
            role: HERO_ROLE.to_owned(),
            sprite: format!("{GENERATED}/sprite-hero-{}", index + 1),
            portrait: format!("{GENERATED}/figure-hero-{}-documentary-photo", index + 1),
            hit_points: hit_points(member),
            speaking: fighting && member.active_turn,
            down: is_down(member),
            // PLAY-H01: the server's health variant of this hero, by name.
            live_sprite: journey
                .live_faces
                .iter()
                .find(|face| face.name == member.name)
                .and_then(|face| imagery_url(&face.sprite)),
        })
        .collect();
    let participants = journey
        .combat
        .as_ref()
        .map_or(&[][..], |combat| &combat.participants[..]);
    let heroes = journey.party.len();
    cast.extend(
        scene
            .cast
            .iter()
            .take(MAX_NPCS)
            .enumerate()
            .map(|(position, actor)| {
                let participant = participants
                    .iter()
                    .skip(heroes)
                    .find(|participant| participant.name == actor.name);
                StageActor {
                    slot: ActorSlot::Npc(position),
                    actor_id: actor.id.clone(),
                    name: actor.name.clone(),
                    role: String::new(),
                    sprite: asset_url(&actor.sprite).unwrap_or_default(),
                    portrait: asset_url(&actor.portrait).unwrap_or_default(),
                    hit_points: participant.and_then(hit_points),
                    speaking: if fighting {
                        participant.is_some_and(|participant| participant.active_turn)
                    } else {
                        actor.speaking
                    },
                    down: participant.is_some_and(|participant| {
                        participant.maximum_hit_points > 0 && participant.hit_points == 0
                    }),
                    // PLAY-L01: the server's generated expression variant of this cut-out.
                    live_sprite: imagery_url(&actor.live_sprite),
                }
            }),
    );
    cast
}

/// Who a table line is attributed to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TableRole {
    /// The party member at this position of the view's party list.
    Hero(usize),
    Npc,
    Narrator,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TableKind {
    Action,
    Speech,
}

/// One short line the whole room can follow.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TableLine {
    pub(crate) actor_id: String,
    /// The server's increasing sequence. Consumers show a line once per sequence, so a redraw
    /// of the same view never repeats it.
    pub(crate) sequence: u64,
    pub(crate) speaker: String,
    pub(crate) role: TableRole,
    pub(crate) kind: TableKind,
    /// The whole line. A compact surface shows [`TableLine::short_text`].
    pub(crate) text: String,
    /// The scene's own authored speaker line, as opposed to something said at the table.
    pub(crate) scripted: bool,
}

impl TableLine {
    /// The line clamped to about [`LINE_LIMIT`] characters.
    pub(crate) fn short_text(&self) -> String {
        clamp_line(&self.text)
    }
}

fn clamp_line(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= LINE_LIMIT {
        return text.to_owned();
    }
    let kept: String = text.chars().take(LINE_LIMIT - 1).collect();
    format!("{}…", kept.trim_end())
}

/// A short phrase for a roll of the combat window, used only when the table log has no line
/// for the attacker yet.
pub(crate) fn action_text(outcome: &rpc::CombatOutcome) -> String {
    let target = &outcome.target_name;
    let mut text = if outcome.hit && outcome.critical {
        format!("lands a critical blow on {target} for {}", outcome.damage)
    } else if outcome.hit {
        format!("strikes at {target} — hit for {}", outcome.damage)
    } else if outcome.grazed {
        format!("grazes {target} for {}", outcome.damage)
    } else {
        format!("swings at {target} and misses")
    };
    if outcome.knocked_out {
        text.push_str(" — down!");
    }
    text
}

/// The line the scene itself opens with: its speaker's words, which the server's table log does
/// not carry because they belong to the scene, not to anything said at the table. Its identity
/// is a hash of the scene and the line, so it is shown once per scene and survives a redraw.
fn scene_line(journey: &rpc::JourneyView, scene: Option<&rpc::GameplayScene>) -> Option<TableLine> {
    let text = journey.dialogue.trim();
    if text.is_empty() {
        return None;
    }
    let title = scene.map_or("", |scene| scene.title.as_str());
    let (speaker, actor_id, role) = if journey.speaker.trim().is_empty() {
        ("", "", TableRole::Narrator)
    } else {
        match crate::voice_line::scene_speaker(scene, &journey.speaker) {
            crate::voice_line::SceneSpeaker::Matched(actor) => {
                (actor.name.as_str(), actor.id.as_str(), TableRole::Npc)
            }
            crate::voice_line::SceneSpeaker::Missing => {
                (journey.speaker.trim(), "", TableRole::Npc)
            }
            crate::voice_line::SceneSpeaker::Ambiguous => return None,
        }
    };
    Some(TableLine {
        sequence: fnv(&[title.as_bytes(), speaker.as_bytes(), text.as_bytes()]),
        actor_id: actor_id.to_owned(),
        speaker: speaker.to_owned(),
        role,
        kind: TableKind::Speech,
        text: text.to_owned(),
        scripted: true,
    })
}

/// Who the server currently marks as writing a reply to a hero's typed words.
pub(crate) fn replying_speaker(view: &rpc::ViewMessage) -> Option<String> {
    let (journey, scene) = match view.audience.as_ref()? {
        rpc::view_message::Audience::Display(display) => {
            (display.journey.as_ref(), display.scene.as_ref())
        }
        rpc::view_message::Audience::Player(player) => {
            (player.journey.as_ref(), player.scene.as_ref())
        }
    };
    pending_reply_speaker(journey?, scene)
}

/// Whether a reply to the hero's newest line is still pending for this scene. The server marks the
/// reply before its answer is logged, and the mark can outlive the answer; once the marked
/// speaker's spoken line is logged after the hero's newest turn, the reply is no longer pending.
pub(crate) fn scene_reply_pending(
    journey: &rpc::JourneyView,
    scene: Option<&rpc::GameplayScene>,
) -> bool {
    pending_reply_speaker(journey, scene).is_some()
}

/// The speaker of a reply still awaiting its answer, with the speaker's display name. The server
/// marker alone is not enough: an answer logged after the hero's newest turn ends the reply even
/// while the marker is still set, so neither the thinking indicator nor the caption outlives it.
fn pending_reply_speaker(
    journey: &rpc::JourneyView,
    scene: Option<&rpc::GameplayScene>,
) -> Option<String> {
    let speaker = journey.reply_speaker.trim();
    if speaker.is_empty() {
        return None;
    }
    let speaker = match crate::voice_line::scene_speaker(scene, speaker) {
        crate::voice_line::SceneSpeaker::Matched(actor) => actor.name.as_str(),
        crate::voice_line::SceneSpeaker::Missing => speaker,
        crate::voice_line::SceneSpeaker::Ambiguous => return None,
    };
    if answered_after_latest_hero_turn(journey, speaker) {
        return None;
    }
    Some(speaker.to_owned())
}

/// Whether `speaker` has a spoken line logged after the hero's newest turn. An older answer from
/// the same speaker cannot satisfy a newer question. Before any hero turn, only the speaker's
/// line as the newest row answers, so an earlier line under later narration never resurfaces.
fn answered_after_latest_hero_turn(journey: &rpc::JourneyView, speaker: &str) -> bool {
    let spoken_by_speaker = |line: &rpc::TableLine| {
        rpc::TableRole::try_from(line.role) == Ok(rpc::TableRole::Npc)
            && rpc::TableKind::try_from(line.kind) == Ok(rpc::TableKind::Speech)
            && crate::voice_line::speaker_names_match(&line.speaker, speaker)
            && !line.text.trim().is_empty()
    };
    match latest_hero_turn(journey) {
        Some(turn) => journey
            .table_log
            .iter()
            .any(|line| line.sequence > turn && spoken_by_speaker(line)),
        None => journey.table_log.last().is_some_and(spoken_by_speaker),
    }
}

fn latest_hero_turn(journey: &rpc::JourneyView) -> Option<u64> {
    journey
        .table_log
        .iter()
        .filter(|line| {
            rpc::TableRole::try_from(line.role) == Ok(rpc::TableRole::Hero)
                && matches!(
                    rpc::TableKind::try_from(line.kind),
                    Ok(rpc::TableKind::Speech | rpc::TableKind::Action)
                )
        })
        .map(|line| line.sequence)
        .max()
}

/// The hero's newest typed or acted line, the one a pending reply answers, if it has words.
pub(crate) fn newest_hero_line(journey: &rpc::JourneyView) -> Option<(u64, &str, &str)> {
    journey.table_log.iter().rev().find_map(|line| {
        (rpc::TableRole::try_from(line.role) == Ok(rpc::TableRole::Hero)
            && matches!(
                rpc::TableKind::try_from(line.kind),
                Ok(rpc::TableKind::Speech | rpc::TableKind::Action)
            )
            && !line.text.trim().is_empty())
        .then_some((line.sequence, line.speaker.as_str(), line.text.trim()))
    })
}

/// The newest public reply from the scene's speaker, if the accepted table log has one.
pub(crate) fn newest_scene_speech(journey: &rpc::JourneyView) -> Option<(u64, &str, &str)> {
    let speaker = journey.speaker.trim();
    if speaker.is_empty() {
        return None;
    }
    let scene_start = latest_hero_turn(journey).unwrap_or(0);
    journey.table_log.iter().rev().find_map(|line| {
        (rpc::TableRole::try_from(line.role) == Ok(rpc::TableRole::Npc)
            && rpc::TableKind::try_from(line.kind) == Ok(rpc::TableKind::Speech)
            && crate::voice_line::speaker_names_match(&line.speaker, speaker)
            && line.sequence > scene_start
            && !line.text.trim().is_empty())
        .then_some((line.sequence, line.speaker.as_str(), line.text.trim()))
    })
}

/// The table lines of one audience view, oldest first: the scene's own speaker line, then the
/// server's public table log. A player view omits the newest scene-speaker reply its story card
/// shows, and shows the scene's authored line only once even when the log repeats it; every other
/// public line appears in server order. Display views retain the complete feed.
///
/// SERVER FEED SWITCH POINT: this is the one place the feed is read; consumers in
/// `live_motion` depend only on this signature. A log line whose role or kind the server left
/// unspecified is skipped. The private clue is never read.
pub(crate) fn table_lines(view: &rpc::ViewMessage) -> Vec<TableLine> {
    let (journey, scene) = match &view.audience {
        Some(rpc::view_message::Audience::Display(display)) => {
            (display.journey.as_ref(), display.scene.as_ref())
        }
        Some(rpc::view_message::Audience::Player(player)) => {
            (player.journey.as_ref(), player.scene.as_ref())
        }
        None => (None, None),
    };
    let Some(journey) = journey else {
        return Vec::new();
    };
    let player_audience = matches!(&view.audience, Some(rpc::view_message::Audience::Player(_)));
    let player_card = player_audience
        && !journey.adventure_ended
        && match rpc::JourneyPhase::try_from(journey.phase) {
            Ok(
                rpc::JourneyPhase::Room | rpc::JourneyPhase::Opening | rpc::JourneyPhase::Dialogue,
            ) => true,
            Ok(rpc::JourneyPhase::Complete) => {
                journey.combat.is_none()
                    || scene.is_some_and(|scene| {
                        scene.destination == rpc::HarborDestination::HarborInn as i32
                    })
            }
            _ => false,
        };
    // The story card already shows the scene speaker's newest reply, so the phone leaves that one
    // line out of the feed. Every other public line, from any hero or NPC, stays in server order.
    let newest_reply = if player_card {
        newest_scene_speech(journey)
    } else {
        None
    };
    let logged: Vec<TableLine> = logged_lines(journey)
        .filter(|line| !newest_reply.is_some_and(|(sequence, _, _)| line.sequence == sequence))
        .collect();
    // A phone shows the same words once: when the server's log carries the scene's line, the
    // server's copy is the line. The display keeps its authored line as before.
    let authored_line = if player_card {
        None
    } else {
        scene_line(journey, scene)
    }
    .filter(|authored| {
        !(player_audience && logged.iter().any(|line| same_utterance(line, authored)))
    });
    authored_line.into_iter().chain(logged).collect()
}

/// Whether two lines are the same words by the same speaker, role and kind. The scene's authored
/// line and the server's copy of it have different sequences, so the words identify them.
pub(crate) fn same_utterance(left: &TableLine, right: &TableLine) -> bool {
    left.role == right.role
        && left.kind == right.kind
        && left.speaker == right.speaker
        && left.text == right.text
}

fn logged_lines(journey: &rpc::JourneyView) -> impl Iterator<Item = TableLine> + '_ {
    journey.table_log.iter().filter_map(|line| {
        let role = match rpc::TableRole::try_from(line.role).ok()? {
            rpc::TableRole::Hero => TableRole::Hero(usize::try_from(line.hero_index).ok()?),
            rpc::TableRole::Npc => TableRole::Npc,
            rpc::TableRole::Narrator => TableRole::Narrator,
            rpc::TableRole::Unspecified => return None,
        };
        let kind = match rpc::TableKind::try_from(line.kind).ok()? {
            rpc::TableKind::Action => TableKind::Action,
            rpc::TableKind::Speech => TableKind::Speech,
            rpc::TableKind::Unspecified => return None,
        };
        Some(TableLine {
            sequence: line.sequence,
            actor_id: line.actor_id.clone(),
            speaker: line.speaker.clone(),
            role,
            kind,
            text: line.text.trim().to_owned(),
            scripted: false,
        })
    })
}

#[cfg(test)]
#[path = "stage_director.rs"]
mod s08_stage_director;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn stale_combat_views_replayed_at_the_recorded_times_keep_health_and_turn() {
        use df_render::battle::{HitPointChange, hit_points_at_sequence};

        let changes = [
            HitPointChange {
                sequence: 1,
                before: 11,
                after: 2,
            },
            HitPointChange {
                sequence: 2,
                before: 2,
                after: 0,
            },
        ];
        let mut latest_sequence = 0;
        let mut presented_sequence = 0;
        let mut current_hit_points = 11;
        let mut active = "Brakka";
        let views = [
            (11_600, 1, 2, "Sigrun", Some(1)),
            (12_500, 0, 11, "Bandit 1", None),
            (20_400, 1, 2, "Sigrun", None),
            (27_300, 2, 0, "Sigrun", Some(2)),
            (29_400, 1, 2, "Bandit 2", None),
        ];

        for (at_ms, sequence, hit_points, next_active, impact) in views {
            if let Some(impact) = impact {
                presented_sequence = presented_sequence.max(impact);
            }
            if !is_stale_combat_sequence(latest_sequence, sequence) {
                latest_sequence = sequence;
                current_hit_points = hit_points;
                active = next_active;
            }

            let projected =
                hit_points_at_sequence(current_hit_points, presented_sequence, &changes);
            match at_ms {
                11_600 | 12_500 | 20_400 => {
                    assert_eq!(projected, 2, "health at {at_ms} ms");
                }
                27_300 | 29_400 => {
                    assert_eq!(projected, 0, "health at {at_ms} ms");
                }
                _ => unreachable!(),
            }
            assert_eq!(active, "Sigrun", "active heading at {at_ms} ms");
        }
    }

    fn battle_token(name: &str, side: df_render::battle::Side) -> df_render::battle::Token {
        use df_render::battle::{AttackStyle, Cell, HitPoints, Token, TokenId};
        Token {
            id: TokenId::new(name),
            side,
            name: name.to_owned(),
            cell: Cell::new(3, 3),
            height_m: 1.8,
            figure: None,
            portrait: None,
            hit_points: Some(HitPoints {
                current: 13,
                maximum: 13,
            }),
            statuses: Vec::new(),
            attack: AttackStyle::Melee,
            voice: None,
        }
    }

    #[test]
    fn live_a_skipped_blow_lands_on_its_position_and_exact_health_result() {
        use df_render::battle::{Cell, Side, Stage, load_embedded};
        let tokens = [
            battle_token("Weasel 2", Side::Enemy),
            battle_token("Brakka", Side::Hero),
        ];
        let after = skipped_blow_tokens(&tokens, "Weasel 2", "Brakka", Some(Cell::new(2, 3)), 8);
        assert_eq!(after[0].cell, Cell::new(2, 3));
        assert_eq!(after[1].hit_points.unwrap().current, 8);
        assert_eq!(
            tokens[1].hit_points.unwrap().current,
            13,
            "borrowed snapshot stays intact"
        );
        let mut stage = Stage::new(load_embedded("salt-quay").unwrap().unwrap());
        stage.set_tokens(tokens.to_vec());
        stage.skip_to_idle();
        stage.set_tokens(after);
        stage.skip_to_idle();
        assert!(
            !stage.is_busy(),
            "skipped movement is landed, not queued for replay"
        );
        let frame = stage.frame();
        let attacker = frame
            .figures
            .iter()
            .find(|figure| figure.name == "Weasel 2")
            .unwrap();
        assert_eq!(attacker.position, stage.grid().cell_centre(Cell::new(2, 3)));
        let hero = frame
            .figures
            .iter()
            .find(|figure| figure.name == "Brakka")
            .unwrap();
        assert_eq!(hero.hit_points.unwrap().current, 8);
    }

    #[test]
    fn live_enemy_headings_remain_on_the_finished_enemy_between_cues() {
        use df_render::battle::{Beat, Side, Stage, TokenId, hold_server_turn, load_embedded};
        let mut stage = Stage::new(load_embedded("salt-quay").unwrap().unwrap());
        stage.set_tokens(vec![
            battle_token("Sigrun", Side::Hero),
            battle_token("Weasel 1", Side::Enemy),
            battle_token("Weasel 2", Side::Enemy),
            battle_token("Weasel 3", Side::Enemy),
        ]);
        let mut headings = Vec::new();
        for (sequence, name) in [(3, "Weasel 1"), (4, "Weasel 2"), (5, "Weasel 3")] {
            stage.hold_active(hold_server_turn(true, 5, sequence - 1));
            stage.show_turn_now(Some(TokenId::new(name)));
            stage
                .stage(Beat::TurnStart {
                    actor: TokenId::new(name),
                })
                .unwrap();
            for _ in 0..120 {
                stage.set_active(Some(TokenId::new("Sigrun")));
                stage.update(std::time::Duration::from_millis(50));
                let shown = stage.frame().hud.active.as_ref().unwrap().0.clone();
                assert_eq!(shown, name, "the raw view must not take the gap's heading");
            }
            headings.push(stage.frame().hud.active.as_ref().unwrap().0.clone());
        }
        assert_eq!(headings, ["Weasel 1", "Weasel 2", "Weasel 3"]);
        stage.hold_active(false);
        stage.update(std::time::Duration::from_millis(50));
        assert_eq!(stage.frame().hud.active.as_ref().unwrap().0, "Sigrun");
    }

    #[test]
    fn live_moves_and_attacks_are_merged_in_server_order_including_move_only_turns() {
        let combat = rpc::CombatView {
            outcomes: vec![rpc::CombatOutcome {
                sequence: 4,
                actor_name: "Weasel 2".to_owned(),
                ..Default::default()
            }],
            move_outcomes: vec![
                rpc::CombatOutcome {
                    sequence: 3,
                    actor_name: "Weasel 1".to_owned(),
                    kind: rpc::OutcomeKind::Move as i32,
                    ..Default::default()
                },
                rpc::CombatOutcome {
                    sequence: 5,
                    actor_name: "Weasel 3".to_owned(),
                    kind: rpc::OutcomeKind::Move as i32,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let fresh = fresh_combat_beats(&combat, 2);
        assert_eq!(
            fresh
                .iter()
                .map(|outcome| outcome.sequence)
                .collect::<Vec<_>>(),
            [3, 4, 5]
        );
        let marks = turn_marks(&fresh, &HashMap::new(), &HashMap::new());
        assert!(marks.iter().all(|mark| mark.turn_start));
        assert_eq!(fresh_combat_beats(&combat, 4)[0].actor_name, "Weasel 3");
    }

    #[test]
    fn live_move_only_turn_stays_before_later_actor_turns() {
        let combat = rpc::CombatView {
            move_outcomes: vec![rpc::CombatOutcome {
                sequence: 1,
                actor_name: "Weasel 1".to_owned(),
                kind: rpc::OutcomeKind::Move as i32,
                ..Default::default()
            }],
            outcomes: vec![
                rpc::CombatOutcome {
                    sequence: 2,
                    actor_name: "Weasel 2".to_owned(),
                    ..Default::default()
                },
                rpc::CombatOutcome {
                    sequence: 3,
                    actor_name: "Weasel 1".to_owned(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        assert_eq!(
            fresh_combat_beats(&combat, 0)
                .iter()
                .map(|outcome| outcome.sequence)
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );
    }

    /// SRD 5.2.1 p. 177 (Attack [Action]); this test covers display ordering only.
    #[test]
    fn live_recorded_enemy_moves_match_the_split_server_sequences() {
        let actors = ["Weasel 1", "Weasel 2", "Weasel 3"];
        let combat = rpc::CombatView {
            active_actor: "Brakka".to_owned(),
            latest_sequence: 6,
            outcomes: actors
                .iter()
                .enumerate()
                .map(|(index, actor)| rpc::CombatOutcome {
                    sequence: index as u64 * 2 + 2,
                    actor_name: (*actor).to_owned(),
                    target_name: "Brakka".to_owned(),
                    ..Default::default()
                })
                .collect(),
            move_outcomes: actors
                .iter()
                .enumerate()
                .map(|(index, actor)| rpc::CombatOutcome {
                    sequence: index as u64 * 2 + 1,
                    actor_name: (*actor).to_owned(),
                    kind: rpc::OutcomeKind::Move as i32,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };

        let fresh = fresh_combat_beats(&combat, 0);
        assert_eq!(
            fresh
                .iter()
                .map(|outcome| outcome.sequence)
                .collect::<Vec<_>>(),
            [2, 4, 6]
        );
        assert_eq!(
            fresh
                .iter()
                .map(|outcome| outcome.actor_name.as_str())
                .collect::<Vec<_>>(),
            actors
        );
    }

    /// SRD 5.2.1 p. 177 (Attack [Action]); this test covers display ordering only.
    #[test]
    fn live_latest_enemy_move_waits_for_its_attack_or_the_next_turn() {
        let movement = rpc::CombatOutcome {
            sequence: 1,
            actor_name: "Weasel 1".to_owned(),
            kind: rpc::OutcomeKind::Move as i32,
            ..Default::default()
        };
        let combat = rpc::CombatView {
            active_actor: "Weasel 1".to_owned(),
            latest_sequence: 1,
            move_outcomes: vec![movement.clone()],
            ..Default::default()
        };

        assert!(
            fresh_combat_beats(&combat, 0).is_empty(),
            "an in-turn move must remain available to merge with the next view's bite"
        );

        let followup = rpc::CombatView {
            active_actor: "Brakka".to_owned(),
            latest_sequence: 2,
            outcomes: vec![rpc::CombatOutcome {
                sequence: 2,
                actor_name: "Weasel 1".to_owned(),
                target_name: "Brakka".to_owned(),
                ..Default::default()
            }],
            move_outcomes: vec![movement.clone()],
            ..Default::default()
        };
        let fresh = fresh_combat_beats(&followup, 1);
        assert_eq!(
            fresh
                .iter()
                .map(|outcome| outcome.sequence)
                .collect::<Vec<_>>(),
            [2]
        );
        let before = HashMap::from([("Weasel 1".to_owned(), (3, 1))]);
        let standing = HashMap::from([("Weasel 1".to_owned(), (3, 3))]);
        assert_eq!(
            turn_marks(&fresh, &before, &standing)[0].walk,
            Some(((3, 1), (3, 3)))
        );

        let completed = rpc::CombatView {
            active_actor: "Brakka".to_owned(),
            ..combat
        };
        assert_eq!(fresh_combat_beats(&completed, 0), [&movement]);
    }

    /// SRD 5.2.1 p. 177: “When you take the Attack action, you can make one attack roll.”
    #[test]
    fn live_three_enemy_phase_uses_the_real_stage_with_a_controlled_clock() {
        use df_render::battle::{AttackResult, Beat, Cell, Side, Stage, TokenId, load_embedded};
        use std::time::Duration;

        const TICK: Duration = Duration::from_millis(50);
        const ROLL_MS: u64 = 1_150;
        const IMPACT_MINIMUM_MS: u64 = 650;

        let mut stage = Stage::new(load_embedded("salt-quay").unwrap().unwrap());
        let grid = stage.grid();
        let mut routes = Vec::new();
        let mut used_cells = Vec::new();
        'routes: for row in 0..grid.rows() {
            for col in 0..grid.cols() {
                let from = Cell::new(col as i32, row as i32);
                if !grid.is_walkable(from) || used_cells.contains(&from) {
                    continue;
                }
                for to_row in 0..grid.rows() {
                    for to_col in 0..grid.cols() {
                        let to = Cell::new(to_col as i32, to_row as i32);
                        if used_cells.contains(&to) {
                            continue;
                        }
                        let Some(path) = grid.path(from, to) else {
                            continue;
                        };
                        let squares = path.len().saturating_sub(1);
                        if !(3..=4).contains(&squares) {
                            continue;
                        }
                        routes.push((from, to));
                        used_cells.extend([from, to]);
                        if routes.len() == 3 {
                            break 'routes;
                        }
                        break;
                    }
                }
            }
        }
        assert_eq!(routes.len(), 3, "the stage scene needs three enemy routes");

        let target_cell = (0..grid.rows())
            .flat_map(|row| (0..grid.cols()).map(move |col| Cell::new(col as i32, row as i32)))
            .find(|cell| grid.is_walkable(*cell) && !used_cells.contains(cell))
            .expect("the stage scene has room for the target");
        let mut target = battle_token("Brakka", Side::Hero);
        target.cell = target_cell;
        target.hit_points = Some(df_render::battle::HitPoints {
            current: 20,
            maximum: 20,
        });
        let mut tokens = vec![target];
        let names = ["Weasel 1", "Weasel 2", "Weasel 3"];
        for (name, (from, _)) in names.iter().zip(&routes) {
            let mut enemy = battle_token(name, Side::Enemy);
            enemy.cell = *from;
            tokens.push(enemy);
        }
        stage.set_tokens(tokens);

        let hero = TokenId::new("Brakka");
        let mut now_ms = 0_u64;
        let mut heading_sequence = Vec::new();
        let mut shares_ms = Vec::new();
        let mut result_times = Vec::new();
        let mut roll_starts = Vec::new();
        let mut expected_hit_points = 20;

        for (name, (_, to)) in names.iter().zip(&routes) {
            let actor = TokenId::new(*name);
            let turn_started = now_ms;
            roll_starts.push(turn_started);
            stage.hold_active(true);
            stage.show_turn_now(Some(actor.clone()));
            heading_sequence.push(name.to_string());
            stage
                .stage(Beat::TurnStart {
                    actor: actor.clone(),
                })
                .unwrap();
            stage.stage_move(&actor, *to).unwrap();
            stage
                .stage(Beat::Approach {
                    attacker: actor.clone(),
                    target: hero.clone(),
                })
                .unwrap();

            while now_ms - turn_started < ROLL_MS {
                stage.set_active(Some(hero.clone()));
                stage.update(TICK);
                now_ms += TICK.as_millis() as u64;
                assert_eq!(stage.frame().hud.active.as_ref().unwrap().0, *name);
            }

            expected_hit_points -= 2;
            let impact_started = now_ms;
            stage
                .stage(Beat::Strike {
                    attacker: actor.clone(),
                    target: hero.clone(),
                    result: AttackResult::Hit {
                        damage: 2,
                        critical: false,
                        downed: false,
                    },
                    target_hit_points_after: Some(expected_hit_points),
                })
                .unwrap();
            let mut result_at = None;
            while stage.is_busy() || now_ms - impact_started < IMPACT_MINIMUM_MS {
                stage.set_active(Some(hero.clone()));
                stage.update(TICK);
                now_ms += TICK.as_millis() as u64;
                let landed = stage
                    .take_impacts()
                    .into_iter()
                    .any(|impact| impact.attacker == actor && impact.target == hero);
                let shown = stage
                    .frame()
                    .figures
                    .iter()
                    .find(|figure| figure.name == "Brakka")
                    .unwrap()
                    .hit_points
                    .unwrap()
                    .current;
                if landed {
                    assert_eq!(shown, expected_hit_points);
                    result_at.get_or_insert(now_ms);
                } else if result_at.is_none() {
                    assert_eq!(shown, expected_hit_points + 2);
                }
                assert_eq!(stage.frame().hud.active.as_ref().unwrap().0, *name);
            }
            let result_at = result_at.expect("the strike result reaches its health card");
            result_times.push(result_at);
            shares_ms.push(now_ms - turn_started);
        }

        stage.set_tokens(stage.tokens().cloned().collect());
        assert!(
            !stage.is_busy(),
            "the final snapshot must not add another move cue"
        );
        stage.hold_active(false);
        stage.set_active(Some(hero));
        assert_eq!(stage.frame().hud.active.as_ref().unwrap().0, "Brakka");
        heading_sequence.push("Brakka".to_owned());
        assert_eq!(
            heading_sequence,
            ["Weasel 1", "Weasel 2", "Weasel 3", "Brakka"]
        );
        assert!(
            shares_ms.iter().all(|share| *share <= 3_500),
            "{shares_ms:?}"
        );
        assert!(now_ms <= 10_500, "whole enemy phase took {now_ms} ms");
        assert!(
            roll_starts
                .iter()
                .skip(1)
                .zip(&result_times)
                .all(|(roll_start, result)| {
                    roll_start >= result && roll_start - result <= 1_500
                }),
            "the next roll starts within 1.5 seconds of the prior readable result"
        );
    }

    #[test]
    fn live_round_card_values_follow_each_presented_blow_heal_and_next_heading() {
        use super::s08_stage_director::{Command, StageDirector, StageState, ViewDiff};
        use df_render::battle::{
            AttackResult, AttackStyle, Beat, CombatFeed, CombatSnapshot, FeedMode, Member, Outcome,
            Side, Stage, TokenId, hit_points_at_sequence, load_embedded,
        };
        use std::time::Duration;

        #[derive(Clone, Copy)]
        struct Blow {
            sequence: u64,
            actor: &'static str,
            target: &'static str,
            damage: u32,
            after: u32,
            downed: bool,
        }

        fn member(id: &str, side: Side, hp: u32) -> Member {
            Member {
                id: id.to_owned(),
                name: id.to_owned(),
                side,
                hit_points: hp,
                maximum_hit_points: if id.starts_with("Bandit") { 11 } else { 13 },
                height_m: 1.5,
                figure: None,
                portrait: None,
                attack: AttackStyle::Melee,
                voice: None,
                cell: None,
                position_sequence: 0,
                statuses: Vec::new(),
            }
        }

        fn outcome(blow: Blow) -> Outcome {
            Outcome {
                actor: blow.actor.to_owned(),
                target: blow.target.to_owned(),
                hit: true,
                critical: false,
                damage: blow.damage,
                target_hit_points_after: blow.after,
                downed: blow.downed,
                weapon: "shortsword".to_owned(),
                grazed: false,
                damage_type: "piercing".to_owned(),
            }
        }

        fn snapshot(cursor: u64, healed: bool, outcomes: &[Outcome]) -> CombatSnapshot {
            let bandit_changes = [
                df_render::battle::HitPointChange {
                    sequence: 1,
                    before: 11,
                    after: 2,
                },
                df_render::battle::HitPointChange {
                    sequence: 2,
                    before: 2,
                    after: 0,
                },
            ];
            let brakka_changes = [df_render::battle::HitPointChange {
                sequence: 3,
                before: 13,
                after: 4,
            }];
            let sigrun_changes = [df_render::battle::HitPointChange {
                sequence: 4,
                before: 12,
                after: 10,
            }];
            let mut members = vec![
                member("Brakka", Side::Hero, if healed { 8 } else { 4 }),
                member("Sigrun", Side::Hero, 10),
                member("Bandit 1", Side::Enemy, 0),
                member("Bandit 2", Side::Enemy, 11),
                member("Bandit 3", Side::Enemy, 11),
            ];
            members[0].hit_points = if healed && cursor >= 4 {
                8
            } else {
                hit_points_at_sequence(members[0].hit_points, cursor, &brakka_changes)
            };
            members[2].hit_points = hit_points_at_sequence(0, cursor, &bandit_changes);
            members[1].hit_points =
                hit_points_at_sequence(members[1].hit_points, cursor, &sigrun_changes);
            CombatSnapshot {
                members,
                active: Some("Brakka".to_owned()),
                round: 1,
                latest_sequence: 4,
                presented_sequence: cursor,
                outcomes: outcomes.to_vec(),
                initiative: Vec::new(),
                begun_live: false,
            }
        }

        fn hp(stage: &Stage, name: &str) -> u32 {
            stage
                .frame()
                .figures
                .iter()
                .find(|figure| figure.name == name)
                .and_then(|figure| figure.hit_points)
                .unwrap_or_else(|| panic!("missing health card for {name}"))
                .current
        }

        fn apply_commands(
            commands: Vec<Command<Blow>>,
            stage: &mut Stage,
            headings: &mut Vec<(u64, String)>,
            now: u64,
        ) {
            for command in commands {
                let Command::Enter {
                    state,
                    detail: Some(blow),
                    ..
                } = command
                else {
                    continue;
                };
                match state {
                    StageState::Rolling => {
                        let actor = TokenId::new(blow.actor);
                        stage.hold_active(true);
                        stage.show_turn_now(Some(actor));
                        headings.push((now, blow.actor.to_owned()));
                    }
                    StageState::Impact => {
                        stage.protect_hit_points_through(blow.sequence);
                        stage
                            .stage(Beat::Strike {
                                attacker: TokenId::new(blow.actor),
                                target: TokenId::new(blow.target),
                                result: AttackResult::Hit {
                                    damage: blow.damage,
                                    critical: false,
                                    downed: blow.downed,
                                },
                                target_hit_points_after: Some(blow.after),
                            })
                            .unwrap();
                    }
                    StageState::Turn => {
                        stage.hold_active(false);
                        stage.show_turn_now(Some(TokenId::new(blow.actor)));
                        headings.push((now, blow.actor.to_owned()));
                    }
                    _ => {}
                }
            }
        }

        const TICK: Duration = Duration::from_millis(100);
        let blows = [
            Blow {
                sequence: 1,
                actor: "Brakka",
                target: "Bandit 1",
                damage: 9,
                after: 2,
                downed: false,
            },
            Blow {
                sequence: 2,
                actor: "Sigrun",
                target: "Bandit 1",
                damage: 2,
                after: 0,
                downed: true,
            },
            Blow {
                sequence: 3,
                actor: "Bandit 2",
                target: "Brakka",
                damage: 9,
                after: 4,
                downed: false,
            },
            Blow {
                sequence: 4,
                actor: "Bandit 3",
                target: "Sigrun",
                damage: 2,
                after: 10,
                downed: false,
            },
        ];
        let outcomes: Vec<Outcome> = blows.iter().copied().map(outcome).collect();
        let mut stage = Stage::new(load_embedded("salt-quay").unwrap().unwrap());
        let mut feed = CombatFeed::new(FeedMode::HoldForCues);
        feed.apply(
            &mut stage,
            &CombatSnapshot {
                members: vec![
                    member("Brakka", Side::Hero, 13),
                    member("Sigrun", Side::Hero, 12),
                    member("Bandit 1", Side::Enemy, 11),
                    member("Bandit 2", Side::Enemy, 11),
                    member("Bandit 3", Side::Enemy, 11),
                ],
                active: Some("Brakka".to_owned()),
                round: 1,
                ..CombatSnapshot::default()
            },
        )
        .unwrap();
        let mut director = StageDirector::new();
        let mut headings = Vec::new();
        for blow in blows {
            let commands = director.on_view(
                0,
                ViewDiff {
                    roll: Some(blow),
                    impact: Some(blow),
                    blow_only: true,
                    sequence: blow.sequence,
                    ..ViewDiff::default()
                },
            );
            apply_commands(commands, &mut stage, &mut headings, 0);
        }
        let next_hero = Blow {
            sequence: 0,
            actor: "Brakka",
            target: "Brakka",
            damage: 0,
            after: 4,
            downed: false,
        };
        let commands = director.on_view(
            0,
            ViewDiff {
                turn_changed: true,
                turn: Some(next_hero),
                ..ViewDiff::default()
            },
        );
        apply_commands(commands, &mut stage, &mut headings, 0);

        let mut cursor = director.presented_sequence();
        feed.apply(&mut stage, &snapshot(cursor, false, &outcomes))
            .unwrap();
        let mut last_bandit = hp(&stage, "Bandit 1");
        let mut last_brakka = hp(&stage, "Brakka");
        let mut last_sigrun = hp(&stage, "Sigrun");
        let mut bandit_changes = vec![(last_bandit, cursor, 0)];
        let mut brakka_changes = vec![(last_brakka, cursor, 0)];
        let mut sigrun_changes = vec![(last_sigrun, cursor, 0)];
        let mut impact_times = std::collections::HashMap::new();
        let mut now = 0_u64;
        while now < 30_000
            && !(director.is_idle()
                && !stage.is_busy()
                && headings.last().is_some_and(|(_, actor)| actor == "Brakka"))
        {
            feed.apply(&mut stage, &snapshot(cursor, false, &outcomes))
                .unwrap();
            director.set_battlefield_busy(stage.is_busy());
            let commands = director.on_tick(now);
            apply_commands(commands, &mut stage, &mut headings, now);
            stage.update(TICK);
            for impact in stage.take_impacts() {
                let blow = blows
                    .iter()
                    .find(|blow| blow.actor == impact.attacker.0 && blow.target == impact.target.0)
                    .unwrap();
                director.observe_presented(blow.sequence);
                stage.observe_presented_sequence(blow.sequence);
                cursor = director.presented_sequence();
                impact_times.insert(blow.sequence, now + 100);
            }
            feed.apply(&mut stage, &snapshot(cursor, false, &outcomes))
                .unwrap();
            let bandit = hp(&stage, "Bandit 1");
            let brakka = hp(&stage, "Brakka");
            assert_eq!(
                bandit,
                if cursor >= 2 {
                    0
                } else if cursor >= 1 {
                    2
                } else {
                    11
                },
                "Bandit 1 at {now} ms, sequence {cursor}"
            );
            assert_eq!(
                brakka,
                if cursor >= 3 { 4 } else { 13 },
                "Brakka at {now} ms, sequence {cursor}"
            );
            let sigrun = hp(&stage, "Sigrun");
            assert_eq!(
                sigrun,
                if cursor >= 4 { 10 } else { 12 },
                "Sigrun at {now} ms, sequence {cursor}"
            );
            assert_eq!(hp(&stage, "Bandit 2"), 11);
            assert_eq!(hp(&stage, "Bandit 3"), 11);
            if bandit != last_bandit {
                assert!(bandit < last_bandit, "Bandit 1 health went backwards");
                bandit_changes.push((bandit, cursor, now + 100));
                last_bandit = bandit;
            }
            if brakka != last_brakka {
                assert!(brakka < last_brakka, "Brakka health went backwards");
                brakka_changes.push((brakka, cursor, now + 100));
                last_brakka = brakka;
            }
            if sigrun != last_sigrun {
                assert!(sigrun < last_sigrun, "Sigrun health went backwards");
                sigrun_changes.push((sigrun, cursor, now + 100));
                last_sigrun = sigrun;
            }
            now += 100;
        }
        assert!(
            director.is_idle() && !stage.is_busy(),
            "fight cues did not drain"
        );
        assert_eq!(
            headings
                .iter()
                .map(|(_, actor)| actor.as_str())
                .collect::<Vec<_>>(),
            ["Brakka", "Sigrun", "Bandit 2", "Bandit 3", "Brakka"]
        );
        assert_eq!(
            bandit_changes,
            [
                (11, 0, 0),
                (2, 1, impact_times[&1]),
                (0, 2, impact_times[&2])
            ]
        );
        assert_eq!(brakka_changes, [(13, 0, 0), (4, 3, impact_times[&3])]);
        assert_eq!(sigrun_changes, [(12, 0, 0), (10, 4, impact_times[&4])]);
        let final_blow_at = impact_times[&4];
        let next_hero_at = headings.last().unwrap().0;
        assert!(
            next_hero_at >= final_blow_at && next_hero_at - final_blow_at <= 1_500,
            "next active hero heading at {next_hero_at} ms after blow at {final_blow_at} ms"
        );

        feed.apply(&mut stage, &snapshot(cursor, true, &outcomes))
            .unwrap();
        let mut healed_at = None;
        for heal_tick in 0_u64..100 {
            stage.update(TICK);
            let brakka = stage
                .frame()
                .figures
                .into_iter()
                .find(|figure| figure.name == "Brakka")
                .unwrap();
            if brakka.hit_points.unwrap().current != last_brakka {
                assert_eq!(brakka.hit_points.unwrap().current, 8);
                assert!(
                    brakka.heal_glow > 0.0,
                    "the HP rise must land with the heal"
                );
                healed_at = Some(now + (heal_tick + 1) * 100);
                last_brakka = 8;
            }
            assert_eq!(
                brakka.hit_points.unwrap().current,
                if healed_at.is_some() { 8 } else { 4 },
                "Brakka's healing card at {} ms",
                now + (heal_tick + 1) * 100
            );
            feed.apply(&mut stage, &snapshot(cursor, true, &outcomes))
                .unwrap();
            if healed_at.is_some() && !stage.is_busy() {
                break;
            }
        }
        assert!(
            healed_at.is_some(),
            "Second Wind never changed Brakka's card"
        );
        assert_eq!(last_brakka, 8);
        assert_eq!(hp(&stage, "Bandit 1"), 0, "the kill remains on the card");
    }

    #[test]
    fn live_an_attacking_enemys_walk_is_part_of_its_roll_without_an_extra_gap() {
        let combat = rpc::CombatView {
            move_outcomes: vec![rpc::CombatOutcome {
                sequence: 3,
                actor_name: "Weasel 1".to_owned(),
                kind: rpc::OutcomeKind::Move as i32,
                ..Default::default()
            }],
            outcomes: vec![rpc::CombatOutcome {
                sequence: 4,
                actor_name: "Weasel 1".to_owned(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let fresh = fresh_combat_beats(&combat, 2);
        assert_eq!(
            fresh
                .iter()
                .map(|outcome| outcome.sequence)
                .collect::<Vec<_>>(),
            [4]
        );
        let before = HashMap::from([("Weasel 1".to_owned(), (3, 1))]);
        let now = HashMap::from([("Weasel 1".to_owned(), (3, 3))]);
        assert_eq!(
            turn_marks(&fresh, &before, &now)[0].walk,
            Some(((3, 1), (3, 3)))
        );
    }

    fn strike(actor: &str, target: &str) -> rpc::CombatOutcome {
        rpc::CombatOutcome {
            actor_name: actor.to_owned(),
            target_name: target.to_owned(),
            hit: true,
            ..Default::default()
        }
    }

    /// Three enemy turns arrive in one view: a move and an attack each, the second creature
    /// attacking twice. They split into turns in the server's order, each turn beginning at its
    /// creature's first outcome with the walk that brought it there, and a creature that did not
    /// move has no walk.
    #[test]
    fn a_view_with_three_enemy_turns_is_split_into_ordered_turns_with_their_walks() {
        let outcomes = [
            strike("Kobold 1", "Brakka"),
            strike("Kobold 2", "Sylvie"),
            strike("Kobold 2", "Sylvie"),
            strike("Kobold 3", "Brakka"),
        ];
        let refs: Vec<&rpc::CombatOutcome> = outcomes.iter().collect();
        let before = HashMap::from([
            ("Kobold 1".to_owned(), (5, 1)),
            ("Kobold 2".to_owned(), (6, 1)),
            ("Kobold 3".to_owned(), (3, 3)),
        ]);
        let now = HashMap::from([
            ("Kobold 1".to_owned(), (2, 4)),
            ("Kobold 2".to_owned(), (1, 4)),
            ("Kobold 3".to_owned(), (3, 3)),
        ]);

        let marks = turn_marks(&refs, &before, &now);

        assert_eq!(
            marks,
            [
                TurnMark {
                    turn_start: true,
                    walk: Some(((5, 1), (2, 4)))
                },
                TurnMark {
                    turn_start: true,
                    walk: Some(((6, 1), (1, 4)))
                },
                TurnMark {
                    turn_start: false,
                    walk: None
                },
                TurnMark {
                    turn_start: true,
                    walk: None
                },
            ]
        );
    }

    #[test]
    fn plate_actor_heads_are_validated_against_exact_current_cast_identity() {
        let mut scene = plate_scene(
            rpc::PlateState::Ready,
            &format!("/gameplay/imagery/{}", "a".repeat(64)),
            rpc::PlateSide::Left,
        );
        scene.cast.push(rpc::SceneActor {
            id: "bandit".into(),
            name: "Bandit".into(),
            ..Default::default()
        });
        let head = rpc::PlateActorHead {
            actor_id: "bandit".into(),
            bubble_side: rpc::PlateSide::Left as i32,
            head_x_permille: 800,
            head_y_permille: 300,
            head_width_permille: 100,
            head_height_permille: 120,
        };
        scene.plate_layout.as_mut().unwrap().actors = vec![
            head.clone(),
            rpc::PlateActorHead {
                actor_id: "departed".into(),
                ..head.clone()
            },
            rpc::PlateActorHead {
                actor_id: "bandit".into(),
                head_x_permille: 2000,
                ..head
            },
        ];
        let actors = plate_of(&scene).layout.unwrap().actors;
        assert_eq!(actors.len(), 1);
        assert_eq!(actors[0].actor_id, "bandit");
        assert_eq!(actors[0].head_x, 800);
    }

    fn plate_scene(state: rpc::PlateState, path: &str, side: rpc::PlateSide) -> rpc::GameplayScene {
        rpc::GameplayScene {
            live_plate: path.to_owned(),
            plate_state: state as i32,
            plate_layout: Some(rpc::PlateLayout {
                actors: Vec::new(),
                bubble_side: side as i32,
                head_x_permille: 640,
                head_y_permille: 370,
                head_width_permille: 120,
                head_height_permille: 180,
                heroes: vec![rpc::PlateHead {
                    hero_index: 1,
                    bubble_side: rpc::PlateSide::Right as i32,
                    head_x_permille: 150,
                    head_y_permille: 420,
                    head_width_permille: 100,
                    head_height_permille: 150,
                }],
            }),
            ..Default::default()
        }
    }

    #[test]
    fn a_ready_plate_needs_a_well_formed_path_and_carries_its_layout() {
        let path = format!("/gameplay/imagery/{}", "a".repeat(64));
        let ready = plate_of(&plate_scene(
            rpc::PlateState::Ready,
            &path,
            rpc::PlateSide::Left,
        ));
        assert_eq!(ready.state, PlateState::Ready);
        assert_eq!(ready.url.as_deref(), Some(path.as_str()));
        let layout = ready.layout.unwrap();
        assert_eq!(
            (layout.bubble_side, layout.head_x, layout.head_y),
            (crate::plate_layout::Side::Left, 640, 370)
        );
        assert_eq!(
            layout.heroes,
            vec![PlateHero {
                index: 1,
                bubble_side: crate::plate_layout::Side::Right,
                head_x: 150,
                head_y: 420,
                head_width: 100,
                head_height: 150,
            }]
        );
        let bad = plate_of(&plate_scene(
            rpc::PlateState::Ready,
            "https://elsewhere.example/a.png",
            rpc::PlateSide::Left,
        ));
        assert_eq!(
            (bad.state, bad.url, bad.layout),
            (PlateState::Failed, None, None)
        );
        let pending = plate_of(&plate_scene(
            rpc::PlateState::Pending,
            "",
            rpc::PlateSide::Right,
        ));
        assert_eq!((pending.state, pending.url), (PlateState::Pending, None));
        assert_eq!(
            plate_of(&rpc::GameplayScene::default()).state,
            PlateState::None
        );
        for state in [
            PlateState::None,
            PlateState::Pending,
            PlateState::Ready,
            PlateState::Failed,
        ] {
            assert!(!state.name().is_empty());
        }
    }

    #[test]
    fn a_scene_shows_its_cut_outs_only_when_no_plate_will_come() {
        for narration_over in [false, true] {
            for loaded in [false, true] {
                assert_eq!(
                    shot_of(PlateState::None, loaded, narration_over),
                    Shot::Stage
                );
                assert_eq!(
                    shot_of(PlateState::Failed, loaded, narration_over),
                    Shot::Stage
                );
                assert_eq!(
                    shot_of(PlateState::Pending, loaded, narration_over),
                    Shot::Establishing,
                    "a plate that is on its way is never drawn as cut-outs"
                );
                assert_eq!(
                    shot_of(PlateState::Unavailable, loaded, narration_over),
                    Shot::Establishing,
                    "a scene with no plate holds its establishing picture"
                );
            }
        }
    }

    #[test]
    fn a_ready_plate_is_cut_to_only_after_it_has_loaded_and_the_narration_is_over() {
        assert_eq!(shot_of(PlateState::Ready, false, false), Shot::Establishing);
        assert_eq!(shot_of(PlateState::Ready, true, false), Shot::Establishing);
        assert_eq!(shot_of(PlateState::Ready, false, true), Shot::Establishing);
        assert_eq!(shot_of(PlateState::Ready, true, true), Shot::Plate);
        for (shot, name) in [
            (Shot::Stage, "stage"),
            (Shot::Establishing, "establishing"),
            (Shot::Plate, "plate"),
        ] {
            assert_eq!(shot.name(), name);
        }
        for state in [
            PlateState::None,
            PlateState::Pending,
            PlateState::Ready,
            PlateState::Failed,
            PlateState::Unavailable,
        ] {
            assert_eq!(PlateState::from_name(state.name()), state);
        }
    }

    fn outcome(actor: &str, target: &str, die: u32, hit: bool) -> rpc::CombatOutcome {
        rpc::CombatOutcome {
            actor_name: actor.to_owned(),
            target_name: target.to_owned(),
            attack_die: die,
            hit,
            damage: if hit { 6 } else { 0 },
            ..Default::default()
        }
    }

    fn journey(
        phase: rpc::JourneyPhase,
        finished: bool,
        rolls: Vec<rpc::CombatOutcome>,
    ) -> rpc::JourneyView {
        rpc::JourneyView {
            phase: phase as i32,
            party: vec![
                rpc::PartyMemberView {
                    name: "Brakka".to_owned(),
                    ..Default::default()
                },
                rpc::PartyMemberView {
                    name: "Thrain".to_owned(),
                    ..Default::default()
                },
            ],
            combat: Some(rpc::CombatView {
                finished,
                active_actor: "Brakka".to_owned(),
                outcomes: rolls,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn view(journey: rpc::JourneyView) -> rpc::ViewMessage {
        rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Display(
                rpc::DisplayGameplayView {
                    journey: Some(journey),
                    ..Default::default()
                },
            )),
            ..Default::default()
        }
    }

    fn scene() -> rpc::GameplayScene {
        rpc::GameplayScene::default()
    }

    #[test]
    fn defeat_dialogue_does_not_make_the_encounter_a_victory() {
        let mut defeated = journey(rpc::JourneyPhase::Complete, true, vec![]);
        defeated.dialogue = "The bandit takes the packet and leaves.".to_owned();
        defeated.combat.as_mut().unwrap().participants = vec![rpc::PartyMemberView {
            name: "Bandit".to_owned(),
            side: 2,
            hit_points: 4,
            ..Default::default()
        }];

        assert!(!is_victory(&defeated));
    }

    #[test]
    fn first_view_presents_nothing() {
        let first = snapshot(
            &journey(
                rpc::JourneyPhase::Combat,
                false,
                vec![outcome("Brakka", "Perrin", 13, true)],
            ),
            &scene(),
        );
        assert_eq!(transitions(None, &first), Transitions::default());
    }

    #[test]
    fn every_outcome_a_view_adds_is_new_oldest_first() {
        let before = snapshot(
            &journey(
                rpc::JourneyPhase::Combat,
                false,
                vec![outcome("Brakka", "Weasel 1", 13, true)],
            ),
            &scene(),
        );
        let after = journey(
            rpc::JourneyPhase::Combat,
            false,
            vec![
                outcome("Brakka", "Weasel 1", 13, true),
                outcome("Weasel 1", "Brakka", 7, false),
                outcome("Weasel 2", "Brakka", 11, true),
            ],
        );
        let added: Vec<&str> = new_outcomes(Some(&before), &after)
            .into_iter()
            .map(|outcome| outcome.actor_name.as_str())
            .collect();
        assert_eq!(added, ["Weasel 1", "Weasel 2"]);
        assert_eq!(new_outcomes(None, &after).len(), 3);
    }

    #[test]
    fn a_new_roll_is_reported_once() {
        let before = snapshot(&journey(rpc::JourneyPhase::Combat, false, vec![]), &scene());
        let after = snapshot(
            &journey(
                rpc::JourneyPhase::Combat,
                false,
                vec![outcome("Brakka", "Perrin", 13, true)],
            ),
            &scene(),
        );
        assert!(transitions(Some(&before), &after).new_roll);
        assert!(!transitions(Some(&after), &after).new_roll);
    }

    #[test]
    fn a_killing_blow_reports_both_the_roll_and_the_finish() {
        let fight = snapshot(&journey(rpc::JourneyPhase::Combat, false, vec![]), &scene());
        let mut ended = journey(
            rpc::JourneyPhase::Complete,
            true,
            vec![outcome("Brakka", "Perrin", 20, true)],
        );
        ended.dialogue = "Thank you.".to_owned();
        ended.combat.as_mut().unwrap().participants = vec![rpc::PartyMemberView {
            name: "Perrin".to_owned(),
            side: 2,
            ..Default::default()
        }];
        let over = snapshot(&ended, &scene());
        let changes = transitions(Some(&fight), &over);
        assert!(changes.new_roll);
        assert_eq!(changes.finished, Some(true));
    }

    #[test]
    fn a_fight_that_ends_without_dialogue_is_a_defeat() {
        let fight = snapshot(&journey(rpc::JourneyPhase::Combat, false, vec![]), &scene());
        let over = snapshot(
            &journey(rpc::JourneyPhase::Complete, true, vec![]),
            &scene(),
        );
        assert_eq!(transitions(Some(&fight), &over).finished, Some(false));
    }

    #[test]
    fn a_short_closing_uses_the_largest_type() {
        assert_eq!(narrative_tier(""), "short");
        assert_eq!(narrative_tier(&"word ".repeat(90)[..450]), "short");
    }

    #[test]
    fn longer_closings_step_down_without_ever_clipping() {
        assert_eq!(narrative_tier(&"word ".repeat(91)[..451]), "full");
        assert_eq!(narrative_tier(&"word ".repeat(180)[..900]), "full");
        assert_eq!(narrative_tier(&"word ".repeat(181)[..901]), "long");
        assert_eq!(narrative_tier(&"word ".repeat(320)[..1600]), "long");
        assert_eq!(narrative_tier(&"word ".repeat(321)[..1601]), "epic");
    }

    #[test]
    fn tiers_count_characters_not_bytes() {
        let text = "é".repeat(450);
        assert_eq!(narrative_tier(&text), "short");
        assert_eq!(narrative_tier(&format!("{text}x")), "full");
    }

    #[test]
    fn the_inn_scene_keeping_the_finished_fight_is_not_a_second_finish() {
        let mut inn = scene();
        inn.destination = rpc::HarborDestination::HarborInn as i32;
        let aftermath = snapshot(
            &journey(rpc::JourneyPhase::Complete, true, vec![]),
            &scene(),
        );
        let later = snapshot(&journey(rpc::JourneyPhase::Complete, true, vec![]), &inn);
        assert_eq!(transitions(Some(&aftermath), &later).finished, None);
    }

    #[test]
    fn turn_changes_only_count_while_fighting() {
        let first = snapshot(&journey(rpc::JourneyPhase::Combat, false, vec![]), &scene());
        let mut other = journey(rpc::JourneyPhase::Combat, false, vec![]);
        other.combat.as_mut().unwrap().active_actor = "Thrain".to_owned();
        let second = snapshot(&other, &scene());
        assert!(transitions(Some(&first), &second).turn_changed);
        assert!(!transitions(Some(&first), &first).turn_changed);
    }

    fn server_line(
        sequence: u64,
        speaker: &str,
        role: rpc::TableRole,
        hero_index: u32,
        kind: rpc::TableKind,
        text: &str,
    ) -> rpc::TableLine {
        rpc::TableLine {
            actor_id: String::new(),
            sequence,
            speaker: speaker.to_owned(),
            role: role as i32,
            hero_index,
            kind: kind as i32,
            text: text.to_owned(),
        }
    }

    fn talk_view(sequence: u64, replying: &str, last: Option<rpc::TableLine>) -> rpc::ViewMessage {
        let mut shown = journey(rpc::JourneyPhase::Opening, false, vec![]);
        shown.presentation_sequence = sequence;
        shown.reply_speaker = replying.to_owned();
        shown.table_log = last.into_iter().collect();
        view(shown)
    }

    #[test]
    fn a_reply_in_progress_is_named_until_the_speaker_answers_the_newest_turn() {
        assert_eq!(replying_speaker(&talk_view(1, "", None)), None);
        let asked = server_line(
            1,
            "Brakka",
            rpc::TableRole::Hero,
            0,
            rpc::TableKind::Speech,
            "Who sent you?",
        );
        assert_eq!(
            replying_speaker(&talk_view(2, "Courier", Some(asked.clone()))).as_deref(),
            Some("Courier")
        );
        // The marker can outlive the answer: once Courier answers the newest hero turn, the
        // thinking indicator stops even though the marker is still set.
        let answered = server_line(
            2,
            "Courier",
            rpc::TableRole::Npc,
            0,
            rpc::TableKind::Speech,
            "Keep your voice down.",
        );
        let mut shown = journey(rpc::JourneyPhase::Opening, false, vec![]);
        shown.presentation_sequence = 3;
        shown.reply_speaker = "Courier".to_owned();
        shown.table_log = vec![asked.clone(), answered];
        assert_eq!(replying_speaker(&view(shown)), None);
        // An older answer, from before the newest hero line, does not clear a newer question.
        let older_answer = server_line(
            1,
            "Courier",
            rpc::TableRole::Npc,
            0,
            rpc::TableKind::Speech,
            "Keep your voice down.",
        );
        let newest_question = server_line(
            2,
            "Brakka",
            rpc::TableRole::Hero,
            0,
            rpc::TableKind::Speech,
            "Who sent you?",
        );
        let mut shown = journey(rpc::JourneyPhase::Opening, false, vec![]);
        shown.presentation_sequence = 4;
        shown.reply_speaker = "Courier".to_owned();
        shown.table_log = vec![older_answer, newest_question];
        assert_eq!(replying_speaker(&view(shown)).as_deref(), Some("Courier"));
        assert_eq!(replying_speaker(&talk_view(5, "", None)), None);
    }

    #[test]
    fn table_lines_read_the_server_log_with_its_own_sequence() {
        let mut shown = journey(rpc::JourneyPhase::Combat, false, vec![]);
        shown.table_log = vec![
            server_line(
                4,
                "Thrain",
                rpc::TableRole::Hero,
                1,
                rpc::TableKind::Speech,
                &"x".repeat(200),
            ),
            server_line(
                5,
                "Perrin",
                rpc::TableRole::Npc,
                0,
                rpc::TableKind::Action,
                "misses Thrain",
            ),
            server_line(
                6,
                "",
                rpc::TableRole::Narrator,
                0,
                rpc::TableKind::Speech,
                "  The door creaks.  ",
            ),
            server_line(
                7,
                "Nobody",
                rpc::TableRole::Unspecified,
                0,
                rpc::TableKind::Speech,
                "dropped",
            ),
        ];
        let lines = table_lines(&view(shown));
        assert_eq!(lines.len(), 3, "an empty scene line adds nothing");
        assert_eq!(lines[0].sequence, 4);
        assert_eq!(lines[0].role, TableRole::Hero(1));
        assert_eq!(lines[0].kind, TableKind::Speech);
        assert!(lines[0].short_text().chars().count() <= LINE_LIMIT);
        assert_eq!(lines[1].role, TableRole::Npc);
        assert_eq!(lines[1].kind, TableKind::Action);
        assert_eq!(lines[2].role, TableRole::Narrator);
        assert_eq!(lines[2].text, "The door creaks.");
    }

    #[test]
    fn the_scene_speaker_line_leads_the_log_once_per_scene() {
        let mut shown = journey(
            rpc::JourneyPhase::Combat,
            false,
            vec![outcome("Brakka", "Perrin", 13, true)],
        );
        shown.speaker = "Courier".to_owned();
        shown.dialogue = "Quick.".to_owned();
        shown.table_log = vec![server_line(
            2,
            "Brakka",
            rpc::TableRole::Hero,
            0,
            rpc::TableKind::Speech,
            "Easy now.",
        )];
        let lines = table_lines(&view(shown.clone()));
        assert_eq!(lines.len(), 2, "a roll is not a table line by itself");
        assert_eq!(lines[0].speaker, "Courier");
        assert_eq!(lines[0].role, TableRole::Npc);
        assert_eq!(lines[0].kind, TableKind::Speech);
        assert_eq!(lines[1].sequence, 2);
        // The same line in another scene is a new line; in the same scene, the same one.
        assert_eq!(table_lines(&view(shown.clone()))[0], lines[0]);
        let other_scene = rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Display(
                rpc::DisplayGameplayView {
                    journey: Some(shown),
                    scene: Some(rpc::GameplayScene {
                        title: "Elsewhere".to_owned(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        assert_ne!(table_lines(&other_scene)[0].sequence, lines[0].sequence);
    }

    #[test]
    fn a_line_with_no_named_speaker_is_the_narrators() {
        let mut shown = journey(rpc::JourneyPhase::Dialogue, false, vec![]);
        shown.dialogue = "A bell tolls.".to_owned();
        let lines = table_lines(&view(shown));
        assert_eq!(lines[0].role, TableRole::Narrator);
    }

    #[test]
    fn newest_scene_speech_uses_the_latest_reply_from_the_current_speaker() {
        let mut shown = journey(rpc::JourneyPhase::Dialogue, false, vec![]);
        shown.speaker = "Courier".to_owned();
        shown.table_log = vec![
            server_line(
                1,
                "Courier",
                rpc::TableRole::Npc,
                0,
                rpc::TableKind::Speech,
                "First answer.",
            ),
            server_line(
                2,
                "Bandit",
                rpc::TableRole::Npc,
                0,
                rpc::TableKind::Speech,
                "A different speaker.",
            ),
            server_line(
                3,
                "Brakka",
                rpc::TableRole::Hero,
                0,
                rpc::TableKind::Action,
                "asks again",
            ),
            server_line(
                4,
                "Courier",
                rpc::TableRole::Npc,
                0,
                rpc::TableKind::Speech,
                "Newest answer.",
            ),
        ];

        assert_eq!(
            newest_scene_speech(&shown),
            Some((4, "Courier", "Newest answer."))
        );
    }

    #[test]
    fn newest_scene_speech_ignores_a_reply_before_the_latest_hero_action() {
        let mut shown = journey(rpc::JourneyPhase::Dialogue, false, vec![]);
        shown.speaker = "Courier".to_owned();
        shown.table_log = vec![
            server_line(
                1,
                "Courier",
                rpc::TableRole::Npc,
                0,
                rpc::TableKind::Speech,
                "The previous scene's reply.",
            ),
            server_line(
                2,
                "Brakka",
                rpc::TableRole::Hero,
                0,
                rpc::TableKind::Action,
                "travels onward",
            ),
        ];

        assert_eq!(newest_scene_speech(&shown), None);
    }

    #[test]
    fn a_new_question_supersedes_the_previous_purchase_reply_until_answered() {
        let mut shown = journey(rpc::JourneyPhase::Dialogue, false, vec![]);
        shown.speaker = "Marek".to_owned();
        shown.dialogue = "Marek hands over the fine clothes.".to_owned();
        shown.reply_speaker = "Marek".to_owned();
        shown.table_log = vec![
            server_line(
                1,
                "Narrator",
                rpc::TableRole::Narrator,
                0,
                rpc::TableKind::Action,
                "Marek hands over the fine clothes for 200 copper.",
            ),
            server_line(
                2,
                "Marek",
                rpc::TableRole::Npc,
                0,
                rpc::TableKind::Speech,
                "The tailor folds the purchased clothes into paper.",
            ),
            server_line(
                3,
                "Brakka",
                rpc::TableRole::Hero,
                0,
                rpc::TableKind::Speech,
                "Could I inspect the other fine clothes?",
            ),
        ];

        assert_eq!(newest_scene_speech(&shown), None);
        assert!(scene_reply_pending(&shown, None));

        shown.table_log.push(server_line(
            4,
            "Marek",
            rpc::TableRole::Npc,
            0,
            rpc::TableKind::Speech,
            "Marek shows Brakka the remaining wool coats.",
        ));
        assert_eq!(
            newest_scene_speech(&shown),
            Some((4, "Marek", "Marek shows Brakka the remaining wool coats."))
        );
        assert!(!scene_reply_pending(&shown, None));
    }

    #[test]
    fn player_table_lines_omit_the_story_card_line_but_display_feed_keeps_it() {
        let mut shown = journey(rpc::JourneyPhase::Dialogue, false, vec![]);
        shown.speaker = "Courier".to_owned();
        shown.dialogue = "The authored opening.".to_owned();
        shown.table_log = vec![
            server_line(
                1,
                "Courier",
                rpc::TableRole::Npc,
                0,
                rpc::TableKind::Speech,
                "An earlier answer.",
            ),
            server_line(
                2,
                "Courier",
                rpc::TableRole::Npc,
                0,
                rpc::TableKind::Speech,
                "The newest answer.",
            ),
        ];
        let player = rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Player(
                rpc::PlayerGameplayView {
                    journey: Some(shown.clone()),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };

        let phone_lines = table_lines(&player);
        assert_eq!(phone_lines.len(), 1);
        assert_eq!(phone_lines[0].sequence, 1);
        assert_eq!(phone_lines[0].text, "An earlier answer.");
        assert_eq!(table_lines(&view(shown)).len(), 3);
    }

    #[test]
    fn player_table_lines_keep_speech_when_the_phone_has_no_story_card() {
        let mut shown = journey(rpc::JourneyPhase::Combat, false, vec![]);
        shown.speaker = "Courier".to_owned();
        shown.table_log = vec![server_line(
            1,
            "Courier",
            rpc::TableRole::Npc,
            0,
            rpc::TableKind::Speech,
            "Hold the gate!",
        )];
        let player = rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Player(
                rpc::PlayerGameplayView {
                    journey: Some(shown),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };

        let lines = table_lines(&player);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "Hold the gate!");
    }

    #[test]
    fn player_feed_keeps_every_public_line_across_a_finished_fight() {
        let mut shown = journey(rpc::JourneyPhase::Dialogue, false, vec![]);
        shown.combat.as_mut().unwrap().finished = true;
        shown.combat.as_mut().unwrap().participants = vec![rpc::PartyMemberView {
            side: 2,
            hit_points: 0,
            ..Default::default()
        }];
        shown.table_log = vec![
            server_line(
                1,
                "Brakka",
                rpc::TableRole::Hero,
                0,
                rpc::TableKind::Action,
                "swings at the weasel",
            ),
            server_line(
                2,
                "",
                rpc::TableRole::Narrator,
                0,
                rpc::TableKind::Action,
                "Brakka strikes the weasel",
            ),
            server_line(
                3,
                "Brakka",
                rpc::TableRole::Hero,
                0,
                rpc::TableKind::Action,
                "leads on to the bothy",
            ),
            server_line(
                4,
                "Calla",
                rpc::TableRole::Npc,
                0,
                rpc::TableKind::Speech,
                "Welcome, friends.",
            ),
        ];
        let fight_lines = shown.table_log.clone();
        let player = rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Player(
                rpc::PlayerGameplayView {
                    journey: Some(shown),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };

        let lines = table_lines(&player);
        let texts: Vec<&str> = lines.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "swings at the weasel",
                "Brakka strikes the weasel",
                "leads on to the bothy",
                "Welcome, friends.",
            ],
            "a finished fight stays in the phone feed, in server order"
        );

        let mut fighting = journey(rpc::JourneyPhase::Combat, false, vec![]);
        fighting.table_log = fight_lines.clone();
        let player_in_fight = rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Player(
                rpc::PlayerGameplayView {
                    journey: Some(fighting),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        assert_eq!(table_lines(&player_in_fight).len(), 4);

        let mut defeated = journey(rpc::JourneyPhase::Dialogue, true, vec![]);
        defeated.combat.as_mut().unwrap().participants = vec![rpc::PartyMemberView {
            side: 2,
            hit_points: 1,
            ..Default::default()
        }];
        defeated.table_log = fight_lines;
        let player_after_defeat = rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Player(
                rpc::PlayerGameplayView {
                    journey: Some(defeated),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        assert_eq!(
            table_lines(&player_after_defeat)[0].text,
            "swings at the weasel"
        );
    }

    #[test]
    fn player_table_lines_omit_the_authored_fallback_from_the_story_card() {
        let mut shown = journey(rpc::JourneyPhase::Dialogue, false, vec![]);
        shown.speaker = "Courier".to_owned();
        shown.dialogue = "The authored opening.".to_owned();
        let player = rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Player(
                rpc::PlayerGameplayView {
                    journey: Some(shown),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };

        assert!(table_lines(&player).is_empty());
    }

    #[test]
    fn action_text_describes_a_roll_by_its_result() {
        assert_eq!(
            action_text(&outcome("Brakka", "Perrin", 13, true)),
            "strikes at Perrin — hit for 6"
        );
        assert_eq!(
            action_text(&outcome("Perrin", "Thrain", 4, false)),
            "swings at Thrain and misses"
        );
    }

    #[test]
    fn table_lines_never_read_the_private_clue() {
        let player = rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Player(
                rpc::PlayerGameplayView {
                    private_clue: "secret".to_owned(),
                    narration: "secret".to_owned(),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        assert!(table_lines(&player).is_empty());
    }

    fn phone(journey: rpc::JourneyView) -> rpc::ViewMessage {
        rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Player(
                rpc::PlayerGameplayView {
                    journey: Some(journey),
                    ..Default::default()
                },
            )),
            ..Default::default()
        }
    }

    #[test]
    fn a_phone_feed_shows_every_hero_and_npc_line_in_order_on_each_phone() {
        // Arrange: Sigrun and Brakka talk to Jessa and to each other in one dialogue scene.
        let mut shown = journey(rpc::JourneyPhase::Dialogue, false, vec![]);
        shown.speaker = "Jessa".to_owned();
        shown.table_log = vec![
            server_line(
                29,
                "Sigrun",
                rpc::TableRole::Hero,
                1,
                rpc::TableKind::Speech,
                "Jessa, you seem to know Marek.",
            ),
            server_line(
                30,
                "Jessa",
                rpc::TableRole::Npc,
                0,
                rpc::TableKind::Speech,
                "Marek and I drink at the same tavern.",
            ),
            server_line(
                31,
                "Sigrun",
                rpc::TableRole::Hero,
                1,
                rpc::TableKind::Speech,
                "Would you put in a good word?",
            ),
            server_line(
                32,
                "Jessa",
                rpc::TableRole::Npc,
                0,
                rpc::TableKind::Speech,
                "I am off duty this afternoon.",
            ),
            server_line(
                33,
                "Brakka",
                rpc::TableRole::Hero,
                0,
                rpc::TableKind::Speech,
                "Sigrun, should we head to the market?",
            ),
            server_line(
                34,
                "Sigrun",
                rpc::TableRole::Hero,
                1,
                rpc::TableKind::Speech,
                "Brakka, yes.",
            ),
        ];
        // Act: both phones read the same accepted log.
        let brakka_phone = table_lines(&phone(shown.clone()));
        let sigrun_phone = table_lines(&phone(shown));
        // Assert: each phone carries all six lines, the other hero's included, in server order.
        let sequences: Vec<u64> = brakka_phone.iter().map(|line| line.sequence).collect();
        assert_eq!(sequences, [29, 30, 31, 32, 33, 34]);
        assert_eq!(brakka_phone, sigrun_phone);
        assert_eq!(brakka_phone[1].role, TableRole::Npc);
        assert_eq!(brakka_phone[4].role, TableRole::Hero(0));
    }

    #[test]
    fn a_repeated_words_line_is_shown_each_time_it_is_said() {
        let mut shown = journey(rpc::JourneyPhase::Dialogue, false, vec![]);
        shown.speaker = "Marek".to_owned();
        shown.table_log = vec![
            server_line(
                5,
                "Marek",
                rpc::TableRole::Npc,
                0,
                rpc::TableKind::Speech,
                "Yes.",
            ),
            server_line(
                6,
                "Brakka",
                rpc::TableRole::Hero,
                0,
                rpc::TableKind::Speech,
                "Good.",
            ),
            server_line(
                7,
                "Marek",
                rpc::TableRole::Npc,
                0,
                rpc::TableKind::Speech,
                "Yes.",
            ),
        ];
        let lines = table_lines(&phone(shown));
        let sequences: Vec<u64> = lines.iter().map(|line| line.sequence).collect();
        assert_eq!(
            sequences,
            [5, 6],
            "the newest reply is on the story card, not the feed"
        );
    }

    #[test]
    fn the_scene_greeting_is_shown_once_when_the_server_log_repeats_it() {
        const GREETING: &str =
            "Welcome. Have a look around; let me know if you want to try anything on.";
        // Arrange: the story card's authored greeting is also the server's logged line.
        let mut shown = journey(rpc::JourneyPhase::Combat, false, vec![]);
        shown.speaker = "Marek".to_owned();
        shown.dialogue = GREETING.to_owned();
        shown.table_log = vec![
            server_line(
                5,
                "Marek",
                rpc::TableRole::Npc,
                0,
                rpc::TableKind::Speech,
                GREETING,
            ),
            server_line(
                6,
                "Brakka",
                rpc::TableRole::Hero,
                0,
                rpc::TableKind::Speech,
                "What do you sell?",
            ),
        ];
        // Act and assert: a phone without a story card shows the greeting once, as the server's line.
        let lines = table_lines(&phone(shown.clone()));
        let sequences: Vec<u64> = lines.iter().map(|line| line.sequence).collect();
        assert_eq!(sequences, [5, 6]);
        assert!(!lines[0].scripted);
        // The display keeps its authored line as before.
        assert_eq!(table_lines(&view(shown)).len(), 3);
    }

    #[test]
    fn the_authored_greeting_comes_before_the_server_lines_when_the_log_lacks_it() {
        const GREETING: &str =
            "Welcome. Have a look around; let me know if you want to try anything on.";
        // The server's log has moved on: the buyback follow-up is newer than the scene's greeting
        // and the log does not repeat the greeting.
        let mut shown = journey(rpc::JourneyPhase::Combat, false, vec![]);
        shown.speaker = "Marek".to_owned();
        shown.dialogue = GREETING.to_owned();
        shown.table_log = vec![
            server_line(
                8,
                "Brakka",
                rpc::TableRole::Hero,
                0,
                rpc::TableKind::Speech,
                "Buy these back for 1 gold?",
            ),
            server_line(
                9,
                "Marek",
                rpc::TableRole::Npc,
                0,
                rpc::TableKind::Speech,
                "Let me look at them.",
            ),
        ];
        let lines = table_lines(&view(shown));
        let texts: Vec<&str> = lines.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                GREETING,
                "Buy these back for 1 gold?",
                "Let me look at them."
            ],
            "the authored greeting opens the feed, before the first server line"
        );
        assert!(lines[0].scripted);
    }

    #[test]
    fn the_scene_greeting_is_not_shown_twice_on_a_story_card_phone() {
        const GREETING: &str =
            "Welcome. Have a look around; let me know if you want to try anything on.";
        let mut shown = journey(rpc::JourneyPhase::Dialogue, false, vec![]);
        shown.speaker = "Marek".to_owned();
        shown.dialogue = GREETING.to_owned();
        shown.table_log = vec![server_line(
            5,
            "Marek",
            rpc::TableRole::Npc,
            0,
            rpc::TableKind::Speech,
            GREETING,
        )];
        assert!(
            table_lines(&phone(shown)).is_empty(),
            "the story card already shows this newest reply"
        );
    }

    fn actor(id: &str, name: &str, speaking: bool) -> rpc::SceneActor {
        rpc::SceneActor {
            id: id.to_owned(),
            name: name.to_owned(),
            role: rpc::SceneRole::Patron as i32,
            sprite: format!("assets/ui/generated/sprite-{id}"),
            portrait: format!("assets/ui/generated/portrait-{id}"),
            speaking,
            live_sprite: String::new(),
        }
    }

    fn view_with_cast(journey: rpc::JourneyView, cast: Vec<rpc::SceneActor>) -> rpc::ViewMessage {
        rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Display(
                rpc::DisplayGameplayView {
                    journey: Some(journey),
                    scene: Some(rpc::GameplayScene {
                        cast,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )),
            ..Default::default()
        }
    }

    #[test]
    fn the_cast_is_the_ready_heroes_then_the_servers_cast_with_its_art_and_speaker() {
        let mut talking = journey(rpc::JourneyPhase::Dialogue, false, vec![]);
        talking.combat = None;
        talking.party[0].character_ready = true;
        talking.party[0].maximum_hit_points = 13;
        talking.party[0].hit_points = 13;
        let cast = scene_cast(&view_with_cast(
            talking,
            vec![
                actor("patron", "Fenn", true),
                actor("villain", "Perrin", false),
            ],
        ));
        assert_eq!(cast.len(), 3);
        assert_eq!(cast[0].slot, ActorSlot::Hero(0));
        assert_eq!(cast[0].hit_points, Some((13, 13)));
        assert_eq!(cast[0].sprite, "/assets/ui/generated/sprite-hero-1");
        assert_eq!(cast[1].slot, ActorSlot::Npc(0));
        assert_eq!(cast[1].name, "Fenn");
        // A painted stem resolves to nothing under the art contract: the stage then shows the
        // live photograph the server made, or nothing.
        assert_eq!(cast[1].sprite, "");
        assert_eq!(cast[1].portrait, "");
        assert!(cast[1].speaking);
        assert_eq!(cast[2].slot, ActorSlot::Npc(1));
        assert!(!cast[2].speaking);
    }

    #[test]
    fn a_fight_takes_the_foes_hit_points_and_turn_from_the_combat_by_name() {
        let mut fight = journey(rpc::JourneyPhase::Combat, false, vec![]);
        fight.combat.as_mut().unwrap().participants = vec![
            rpc::PartyMemberView {
                name: "Brakka".to_owned(),
                ..Default::default()
            },
            rpc::PartyMemberView {
                name: "Thrain".to_owned(),
                ..Default::default()
            },
            rpc::PartyMemberView {
                name: "Perrin".to_owned(),
                active_turn: true,
                maximum_hit_points: 11,
                hit_points: 5,
                ..Default::default()
            },
        ];
        let cast = scene_cast(&view_with_cast(
            fight,
            vec![
                actor("patron", "Fenn", true),
                actor("villain", "Perrin", false),
            ],
        ));
        let patron = &cast[0];
        assert_eq!(patron.name, "Fenn");
        assert!(!patron.speaking, "in a fight only the active turn is lit");
        let foe = cast.last().unwrap();
        assert_eq!(foe.name, "Perrin");
        assert_eq!(foe.hit_points, Some((5, 11)));
        assert!(foe.speaking);
    }

    #[test]
    fn the_stage_holds_at_most_three_cast_members_and_art_outside_assets_is_dropped() {
        let mut crowd: Vec<rpc::SceneActor> = (0..5)
            .map(|number| actor(&format!("a{number}"), &format!("Name {number}"), false))
            .collect();
        crowd[0].sprite = "https://example.com/x".to_owned();
        let mut talking = journey(rpc::JourneyPhase::Dialogue, false, vec![]);
        talking.combat = None;
        let cast = scene_cast(&view_with_cast(talking, crowd));
        assert_eq!(cast.len(), MAX_NPCS);
        assert_eq!(cast[0].sprite, "");
    }

    #[test]
    fn the_story_beginning_is_the_room_giving_way_to_the_opening() {
        let room = snapshot(&journey(rpc::JourneyPhase::Room, false, vec![]), &scene());
        let opening = snapshot(
            &journey(rpc::JourneyPhase::Opening, false, vec![]),
            &scene(),
        );
        let dialogue = snapshot(
            &journey(rpc::JourneyPhase::Dialogue, false, vec![]),
            &scene(),
        );
        assert!(transitions(Some(&room), &opening).story_begun);
        assert!(!transitions(Some(&opening), &dialogue).story_begun);
        assert!(!transitions(Some(&opening), &opening).story_begun);
        assert!(
            !transitions(None, &opening).story_begun,
            "a reload shows no card"
        );
    }

    #[test]
    fn the_title_card_omits_the_sentence_that_is_narrated_in_full() {
        let opening = rpc::GameplayScene {
            title: "A Parcel for Nolan".to_owned(),
            place: "The Salt Quay".to_owned(),
            ..Default::default()
        };
        let narration = "Gulls mutter in the rigging of sleeping boats. A merchant hurries out.";
        let card = title_card(&opening, narration);
        assert_eq!(card.kind, CardKind::Title);
        assert_eq!(card.headline, "A Parcel for Nolan");
        assert!(card.subline.is_empty());
        assert_eq!(card.place, "The Salt Quay");
        assert!(card.heroes.is_empty());
    }

    #[test]
    fn the_closing_card_names_the_ready_heroes_and_the_outcome() {
        let mut ended = journey(rpc::JourneyPhase::Complete, true, vec![]);
        ended.party[0].character_ready = true;
        ended.party[1].character_ready = true;
        let closing = rpc::GameplayScene {
            title: "The Bell Remembers".to_owned(),
            ..Default::default()
        };
        let won = closing_card(&ended, &closing, "The bandit yields. Peace returns.", true);
        assert_eq!(won.kind, CardKind::Victory);
        assert_eq!(won.headline, "The Bell Remembers");
        assert_eq!(won.subline, "The bandit yields.");
        let names: Vec<&str> = won.heroes.iter().map(|hero| hero.name.as_str()).collect();
        assert_eq!(names, ["Brakka", "Thrain"]);
        assert_eq!(
            won.heroes[1].portrait,
            "/assets/ui/generated/figure-hero-2-documentary-photo"
        );
        let lost = closing_card(&ended, &closing, "Darkness.", false);
        assert_eq!(lost.kind, CardKind::Defeat);
        assert_ne!(lost.kicker, won.kicker);
        ended.party[1].character_ready = false;
        assert_eq!(closing_card(&ended, &closing, "x", true).heroes.len(), 1);
    }

    #[test]
    fn first_sentence_stops_at_the_sentence_or_the_last_clause_break_within_the_limit() {
        assert_eq!(first_sentence("One. Two."), "One.");
        assert_eq!(first_sentence("No full stop"), "No full stop");
        let long = format!(
            "{}, and then something else entirely that runs on and on and on and on and on.",
            "A".repeat(60)
        );
        let cut = first_sentence(&long);
        assert!(cut.chars().count() <= LEDE_CHARACTERS + 1, "{cut}");
        assert!(cut.ends_with('.'));
    }

    #[test]
    fn scene_family_follows_the_asset_name() {
        assert_eq!(SceneFamily::Interior.name(), "interior");
        assert_eq!(SceneFamily::Exterior.name(), "exterior");

        // The server declares it; a name, an art reference or a place never decides it.
        let shop = rpc::GameplayScene {
            scene_asset: "live-room-photo".to_owned(),
            place: "The Quay".to_owned(),
            interior: true,
            ..Default::default()
        };
        assert_eq!(scene_family(&shop), SceneFamily::Interior);
        let quay = rpc::GameplayScene {
            scene_asset: "live-room-photo".to_owned(),
            place: "Marek’s Cloth Shop".to_owned(),
            interior: false,
            ..Default::default()
        };
        assert_eq!(scene_family(&quay), SceneFamily::Exterior);
    }
}

#[cfg(test)]
mod scene_art_admission {
    use super::*;

    /// A view is shown whatever its base scene art is. Every reference the art contract lets the
    /// server send is admitted: each role token, a reviewed photograph by stem and by path, a
    /// live image path, the reviewed harbour plate, and no art at all.
    #[test]
    fn every_art_contract_reference_a_view_can_carry_is_admitted() {
        let live = format!("{}{}", art_contract::LIVE_IMAGERY_PREFIX, "a".repeat(64));
        let mut references = vec![
            "",
            art_contract::ROOM_PHOTO,
            art_contract::CANONICAL_PHOTO,
            art_contract::NEUTRAL,
            art_contract::HARBOR_PHOTO,
            "assets/ui/scenes/mara-harbor-v4.png",
            "assets/concept-art/scene-tavern-barkeep-talk-rain.webp",
            live.as_str(),
        ];
        let reviewed_paths: Vec<String> = art_contract::REVIEWED_PHOTOS
            .iter()
            .map(|stem| format!("assets/ui/generated/{stem}.png"))
            .collect();
        references.extend(reviewed_paths.iter().map(String::as_str));
        references.extend(art_contract::REVIEWED_PHOTOS);
        for reference in references {
            assert!(
                scene_art_admitted(reference),
                "the art contract's own reference {reference:?} must be admitted"
            );
        }
    }

    #[test]
    fn only_a_deleted_painting_is_not_admitted() {
        assert!(!scene_art_admitted(
            "assets/ui/generated/scene-harbor-painted-concept.png"
        ));
    }

    #[test]
    fn role_tokens_draw_nothing_themselves_so_the_neutral_stage_stands() {
        for token in [
            art_contract::ROOM_PHOTO,
            art_contract::CANONICAL_PHOTO,
            art_contract::NEUTRAL,
        ] {
            assert_eq!(asset_url(token), None);
            assert_eq!(
                place_picture(art_contract::Role::LobbyArt, token, "", ""),
                None
            );
        }
    }

    /// The display walks every element it draws, SVG icons included. `className` is an
    /// SVGAnimatedString on SVG, which the string binding cannot take: it aborted the TV client
    /// in a panic loop. The client reads the `class` attribute instead.
    #[test]
    fn the_client_never_reads_class_name_from_an_element_it_walks() {
        let source = include_str!("gameplay_browser.rs");
        assert!(!source.contains(".class_name()"));
    }
}
