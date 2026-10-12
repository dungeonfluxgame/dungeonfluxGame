//! Presentation-only reading of accepted views: what changed since the last view, and which
//! short "table lines" the room can follow. Nothing here decides a game outcome or sends
//! anything; every input is a field the server already put in the audience's own view, so
//! the browser can animate and voice a change without owning any rule.
use df_protocol::common as rpc;

/// Longest text a compact table line shows, in characters.
const LINE_LIMIT: usize = 90;
/// The fixed class of the two playable heroes; the server offers no other.
pub(crate) const HERO_ROLE: &str = "Dwarf Fighter";
const DM_NAME: &str = "Dungeon Master";
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

/// "Victory" when the server projects the courier's closing line, which only the won
/// encounter has. The aftermath screen uses this same rule for its heading.
pub(crate) fn is_victory(journey: &rpc::JourneyView) -> bool {
    !journey.dialogue.is_empty()
}

pub(crate) fn snapshot(journey: &rpc::JourneyView, scene: &rpc::GameplayScene) -> CombatSnapshot {
    let Some(combat) = journey.combat.as_ref() else {
        return CombatSnapshot {
            stage: CombatStage::Absent,
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
        rolls: combat.outcomes.iter().map(fingerprint).collect(),
        active: if combat.finished {
            String::new()
        } else {
            combat.active_actor.clone()
        },
    }
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

pub(crate) fn scene_family(scene_asset: &str) -> SceneFamily {
    const INDOOR: [&str; 9] = [
        "inn", "tavern", "interior", "hall", "cellar", "crypt", "temple", "shop", "library",
    ];
    let name = scene_asset.to_ascii_lowercase();
    if INDOOR.iter().any(|word| name.contains(word)) {
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
    /// The one non-player character in focus: the speaker, or the foe in combat.
    Npc,
}

/// One character standing in the scene.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StageActor {
    pub(crate) slot: ActorSlot,
    pub(crate) name: String,
    pub(crate) role: String,
    /// Served path of the small portrait; the cut-out and 2:3 portrait names derive from it.
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
pub(crate) fn asset_url(path: &str) -> Option<String> {
    let path = path.trim_start_matches('/');
    (path.starts_with("assets/")
        && path
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'/' | b'-' | b'.' | b'_')))
    .then(|| format!("/{path}"))
}

// PLAY-L01: a server-generated image path: `/gameplay/imagery/` and a 64-digit lowercase
// hexadecimal key, nothing else. Only these reach CSS or `src` besides local assets.
pub(crate) fn imagery_url(path: &str) -> Option<String> {
    let key = path.strip_prefix("/gameplay/imagery/")?;
    let plain = key.len() == 64 && key.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'));
    plain.then(|| path.to_owned())
}

// PLAY-L01: the expression variant the scene's cast gives the named character, if any.
fn live_sprite_of(scene: &rpc::GameplayScene, name: &str) -> Option<String> {
    scene
        .cast
        .iter()
        .find(|actor| actor.name == name)
        .and_then(|actor| imagery_url(&actor.live_sprite))
}

/// Generated portrait stem of a speaker the art pack names by role.
pub(crate) fn known_portrait_stem(name: &str) -> Option<&'static str> {
    match name {
        "Courier" => Some("portrait-courier"),
        "Bandit" => Some("portrait-bandit"),
        _ => None,
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
/// are ready, the speaker the view names, and the foe while a fight is on.
///
/// SERVER CAST SWITCH POINT: when the server adds a per-scene cast to the view, replace the
/// body of this function with a mapping of that list. A character that stays after the
/// speaker line moves on is the consumer's job (`live_stage` remembers the last non-player
/// character until the scene changes); the function itself only reports this view.
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
            name: member.name.clone(),
            role: HERO_ROLE.to_owned(),
            portrait: format!("{GENERATED}/portrait-hero-{}", index + 1),
            hit_points: hit_points(member),
            speaking: fighting && member.active_turn,
            down: is_down(member),
            live_sprite: None,
        })
        .collect();
    let speaker_portrait = asset_url(&scene.speaker_portrait);
    if fighting {
        let foe = journey
            .combat
            .as_ref()
            .and_then(|combat| combat.participants.get(journey.party.len()));
        if let Some(foe) = foe {
            cast.push(StageActor {
                slot: ActorSlot::Npc,
                name: foe.name.clone(),
                role: String::new(),
                portrait: speaker_portrait
                    .unwrap_or_else(|| format!("{GENERATED}/portrait-bandit")),
                hit_points: hit_points(foe),
                speaking: foe.active_turn,
                down: foe.maximum_hit_points > 0 && foe.hit_points == 0,
                live_sprite: None,
            });
        }
    } else if !journey.speaker.is_empty() && !journey.dialogue.is_empty() {
        let portrait = speaker_portrait.or_else(|| {
            known_portrait_stem(&journey.speaker).map(|stem| format!("{GENERATED}/{stem}"))
        });
        if let Some(portrait) = portrait {
            cast.push(StageActor {
                slot: ActorSlot::Npc,
                name: journey.speaker.clone(),
                role: String::new(),
                portrait,
                hit_points: None,
                speaking: true,
                down: false,
                live_sprite: live_sprite_of(scene, &journey.speaker),
            });
        }
    }
    cast
}

/// Who a table line is attributed to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TableRole {
    /// The party member at this position of the view's party list.
    Hero(usize),
    Npc,
    #[allow(dead_code, reason = "reserved for the server feed's narrator lines")]
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
    /// Stable identity of the line. Consumers show a line once per sequence. Derived lines
    /// use a content hash; the server feed will carry its own increasing sequence.
    pub(crate) sequence: u64,
    pub(crate) speaker: String,
    pub(crate) role: TableRole,
    pub(crate) kind: TableKind,
    /// The whole line. A compact surface shows [`TableLine::short_text`].
    pub(crate) text: String,
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

fn roll_line(party: &[rpc::PartyMemberView], outcome: &rpc::CombatOutcome) -> TableLine {
    let role = party
        .iter()
        .position(|member| member.name == outcome.actor_name)
        .map_or(TableRole::Npc, TableRole::Hero);
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
    TableLine {
        sequence: fingerprint(outcome),
        speaker: outcome.actor_name.clone(),
        role,
        kind: TableKind::Action,
        text,
    }
}

/// The table lines one audience view justifies, oldest first.
///
/// Today these are derived from the view itself: the scene speaker's current line is an NPC
/// speech line and every roll of the combat window is an action line, attributed only by
/// names the view already carries. A choice that cannot be attributed produces no line.
/// The private clue is never read.
///
/// SERVER FEED SWITCH POINT: when the server adds a public table feed to the view, replace
/// the body of this function with a mapping of that feed (keeping its own `sequence`); the
/// consumers in `live_motion` depend only on this signature.
pub(crate) fn table_lines(view: &rpc::ViewMessage) -> Vec<TableLine> {
    let journey = match &view.audience {
        Some(rpc::view_message::Audience::Display(display)) => display.journey.as_ref(),
        Some(rpc::view_message::Audience::Player(player)) => player.journey.as_ref(),
        None => None,
    };
    let Some(journey) = journey else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    if let Some(combat) = journey.combat.as_ref() {
        lines.extend(
            combat
                .outcomes
                .iter()
                .map(|outcome| roll_line(&journey.party, outcome)),
        );
    }
    if !journey.dialogue.is_empty() {
        let speaker = if journey.speaker.is_empty() {
            DM_NAME
        } else {
            journey.speaker.as_str()
        };
        lines.push(TableLine {
            sequence: fnv(&[speaker.as_bytes(), journey.dialogue.as_bytes()]),
            speaker: speaker.to_owned(),
            role: if journey.speaker.is_empty() {
                TableRole::Narrator
            } else {
                TableRole::Npc
            },
            kind: TableKind::Speech,
            text: journey.dialogue.trim().to_owned(),
        });
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn table_lines_attribute_rolls_by_party_name_and_clamp_text() {
        let mut shown = journey(
            rpc::JourneyPhase::Combat,
            false,
            vec![
                outcome("Brakka", "Perrin", 13, true),
                outcome("Perrin", "Thrain", 4, false),
            ],
        );
        shown.speaker = "Courier".to_owned();
        shown.dialogue = "x".repeat(200);
        let lines = table_lines(&view(shown));
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].role, TableRole::Hero(0));
        assert_eq!(lines[0].kind, TableKind::Action);
        assert_eq!(lines[0].text, "strikes at Perrin — hit for 6");
        assert_eq!(lines[1].role, TableRole::Npc);
        assert_eq!(lines[1].text, "swings at Thrain and misses");
        assert_eq!(lines[2].kind, TableKind::Speech);
        assert!(lines[2].short_text().chars().count() <= LINE_LIMIT);
        assert_ne!(lines[0].sequence, lines[1].sequence);
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

    fn view_with_scene(journey: rpc::JourneyView, speaker_portrait: &str) -> rpc::ViewMessage {
        rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Display(
                rpc::DisplayGameplayView {
                    journey: Some(journey),
                    scene: Some(rpc::GameplayScene {
                        speaker_portrait: speaker_portrait.to_owned(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )),
            ..Default::default()
        }
    }

    #[test]
    fn the_cast_has_ready_heroes_the_speaker_and_the_foe() {
        let mut talking = journey(rpc::JourneyPhase::Dialogue, false, vec![]);
        talking.combat = None;
        talking.party[0].character_ready = true;
        talking.party[0].maximum_hit_points = 13;
        talking.party[0].hit_points = 13;
        talking.speaker = "Courier".to_owned();
        talking.dialogue = "Quickly now.".to_owned();
        let cast = scene_cast(&view_with_scene(
            talking,
            "assets/ui/generated/portrait-courier",
        ));
        assert_eq!(cast.len(), 2);
        assert_eq!(cast[0].slot, ActorSlot::Hero(0));
        assert_eq!(cast[0].hit_points, Some((13, 13)));
        assert_eq!(cast[1].slot, ActorSlot::Npc);
        assert_eq!(cast[1].portrait, "/assets/ui/generated/portrait-courier");
        assert!(cast[1].speaking);

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
        let cast = scene_cast(&view_with_scene(fight, ""));
        let foe = cast.last().unwrap();
        assert_eq!(foe.name, "Perrin");
        assert_eq!(foe.portrait, "/assets/ui/generated/portrait-bandit");
        assert!(foe.speaking);
    }

    #[test]
    fn a_speaker_without_art_is_not_drawn() {
        let mut talking = journey(rpc::JourneyPhase::Dialogue, false, vec![]);
        talking.combat = None;
        talking.speaker = "Stranger".to_owned();
        talking.dialogue = "Hello.".to_owned();
        assert!(scene_cast(&view_with_scene(talking, "")).is_empty());
    }

    #[test]
    fn scene_family_follows_the_asset_name() {
        assert_eq!(SceneFamily::Interior.name(), "interior");
        assert_eq!(SceneFamily::Exterior.name(), "exterior");

        assert_eq!(
            scene_family("/assets/ui/generated/scene-harbor-inn"),
            SceneFamily::Interior
        );
        assert_eq!(
            scene_family("/assets/ui/generated/scene-courier-road"),
            SceneFamily::Exterior
        );
    }
}
