//! K1: kill-shot videos for the battle phase.
//!
//! When a fight's committed outcome is a killing blow, one photographic still of
//! that exact blow is composed by the imagery pipeline and one 5 s Kling clip is
//! made from it. Cosmetic only: it never changes an outcome or a timing, never
//! delays a line, the fight's close or the next scene, and a pending clip shows
//! nothing. Generation is on first need only, after the blow is committed;
//! nothing speculative is made before it.
//!
//! This module is the pure part: the trigger, the prompts (committed facts
//! only), the audit line and the one-shot rule. The clip engine, the hub key
//! and the view live in `home_video` / `video_clip_spec`; the still itself is
//! an imagery `ScenePlate` request (so the photo check judges it like any
//! plate) built from [`still_prompt`] and the references in [`StillPlan`].

use sha2::{Digest, Sha256};
use std::collections::HashSet;

use super::super::imagery::FILM_LOOK;
use super::super::startup_configuration::ContentRating;

/// The single-frame wording of `imagery::prompt::SINGLE_FRAME`. That constant
/// is private to the imagery module today; this copy keeps the exact wording
/// until imagery re-exports it (see the lane request).
pub(in crate::gameplay) const SINGLE_FRAME: &str = "The output is ONE single continuous photographic camera frame, one uninterrupted \
    view of one place at one moment: never a grid, collage, contact sheet, split screen, panels, \
    diptych or montage, and never subjects pasted onto a background.";

const MATURE_STANCE: &str = "Mature content rating: violence, horror and desire are shown as \
    the story demands, not softened.";
const RESTRAINED_LIMITS: &str = "No gore, no nudity, no sexual content.";
const NO_TEXT: &str = "No text, letters, numbers, signage, signs, logos or watermarks anywhere, \
    including on cloth, boards, banners and walls; any such surface is plain and blank.";
const MAXIMUM_FACT_CHARACTERS: usize = 80;

/// The committed facts of one blow, as the rules left them. Nothing here is
/// player text: names, weapon, damage type and place come from the committed
/// outcome and the fight record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct CommittedBlow {
    pub killer: String,
    pub victim: String,
    pub weapon: String,
    pub damage_type: String,
    pub place: String,
    /// The combat outcome's sequence; part of the kill's identity.
    pub sequence: u64,
    pub hit: bool,
    pub damage: u32,
    pub target_hit_points: u32,
    /// The victim is dead (not merely at 0 hit points).
    pub victim_dead: bool,
    /// The victim was knocked out instead of killed.
    pub knocked_out: bool,
}

/// Why a committed blow gets no kill shot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum Skip {
    NotAHit,
    NoDamage,
    TargetNotAtZero,
    KnockedOut,
    DownedNotDead,
    MissingFact,
}

impl Skip {
    pub(in crate::gameplay) const fn label(self) -> &'static str {
        match self {
            Self::NotAHit => "not_a_hit",
            Self::NoDamage => "no_damage",
            Self::TargetNotAtZero => "target_not_at_zero",
            Self::KnockedOut => "knocked_out",
            Self::DownedNotDead => "downed_not_dead",
            Self::MissingFact => "missing_fact",
        }
    }
}

/// One requested kill shot: the blow, its deterministic identity and its epoch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct KillShotRequest {
    pub id: String,
    pub killer: String,
    pub victim: String,
    pub weapon: String,
    pub damage_type: String,
    pub place: String,
    pub sequence: u64,
    pub epoch: u64,
}

fn clean_fact(text: &str) -> Option<String> {
    let cleaned: String = text
        .trim()
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let cleaned = cleaned.trim().to_owned();
    if cleaned.is_empty() {
        return None;
    }
    Some(cleaned.chars().take(MAXIMUM_FACT_CHARACTERS).collect())
}

/// The deterministic identity of a blow: same blow, same id, so a replayed
/// view reuses its clip and the client never plays it twice.
pub(in crate::gameplay) fn kill_id(blow: &CommittedBlow) -> String {
    let digest = Sha256::digest(
        format!(
            "{}|{}|{}|{}|{}|{}",
            blow.killer, blow.victim, blow.weapon, blow.damage_type, blow.place, blow.sequence
        )
        .as_bytes(),
    );
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("kill-{}-{hex}", blow.sequence)
}

/// The trigger. Only a committed killing blow qualifies: a hit that dealt
/// damage, left the victim at 0 hit points, dead and not knocked out. A
/// knocked-out foe, a downed hero at 0 hit points who is not dead, a miss and
/// a blow that leaves hit points all get nothing.
pub(in crate::gameplay) fn decide(
    blow: &CommittedBlow,
    epoch: u64,
) -> Result<KillShotRequest, Skip> {
    if !blow.hit {
        return Err(Skip::NotAHit);
    }
    if blow.damage == 0 {
        return Err(Skip::NoDamage);
    }
    if blow.knocked_out {
        return Err(Skip::KnockedOut);
    }
    if blow.target_hit_points != 0 {
        return Err(Skip::TargetNotAtZero);
    }
    if !blow.victim_dead {
        return Err(Skip::DownedNotDead);
    }
    let killer = clean_fact(&blow.killer).ok_or(Skip::MissingFact)?;
    let victim = clean_fact(&blow.victim).ok_or(Skip::MissingFact)?;
    let weapon = clean_fact(&blow.weapon).ok_or(Skip::MissingFact)?;
    let damage_type = clean_fact(&blow.damage_type).ok_or(Skip::MissingFact)?;
    let place = clean_fact(&blow.place).ok_or(Skip::MissingFact)?;
    Ok(KillShotRequest {
        id: kill_id(blow),
        killer,
        victim,
        weapon,
        damage_type,
        place,
        sequence: blow.sequence,
        epoch,
    })
}

fn rating_stance(rating: ContentRating) -> &'static str {
    match rating {
        ContentRating::Mature => MATURE_STANCE,
        ContentRating::Restrained => RESTRAINED_LIMITS,
    }
}

/// The still's prompt, from the committed facts only: who struck, with what,
/// the strike landing, the victim falling, in the place. Photographic wording
/// only (`FILM_LOOK`, `SINGLE_FRAME`, the rating stance as the other plate
/// prompts state it); it never names another medium.
pub(in crate::gameplay) fn still_prompt(
    request: &KillShotRequest,
    rating: ContentRating,
) -> String {
    format!(
        "{FILM_LOOK} {SINGLE_FRAME} In {place}, {killer} lands a killing blow on {victim} with {weapon}: \
         the strike is landing now and {victim} is falling, struck by {damage_type} damage. \
         Keep the exact faces, hair, clothing and equipment of the reference photographs. \
         {NO_TEXT} {stance}",
        place = request.place,
        killer = request.killer,
        victim = request.victim,
        weapon = request.weapon,
        damage_type = request.damage_type,
        stance = rating_stance(rating),
    )
}

/// How the still is composed: the killer's and the victim's live canonical
/// pictures (same identities as the fight) and the place's canonical room
/// photograph, as imagery references, rendered as a scene plate so the photo
/// check judges it like any plate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct StillPlan {
    pub prompt: String,
    /// Reference roles in order: killer canonical, victim canonical, room.
    pub reference_roles: [&'static str; 3],
    /// Imagery kind label; `scene_plate` routes through the photo check.
    pub kind_label: &'static str,
    pub width: u32,
    pub height: u32,
}

pub(in crate::gameplay) fn still_plan(
    request: &KillShotRequest,
    rating: ContentRating,
) -> StillPlan {
    StillPlan {
        prompt: still_prompt(request, rating),
        reference_roles: [
            "killer_canonical_photo",
            "victim_canonical_photo",
            "place_canonical_room_photo",
        ],
        kind_label: "scene_plate",
        width: 2560,
        height: 1440,
    }
}

/// The clip's motion prompt: short, real time, from the still. The blow lands
/// and the victim falls; the camera stays nearly still; no cuts, no new
/// people, no new objects or text.
pub(in crate::gameplay) fn motion_prompt(request: &KillShotRequest) -> String {
    format!(
        "Real-time motion from this single frame: {killer}'s {weapon} strike lands and {victim} falls where they stood in {place}. \
         Camera nearly still: no pan, no zoom, no reframing and no cuts. Keep every face, hair, clothing, body and the place unchanged. \
         No one enters or leaves; add no people, creatures, objects, signs or text.",
        killer = request.killer,
        weapon = request.weapon,
        victim = request.victim,
        place = request.place,
    )
}

fn audit_value(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_whitespace() || c.is_control() || c == '"' {
                '_'
            } else {
                c
            }
        })
        .take(64)
        .collect()
}

/// One `KILL_SHOT` audit line.
pub(in crate::gameplay) fn audit_line(
    status: &str,
    killer: &str,
    victim: &str,
    elapsed_ms: u128,
    reason: &str,
) -> String {
    format!(
        "KILL_SHOT status={status} killer={} victim={} elapsed_ms={elapsed_ms} reason={}",
        audit_value(killer),
        audit_value(victim),
        audit_value(reason),
    )
}

/// The view's kill shot for one request: paths and names, with the clip path
/// present only when the clip is ready. A pending clip carries no path, so
/// the client shows nothing for it.
pub(in crate::gameplay) fn view_projection(
    request: &KillShotRequest,
    clip_path: Option<String>,
    state: &str,
    still_path: &str,
) -> super::super::rpc::KillShot {
    let state = match state {
        "ready" => super::super::rpc::HomeVideoState::Ready,
        "failed" => super::super::rpc::HomeVideoState::Failed,
        "pending" => super::super::rpc::HomeVideoState::Pending,
        _ => super::super::rpc::HomeVideoState::None,
    };
    super::super::rpc::KillShot {
        clip_path: if state == super::super::rpc::HomeVideoState::Ready {
            clip_path.unwrap_or_default()
        } else {
            String::new()
        },
        still_path: still_path.to_owned(),
        killer: request.killer.clone(),
        victim: request.victim.clone(),
        state: state as i32,
        id: request.id.clone(),
        sequence: request.sequence,
        epoch: request.epoch,
    }
}

/// The one-shot rule: a kill shot plays once per (id, epoch). A replayed view,
/// a reconnect or a repeated snapshot with the same pair never plays again; a
/// new epoch (a new fight presentation) may show a new blow.
#[derive(Clone, Debug, Default)]
pub(in crate::gameplay) struct ShownShots {
    shown: HashSet<(String, u64)>,
}

impl ShownShots {
    pub(in crate::gameplay) fn new() -> Self {
        Self::default()
    }

    /// `true` the first time this (id, epoch) is claimed, `false` after.
    pub(in crate::gameplay) fn claim(&mut self, id: &str, epoch: u64) -> bool {
        self.shown.insert((id.to_owned(), epoch))
    }

    pub(in crate::gameplay) fn was_shown(&self, id: &str, epoch: u64) -> bool {
        self.shown.contains(&(id.to_owned(), epoch))
    }
}
