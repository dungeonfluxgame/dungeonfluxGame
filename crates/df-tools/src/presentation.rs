//! Presentation-only reading of accepted views: what changed since the last view, and which
//! short "table lines" the room can follow. Nothing here decides a game outcome or sends
//! anything; every input is a field the server already put in the audience's own view, so
//! the browser can animate and voice a change without owning any rule.
use df_protocol::common as rpc;

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

/// "Victory" when the server projects the courier's closing line, which only the won
/// encounter has. The aftermath screen uses this same rule for its heading.
pub(crate) fn is_victory(journey: &rpc::JourneyView) -> bool {
    !journey.dialogue.is_empty()
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

/// The card shown when the story begins: the opening scene's title, one evocative line and
/// the place.
pub(crate) fn title_card(scene: &rpc::GameplayScene, narration: &str) -> Card {
    Card {
        kind: CardKind::Title,
        kicker: "A tale begins".to_owned(),
        headline: scene.title.clone(),
        subline: first_sentence(narration),
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
                portrait: format!("{GENERATED}/portrait-hero-{}", index + 1),
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
    /// A non-player character of the scene's cast, by position in the server's cast list.
    Npc(usize),
}

/// How many non-player figures the stage can hold; a longer cast list shows its first ones.
pub(crate) const MAX_NPCS: usize = 3;

/// One character standing in the scene.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StageActor {
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
            name: member.name.clone(),
            role: HERO_ROLE.to_owned(),
            sprite: format!("{GENERATED}/sprite-hero-{}", index + 1),
            portrait: format!("{GENERATED}/portrait-hero-{}", index + 1),
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
    let (speaker, role) = if journey.speaker.is_empty() {
        ("", TableRole::Narrator)
    } else {
        (journey.speaker.as_str(), TableRole::Npc)
    };
    Some(TableLine {
        sequence: fnv(&[title.as_bytes(), speaker.as_bytes(), text.as_bytes()]),
        speaker: speaker.to_owned(),
        role,
        kind: TableKind::Speech,
        text: text.to_owned(),
        scripted: true,
    })
}

/// Who is writing a reply to a hero's typed words right now, as the server says, unless that
/// speaker's answer is already the last thing logged (the mark can outlive the line by a moment).
pub(crate) fn replying_speaker(view: &rpc::ViewMessage) -> Option<String> {
    let journey = match view.audience.as_ref()? {
        rpc::view_message::Audience::Display(display) => display.journey.as_ref(),
        rpc::view_message::Audience::Player(player) => player.journey.as_ref(),
    }?;
    let speaker = journey.reply_speaker.trim();
    if speaker.is_empty() {
        return None;
    }
    let answered = journey.table_log.last().is_some_and(|last| {
        rpc::TableRole::try_from(last.role) == Ok(rpc::TableRole::Npc)
            && rpc::TableKind::try_from(last.kind) == Ok(rpc::TableKind::Speech)
            && last.speaker.trim() == speaker
    });
    (!answered).then(|| speaker.to_owned())
}

/// The table lines of one audience view, oldest first: the scene's own speaker line, then the
/// server's public table log.
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
    scene_line(journey, scene)
        .into_iter()
        .chain(logged_lines(journey))
        .collect()
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
            speaker: line.speaker.clone(),
            role,
            kind,
            text: line.text.trim().to_owned(),
            scripted: false,
        })
    })
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

    fn server_line(
        sequence: u64,
        speaker: &str,
        role: rpc::TableRole,
        hero_index: u32,
        kind: rpc::TableKind,
        text: &str,
    ) -> rpc::TableLine {
        rpc::TableLine {
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
    fn a_reply_in_progress_is_named_until_that_speaker_has_answered() {
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
            replying_speaker(&talk_view(2, "Courier", Some(asked))).as_deref(),
            Some("Courier")
        );
        let answered = server_line(
            2,
            "Courier",
            rpc::TableRole::Npc,
            0,
            rpc::TableKind::Speech,
            "Keep your voice down.",
        );
        assert_eq!(
            replying_speaker(&talk_view(3, "Courier", Some(answered))),
            None
        );
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
        assert_eq!(cast[1].sprite, "/assets/ui/generated/sprite-patron");
        assert_eq!(cast[1].portrait, "/assets/ui/generated/portrait-patron");
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
    fn the_title_card_uses_only_what_the_view_carries() {
        let opening = rpc::GameplayScene {
            title: "A Parcel for Nolan".to_owned(),
            place: "The Salt Quay".to_owned(),
            ..Default::default()
        };
        let card = title_card(
            &opening,
            "Gulls mutter in the rigging of sleeping boats. A merchant hurries out.",
        );
        assert_eq!(card.kind, CardKind::Title);
        assert_eq!(card.headline, "A Parcel for Nolan");
        assert_eq!(
            card.subline,
            "Gulls mutter in the rigging of sleeping boats."
        );
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
            "/assets/ui/generated/portrait-hero-2"
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
