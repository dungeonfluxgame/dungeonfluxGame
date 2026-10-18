//! Fights that begin inside a freeplay scene.
//!
//! A hero's SCENE_ASSAULT turns a scene person with a stat block into a real 2024 tactical fight
//! on the pack's battle scene. The fight is an encounter inside the freeplay beat: the beat never
//! changes, the derived phase is Combat while the encounter is open, and when either side has no
//! conscious combatant the encounter closes and play returns to the same freeplay beat. There is
//! no victory or defeat beat, no experience and no payment.
//!
//! Who fights is decided once, when the fight starts (the bystander rule, `stance`). The rule
//! needs the people's attitudes toward the aggressor, which live in the social ledger and not in
//! the checkpoint, so the caller that holds the ledger passes the outcome as the `scene-roster`
//! choice and staging only validates it. Without a roster the attitudes are assumed Indifferent.
//!
//! Scene people become world entities and `NpcState` only when a fight needs them. Their entity
//! ids and identity labels derive from the scene person id, so a replay builds the same ones.

use super::*;
use crate::gameplay::adventure::dossier::Attitude;
use crate::gameplay::social::SocialLedger;
use df_rules::tactical as tactical_rules;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tactical_rules::{Battle, Side};

/// The objective a scene assault carries (the courier's scene assault shares it).
pub(super) const OBJECTIVE: &str = "scene-npc-assault";
/// The submitted action entry of a scene assault.
pub(super) const ACTION: &str = "scene-assault";
/// The one world-entity definition every scene person's entity carries.
pub(super) const PERSON: &str = "scene-person";
/// The choice that names the person the hero attacks.
pub(in crate::gameplay) const TARGET_GROUP: &str = "scene-target";
/// The choice that names who joins, flees or stays out, as `join:id/flee:id/out:id`.
pub(in crate::gameplay) const ROSTER_GROUP: &str = "scene-roster";
/// The stat block of people who never take up arms.
const NON_COMBATANT: &str = "commoner";
/// The stat block of people who keep the peace and step in against an aggressor.
const PEACEKEEPER: &str = "guard";
/// A fight holds its target and up to three people who join it: four enemy spawn cells.
const MAXIMUM_FOES: usize = tactical::MAXIMUM_ENEMIES;
/// Resource that marks a person (or hero) who died, since 0 hit points alone does not say.
const DEAD: &str = "dead";

fn freeplay() -> bool {
    pack::with_content(|content| content.is_freeplay())
}

/// Every cast member of the pack's scenes once, by id.
fn pack_cast() -> Vec<pack::SceneCastMember> {
    pack::with_content(|content| {
        let mut people: Vec<pack::SceneCastMember> = Vec::new();
        for member in content.scenes.iter().flat_map(|scene| &scene.cast) {
            if !people.iter().any(|known| known.id == member.id) {
                people.push(member.clone());
            }
        }
        people
    })
}

/// Canonical created people, including people in places the party has since left. Their combat
/// resources must remain admissible after travel and recovery, without a live scene registry.
fn created_cast(state: &GameState) -> Result<Vec<pack::SceneCastMember>, RepositoryError> {
    use crate::gameplay::place;
    let places = place::created(state)?;
    if places.len() > place::MAXIMUM_CREATED_PLACES {
        return Err(RepositoryError::Capacity);
    }
    let mut known = pack_cast();
    let mut created = Vec::new();
    for record in places.into_values() {
        record.validate().map_err(bad)?;
        for person in record.expand().map_err(bad)?.cast {
            if known.iter().any(|member| member.id == person.id) {
                return Err(RepositoryError::InvalidCandidate);
            }
            known.push(person.clone());
            created.push(person);
        }
    }
    Ok(created)
}

fn cast_in(state: &GameState) -> Result<Vec<pack::SceneCastMember>, RepositoryError> {
    let mut cast = pack_cast();
    cast.extend(created_cast(state)?);
    Ok(cast)
}

/// The current place's original cast, including people who fled or fell. The runtime scene
/// filters those people out for conversation, but fight narration/validation still needs them.
fn scene_cast(state: &GameState) -> Result<Vec<pack::SceneCastMember>, RepositoryError> {
    let scene = super::super::beat_script_runtime::scene(state)?;
    // A spill keeps the source's fighters/bystanders; the destination's local cast does not
    // replace them merely because the party's place overlay has changed.
    if let Some(entry) = opening_move(state)? {
        if let Some(record) = crate::gameplay::place::created(state)?.remove(&entry.change.from) {
            return Ok(record.expand().map_err(bad)?.cast);
        }
        return pack::with_content(|content| {
            content
                .scenes
                .iter()
                .chain(
                    content
                        .beat_script
                        .as_ref()
                        .into_iter()
                        .flat_map(|script| script.scenes.values()),
                )
                .find(|source| {
                    source
                        .setting
                        .as_ref()
                        .is_some_and(|setting| setting.place_id == entry.change.from)
                })
                .map(|source| source.cast.clone())
                .ok_or(RepositoryError::InvalidCandidate)
        });
    }
    if let Some(setting) = scene.setting
        && let Some(record) = crate::gameplay::place::created(state)?.remove(&setting.place_id)
    {
        return Ok(record.expand().map_err(bad)?.cast);
    }
    Ok(pack::scene(scene.key).cast)
}

/// The SRD 5.2.1 creature a stat block id names.
pub(in crate::gameplay) fn creature_for(
    stat_block: Option<&str>,
) -> Option<&'static df_rules::SrdCreature> {
    let id = stat_block?;
    df_rules::SRD_521_CREATURES
        .iter()
        .chain([&df_rules::SCENE_COMMONER])
        .find(|creature| creature.id == id)
}

/// Authored people and their SRD profiles, for the pack's static resource inventory. Call
/// `combatants_in` for the full checkpoint inventory, including created places.
pub(super) fn combatants() -> Vec<(String, &'static df_rules::SrdCreature)> {
    pack_cast()
        .into_iter()
        .filter_map(|member| {
            creature_for(member.stat_block.as_deref()).map(|creature| (member.id, creature))
        })
        .collect()
}

/// All fightable identities of this checkpoint, in stable pack/created-place order. Roster
/// selection still uses only the current scene; this inventory also retains past combatants.
pub(super) fn combatants_in(
    state: &GameState,
) -> Result<Vec<(String, &'static df_rules::SrdCreature)>, RepositoryError> {
    Ok(cast_in(state)?
        .into_iter()
        .filter_map(|member| {
            creature_for(member.stat_block.as_deref()).map(|creature| (member.id, creature))
        })
        .collect())
}

/// Additional inventory derived from bounded, validated place records, never from stored resource
/// values. Checkpoint/command/recovery inventories must include these beside authored resources.
pub(super) fn created_resources(
    state: &GameState,
) -> Result<Vec<ResourceConstraint>, RepositoryError> {
    let authored = combatants();
    let mut resources = Vec::new();
    for (person, creature) in combatants_in(state)? {
        if authored.iter().any(|(id, _)| *id == person) {
            continue;
        }
        let owner = person_entity(&person)?;
        let spec =
            tactical_rules::CombatantSpec::creature(3, creature.id, tactical_rules::Cell(0, 0), 0)
                .map_err(bad)?;
        for (name, minimum, maximum) in [
            ("hit-points", 0, i64::from(spec.hit_points)),
            (
                "armor-class",
                i64::from(spec.armor_class.saturating_sub(2)),
                i64::from(spec.armor_class),
            ),
            ("strength", 1, 20),
            ("dexterity", 1, 20),
            ("constitution", 1, 20),
            ("intelligence", 1, 20),
            ("wisdom", 1, 20),
            ("charisma", 1, 20),
            ("second-wind", 0, i64::from(pack::second_wind().uses_max)),
            ("hit-point-dice", 0, 0),
            ("short-rest-choice", 0, 0),
            ("short-rest-die-roll", 0, 0),
            ("healer-kit-uses", 0, 0),
            ("unconscious", 0, 1),
            ("prone", 0, 1),
            ("held-weapon", 0, 1),
            ("action-used", 0, 1),
            ("bonus-used", 0, 1),
            ("dead", 0, 1),
        ] {
            resources.push(ResourceConstraint {
                owner,
                resource: model::label(name)?,
                minimum,
                maximum,
                source: rule()?,
            });
        }
    }
    Ok(resources)
}

/// The stable world entity of a scene person.
pub(in crate::gameplay) fn person_entity(person_id: &str) -> Result<EntityId, RepositoryError> {
    let digest = Sha256::digest([b"df-scene-person-1:".as_slice(), person_id.as_bytes()].concat());
    entity(<[u8; 16]>::try_from(&digest[..16]).map_err(bad)?)
}

fn identity(person_id: &str) -> Result<df_types::RevisionLabel, RepositoryError> {
    model::label(&format!("scene-person-{person_id}-1"))
}

fn person_of(world: &WorldEntity) -> Option<&str> {
    if world.definition.entry.as_str() != PERSON {
        return None;
    }
    world
        .identity_revision
        .as_str()
        .strip_prefix("scene-person-")?
        .strip_suffix("-1")
}

/// The name of a scene person's entity, for logs and views.
pub(super) fn person_name(state: &GameState, who: EntityId) -> Option<String> {
    let id = person_of(state.entities.iter().find(|world| world.id == who)?)?;
    cast_in(state)
        .ok()?
        .into_iter()
        .find(|member| member.id == id)
        .map(|member| member.name)
}

/// Whether `who` is a scene person's entity.
pub(super) fn is_person(state: &GameState, who: EntityId) -> bool {
    state
        .entities
        .iter()
        .any(|world| world.id == who && person_of(world).is_some())
}

/// Whether a freeplay scene fight is open: the one encounter is a scene assault.
pub(in crate::gameplay) fn open(state: &GameState) -> bool {
    freeplay()
        && matches!(state.encounters.as_slice(), [encounter]
            if encounter.tactical.is_some()
                && encounter.objectives.len() == 1
                && encounter.objectives[0].entry.as_str() == OBJECTIVE)
}

fn flag(state: &GameState, who: EntityId, name: &str) -> bool {
    value(state, who, name).is_ok_and(|value| value != 0)
}

/// A hero who died in a freeplay fight. Dead heroes get no offers: nothing revives them.
pub(in crate::gameplay) fn hero_dead(state: &GameState, who: EntityId) -> bool {
    flag(state, who, DEAD)
}

// ---------------------------------------------------------------------------------------------
// Who is still in the scene
// ---------------------------------------------------------------------------------------------

/// Where a scene person stands once fights have touched the scene.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum Presence {
    /// Never in a fight, or in one and still on their feet.
    Standing,
    /// Knocked out, or dead at 0 hit points: the monster rules of the tactical engine.
    Down { dead: bool },
    /// Fled the scene when a fight started.
    Gone,
}

pub(in crate::gameplay) fn presence(
    state: &GameState,
    person_id: &str,
) -> Result<Presence, RepositoryError> {
    let who = person_entity(person_id)?;
    let Some(world) = state.entities.iter().find(|world| world.id == who) else {
        return Ok(Presence::Standing);
    };
    if world.location.is_none() {
        return Ok(Presence::Gone);
    }
    if value(state, who, "unconscious")? != 0 {
        return Ok(Presence::Down {
            dead: value(state, who, "hit-points")? == 0,
        });
    }
    Ok(Presence::Standing)
}

/// The foes still on their feet in the open scene fight, in initiative order: the people a hero's
/// words reach mid-fight. Empty when no scene fight is open.
pub(in crate::gameplay) fn standing_foes(
    state: &GameState,
) -> Result<Vec<Person>, RepositoryError> {
    if !open(state) {
        return Ok(Vec::new());
    }
    let battle = tactical::read(state)?;
    let cast = cast_in(state)?;
    let mut foes = Vec::new();
    for id in battle.initiative_order() {
        let actor = battle.actor(*id).map_err(bad)?;
        if actor.side() == Side::Heroes || actor.health().incapacitated() {
            continue;
        }
        let who = tactical::canonical(state, *id)?;
        let Some(person_id) = state
            .entities
            .iter()
            .find(|world| world.id == who)
            .and_then(person_of)
        else {
            continue;
        };
        if let Some(member) = cast.iter().find(|member| member.id == person_id) {
            foes.push(Person::of(member));
        }
    }
    Ok(foes)
}

/// Whether `member`'s hero can take the Influence action now (SRD 5.2.1 p. 184: Influence is an
/// action): a scene fight is open, it is that hero's own turn with no reaction pending, the hero
/// is conscious and the turn's action is unspent. Speaking alone needs none of this.
pub(in crate::gameplay) fn influence_action_available(
    current: &Checkpoint,
    member: MemberId,
) -> Result<bool, RepositoryError> {
    let state = current.state();
    if !open(state) {
        return Ok(false);
    }
    let battle = tactical::read(state)?;
    let id = tactical::actor_id(state, player_entity(member, current)?)?;
    let hero = battle.actor(id).map_err(bad)?;
    Ok(battle.reaction_offers().is_empty()
        && battle.active_actor() == Some(id)
        && !hero.budget().action_used
        && !hero.health().incapacitated())
}

/// The scene people who can be attacked now: standing, with a stat block.
fn targets<'a>(
    state: &GameState,
    scene: &'a pack::Scene,
) -> Result<Vec<&'a pack::SceneCastMember>, RepositoryError> {
    let mut result = Vec::new();
    for member in &scene.cast {
        if creature_for(member.stat_block.as_deref()).is_some()
            && presence(state, &member.id)? == Presence::Standing
        {
            result.push(member);
        }
    }
    Ok(result)
}

/// The scene person a hero's words name: by id, by name, or by first name.
pub(in crate::gameplay) fn person_named<'a>(
    scene: &'a pack::Scene,
    text: &str,
) -> Option<&'a pack::SceneCastMember> {
    let wanted = text.trim().trim_start_matches("the ").trim();
    scene.cast.iter().find(|member| {
        member.id.eq_ignore_ascii_case(wanted)
            || member.name.eq_ignore_ascii_case(wanted)
            || member
                .name
                .split_whitespace()
                .next()
                .is_some_and(|first| first.eq_ignore_ascii_case(wanted))
    })
}

/// Whether this person is eligible to be attacked now, and the person's id if so.
pub(in crate::gameplay) fn attackable<'a>(
    state: &GameState,
    scene: &'a pack::Scene,
    text: &str,
) -> Result<Option<&'a pack::SceneCastMember>, RepositoryError> {
    let Some(member) = person_named(scene, text) else {
        return Ok(None);
    };
    Ok((creature_for(member.stat_block.as_deref()).is_some()
        && presence(state, &member.id)? == Presence::Standing)
        .then_some(member))
}

// ---------------------------------------------------------------------------------------------
// Offers
// ---------------------------------------------------------------------------------------------

/// The opaque id of the scene-assault offer at this revision.
pub(in crate::gameplay) fn offer_id(current: &Checkpoint) -> String {
    format!(
        "scene-assault-{}-{}",
        current.basis().revision.epoch().get(),
        current.basis().revision.sequence()
    )
}

pub(in crate::gameplay) const OFFER_LABEL: &str = "Start a fight";

/// Whether a fight can start now: freeplay with scene assaults, the story begun, no fight open,
/// and every hero on their feet.
fn available(current: &Checkpoint) -> Result<bool, RepositoryError> {
    if !freeplay() || !tactical::scene_assault_only() {
        return Ok(false);
    }
    let state = current.state();
    if !state.encounters.is_empty() || phase(current)? != rpc::JourneyPhase::Opening {
        return Ok(false);
    }
    let heroes = participants(current);
    if heroes.is_empty() {
        return Ok(false);
    }
    for link in heroes {
        let Some(who) = link.character else {
            return Ok(false);
        };
        if value(state, who, "hit-points")? == 0
            || value(state, who, "unconscious")? != 0
            || hero_dead(state, who)
        {
            return Ok(false);
        }
    }
    Ok(true)
}

/// The people this hero can attack now, as `(id, name)`; empty when no fight can start.
pub(in crate::gameplay) fn offered_targets(
    current: &Checkpoint,
    member: MemberId,
) -> Result<Vec<(String, String)>, RepositoryError> {
    player_entity(member, current)?;
    if !available(current)? {
        return Ok(Vec::new());
    }
    let scene = super::super::beat_script_runtime::scene(current.state())?;
    Ok(targets(current.state(), &scene)?
        .into_iter()
        .map(|member| (member.id.clone(), member.name.clone()))
        .collect())
}

/// The choice group the scene-assault offer carries: who to attack.
pub(in crate::gameplay) fn target_group(
    current: &Checkpoint,
    member: MemberId,
) -> Result<rpc::JourneyGroup, RepositoryError> {
    Ok(rpc::JourneyGroup {
        group_id: TARGET_GROUP.to_owned(),
        label: "Who do you attack?".to_owned(),
        options: offered_targets(current, member)?
            .into_iter()
            .map(|(id, name)| rpc::JourneyOption {
                option_id: id,
                description: format!("Attack {name} and start a fight."),
                label: name,
                requires_group: String::new(),
                requires_options: Vec::new(),
            })
            .collect(),
    })
}

// ---------------------------------------------------------------------------------------------
// The bystander rule
// ---------------------------------------------------------------------------------------------

/// What a person who is not the target does when a fight starts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum Stance {
    /// Takes up arms against the aggressor.
    Join,
    /// Steps back and stays out of it.
    StayOut,
    /// Leaves the scene shouting for the watch; never a combatant.
    Flee,
}

impl Stance {
    fn word(self) -> &'static str {
        match self {
            Self::Join => "join",
            Self::StayOut => "out",
            Self::Flee => "flee",
        }
    }

    fn parse(word: &str) -> Option<Self> {
        [Self::Join, Self::StayOut, Self::Flee]
            .into_iter()
            .find(|stance| stance.word() == word)
    }
}

/// The bystander rule, in one place. A person who is not the target chooses a stance toward the
/// aggressor from their stat block and their attitude to the aggressor (house rule over the SRD
/// 5.2.1 attitudes Friendly, Indifferent and Hostile):
///
/// - Non-combatants (no stat block, or a Commoner) flee and shout for the watch.
/// - The watch (a Guard) joins against the aggressor unless Friendly to them.
/// - Any other combatant joins only if Hostile to the aggressor, and otherwise stays out.
///
/// The attacked person is never asked: they always fight.
pub(in crate::gameplay) fn stance(stat_block: Option<&str>, attitude: Attitude) -> Stance {
    match stat_block {
        None | Some(NON_COMBATANT) => Stance::Flee,
        Some(PEACEKEEPER) => {
            if attitude == Attitude::Friendly {
                Stance::StayOut
            } else {
                Stance::Join
            }
        }
        Some(_) => {
            if attitude == Attitude::Hostile {
                Stance::Join
            } else {
                Stance::StayOut
            }
        }
    }
}

/// Whether a person can join a fight at all: they have a stat block that is not a Commoner's.
fn can_join(member: &pack::SceneCastMember) -> bool {
    creature_for(member.stat_block.as_deref()).is_some_and(|creature| creature.id != NON_COMBATANT)
}

/// A scene person as a plan names them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct Person {
    pub(in crate::gameplay) id: String,
    pub(in crate::gameplay) name: String,
}

impl Person {
    fn of(member: &pack::SceneCastMember) -> Self {
        Self {
            id: member.id.clone(),
            name: member.name.clone(),
        }
    }
}

/// Who fights, who leaves and who stays out when one scene person is attacked.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct Plan {
    pub(in crate::gameplay) target: Person,
    pub(in crate::gameplay) joiners: Vec<Person>,
    pub(in crate::gameplay) fleers: Vec<Person>,
    pub(in crate::gameplay) out: Vec<Person>,
}

impl Plan {
    /// The `scene-roster` choice that carries this plan: `join:id/flee:id/out:id`.
    pub(in crate::gameplay) fn roster(&self) -> String {
        [
            (Stance::Join, &self.joiners),
            (Stance::Flee, &self.fleers),
            (Stance::StayOut, &self.out),
        ]
        .into_iter()
        .flat_map(|(stance, people)| {
            people
                .iter()
                .map(move |person| format!("{}:{}", stance.word(), person.id))
        })
        .collect::<Vec<_>>()
        .join("/")
    }
}

/// Plans the fight for `target_text` with the attitudes the caller supplies (the social ledger).
pub(in crate::gameplay) fn plan_with(
    state: &GameState,
    scene: &pack::Scene,
    target_text: &str,
    attitude: &dyn Fn(&pack::SceneCastMember) -> Attitude,
) -> Result<Plan, RepositoryError> {
    let Some(target) = attackable(state, scene, target_text)? else {
        refusal_detail::note("scene-fight-target");
        return Err(RepositoryError::InvalidCandidate);
    };
    let mut joined = 0;
    let mut entries = Vec::new();
    for member in standing_others(state, scene, &target.id)? {
        let mut chosen = stance(member.stat_block.as_deref(), attitude(member));
        if chosen == Stance::Join {
            if joined + 1 >= MAXIMUM_FOES {
                chosen = Stance::StayOut;
            } else {
                joined += 1;
            }
        }
        entries.push(format!("{}:{}", chosen.word(), member.id));
    }
    plan_from_roster(state, scene, &target.id, Some(&entries.join("/")))
}

/// The people on stage other than the target who are still standing, in cast order.
fn standing_others<'a>(
    state: &GameState,
    scene: &'a pack::Scene,
    target: &str,
) -> Result<Vec<&'a pack::SceneCastMember>, RepositoryError> {
    let mut result = Vec::new();
    for member in &scene.cast {
        if member.id != target && presence(state, &member.id)? == Presence::Standing {
            result.push(member);
        }
    }
    Ok(result)
}

/// Validates a roster against the scene and builds the plan. A person the roster omits takes the
/// stance the rule gives an Indifferent person; a roster may not invent people, repeat one,
/// make a non-combatant join, or field more than the four spawn cells.
fn plan_from_roster(
    state: &GameState,
    scene: &pack::Scene,
    target_id: &str,
    roster: Option<&str>,
) -> Result<Plan, RepositoryError> {
    let refuse = |step: &'static str| {
        refusal_detail::note(step);
        RepositoryError::InvalidCandidate
    };
    let eligible = targets(state, scene)?;
    let target = eligible
        .into_iter()
        .find(|member| member.id == target_id)
        .ok_or_else(|| refuse("scene-fight-target"))?;
    let others = standing_others(state, scene, target_id)?;
    let mut stances: Vec<(&pack::SceneCastMember, Stance)> = others
        .iter()
        .map(|member| {
            (
                *member,
                stance(member.stat_block.as_deref(), Attitude::Indifferent),
            )
        })
        .collect();
    if let Some(roster) = roster.filter(|roster| !roster.is_empty()) {
        let mut seen: Vec<&str> = Vec::new();
        for entry in roster.split('/') {
            let (word, id) = entry
                .split_once(':')
                .ok_or_else(|| refuse("scene-fight-roster"))?;
            let chosen = Stance::parse(word).ok_or_else(|| refuse("scene-fight-roster"))?;
            let slot = stances
                .iter_mut()
                .find(|(member, _)| member.id == id)
                .ok_or_else(|| refuse("scene-fight-roster"))?;
            if seen.contains(&id) || (chosen == Stance::Join && !can_join(slot.0)) {
                return Err(refuse("scene-fight-roster"));
            }
            seen.push(id);
            slot.1 = chosen;
        }
    }
    let of = |wanted: Stance| {
        stances
            .iter()
            .filter(|(_, chosen)| *chosen == wanted)
            .map(|(member, _)| Person::of(member))
            .collect::<Vec<_>>()
    };
    let plan = Plan {
        target: Person::of(target),
        joiners: of(Stance::Join),
        fleers: of(Stance::Flee),
        out: of(Stance::StayOut),
    };
    if plan.joiners.len() + 1 > MAXIMUM_FOES {
        return Err(refuse("scene-fight-roster"));
    }
    Ok(plan)
}

/// The social role a cast member's dossier is keyed by.
fn scene_role(role: pack::CastRole) -> rpc::SceneRole {
    match role {
        pack::CastRole::Patron => rpc::SceneRole::Patron,
        pack::CastRole::Contact => rpc::SceneRole::Contact,
        pack::CastRole::Antagonist => rpc::SceneRole::Antagonist,
        pack::CastRole::Bystander => rpc::SceneRole::Bystander,
    }
}

/// The attitude a person holds toward `member`, from the ledger; Indifferent without a dossier.
pub(in crate::gameplay) fn attitude_of(
    ledger: &SocialLedger,
    member: MemberId,
    person: &pack::SceneCastMember,
) -> Attitude {
    ledger
        .dossier_of(scene_role(person.role), &person.id)
        .map_or(Attitude::Indifferent, |dossier| {
            ledger.standing(member, dossier).attitude
        })
}

/// Records the assault in the ledger for the aggressor only: the target and every witness who
/// joined or fled keep a grievance and turn Hostile toward `member`; people who stay out record
/// nothing. Returns how many standings changed, so the caller saves.
pub(in crate::gameplay) fn record_consequences(
    ledger: &SocialLedger,
    member: MemberId,
    scene_key: &str,
    scene: &pack::Scene,
    plan: &Plan,
    hero_line: &str,
) -> usize {
    let mut recorded = 0;
    for person in std::iter::once(&plan.target)
        .chain(&plan.joiners)
        .chain(&plan.fleers)
    {
        let Some(cast) = scene.cast.iter().find(|member| member.id == person.id) else {
            continue;
        };
        let Some(dossier) = ledger.dossier_of(scene_role(cast.role), &person.id) else {
            continue;
        };
        ledger.record_assault(member, dossier, scene_key, hero_line);
        recorded += 1;
    }
    recorded
}

/// The table's narration of how a fight started, one line per person, and the audit text.
pub(in crate::gameplay) fn announce(hero: &str, plan: &Plan) -> (Vec<String>, String) {
    let mut lines = vec![format!("{hero} attacks {}.", plan.target.name)];
    lines.extend(
        plan.joiners
            .iter()
            .map(|person| format!("{} joins the fight against {hero}.", person.name)),
    );
    lines.extend(
        plan.fleers
            .iter()
            .map(|person| format!("{} flees, shouting for the watch.", person.name)),
    );
    lines.extend(
        plan.out
            .iter()
            .map(|person| format!("{} steps back and stays out of it.", person.name)),
    );
    let names = |people: &[Person]| {
        people
            .iter()
            .map(|person| person.id.as_str())
            .collect::<Vec<_>>()
            .join(",")
    };
    let audit = format!(
        "SCENE-FIGHT aggressor={hero:?} target={:?} join=[{}] flee=[{}] out=[{}] rules=\"tactical.cover-centre-line: cover is adjudicated along cell centres\"",
        plan.target.id,
        names(&plan.joiners),
        names(&plan.fleers),
        names(&plan.out),
    );
    (lines, audit)
}

// ---------------------------------------------------------------------------------------------
// Staging a fight
// ---------------------------------------------------------------------------------------------

/// One person who fights.
pub(super) struct Foe {
    pub(super) person: Person,
    pub(super) entity: EntityId,
    pub(super) creature: &'static df_rules::SrdCreature,
}

/// One person who leaves the scene.
pub(super) struct Leaver {
    pub(super) person: Person,
    pub(super) entity: EntityId,
}

/// A scene assault as the command asks for it, validated against the scene.
pub(super) struct Fight {
    pub(super) plan: Plan,
    /// The target first, then the people who join, in cast order.
    pub(super) foes: Vec<Foe>,
    pub(super) fleers: Vec<Leaver>,
}

/// The fight a scene-assault command asks for; `None` for any other command.
pub(super) fn fight_of(
    current: &Checkpoint,
    command: &CommandInput,
) -> Result<Option<Fight>, RepositoryError> {
    let GameCommand::ProposeAction {
        action,
        targets,
        choices,
        ..
    } = &command.command
    else {
        return Ok(None);
    };
    if action.entry.as_str() != ACTION {
        return Ok(None);
    }
    let refuse = |step: &'static str| {
        refusal_detail::note(step);
        RepositoryError::InvalidCandidate
    };
    if !targets.is_empty() {
        return Err(refuse("scene-fight-command"));
    }
    let (mut target, mut roster) = (None, None);
    for (group, option) in choices {
        match group.as_str() {
            TARGET_GROUP if target.is_none() => target = Some(option.as_str()),
            ROSTER_GROUP if roster.is_none() => roster = Some(option.as_str()),
            _ => return Err(refuse("scene-fight-command")),
        }
    }
    let target = target.ok_or_else(|| refuse("scene-fight-command"))?;
    let state = current.state();
    let scene = super::super::beat_script_runtime::scene(state)?;
    let plan = plan_from_roster(state, &scene, target, roster)?;
    let creature_of = |person: &Person| {
        scene
            .cast
            .iter()
            .find(|member| member.id == person.id)
            .and_then(|member| creature_for(member.stat_block.as_deref()))
            .ok_or_else(|| refuse("scene-fight-stat-block"))
    };
    let mut foes = Vec::new();
    for person in std::iter::once(&plan.target).chain(&plan.joiners) {
        foes.push(Foe {
            person: person.clone(),
            entity: person_entity(&person.id)?,
            creature: creature_of(person)?,
        });
    }
    let fleers = plan
        .fleers
        .iter()
        .map(|person| {
            Ok(Leaver {
                person: person.clone(),
                entity: person_entity(&person.id)?,
            })
        })
        .collect::<Result<Vec<_>, RepositoryError>>()?;
    Ok(Some(Fight { plan, foes, fleers }))
}

/// Whether a foe is admissible when the battle begins: a person this command just brought on
/// stage, now standing on the grid, on their feet.
pub(super) fn foe_status(
    current: &GameState,
    staged: &GameState,
    candidate: &WorldEntity,
    foe: &Foe,
) -> Result<df_encounter::participants::ParticipantStatus, RepositoryError> {
    use df_encounter::participants::ParticipantStatus as Status;
    if candidate.definition != model::content(PERSON)?
        || candidate.identity_revision != identity(&foe.person.id)?
        || current.entities.iter().any(|world| world.id == foe.entity)
    {
        return Ok(Status::Undisclosed);
    }
    if candidate.location != Some(room_entity()?)
        || candidate.position.is_none()
        || value(staged, foe.entity, "hit-points")? == 0
        || value(staged, foe.entity, "unconscious")? != 0
    {
        return Ok(Status::Unavailable);
    }
    Ok(Status::Available)
}

/// Gives each person the fight involves a world entity and an `NpcState`. Foes enter the scene's
/// grid when the battle begins; people who flee never have a place in the room.
fn bring_on_stage(state: &mut GameState, fight: &Fight) -> Result<(), RepositoryError> {
    let people = fight
        .foes
        .iter()
        .map(|foe| (&foe.person, foe.entity))
        .chain(fight.fleers.iter().map(|gone| (&gone.person, gone.entity)));
    for (person, who) in people {
        if state.entities.iter().any(|world| world.id == who)
            || state.continuity.npcs.iter().any(|npc| npc.entity == who)
        {
            refusal_detail::note("scene-fight-person-reused");
            return Err(RepositoryError::InvalidCandidate);
        }
        state.entities.push(WorldEntity {
            id: who,
            definition: model::content(PERSON)?,
            location: None,
            position: None,
            identity_revision: identity(&person.id)?,
        });
        state.continuity.npcs.push(NpcState {
            entity: who,
            role: model::content(PERSON)?,
            personality: model::content(PERSON)?,
            motivations: Vec::new(),
            goals: Vec::new(),
            needs: Vec::new(),
            fears: Vec::new(),
            known_facts: Vec::new(),
            beliefs: Vec::new(),
            secrets: Vec::new(),
        });
    }
    Ok(())
}

/// Stages a hero's scene assault: the people involved come on stage, the battle begins on the
/// pack's battle scene, and the one decision records it. The freeplay beat does not change.
pub(super) fn stage_assault(
    current: &Checkpoint,
    command: &CommandInput,
    supplier: DiceSource<'_>,
) -> Result<Checkpoint, RepositoryError> {
    let scene = super::super::beat_script_runtime::scene(current.state())?;
    let proposed = scene
        .setting
        .as_ref()
        .and_then(|setting| crate::gameplay::fight_foresight::proposed_arena(&setting.place_id));
    let destination = scene.setting.as_ref().and_then(|setting| {
        crate::gameplay::fight_foresight::proposed_destination(&setting.place_id)
    });
    stage_assault_at(
        current,
        command,
        supplier,
        proposed.as_deref(),
        destination.as_ref(),
    )
}

fn stage_assault_at(
    current: &Checkpoint,
    command: &CommandInput,
    supplier: DiceSource<'_>,
    proposed: Option<&str>,
    created: Option<&crate::gameplay::place::PlaceRecord>,
) -> Result<Checkpoint, RepositoryError> {
    use session_machine::{Event, SessionState};
    let refuse = |step: &'static str| {
        refusal_detail::note(step);
        RepositoryError::InvalidCandidate
    };
    let GameCommand::ProposeAction { actor, .. } = &command.command else {
        return Err(refuse("scene-fight-command"));
    };
    let machine = SessionState::of(current)?;
    let transition =
        session_machine::next(machine, Event::StartFight).map_err(|_| refuse("session-machine"))?;
    let fight = fight_of(current, command)?.ok_or_else(|| refuse("scene-fight-command"))?;
    if *actor != player_entity(command.member, current)?
        || !offered_targets(current, command.member)?
            .iter()
            .any(|(id, _)| *id == fight.plan.target.id)
    {
        return Err(refuse("scene-fight-offer"));
    }
    let mut basis = current.basis();
    basis.revision = basis.revision.next_sequence().map_err(bad)?;
    let ArenaStaging {
        mut state,
        arena,
        movement,
    } = stage_arena_move(current, command, proposed, created)?;
    // Created places use the same configured battle-map path as other C1 scene fights.
    // Prefer an authored destination layout when the server holds one.
    let destination = super::super::beat_script_runtime::scene(&state)?;
    let battle_scene = destination
        .setting
        .as_ref()
        .filter(|setting| tactical::holds_scene(&setting.art_key))
        .map(|setting| setting.art_key.clone())
        .or_else(pack::scene_assault_battle)
        .ok_or_else(|| refuse("scene-fight-unavailable"))?;
    bring_on_stage(&mut state, &fight)?;
    let mut draws = Vec::new();
    let mut outcomes = Vec::new();
    tactical::start_scene(
        current,
        command,
        &mut state,
        &mut draws,
        &mut outcomes,
        supplier,
        &battle_scene,
        &fight,
    )?;
    let accepted = rpc::AcceptedAction {
        combat: outcomes,
        phase: rpc::JourneyPhase::Combat as i32,
        ..Default::default()
    };
    let accepted = hex(&prost::Message::encode_to_vec(&accepted));
    // The durable owner admits one decision per submission. Keep the normal place entry
    // inside that opening decision, rather than appending a second decision at the same revision.
    if movement.is_some() {
        let placed = state
            .decisions
            .pop()
            .ok_or(RepositoryError::InvalidCandidate)?;
        if placed.source_policy.as_str() != crate::gameplay::place::POLICY {
            return Err(RepositoryError::InvalidCandidate);
        }
        state
            .facts
            .retain(|fact| fact.operation != placed.operation);
    }
    let semantic = if arena.is_some() {
        format!(
            "{OPENING_PREFIX}{}",
            serde_json::to_string(&ArenaOpening {
                accepted,
                arena,
                movement
            })
            .map_err(bad)?
        )
    } else {
        accepted
    };
    // The recorded receipt is the largest text a checkpoint holds; refuse here, by name.
    if semantic.len() > model::limits().maximum_text_bytes {
        refusal_detail::note("receipt-too-large");
        return Err(RepositoryError::Capacity);
    }
    let candidate = finish(
        current,
        basis,
        state,
        command.operation,
        draws,
        DecisionContent {
            entry: OBJECTIVE,
            semantic: Some(semantic),
            policy: THREAD_POLICY,
            audience: AudienceScope::Shared,
            reaction: None,
            escort_target: None,
            objective_command: None,
            script_choice: None,
        },
    )?;
    if !session_machine::outcome_legal(
        machine,
        Event::StartFight,
        transition,
        SessionState::of(&candidate)?,
    ) {
        return Err(refuse("session-machine-outcome"));
    }
    Ok(candidate)
}

/// A model-approved large place retained beside the opening receipt.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Arena {
    place_id: String,
    name: String,
}

const OPENING_PREFIX: &str = "scene-fight-opening-1:";

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ArenaOpening {
    accepted: String,
    arena: Option<Arena>,
    movement: Option<crate::gameplay::place::PlaceEntry>,
}

struct ArenaStaging {
    state: GameState,
    arena: Option<Arena>,
    movement: Option<crate::gameplay::place::PlaceEntry>,
}

fn flat_arena(current: &Checkpoint) -> ArenaStaging {
    ArenaStaging {
        state: current.state().clone(),
        arena: None,
        movement: None,
    }
}

/// Validate through the normal place planner. Its temporary candidate supplies the map's
/// destination; the final fight retains only its PlaceEntry inside the one opening decision.
fn stage_arena_move(
    current: &Checkpoint,
    command: &CommandInput,
    proposed: Option<&str>,
    created: Option<&crate::gameplay::place::PlaceRecord>,
) -> Result<ArenaStaging, RepositoryError> {
    use crate::gameplay::{beat_script_runtime, place};
    let scene = beat_script_runtime::scene(current.state())?;
    let Some(setting) = scene.setting.as_ref() else {
        return Ok(flat_arena(current));
    };
    let Some(target) = proposed else {
        return Ok(flat_arena(current));
    };
    if target == setting.place_id {
        return Ok(ArenaStaging {
            state: current.state().clone(),
            arena: Some(Arena {
                place_id: target.to_owned(),
                name: setting.name.clone(),
            }),
            movement: None,
        });
    }
    let snapshot = super::living_scene::snapshot(current.state(), &scene)?
        .ok_or(RepositoryError::InvalidCandidate)?;
    let selected_exit = crate::gameplay::fight_foresight::proposed_exit(&setting.place_id);
    let Some(exit) = snapshot.model.ways_out.iter().find(|exit| {
        exit.open
            && (exit.beyond_place_id == target
                || (exit.beyond_place_id == place::UNNAMED
                    && selected_exit.as_deref() == Some(&exit.id)
                    && created.is_some_and(|record| record.place_id == target)))
    }) else {
        crate::gameplay::actor::record_audit_line(
            "gameplay.fight_arena",
            format!(
                "FIGHT_ARENA source={} target={target} outcome=stale_exit flat=true",
                setting.place_id
            ),
        );
        return Ok(flat_arena(current));
    };
    let digest = Sha256::digest(
        [
            b"df-fight-arena-move-1:".as_slice(),
            command.operation.as_bytes(),
        ]
        .concat(),
    );
    let operation = OperationId::from_bytes(&digest[..16]).map_err(bad)?;
    let hero = name(current.state(), player_entity(command.member, current)?)?;
    let mut planned = match place::plan(
        current,
        &hero,
        operation,
        &setting.place_id,
        &exit.id,
        target,
        created,
    ) {
        Ok(planned) => planned,
        Err(place::PlaceFailure::Refused(reason)) => {
            place::audit_refused(&setting.place_id, target, &exit.id, reason.label());
            return Ok(flat_arena(current));
        }
        Err(place::PlaceFailure::State(error)) => return Err(error),
    };
    planned.entry.line = format!("The fight spills out into {}.", planned.to_name);
    let mut state = planned.candidate.state().clone();
    let decision = state
        .decisions
        .iter_mut()
        .find(|decision| decision.operation == operation)
        .ok_or(RepositoryError::InvalidCandidate)?;
    decision.semantic_output = Some(planned.entry.encode()?);
    Ok(ArenaStaging {
        state,
        arena: Some(Arena {
            place_id: target.to_owned(),
            name: planned.to_name,
        }),
        movement: Some(planned.entry),
    })
}

fn arena_opening(decision: &AcceptedDecision) -> Result<Option<ArenaOpening>, RepositoryError> {
    let Some(text) = decision
        .semantic_output
        .as_deref()
        .and_then(|text| text.strip_prefix(OPENING_PREFIX))
    else {
        return Ok(None);
    };
    if decision.source_policy.as_str() != THREAD_POLICY {
        return Err(RepositoryError::InvalidReceipt);
    }
    serde_json::from_str(text).map(Some).map_err(bad)
}

/// Shared receipt reader hook: the wire receipt stays the ordinary AcceptedAction.
pub(in crate::gameplay) fn opening_receipt(
    decision: &AcceptedDecision,
) -> Result<Option<rpc::AcceptedAction>, RepositoryError> {
    use prost::Message;
    arena_opening(decision)?
        .map(|opening| {
            rpc::AcceptedAction::decode(unhex(&opening.accepted)?.as_slice()).map_err(bad)
        })
        .transpose()
}

/// Shared place-journal reader hook: the normal admitted PlaceEntry, in the fight's decision.
pub(in crate::gameplay) fn opening_place_move(
    decision: &AcceptedDecision,
) -> Result<Option<crate::gameplay::place::PlaceEntry>, RepositoryError> {
    Ok(arena_opening(decision)?.and_then(|opening| opening.movement))
}

fn opening_decision(state: &GameState) -> Option<&AcceptedDecision> {
    state.decisions.iter().rev().find(|decision| {
        decision.facts.iter().any(|id| {
            state.facts.iter().any(|fact| {
                fact.id == *id
                    && matches!(&fact.value,
            FactValue::ContentEvent { definition, .. } if definition.entry.as_str() == OBJECTIVE)
            })
        })
    })
}

fn opening_move(
    state: &GameState,
) -> Result<Option<crate::gameplay::place::PlaceEntry>, RepositoryError> {
    let Some(opening) = opening_decision(state) else {
        return Ok(None);
    };
    opening_place_move(opening)
}

/// The ready world may only belong to the large arena retained by the committed fight receipt.
pub(in crate::gameplay) fn world_place(
    state: &GameState,
) -> Result<Option<String>, RepositoryError> {
    if !open(state) {
        return Ok(None);
    }
    let decision = opening_decision(state).ok_or(RepositoryError::InvalidCandidate)?;
    Ok(arena_opening(decision)?
        .and_then(|opening| opening.arena)
        .map(|arena| arena.place_id))
}

/// One narrator row, read from the committed move rather than a presentation forecast.
pub(in crate::gameplay) fn arena_narration(
    state: &GameState,
) -> Result<Option<String>, RepositoryError> {
    Ok(opening_move(state)?.map(|entry| entry.line))
}

// ---------------------------------------------------------------------------------------------
// Ending a fight
// ---------------------------------------------------------------------------------------------

/// Ends a freeplay scene fight: the encounter closes and the scene returns to the same freeplay
/// beat. It is called once the battle is finished, which the tactical rules decide: one side has
/// no conscious combatant.
///
/// The one rule for how it ends (SRD 5.2.1 p. 18, Stable at 0 Hit Points): a hero at 0 hit points
/// who is not dead is Stable, so no Death Saving Throws follow the fight; Stable and unconscious
/// at 0 hit points is the state they stay in. When every hero is down the narration says the
/// watch arrives and guards them where they lie (someone fled to call it), or the survivors leave
/// them where they fell; nobody is moved, because no other place exists to move them to.
/// Heroes who died stay dead. A person at 0 hit points is dead (monster rules), a person knocked
/// out sits at 1 hit point unconscious, and both stay down. Turn budgets reset and nobody lies
/// prone who is awake. No experience, no payment and no beat change.
pub(super) fn close(state: &mut GameState, battle: &Battle) -> Result<(), RepositoryError> {
    for actor in battle.combatants() {
        let who = tactical::canonical(state, actor.id())?;
        let health = actor.health();
        if health.dead && actor.side() == Side::Heroes {
            let label = model::label(DEAD)?;
            if state
                .resources
                .iter()
                .any(|resource| resource.owner == who && resource.resource == label)
            {
                set(state, who, DEAD, 1)?;
            } else {
                add_resource(state, who, DEAD, 1)?;
            }
        }
        set(state, who, "action-used", 0)?;
        set(state, who, "bonus-used", 0)?;
        if !health.incapacitated() {
            set(state, who, "prone", 0)?;
        }
    }
    state.encounters.clear();
    Ok(())
}

/// The decision entry of a scene fight its foes ended by standing down: the scene-assault action
/// entry, which the content inventory already lists and no other decision records (a fight's
/// start records `OBJECTIVE`). A distinct entry would need a new row in the inventory.
pub(super) const STAND_DOWN: &str = ACTION;

/// Ends the open scene fight because its foes stood down: the hero's words reached a willing foe,
/// or the Influence check the foe's reply rolled succeeded (SRD 5.2.1 p. 184, Influence). The
/// encounter closes through `close`, the same close a won or lost fight takes, in one decision with
/// no draws; the freeplay beat does not change, and nobody's hit points move.
pub(in crate::gameplay) fn stand_down(
    current: &Checkpoint,
    operation: OperationId,
) -> Result<Checkpoint, RepositoryError> {
    use session_machine::SessionState;
    let refuse = |step: &'static str| {
        refusal_detail::note(step);
        RepositoryError::InvalidCandidate
    };
    if !open(current.state()) {
        return Err(refuse("scene-stand-down-open"));
    }
    let before = SessionState::of(current)?;
    let mut state = current.state().clone();
    let battle = tactical::read(&state)?;
    close(&mut state, &battle)?;
    let resting = super::super::beat_script::with_script(|script| {
        script
            .current(current.state())
            .map(super::super::beat_script_runtime::phase_of)
    })?;
    let accepted = rpc::AcceptedAction {
        phase: phase_after(&state, resting) as i32,
        ..Default::default()
    };
    let mut basis = current.basis();
    basis.revision = basis.revision.next_sequence().map_err(bad)?;
    let candidate = finish(
        current,
        basis,
        state,
        operation,
        Vec::new(),
        DecisionContent {
            entry: STAND_DOWN,
            semantic: Some(hex(&prost::Message::encode_to_vec(&accepted))),
            policy: THREAD_POLICY,
            audience: AudienceScope::Shared,
            reaction: None,
            escort_target: None,
            objective_command: None,
            script_choice: None,
        },
    )?;
    if !session_machine::scene_fight_outcome_legal(before, SessionState::of(&candidate)?) {
        return Err(refuse("scene-stand-down-machine"));
    }
    Ok(candidate)
}

/// A short closing line after a fight, derived from the committed checkpoint for stable replay.
pub(in crate::gameplay) fn aftermath_narration(
    state: &GameState,
) -> Result<String, RepositoryError> {
    if open(state) || !fought(state) {
        return Ok(String::new());
    }
    let mut outcomes = Vec::new();
    let mut anyone_fled = false;
    let mut standing_people = Vec::new();
    let scene = super::super::beat_script_runtime::scene(state)?;
    for member in scene_cast(state)? {
        match presence(state, &member.id)? {
            Presence::Gone => {
                anyone_fled = true;
                outcomes.push(
                    if scene.setting.as_ref().is_some_and(|setting| {
                        setting.kind == crate::gameplay::adventure::setting::SettingKind::Shop
                    }) {
                        format!("{} fled the shop", member.name)
                    } else {
                        format!("{} fled the scene", member.name)
                    },
                );
            }
            Presence::Down { dead: true } => outcomes.push(format!("{} lies dead", member.name)),
            Presence::Down { dead: false } => {
                outcomes.push(format!("{} lies knocked out where they fell", member.name));
            }
            Presence::Standing => {
                let exists = state
                    .entities
                    .iter()
                    .any(|world| person_of(world) == Some(member.id.as_str()));
                if exists {
                    outcomes.push(format!("{} remains standing", member.name));
                    standing_people.push(member.name);
                } else {
                    outcomes.push(format!("{} stayed out of the fight", member.name));
                }
            }
        }
    }
    let mut every_hero_down = !state.characters.is_empty();
    for character in &state.characters {
        let who = character.entity;
        let name = name(state, who)?;
        if hero_dead(state, who) {
            outcomes.push(format!("{name} is dead"));
        } else if value(state, who, "hit-points")? == 0 {
            outcomes.push(format!(
                "{name} lies unconscious and stable where they fell"
            ));
        } else {
            every_hero_down = false;
        }
    }
    // The outcome leads, briefly, and each fate gets its own line: no wall of text and no
    // room description (the scene text belongs to the room, not to the fight's end).
    let summary = if every_hero_down && !outcomes.is_empty() {
        "The fight is over. Every hero is down."
    } else {
        "The fight is over."
    };
    let mut lines = vec![summary.to_owned()];
    lines.extend(outcomes.iter().map(|outcome| format!("{outcome}.")));
    if every_hero_down && !outcomes.is_empty() {
        if anyone_fled {
            lines.push("The watch arrives and stands guard over the fallen heroes.".to_owned());
        } else if standing_people.is_empty() {
            lines.push("No one is left standing.".to_owned());
        } else {
            lines.push(format!(
                "{} leave the heroes where they fell.",
                standing_people.join(" and ")
            ));
        }
    }
    Ok(lines.join("\n"))
}

/// The fight-start narration for the open scene fight, rebuilt from the committed state so
/// every view tells the same start: who turned on the heroes and who fled. Empty when no
/// scene fight is open. The battle intro shows this, never the room description.
pub(in crate::gameplay) fn opening_narration(state: &GameState) -> Result<String, RepositoryError> {
    if !open(state) {
        return Ok(String::new());
    }
    let foes = standing_foes(state)?;
    let mut lines = if foes.is_empty() {
        vec!["A fight begins.".to_owned()]
    } else {
        let names = foes
            .iter()
            .map(|person| person.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        vec![format!("A fight begins: {names} turn on the heroes.")]
    };
    if let Some(line) = arena_narration(state)? {
        lines.push(line);
    }
    for member in scene_cast(state)? {
        if presence(state, &member.id)? == Presence::Gone {
            lines.push(format!("{} flees, shouting for the watch.", member.name));
        }
    }
    Ok(lines.join("\n"))
}

// ---------------------------------------------------------------------------------------------
// Replay validation
// ---------------------------------------------------------------------------------------------

/// How one recorded decision takes part in a fight, when it does.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum Step {
    Opens,
    Continues,
    Closes,
}

/// Classifies a freeplay decision by its terminal event and recorded phase. `None` means the
/// decision is no part of a fight; an error means it claims to be one and cannot be.
pub(in crate::gameplay) fn replay_step(
    entry: &str,
    recorded_phase: i32,
    open: bool,
) -> Result<Option<Step>, RepositoryError> {
    let combat = rpc::JourneyPhase::Combat as i32;
    let opening = rpc::JourneyPhase::Opening as i32;
    if entry == OBJECTIVE {
        return if open || recorded_phase != combat {
            Err(RepositoryError::InvalidCandidate)
        } else {
            Ok(Some(Step::Opens))
        };
    }
    if entry == STAND_DOWN {
        return if !open || recorded_phase == combat {
            Err(RepositoryError::InvalidCandidate)
        } else {
            Ok(Some(Step::Closes))
        };
    }
    if !open || !wire::action_kind(entry).is_some_and(tactical::combat_action) {
        return Ok(None);
    }
    match recorded_phase {
        phase if phase == combat => Ok(Some(Step::Continues)),
        phase if phase == opening => Ok(Some(Step::Closes)),
        _ => Err(RepositoryError::InvalidCandidate),
    }
}

/// Whether the encounters a freeplay state holds are legal: none, or exactly one open scene
/// fight whose battle is consistent with the checkpoint and whose foes are scene people.
pub(in crate::gameplay) fn encounters_admissible(
    current: &Checkpoint,
) -> Result<bool, RepositoryError> {
    let state = current.state();
    if state.encounters.is_empty() {
        return Ok(true);
    }
    if !tactical::scene_assault_only() || !open(state) {
        refusal_detail::note("scene-fight-open");
        return Ok(false);
    }
    // `validate_encounter` names its own refusing step before this one can.
    if tactical::validate_encounter(current).is_err() {
        refusal_detail::note("scene-fight-battle");
        return Ok(false);
    }
    let known = scene_cast(state)?;
    for participant in &state.encounters[0].participants {
        if state
            .characters
            .iter()
            .any(|character| character.entity == *participant)
        {
            continue;
        }
        let Some(world) = state.entities.iter().find(|world| world.id == *participant) else {
            refusal_detail::note("scene-fight-participant");
            return Ok(false);
        };
        let Some(id) = person_of(world) else {
            refusal_detail::note("scene-fight-participant");
            return Ok(false);
        };
        if !known.iter().any(|member| member.id == id) || person_entity(id)? != *participant {
            refusal_detail::note("scene-fight-participant");
            return Ok(false);
        }
    }
    Ok(true)
}

/// The combat resources a fight changes on a hero, which freeplay's character replay allows to
/// differ from their creation values.
const FIGHT_RESOURCES: [&str; 9] = [
    "hit-points",
    "armor-class",
    "unconscious",
    "prone",
    "held-weapon",
    "second-wind",
    "healer-kit-uses",
    "action-used",
    "bonus-used",
];

/// Whether this freeplay state has seen a fight: a scene person has been brought on stage.
pub(in crate::gameplay) fn fought(state: &GameState) -> bool {
    freeplay()
        && state
            .entities
            .iter()
            .any(|world| person_of(world).is_some())
}

/// Whether a fought state's hero resources still derive from their creation. Each expected
/// resource must remain with its bounds and source, and may differ in value only where a fight
/// changes it; a fight adds only combat resources and the `dead` flag.
pub(super) fn resources_match(expected: &[ResourceState], actual: &[ResourceState]) -> bool {
    expected.iter().all(|before| {
        actual.iter().any(|now| {
            now.owner == before.owner
                && now.resource == before.resource
                && now.minimum == before.minimum
                && now.maximum == before.maximum
                && now.source == before.source
                && (now.value == before.value
                    || FIGHT_RESOURCES.contains(&before.resource.as_str()))
        })
    }) && actual.iter().all(|now| {
        // A fight records the combat resources it changes, including ones creation never set
        // (the Healer's Kit uses), and the death of a hero.
        now.resource.as_str() == DEAD
            || FIGHT_RESOURCES.contains(&now.resource.as_str())
            || expected
                .iter()
                .any(|before| before.owner == now.owner && before.resource == now.resource)
    })
}

/// Whether a fought state's inventory still derives from the heroes' creation kit: nothing new,
/// no more of anything than the kit gave, though thrown or spent items may be gone.
pub(super) fn inventory_matches(expected: &[InventoryItem], actual: &[InventoryItem]) -> bool {
    actual.iter().all(|now| {
        expected.iter().any(|before| {
            before.item == now.item
                && before.owner == now.owner
                && before.source == now.source
                && before.origin == now.origin
                && before.attunement_owner == now.attunement_owner
                && now.quantity <= before.quantity
        })
    })
}

/// The phase of a freeplay state after a fight step: Combat while the encounter is open, else the
/// freeplay beat's own.
pub(in crate::gameplay) fn phase_after(
    state: &GameState,
    resting: rpc::JourneyPhase,
) -> rpc::JourneyPhase {
    if open(state) {
        rpc::JourneyPhase::Combat
    } else {
        resting
    }
}

/// The offer a tactical action stands for in a freeplay fight. The freeplay script has no fight
/// beat, so selection hands back an offer of the right effect (the attack, Second Wind or End
/// Turn offer, which is what the session machine reads as a combat action or an end of turn).
pub(in crate::gameplay) fn step_offer(
    action: rpc::GameplayActionKind,
) -> Result<super::super::beat_script::BeatOffer, RepositoryError> {
    use super::super::beat_script::{BeatEffect, BeatOffer};
    let (id, effect) = match action {
        rpc::GameplayActionKind::SecondWind => ("scene-fight-second-wind", BeatEffect::SecondWind),
        rpc::GameplayActionKind::EndTurn => ("scene-fight-end-turn", BeatEffect::EndTurn),
        _ => ("scene-fight-attack", BeatEffect::GreatswordAttack),
    };
    Ok(BeatOffer {
        id: id.to_owned(),
        label_slot: id.to_owned(),
        effect,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gameplay::beat_script_runtime;
    use crate::gameplay::beat_script_variety_tests as native;
    use rpc::GameplayActionKind as K;

    const SHOP_PACK: &[u8] = include_bytes!("../../../../../content/merchant-shop/pack.json");
    const SHOP_TUNING: &[u8] = include_bytes!("../../../../../content/merchant-shop/tuning.json");
    const SHOP_DOSSIERS: &[u8] =
        include_bytes!("../../../../../content/merchant-shop/dossiers.json");
    const HARBOR_PACK: &[u8] = include_bytes!("../../../../../content/harbor-journey/pack.json");
    const HARBOR_TUNING: &[u8] =
        include_bytes!("../../../../../content/harbor-journey/tuning.json");

    fn with_shop(run: impl FnOnce()) {
        let loaded = pack::load_from_bytes(SHOP_PACK, SHOP_TUNING).unwrap();
        pack::with_test_pack(loaded, run);
    }

    /// Two heroes in Marek's shop with the story begun, the freeplay beat showing.
    pub(super) fn opening() -> Checkpoint {
        native::act(&native::prepared(), native::first(), 5, "begin-story")
    }

    fn ids(people: &[Person]) -> Vec<&str> {
        people.iter().map(|person| person.id.as_str()).collect()
    }

    fn scope(current: &Checkpoint, operation: u8) -> rpc::SubmitActionRequest {
        rpc::SubmitActionRequest {
            session_id: Some(rpc::SessionId {
                value: Some(current.basis().session.as_bytes().to_vec()),
            }),
            run_id: Some(rpc::RunId {
                value: Some(current.basis().run.as_bytes().to_vec()),
            }),
            observed_revision: Some(wire::revision(current.basis().revision)),
            operation_id: Some(rpc::OperationId {
                value: Some(vec![operation; 16]),
            }),
            ..Default::default()
        }
    }

    pub(super) fn assault_request(
        current: &Checkpoint,
        operation: u8,
        target: &str,
        roster: Option<&str>,
    ) -> rpc::SubmitActionRequest {
        let mut choices = vec![rpc::JourneyChoice {
            group_id: TARGET_GROUP.to_owned(),
            option_id: target.to_owned(),
        }];
        if let Some(roster) = roster {
            choices.push(rpc::JourneyChoice {
                group_id: ROSTER_GROUP.to_owned(),
                option_id: roster.to_owned(),
            });
        }
        rpc::SubmitActionRequest {
            offer_id: offer_id(current),
            action_kind: K::SceneAssault as i32,
            action_choices: choices,
            ..scope(current, operation)
        }
    }

    /// The first hero attacks `target`. Initiative favours the heroes; nothing else is rolled.
    fn try_start(
        current: &Checkpoint,
        operation: u8,
        target: &str,
        roster: Option<&str>,
    ) -> Result<Checkpoint, RepositoryError> {
        let request = assault_request(current, operation, target, roster);
        assert_eq!(
            wire::action_offer_rejection(
                current,
                native::first(),
                K::SceneAssault,
                &request.offer_id,
                &request.action_choices
            )
            .unwrap(),
            None,
            "the scene offers the assault"
        );
        let input = wire::journey_input(&request, native::first(), current).unwrap();
        // A refusal's step names the first note since the last take; start from a clean slot.
        let _ = refusal_detail::take();
        let mut initiative = [20_u32, 19, 1, 1, 1, 1].into_iter();
        stage_with_supplier(current, &input, &mut |sides| {
            Ok(if sides == 20 {
                initiative.next().expect("initiative faces")
            } else {
                sides.min(3)
            })
        })
    }

    pub(super) fn start(
        current: &Checkpoint,
        operation: u8,
        target: &str,
        roster: Option<&str>,
    ) -> Checkpoint {
        try_start(current, operation, target, roster).unwrap_or_else(|error| {
            panic!(
                "the assault on {target} was refused: {error:?}; step={:?}",
                refusal_detail::take()
            )
        })
    }

    fn member_with_turn(current: &Checkpoint) -> MemberId {
        [native::first(), native::second()]
            .into_iter()
            .find(|member| wire::current_combat_turn(current, *member).unwrap())
            .expect("a hero has the turn")
    }

    fn first_option(current: &Checkpoint, member: MemberId, kind: K) -> Option<String> {
        tactical::groups(current, member, kind)
            .unwrap()
            .first()
            .and_then(|group| group.options.first())
            .map(|option| option.option_id.clone())
    }

    /// One tactical action of the hero whose turn it is. A d20 is a 15 and a damage die at most 3,
    /// so the heroes hit the shop's people and the people mostly miss.
    pub(super) fn act(
        current: &Checkpoint,
        operation: u8,
        kind: K,
        choice: Option<&str>,
    ) -> Checkpoint {
        let member = member_with_turn(current);
        let request = rpc::SubmitActionRequest {
            offer_id: super::super::offer_id(current, kind),
            action_kind: kind as i32,
            action_choices: choice
                .map(|option| {
                    vec![rpc::JourneyChoice {
                        group_id: "tactical-choice".to_owned(),
                        option_id: option.to_owned(),
                    }]
                })
                .unwrap_or_default(),
            ..scope(current, operation)
        };
        assert_eq!(
            wire::action_offer_rejection(
                current,
                member,
                kind,
                &request.offer_id,
                &request.action_choices
            )
            .unwrap(),
            None,
            "{kind:?} is offered to the hero whose turn it is"
        );
        let input = wire::journey_input(&request, member, current).unwrap();
        stage_with_supplier(current, &input, &mut |sides| {
            Ok(if sides == 20 { 15 } else { sides.min(3) })
        })
        .unwrap_or_else(|error| {
            panic!(
                "{kind:?} operation {operation}: {error:?}; step={:?}",
                refusal_detail::take()
            )
        })
    }

    /// Plays the heroes' turns (attack when they can, otherwise close in) until the fight ends.
    fn play_out(mut current: Checkpoint, first_operation: u8) -> Checkpoint {
        for operation in first_operation..first_operation + 100 {
            if phase(&current).unwrap() != rpc::JourneyPhase::Combat {
                return current;
            }
            let member = member_with_turn(&current);
            let battle = tactical::read(current.state()).unwrap();
            current = if !battle.reaction_offers().is_empty() {
                act(&current, operation, K::DeclineReaction, None)
            } else if let Some(option) = first_option(&current, member, K::GreatswordAttack) {
                act(&current, operation, K::GreatswordAttack, Some(&option))
            } else if let Some(option) = closing_move(&current, member) {
                act(&current, operation, K::Move, Some(&option))
            } else {
                act(&current, operation, K::EndTurn, None)
            };
        }
        panic!("the fight did not end in a hundred actions");
    }

    /// The reachable cell nearest a conscious enemy, if the hero has a move left.
    fn closing_move(current: &Checkpoint, member: MemberId) -> Option<String> {
        let battle = tactical::read(current.state()).unwrap();
        let foes = battle
            .combatants()
            .iter()
            .filter(|actor| actor.side() == Side::Enemies && !actor.health().incapacitated())
            .map(|actor| actor.cell())
            .collect::<Vec<_>>();
        tactical::groups(current, member, K::Move)
            .unwrap()
            .first()?
            .options
            .iter()
            .min_by_key(|option| {
                let (column, row) = option.option_id.split_once('-').unwrap();
                let cell = tactical_rules::Cell(column.parse().unwrap(), row.parse().unwrap());
                foes.iter().map(|foe| cell.distance(*foe)).min()
            })
            .map(|option| option.option_id.clone())
    }

    fn gold(current: &Checkpoint) -> Vec<u64> {
        current
            .state()
            .inventory
            .iter()
            .filter(|item| item.origin.entry.as_str() == "gold-piece")
            .map(|item| item.quantity)
            .collect()
    }

    pub(super) fn with_created_shop(run: impl FnOnce()) {
        use crate::gameplay::{
            adventure::{RulesMode, record, shop},
            startup_configuration::ContentRating,
        };
        let generated = shop::generate(42, RulesMode::Standard, ContentRating::default()).unwrap();
        let documents = generated.documents(true).unwrap();
        let (loaded, _) = record::reload(&documents).unwrap();
        pack::with_test_pack(loaded, run);
    }

    fn created_street(legacy: bool) -> Checkpoint {
        use crate::gameplay::{place, place_creation};
        let story = opening();
        let scene = beat_script_runtime::scene(story.state()).unwrap();
        let setting = scene.setting.as_ref().unwrap();
        let mut request = place_creation::tests::request();
        request.origin_id = setting.place_id.clone();
        request.origin_name = setting.name.clone();
        request.time_of_day = setting.model.as_ref().unwrap().time_of_day.clone();
        request.weather = setting.model.as_ref().unwrap().weather.clone();
        let mut record = place_creation::build(
            &request,
            &place_creation::tests::place_slot().to_string(),
            &place_creation::tests::people_slot().to_string(),
            &place_creation::tests::things_slot().to_string(),
        )
        .unwrap();
        if legacy {
            let mut old = serde_json::to_value(&record).unwrap();
            for person in old["people"].as_array_mut().unwrap() {
                person.as_object_mut().unwrap().remove("stat_block");
            }
            record = serde_json::from_value(old).unwrap();
        } else {
            record.people[1].stat_block = Some(place::PersonStatBlock::Guard);
        }
        place::plan(
            &story,
            "Brynn",
            OperationId::from_bytes(&[40; 16]).unwrap(),
            &setting.place_id,
            place::JOURNEY_EXIT,
            &record.place_id,
            Some(&record),
        )
        .map(|planned| planned.candidate)
        .unwrap_or_else(|error| match error {
            place::PlaceFailure::Refused(reason) => panic!("created street refused: {reason:?}"),
            place::PlaceFailure::State(error) => panic!("created street state: {error:?}"),
        })
    }

    /// Exercise the staged roster through the real rules boundary. Enemies pass in this fixture;
    /// hero moves and blows use legal SRD actions, with every draw retained for exact replay.
    fn resolve_created_battle(
        mut battle: Battle,
        replay: Option<&[Vec<ActualDraw>]>,
    ) -> (Battle, Vec<Vec<ActualDraw>>) {
        use tactical_rules::{Action, AttackSpend, AttackUse, DrawBinding, RuleError, Weapon};
        let source = rule().unwrap();
        let mut recorded = Vec::new();
        for sequence in 50..150 {
            if battle.finished() {
                return (battle, recorded);
            }
            let id = battle
                .reaction_offers()
                .first()
                .copied()
                .unwrap_or_else(|| battle.active_actor().unwrap());
            let actor = battle.actor(id).unwrap();
            let usage = AttackUse {
                weapon: Weapon::Greatsword,
                ranged: false,
                hands: 2,
                finesse_dexterity: false,
                knockout: true,
                savage_attacker: false,
                use_mastery: false,
            };
            let action = if !battle.reaction_offers().is_empty() {
                Action::Opportunity { attack: None }
            } else if actor.side() == Side::Enemies || actor.budget().action_used {
                Action::EndTurn
            } else if let Some(target) = battle.legal_targets(id, usage).unwrap().first() {
                Action::Attack {
                    target: *target,
                    weapon: usage,
                    spend: AttackSpend::Action,
                }
            } else {
                let foe = battle
                    .combatants()
                    .iter()
                    .find(|actor| actor.side() == Side::Enemies && !actor.health().incapacitated())
                    .unwrap();
                let nearest = battle
                    .reachable_cells(id)
                    .unwrap()
                    .into_iter()
                    .min_by_key(|cell| cell.distance(foe.cell()))
                    .unwrap();
                if nearest == actor.cell() {
                    Action::EndTurn
                } else {
                    Action::Move(battle.shortest_path(id, nearest, None).unwrap())
                }
            };
            let operation = OperationId::from_bytes(&[sequence; 16]).unwrap();
            let mut draws = replay.map_or_else(Vec::new, |recorded_actions| {
                recorded_actions
                    .get(recorded.len())
                    .expect("recorded action draws")
                    .clone()
            });
            loop {
                let binding = DrawBinding {
                    operation,
                    resolution: ResolutionId::from_bytes(operation.as_bytes()).unwrap(),
                    window: WindowId::from_bytes(operation.as_bytes()).unwrap(),
                    source: &source,
                    next_ordinal: 0,
                };
                match tactical_rules::resolve(&battle, id, &action, binding, &draws) {
                    Ok(resolution) => {
                        battle = resolution.battle;
                        break;
                    }
                    Err(RuleError::MissingDraw { sides }) => {
                        assert!(replay.is_none(), "a replay must not roll fresh dice");
                        draw(operation, sides, &mut draws, &mut |sides| {
                            Ok(if sides == 20 { 18 } else { sides })
                        })
                        .unwrap();
                    }
                    Err(error) => panic!("created person combat action {action:?}: {error:?}"),
                }
            }
            recorded.push(draws);
        }
        panic!("the created-place fight did not resolve in a hundred rules actions");
    }

    #[test]
    fn created_place_people_have_canonical_assault_offers_rosters_and_srd_resource_bounds() {
        with_created_shop(|| {
            let street = created_street(false);
            let state = street.state();
            let scene = beat_script_runtime::scene(state).unwrap();
            assert_eq!(
                offered_targets(&street, native::first()).unwrap(),
                [
                    ("market-street-hobb".to_owned(), "Hobb".to_owned()),
                    ("market-street-wenna".to_owned(), "Wenna".to_owned()),
                ]
            );
            assert!(attackable(state, &scene, "Perrin").unwrap().is_none());
            assert!(attackable(state, &scene, "Marek").unwrap().is_none());
            let hobb = attackable(state, &scene, "Hobb").unwrap().unwrap();
            assert_eq!(hobb.stat_block.as_deref(), Some("commoner"));
            let plan = plan_with(state, &scene, "Hobb", &|_| Attitude::Indifferent).unwrap();
            assert_eq!(ids(&plan.joiners), ["market-street-wenna"]);
            let friendly = plan_with(state, &scene, "Hobb", &|_| Attitude::Friendly).unwrap();
            assert_eq!(ids(&friendly.out), ["market-street-wenna"]);
            let other = plan_with(state, &scene, "Wenna", &|_| Attitude::Hostile).unwrap();
            assert_eq!(ids(&other.fleers), ["market-street-hobb"]);
            let people = combatants_in(state).unwrap();
            assert!(
                people
                    .iter()
                    .any(|(id, source)| id == &hobb.id && source.id == "commoner")
            );
            let bounds = created_resources(state).unwrap();
            assert_eq!(bounds.len(), 2 * 19);
            for (id, block) in [
                ("market-street-hobb", "commoner"),
                ("market-street-wenna", "guard"),
            ] {
                let entity = person_entity(id).unwrap();
                let source = creature_for(Some(block)).unwrap();
                assert_eq!(
                    bounds
                        .iter()
                        .find(|bound| bound.owner == entity
                            && bound.resource.as_str() == "hit-points")
                        .unwrap()
                        .maximum,
                    i64::from(source.rules.hit_points)
                );
            }
            // The same committed place and choices create the same plan without a provider.
            let recovered = native::persisted(&street);
            assert_eq!(created_resources(recovered.state()).unwrap(), bounds);
            assert_eq!(
                plan_with(
                    recovered.state(),
                    &beat_script_runtime::scene(recovered.state()).unwrap(),
                    "Hobb",
                    &|_| Attitude::Indifferent
                )
                .unwrap(),
                plan,
            );
        });
    }

    #[test]
    fn legacy_created_people_stage_commoner_combat_with_deterministic_snapshots() {
        with_created_shop(|| {
            for legacy in [true, false] {
                let street = created_street(legacy);
                let scene = beat_script_runtime::scene(street.state()).unwrap();
                let plan =
                    plan_with(street.state(), &scene, "Wenna", &|_| Attitude::Indifferent).unwrap();
                assert_eq!(ids(&plan.fleers), ["market-street-hobb"]);
                let request = assault_request(&street, 41, &plan.target.id, Some(&plan.roster()));
                let GameInput::Game(command) =
                    wire::journey_input(&request, native::first(), &street).unwrap()
                else {
                    panic!("an assault is a game command");
                };
                let fight = fight_of(&street, &command).unwrap().unwrap();
                assert_eq!(
                    fight.foes[0].creature.id,
                    if legacy { "commoner" } else { "guard" }
                );
                let staged = || {
                    let mut state = street.state().clone();
                    bring_on_stage(&mut state, &fight).unwrap();
                    let mut draws = Vec::new();
                    let mut outcomes = Vec::new();
                    let mut initiative = [20_u32, 19, 1].into_iter();
                    tactical::start_scene(
                        &street,
                        &command,
                        &mut state,
                        &mut draws,
                        &mut outcomes,
                        &mut |sides| {
                            Ok(if sides == 20 {
                                initiative.next().unwrap()
                            } else {
                                1
                            })
                        },
                        "merchant-shop",
                        &fight,
                    )
                    .unwrap();
                    (state, draws, outcomes)
                };
                let (state, draws, outcomes) = staged();
                let replayed = staged();
                assert_eq!(state, replayed.0);
                assert_eq!(draws, replayed.1);
                assert_eq!(outcomes, replayed.2);
                let target = person_entity("market-street-wenna").unwrap();
                assert_eq!(person_name(&state, target).as_deref(), Some("Wenna"));
                assert_eq!(standing_foes(&state).unwrap(), [Person::of(&scene.cast[1])]);
                assert_eq!(
                    presence(&state, "market-street-hobb").unwrap(),
                    Presence::Gone
                );
                let battle = tactical::read(&state).unwrap();
                assert_eq!(battle.combatants().len(), 3);
                assert_eq!(
                    battle
                        .actor(tactical::actor_id(&state, target).unwrap())
                        .unwrap()
                        .spec()
                        .kind
                        .srd_id(),
                    Some(if legacy { "commoner" } else { "guard" })
                );
                assert!(opening_narration(&state).unwrap().contains("Hobb flees"));
                assert!(!opening_narration(&state).unwrap().contains("Marek"));
                // Checkpoint/recovery owners must supply this same canonical inventory. This
                // exercises that boundary without pretending their static inventory is wired.
                let mut inventory = model::recovery().unwrap();
                inventory
                    .resources
                    .extend(created_resources(&state).unwrap());
                let frozen = Checkpoint::new(
                    street.schema(),
                    street.basis(),
                    street.pins().clone(),
                    state.clone(),
                    ReferenceInventory {
                        rules: &inventory.rules,
                        content: &inventory.content,
                        resources: &inventory.resources,
                        assets: &inventory.assets,
                    },
                    inventory.limits,
                )
                .unwrap();
                assert!(encounters_admissible(&frozen).unwrap());
                let limits = df_persistence::NativeCodecLimits {
                    maximum_document_bytes: 1024 * 1024,
                    maximum_allocated_bytes: 2 * 1024 * 1024,
                    maximum_collection_items: 4096,
                    maximum_text_bytes: model::limits().maximum_text_bytes,
                };
                let encoded =
                    df_persistence::local_demo_scope::encode_owned_demo_checkpoint(&frozen, limits)
                        .unwrap();
                let recovered = df_persistence::local_demo_scope::decode_owned_demo_checkpoint(
                    &encoded,
                    frozen.basis(),
                    frozen.pins(),
                    &inventory,
                    limits,
                )
                .unwrap();
                assert_eq!(recovered, frozen);
                assert!(encounters_admissible(&recovered).unwrap());
                let (finished, action_draws) = resolve_created_battle(battle.clone(), None);
                let (replayed, replay_draws) = resolve_created_battle(battle, Some(&action_draws));
                assert!(finished.finished());
                assert_eq!(finished.snapshot().unwrap(), replayed.snapshot().unwrap());
                assert_eq!(action_draws, replay_draws);
                let foe = finished
                    .actor(tactical::actor_id(&state, target).unwrap())
                    .unwrap();
                assert!(foe.health().knocked_out);
                assert!(!foe.health().dead);
            }
        });
    }

    #[test]
    fn an_attack_on_marek_in_the_shop_starts_a_tactical_fight_on_the_shop_grid() {
        with_shop(|| {
            let story = opening();
            let offers = beat_script_runtime::view_offers(&story, native::first()).unwrap();
            assert!(
                offers
                    .iter()
                    .any(|(kind, label, _)| *kind == K::SceneAssault && label == OFFER_LABEL),
                "the freeplay scene offers a fight"
            );
            assert_eq!(
                offered_targets(&story, native::first())
                    .unwrap()
                    .into_iter()
                    .map(|(id, _)| id)
                    .collect::<Vec<_>>(),
                ["merchant", "shop-adventurer", "shop-soldier"]
            );

            let fight = start(&story, 6, "merchant", None);
            let state = fight.state();
            assert_eq!(phase(&fight).unwrap(), rpc::JourneyPhase::Combat);
            assert_eq!(
                session_machine::SessionState::of(&fight).unwrap(),
                session_machine::SessionState::Fight
            );
            let [encounter] = state.encounters.as_slice() else {
                panic!("one encounter")
            };
            assert_eq!(
                encounter.objectives,
                [model::content("scene-npc-assault").unwrap()]
            );
            let marek = person_entity("merchant").unwrap();
            // Marek fights as the target, and Jessa, Indifferent to the aggressor, joins him.
            assert_eq!(
                &encounter.participants[2..],
                [marek, person_entity("shop-soldier").unwrap()]
            );
            // The grid is the shop's, Marek stands on an enemy spawn and the heroes on theirs.
            let saved = tactical::snapshot(state).unwrap();
            assert_eq!(
                saved.scene.as_slice(),
                tactical::scene_bytes("merchant-shop").unwrap()
            );
            assert!(["counter-side", "rack-aisle"].contains(&saved.layout.as_str()));
            let battle = tactical::read(state).unwrap();
            let marek_actor = battle
                .actor(tactical::actor_id(state, marek).unwrap())
                .unwrap();
            assert_eq!(
                marek_actor.spec().kind,
                tactical_rules::CreatureKind::Commoner
            );
            assert!(battle.map().enemy_spawns().contains(&marek_actor.cell()));
            for hero in battle
                .combatants()
                .iter()
                .filter(|actor| actor.side() == Side::Heroes)
            {
                assert!(battle.map().hero_spawns().contains(&hero.cell()));
            }
            // The freeplay beat did not move, and no fight thread was opened.
            assert_eq!(
                state.narrative.active_beats,
                story.state().narrative.active_beats
            );
            assert_eq!(
                state.narrative.completed_beats,
                story.state().narrative.completed_beats
            );
            assert!(state.narrative.open_threads.is_empty());
            // Marek is a world entity with an NpcState, derived from his scene id.
            assert!(state.continuity.npcs.iter().any(|npc| npc.entity == marek));
            assert_eq!(person_name(state, marek).as_deref(), Some("Marek"));
            // Dialogue and the freeplay offers are suspended: only fight actions are on offer.
            let offered = beat_script_runtime::view_offers(&fight, member_with_turn(&fight))
                .unwrap()
                .into_iter()
                .map(|(kind, _, _)| kind)
                .collect::<Vec<_>>();
            assert!(offered.contains(&K::Move));
            assert!(
                !offered
                    .iter()
                    .any(|kind| matches!(kind, K::SceneAssault | K::ExamineHarborSeal))
            );
            // The checkpoint survives a round trip through storage and the views project.
            native::persisted(&fight);
            let view = wire::journey_view(
                &fight,
                df_persistence::local_demo_scope::LocalDemoRole::Player,
                native::first(),
            )
            .unwrap();
            let Some(rpc::view_message::Audience::Player(player)) = view.audience else {
                panic!("player view")
            };
            let journey = player.journey.unwrap();
            assert_eq!(journey.phase, rpc::JourneyPhase::Combat as i32);
            assert!(
                journey
                    .combat
                    .unwrap()
                    .participants
                    .iter()
                    .any(|member| member.name == "Marek")
            );
        });
    }

    #[test]
    fn the_bystander_rule_sends_commoners_away_and_arms_the_watch_by_attitude() {
        use Attitude::{Friendly, Hostile, Indifferent};
        for attitude in [Friendly, Indifferent, Hostile] {
            assert_eq!(stance(Some("commoner"), attitude), Stance::Flee);
            assert_eq!(stance(None, attitude), Stance::Flee);
        }
        assert_eq!(stance(Some("guard"), Indifferent), Stance::Join);
        assert_eq!(stance(Some("guard"), Hostile), Stance::Join);
        assert_eq!(stance(Some("guard"), Friendly), Stance::StayOut);
        assert_eq!(stance(Some("scout"), Indifferent), Stance::StayOut);
        assert_eq!(stance(Some("scout"), Friendly), Stance::StayOut);
        assert_eq!(stance(Some("scout"), Hostile), Stance::Join);
    }

    #[test]
    fn marek_flees_and_jessa_joins_against_the_aggressor_while_corin_stays_out_unless_hostile() {
        with_shop(|| {
            let story = opening();
            let scene = beat_script_runtime::scene(story.state()).unwrap();
            let calm = |_: &pack::SceneCastMember| Attitude::Indifferent;

            // Corin is attacked: Marek flees, Jessa joins, Corin fights as the target.
            let plan = plan_with(story.state(), &scene, "shop-adventurer", &calm).unwrap();
            assert_eq!(plan.target.id, "shop-adventurer");
            assert_eq!(ids(&plan.joiners), ["shop-soldier"]);
            assert_eq!(ids(&plan.fleers), ["merchant"]);
            assert!(plan.out.is_empty());
            let fight = start(&story, 6, &plan.target.id, Some(&plan.roster()));
            let [encounter] = fight.state().encounters.as_slice() else {
                panic!("one encounter")
            };
            let corin = person_entity("shop-adventurer").unwrap();
            let jessa = person_entity("shop-soldier").unwrap();
            let marek = person_entity("merchant").unwrap();
            assert_eq!(&encounter.participants[2..], [corin, jessa]);
            // Marek left the scene: an entity with no place and no resources, never a combatant.
            assert!(!encounter.participants.contains(&marek));
            let gone = fight
                .state()
                .entities
                .iter()
                .find(|world| world.id == marek)
                .expect("Marek has an entity");
            assert_eq!(gone.location, None);
            assert!(
                !fight
                    .state()
                    .resources
                    .iter()
                    .any(|resource| resource.owner == marek)
            );
            assert_eq!(presence(fight.state(), "merchant").unwrap(), Presence::Gone);
            assert_eq!(
                beat_script_runtime::scene(fight.state())
                    .unwrap()
                    .cast
                    .iter()
                    .map(|member| member.id.as_str())
                    .collect::<Vec<_>>(),
                ["shop-adventurer", "shop-soldier"],
                "Marek is gone from the scene"
            );

            // Jessa is attacked: Corin stays out of it unless he is hostile to the aggressor.
            let jessa_plan = plan_with(story.state(), &scene, "shop-soldier", &calm).unwrap();
            assert!(jessa_plan.joiners.is_empty());
            assert_eq!(ids(&jessa_plan.out), ["shop-adventurer"]);
            assert_eq!(ids(&jessa_plan.fleers), ["merchant"]);
            let corin_hostile = plan_with(story.state(), &scene, "shop-soldier", &|member| {
                if member.id == "shop-adventurer" {
                    Attitude::Hostile
                } else {
                    Attitude::Indifferent
                }
            })
            .unwrap();
            assert_eq!(ids(&corin_hostile.joiners), ["shop-adventurer"]);
            // Jessa steps back when she is Friendly to the aggressor.
            let jessa_friendly = plan_with(story.state(), &scene, "shop-adventurer", &|member| {
                if member.id == "shop-soldier" {
                    Attitude::Friendly
                } else {
                    Attitude::Indifferent
                }
            })
            .unwrap();
            assert!(jessa_friendly.joiners.is_empty());
            assert_eq!(ids(&jessa_friendly.out), ["shop-soldier"]);

            // Staging validates the roster it is given: nobody the rule cannot field joins.
            for roster in ["join:merchant", "join:nobody", "out:merchant/flee:merchant"] {
                assert!(
                    try_start(&story, 7, "shop-adventurer", Some(roster)).is_err(),
                    "{roster} is refused"
                );
            }
            // The table is told each stance, and the audit names them.
            let (lines, audit) = announce("Brynn", &plan);
            assert_eq!(
                lines,
                [
                    "Brynn attacks Corin.",
                    "Jessa joins the fight against Brynn.",
                    "Marek flees, shouting for the watch.",
                ]
            );
            assert!(audit.contains("join=[shop-soldier]") && audit.contains("flee=[merchant]"));
        });
    }

    #[test]
    fn the_fight_ends_when_the_foes_are_down_and_the_scene_returns_to_the_same_freeplay_beat_with_no_xp()
     {
        with_shop(|| {
            let story = opening();
            let purses = gold(&story);
            let fight = start(&story, 6, "merchant", None);
            assert_eq!(fight.state().encounters.len(), 1);
            let over = play_out(fight, 7);
            let state = over.state();

            // The same freeplay beat and the same story ledgers, no encounter, no fight thread.
            assert!(state.encounters.is_empty());
            assert!(!tactical::present(state));
            assert_eq!(phase(&over).unwrap(), rpc::JourneyPhase::Opening);
            assert_eq!(
                session_machine::SessionState::of(&over).unwrap(),
                session_machine::SessionState::Story
            );
            assert_eq!(
                state.narrative.active_beats,
                story.state().narrative.active_beats
            );
            assert_eq!(
                state.narrative.completed_beats,
                story.state().narrative.completed_beats
            );
            assert_eq!(
                state.narrative.accepted_facts,
                story.state().narrative.accepted_facts
            );
            assert!(state.narrative.open_threads.is_empty());
            // No experience, no payment.
            assert_eq!(rewards::encounter_experience(state).unwrap(), None);
            assert!(!rewards::paid(state));
            assert_eq!(gold(&over), purses);
            let view = wire::journey_view(
                &over,
                df_persistence::local_demo_scope::LocalDemoRole::Display,
                bootstrap_member().unwrap(),
            )
            .unwrap();
            let Some(rpc::view_message::Audience::Display(display)) = view.audience else {
                panic!("display view")
            };
            assert!(!display.narration.contains("XP"));
            assert_eq!(display.encounter_xp, None);
            // Marek is down for good, and the heroes' turn budgets are fresh.
            assert_eq!(
                presence(state, "merchant").unwrap(),
                Presence::Down { dead: true }
            );
            for character in &state.characters {
                assert_eq!(value(state, character.entity, "action-used").unwrap(), 0);
            }
            // The replay accepts the closed fight, and play goes on in the freeplay scene.
            native::persisted(&over);
            let looked = native::act(&over, native::first(), 90, "look-around");
            assert_eq!(phase(&looked).unwrap(), rpc::JourneyPhase::Opening);
            // Jessa joined Marek's side, and fell with him. A second fight can start in the same
            // scene against Corin, and ends the same way, with the first fight's people still down.
            assert!(matches!(
                presence(state, "shop-soldier").unwrap(),
                Presence::Down { .. }
            ));
            let again = start(&looked, 91, "shop-adventurer", None);
            assert_eq!(phase(&again).unwrap(), rpc::JourneyPhase::Combat);
            let done = play_out(again, 92);
            assert_eq!(phase(&done).unwrap(), rpc::JourneyPhase::Opening);
            assert!(matches!(
                presence(done.state(), "shop-adventurer").unwrap(),
                Presence::Down { .. }
            ));
            assert_eq!(
                presence(done.state(), "merchant").unwrap(),
                Presence::Down { dead: true },
                "the first fight's casualty stays down"
            );
            native::persisted(&done);
        });
    }

    #[test]
    fn a_downed_npc_stays_downed_and_gets_no_reply() {
        with_shop(|| {
            let over = play_out(start(&opening(), 6, "merchant", None), 7);
            let state = over.state();
            assert_eq!(
                presence(state, "merchant").unwrap(),
                Presence::Down { dead: true }
            );
            // Jessa, Indifferent to the aggressor, joined him and fell too.
            assert_eq!(
                presence(state, "shop-soldier").unwrap(),
                Presence::Down { dead: true }
            );
            // They are no longer on stage, so they are no conversation targets and answer no one.
            let scene = beat_script_runtime::scene(state).unwrap();
            assert_eq!(
                scene
                    .cast
                    .iter()
                    .map(|member| member.id.as_str())
                    .collect::<Vec<_>>(),
                ["shop-adventurer"]
            );
            // They cannot be attacked again, and the scene no longer offers them as targets.
            assert_eq!(
                offered_targets(&over, native::first())
                    .unwrap()
                    .into_iter()
                    .map(|(id, _)| id)
                    .collect::<Vec<_>>(),
                ["shop-adventurer"]
            );
            assert!(try_start(&over, 90, "merchant", None).is_err());
            assert!(try_start(&over, 91, "shop-soldier", None).is_err());
            // The scene's narration says where they lie, the same way on every view.
            let told = aftermath_narration(state).unwrap();
            assert!(told.contains("Marek lies dead"), "{told}");
            assert!(told.contains("Jessa lies dead"), "{told}");
            assert!(told.starts_with("The fight is over."), "{told}");
            let view = wire::journey_view(
                &over,
                df_persistence::local_demo_scope::LocalDemoRole::Display,
                bootstrap_member().unwrap(),
            )
            .unwrap();
            let Some(rpc::view_message::Audience::Display(display)) = view.audience else {
                panic!("display view")
            };
            assert!(display.narration.ends_with(&told));
        });
    }

    #[test]
    fn a_won_fight_closes_with_only_current_scene_outcomes() {
        with_shop(|| {
            let story = opening();
            let fight = start(
                &story,
                6,
                "shop-soldier",
                Some("out:shop-adventurer/flee:merchant"),
            );
            let active_state = fight.state().clone();
            let over = play_out(fight, 7);
            let told = aftermath_narration(over.state()).unwrap();

            assert!(told.contains("Marek fled the shop"), "{told}");
            assert!(told.contains("Corin stayed out of the fight"), "{told}");
            assert!(told.contains("Jessa lies dead"), "{told}");
            assert!(told.starts_with("The fight is over."), "{told}");
            assert!(told.split(". ").count() <= 3, "{told}");
            assert!(!told.contains("compares warm cloaks"), "{told}");

            let view = wire::journey_view(
                &over,
                df_persistence::local_demo_scope::LocalDemoRole::Display,
                bootstrap_member().unwrap(),
            )
            .unwrap();
            let Some(rpc::view_message::Audience::Display(display)) = view.audience else {
                panic!("display view")
            };
            assert_eq!(display.narration, told);
            assert!(!display.narration.contains("folds blue linen"));
            assert!(!display.narration.contains("tests a jacket seam"));
            assert!(!display.narration.contains("compares two warm cloaks"));

            let log = crate::gameplay::table_log::TableLog::new();
            crate::gameplay::actor::publish_fight_close(&log, &active_state, over.state());
            // A replay starts from the already closed checkpoint and cannot republish the line.
            crate::gameplay::actor::publish_fight_close(&log, over.state(), over.state());
            // The close is told once, one table row per line.
            let close_lines = log
                .lines()
                .into_iter()
                .map(|line| line.text)
                .collect::<Vec<_>>();
            assert_eq!(close_lines, told.lines().collect::<Vec<_>>());

            let mut knocked_out_state = over.state().clone();
            let jessa = person_entity("shop-soldier").unwrap();
            set(&mut knocked_out_state, jessa, "hit-points", 1).unwrap();
            set(&mut knocked_out_state, jessa, "unconscious", 1).unwrap();
            let knocked_out = aftermath_narration(&knocked_out_state).unwrap();
            assert!(
                knocked_out.contains("Jessa lies knocked out where they fell"),
                "{knocked_out}"
            );
            assert!(
                knocked_out.starts_with("The fight is over."),
                "{knocked_out}"
            );
        });
    }

    #[test]
    fn the_intro_tells_the_fight_start_and_the_ending_leads_with_the_outcome_in_short_lines() {
        with_shop(|| {
            let story = opening();
            let fight = start(&story, 6, "shop-adventurer", None);
            let intro = opening_narration(fight.state()).unwrap();
            assert!(intro.starts_with("A fight begins:"), "{intro}");
            assert!(intro.contains("Corin"), "{intro}");
            assert!(intro.contains("Jessa"), "{intro}");
            assert!(
                intro.contains("Marek flees, shouting for the watch."),
                "{intro}"
            );
            assert!(!intro.contains("You are inside"), "{intro}");

            let over = play_out(fight, 7);
            let told = aftermath_narration(over.state()).unwrap();
            assert!(told.starts_with("The fight is over."), "{told}");
            assert!(told.lines().count() >= 3, "{told}");
            assert!(told.lines().all(|line| line.len() <= 70), "{told}");
            assert!(!told.contains(';'), "{told}");
            assert!(!told.contains("You are inside"), "{told}");
            // No fight open and none fought: neither narration exists.
            assert_eq!(opening_narration(story.state()).unwrap(), "");
        });
    }

    #[test]
    fn foes_who_stand_down_end_the_fight_through_the_same_close_srd_521_page_184() {
        // SRD 5.2.1 p. 184 (Influence): a willing creature complies without a check, a hesitant
        // one when the check succeeds. Either way the foes stop fighting and the fight closes.
        with_shop(|| {
            let fight = start(&opening(), 6, "merchant", None);
            let mut foes = ids(&standing_foes(fight.state()).unwrap())
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            foes.sort();
            assert_eq!(foes, ["merchant", "shop-soldier"]);
            // Only the hero whose turn it is, with an unspent action, can take the Influence
            // action; the other hero can still speak.
            let acting = member_with_turn(&fight);
            let waiting = [native::first(), native::second()]
                .into_iter()
                .find(|member| *member != acting)
                .unwrap();
            assert!(influence_action_available(&fight, acting).unwrap());
            assert!(!influence_action_available(&fight, waiting).unwrap());

            let operation = OperationId::from_bytes(&[77; 16]).unwrap();
            let _ = refusal_detail::take();
            let over = stand_down(&fight, operation).unwrap_or_else(|error| {
                panic!(
                    "the stand-down was refused: {error:?}; step={:?}",
                    refusal_detail::take()
                )
            });
            let state = over.state();
            assert!(state.encounters.is_empty());
            assert!(!open(state));
            assert_eq!(phase(&over).unwrap(), rpc::JourneyPhase::Opening);
            assert_eq!(
                session_machine::SessionState::of(&over).unwrap(),
                session_machine::SessionState::Story
            );
            assert_eq!(
                state.narrative.active_beats,
                fight.state().narrative.active_beats
            );
            assert_eq!(presence(state, "merchant").unwrap(), Presence::Standing);
            assert_eq!(presence(state, "shop-soldier").unwrap(), Presence::Standing);
            for character in &state.characters {
                assert_eq!(value(state, character.entity, "action-used").unwrap(), 0);
            }
            assert!(standing_foes(state).unwrap().is_empty());
            assert!(!influence_action_available(&over, acting).unwrap());

            // The table hears the same short close a won fight gets, once.
            let told = aftermath_narration(state).unwrap();
            assert!(told.contains("Marek remains standing"), "{told}");
            assert!(told.contains("Jessa remains standing"), "{told}");
            assert!(told.starts_with("The fight is over."), "{told}");
            let log = crate::gameplay::table_log::TableLog::new();
            crate::gameplay::actor::publish_fight_close(&log, fight.state(), state);
            crate::gameplay::actor::publish_fight_close(&log, state, state);
            assert_eq!(
                log.lines()
                    .into_iter()
                    .map(|line| line.text)
                    .collect::<Vec<_>>(),
                told.lines().collect::<Vec<_>>()
            );

            // The replay accepts the closed fight, and nothing is left to stand down.
            native::persisted(&over);
            assert!(stand_down(&over, operation).is_err());
            let looked = native::act(&over, native::first(), 90, "look-around");
            assert_eq!(phase(&looked).unwrap(), rpc::JourneyPhase::Opening);
        });
    }

    #[test]
    fn a_stand_down_closes_only_an_open_fight_on_replay() {
        let opening = rpc::JourneyPhase::Opening as i32;
        let combat = rpc::JourneyPhase::Combat as i32;
        assert_eq!(
            replay_step(STAND_DOWN, opening, true).unwrap(),
            Some(Step::Closes)
        );
        assert!(replay_step(STAND_DOWN, combat, true).is_err());
        assert!(replay_step(STAND_DOWN, opening, false).is_err());
    }

    #[test]
    fn witnesses_record_grievances_only_toward_the_aggressor() {
        with_shop(|| {
            let story = opening();
            let scene = beat_script_runtime::scene(story.state()).unwrap();
            let ledger = || {
                let dossiers: Vec<crate::gameplay::adventure::dossier::Dossier> =
                    serde_json::from_slice(SHOP_DOSSIERS).unwrap();
                SocialLedger::open(dossiers, "0x01", None).unwrap()
            };
            let (aggressor, other) = (native::first(), native::second());
            let standing = |ledger: &SocialLedger, member: MemberId, person: &str| {
                let cast = scene.cast.iter().find(|cast| cast.id == person).unwrap();
                let dossier = ledger
                    .dossier_of(scene_role(cast.role), person)
                    .expect("a dossier");
                (
                    ledger.standing(member, dossier).attitude,
                    ledger.grievance_lines(member, dossier).len(),
                    dossier.starting_attitude,
                )
            };

            // Corin is attacked: he, Jessa (joins) and Marek (flees) remember it.
            let book = ledger();
            let plan = plan_with(story.state(), &scene, "shop-adventurer", &|member| {
                attitude_of(&book, aggressor, member)
            })
            .unwrap();
            let recorded = record_consequences(
                &book,
                aggressor,
                "Opening",
                &scene,
                &plan,
                "I attack Corin.",
            );
            assert_eq!(recorded, 3);
            for person in ["shop-adventurer", "shop-soldier", "merchant"] {
                let (attitude, grievances, _) = standing(&book, aggressor, person);
                assert_eq!(attitude, Attitude::Hostile, "{person} toward the aggressor");
                assert_eq!(grievances, 1, "{person} keeps the wrong");
                let (attitude, grievances, starting) = standing(&book, other, person);
                assert_eq!(attitude, starting, "{person} toward the other hero");
                assert_eq!(
                    grievances, 0,
                    "{person} keeps nothing against the other hero"
                );
            }

            // Jessa is attacked in a fresh ledger: Corin stays out and records nothing.
            let fresh = ledger();
            let plan = plan_with(story.state(), &scene, "shop-soldier", &|member| {
                attitude_of(&fresh, aggressor, member)
            })
            .unwrap();
            assert_eq!(ids(&plan.out), ["shop-adventurer"]);
            assert_eq!(
                record_consequences(
                    &fresh,
                    aggressor,
                    "Opening",
                    &scene,
                    &plan,
                    "I attack Jessa."
                ),
                2,
                "Jessa and Marek only"
            );
            let (attitude, grievances, starting) = standing(&fresh, aggressor, "shop-adventurer");
            assert_eq!(attitude, starting);
            assert_eq!(grievances, 0);
        });
    }

    #[test]
    fn a_freeform_strike_on_corin_starts_a_fight() {
        use crate::gameplay::freeform_action::{
            ActKind, Performance, Resolution, StrikeMeans, Target, TargetKind, Verdict,
            fight_target,
        };
        with_shop(|| {
            let story = opening();
            let scene = beat_script_runtime::scene(story.state()).unwrap();
            let resolution = |act: ActKind, kind: TargetKind, id: &str| Resolution {
                hero: "Brynn".to_owned(),
                act: Some(act),
                sense: None,
                means: StrikeMeans::Unarmed,
                target: Some(Target {
                    kind,
                    id: id.to_owned(),
                    name: id.to_owned(),
                    detail: String::new(),
                }),
                spell: None,
                recipient: None,
                notices: Vec::new(),
                verdict: Verdict::Performed(Performance {
                    check: None,
                    succeeded: true,
                    revealed: None,
                }),
                consequences: Vec::new(),
            };
            let calm = |_: &pack::SceneCastMember| false;
            let strike = resolution(ActKind::Strike, TargetKind::Person, "shop-adventurer");
            let corin = fight_target(&strike, &scene, &calm).expect("a strike at Corin is a fight");
            assert_eq!(corin.id, "shop-adventurer");
            // The fight it starts is the real one.
            let plan =
                plan_with(story.state(), &scene, &corin.id, &|_| Attitude::Indifferent).unwrap();
            let fight = start(&story, 6, &plan.target.id, Some(&plan.roster()));
            assert_eq!(phase(&fight).unwrap(), rpc::JourneyPhase::Combat);
            assert!(
                fight.state().encounters[0]
                    .participants
                    .contains(&person_entity("shop-adventurer").unwrap())
            );
            // Rough contact stays one resolved check, unless the person is already hostile.
            let shove = resolution(ActKind::ContactRough, TargetKind::Person, "shop-adventurer");
            assert!(fight_target(&shove, &scene, &calm).is_none());
            assert_eq!(
                fight_target(&shove, &scene, &|_| true).map(|person| person.id.as_str()),
                Some("shop-adventurer")
            );
            // A person with no stat block, a thing and an unjudged act start nothing.
            let mut bare = scene.clone();
            for member in &mut bare.cast {
                member.stat_block = None;
            }
            assert!(fight_target(&strike, &bare, &calm).is_none());
            let thing = resolution(ActKind::Strike, TargetKind::Thing, "counter");
            assert!(fight_target(&thing, &scene, &calm).is_none());
            let mut unjudged = strike.clone();
            unjudged.verdict = Verdict::Unjudged;
            assert!(fight_target(&unjudged, &scene, &calm).is_none());
        });
    }

    #[test]
    fn closing_a_fight_with_every_hero_down_leaves_them_stable_and_tells_who_took_them() {
        with_shop(|| {
            let story = opening();
            let fight = start(&story, 6, "shop-adventurer", None);
            let mut state = fight.state().clone();
            // Every hero drops to 0 hit points and Corin is still on his feet.
            let mut snapshot: serde_json::Value =
                serde_json::from_slice(&state.encounters[0].tactical.as_ref().unwrap().state)
                    .unwrap();
            let heroes = state
                .characters
                .iter()
                .map(|character| character.entity)
                .collect::<Vec<_>>();
            for actor in snapshot["actors"].as_array_mut().unwrap() {
                let id = actor["spec"]["id"].as_u64().unwrap();
                if id <= 2 {
                    actor["health"]["hit_points"] = serde_json::json!(0);
                    actor["health"]["unconscious"] = serde_json::json!(true);
                    actor["health"]["prone"] = serde_json::json!(true);
                }
            }
            for hero in &heroes {
                set(&mut state, *hero, "hit-points", 0).unwrap();
                set(&mut state, *hero, "unconscious", 1).unwrap();
                set(&mut state, *hero, "action-used", 1).unwrap();
            }
            state.encounters[0].tactical.as_mut().unwrap().state =
                serde_json::to_vec(&snapshot).unwrap();
            let battle = tactical::read(&state).unwrap();
            assert!(battle.finished(), "no hero is conscious");
            close(&mut state, &battle).unwrap();

            assert!(state.encounters.is_empty());
            for hero in &heroes {
                assert_eq!(value(&state, *hero, "hit-points").unwrap(), 0);
                assert_eq!(value(&state, *hero, "unconscious").unwrap(), 1);
                assert_eq!(value(&state, *hero, "action-used").unwrap(), 0);
                assert!(!hero_dead(&state, *hero), "stable, not dead");
            }
            // Marek fled to call the watch, so the watch arrives; the heroes stay where they fell.
            let told = aftermath_narration(&state).unwrap();
            assert!(
                told.lines().any(|line| line == "Marek fled the shop."),
                "{told}"
            );
            assert!(
                told.contains("lies unconscious and stable where they fell"),
                "{told}"
            );
            assert!(
                told.ends_with("The watch arrives and stands guard over the fallen heroes."),
                "{told}"
            );
            assert!(told.contains("Brynn lies unconscious and stable"), "{told}");
            assert!(told.contains("Vale lies unconscious and stable"), "{told}");
            let log = crate::gameplay::table_log::TableLog::new();
            crate::gameplay::actor::publish_fight_close(&log, fight.state(), &state);
            let lines = log
                .lines()
                .into_iter()
                .map(|line| line.text)
                .collect::<Vec<_>>();
            assert_eq!(lines, told.lines().collect::<Vec<_>>());
        });
    }

    #[test]
    fn a_pack_with_scene_assaults_is_freeplay_with_a_roster_free_battle_scene_and_people_to_fight()
    {
        let tuning = |edit: &dyn Fn(&mut serde_json::Value)| {
            let mut value: serde_json::Value = serde_json::from_slice(SHOP_TUNING).unwrap();
            edit(&mut value);
            serde_json::to_vec(&value).unwrap()
        };
        assert!(pack::load_from_bytes(SHOP_PACK, SHOP_TUNING).is_ok());
        // A roster would make random encounters possible.
        let with_roster = tuning(&|value| {
            value["tactical"]["enemies"] =
                serde_json::json!([{"srd_id": "bandit", "count": 1, "ammunition": 0}]);
        });
        assert!(pack::load_from_bytes(SHOP_PACK, &with_roster).is_err());
        // The battle scene must be a grid the server holds.
        let nowhere = tuning(&|value| {
            value["tactical"]["scene_assault"]["battle_scene"] = serde_json::json!("nowhere");
        });
        assert!(pack::load_from_bytes(SHOP_PACK, &nowhere).is_err());
        // A plain roster block stays forbidden in freeplay.
        let plain = tuning(&|value| {
            value["tactical"] =
                serde_json::json!({"enemies": [{"srd_id": "bandit", "count": 1, "ammunition": 0}]});
        });
        assert!(pack::load_from_bytes(SHOP_PACK, &plain).is_err());
        // A quest pack cannot take the scene-assault variant.
        let mut harbor: serde_json::Value = serde_json::from_slice(HARBOR_TUNING).unwrap();
        harbor["tactical"] =
            serde_json::json!({"scene_assault": {"battle_scene": "merchant-shop"}});
        assert!(pack::load_from_bytes(HARBOR_PACK, &serde_json::to_vec(&harbor).unwrap()).is_err());
        // And a pack with nobody who can be fought has no use for it.
        let mut content: serde_json::Value = serde_json::from_slice(SHOP_PACK).unwrap();
        for scene in content["scenes"].as_array_mut().unwrap() {
            for member in scene["cast"].as_array_mut().into_iter().flatten() {
                member.as_object_mut().unwrap().remove("stat_block");
            }
        }
        assert!(
            pack::load_from_bytes(&serde_json::to_vec(&content).unwrap(), SHOP_TUNING).is_err()
        );
        // The tactical engine is on, with no roster and so no random encounter.
        with_shop(|| {
            assert!(tactical::enabled());
            assert!(tactical::scene_assault_only());
            assert!(tactical::enemies().unwrap().is_empty());
            // Character creation is the pack's own, not the tactical build packages.
            let offer = wire::creation_offer().unwrap();
            let groups = offer
                .groups
                .iter()
                .map(|group| group.group_id.as_str())
                .collect::<Vec<_>>();
            assert!(!groups.contains(&"build-package"));
            assert!(
                ["array", "equipment", "style-masteries"]
                    .iter()
                    .all(|group| groups.contains(group))
            );
            // Nothing offers a fight before the story has begun.
            let room = native::prepared();
            assert!(offered_targets(&room, native::first()).unwrap().is_empty());
        });
    }
}

#[cfg(test)]
#[path = "scene_fight_tests.rs"]
mod scene_fight_tests;
