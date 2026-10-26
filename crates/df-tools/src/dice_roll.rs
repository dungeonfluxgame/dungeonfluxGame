//! Which die faces the display shows for a server-resolved d20 test, which clip plays
//! it, and for how long.
//!
//! The browser never decides an outcome: the server's kept face (`attack_die`) is the only
//! number a die settles on, and a second die appears only when the server reports two draws
//! (advantage or disadvantage: `roll_mode` -1/+1 with two `faces`). This module turns one
//! outcome into that display plan plus its timing; the DOM/CSS layer owns the playback.
//! Clip files and the resting-face geometry come from the `dice-rolls.json` manifest the
//! server routes (see `gameplay::startup_configuration::load_dice_assets`); clip choice is
//! a hash of the server's own numbers, so a replay shows the same clip.
use df_observe::presentation::{ComponentPhase, PresentationComponent};
use df_protocol::common as rpc;

/// The live cue reads the accepted view, which can precede the battlefield redraw/cache.
pub(crate) fn live_attack(combat: &rpc::CombatView, sequence: u64) -> Option<&rpc::CombatOutcome> {
    combat.outcomes.iter().find(|outcome| {
        sequence != 0
            && outcome.sequence == sequence
            && matches!(
                rpc::OutcomeKind::try_from(outcome.kind),
                Ok(rpc::OutcomeKind::Attack
                    | rpc::OutcomeKind::OpportunityAttack
                    | rpc::OutcomeKind::Unspecified)
            )
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AttackDisplayAction {
    OpenDie,
    SettleDie,
    OpenCard,
    ReadCard,
    OpenDamage,
    SettleDamage,
    Close,
    Hide,
}

impl AttackDisplayAction {
    pub(crate) fn records(
        self,
        damage_dice: bool,
    ) -> Vec<(PresentationComponent, ComponentPhase, &'static str)> {
        use AttackDisplayAction::*;
        use ComponentPhase::*;
        use PresentationComponent::{Dice, Die, RollCard};
        match self {
            OpenDie => vec![(Die, Opened, "attack-rolled")],
            SettleDie => vec![(Die, Readable, "attack-settled")],
            OpenCard => vec![(RollCard, Opened, "face-dwell-complete")],
            ReadCard => vec![(RollCard, Readable, "attack-breakdown")],
            OpenDamage => vec![(Dice, Opened, "damage-rolled")],
            SettleDamage => vec![(Dice, Readable, "damage-settled")],
            Close | Hide => {
                let (phase, die, card, damage) = if self == Close {
                    (
                        Closing,
                        "attack-clearing",
                        "read-complete",
                        "damage-clearing",
                    )
                } else {
                    (Hidden, "attack-cleared", "card-closed", "damage-cleared")
                };
                let mut records = vec![(Die, phase, die), (RollCard, phase, card)];
                if damage_dice {
                    records.push((Dice, phase, damage));
                }
                records
            }
        }
    }
}

/// Executed by the browser's ordinary attack presenter and by native committed-outcome tests.
/// Times are offsets from opening the die; all values and cards come from the server outcome.
pub(crate) struct AttackDisplay {
    pub(crate) play: DicePlay,
    pub(crate) damage_faces: Vec<u32>,
    pub(crate) attack_card: df_render::battle::RollCard,
    pub(crate) damage_card: df_render::battle::RollCard,
    pub(crate) timing: AttackTiming,
}

impl AttackDisplay {
    pub(crate) fn new(outcome: &rpc::CombatOutcome) -> Option<Self> {
        let play = play_of(outcome.attack_die, &outcome.faces, outcome.roll_mode)?;
        let damage_faces = if !outcome.hit && !outcome.grazed {
            Vec::new()
        } else if outcome.damage_rolls.is_empty() {
            outcome.damage_dice.clone()
        } else {
            outcome
                .damage_rolls
                .iter()
                .filter(|roll| roll.kept)
                .flat_map(|roll| roll.dice.iter().copied())
                .collect()
        };
        Some(Self {
            play,
            timing: attack_timing(outcome.hit || outcome.grazed, !damage_faces.is_empty()),
            damage_faces,
            attack_card: committed_roll_card(outcome, false),
            damage_card: committed_roll_card(outcome, true),
        })
    }

    pub(crate) fn steps(&self) -> Vec<(u64, AttackDisplayAction)> {
        use AttackDisplayAction::*;
        let timing = self.timing;
        let mut steps = vec![
            (0, OpenDie),
            (timing.settled_ms, SettleDie),
            (timing.card_ms, OpenCard),
        ];
        if timing.damage_settled_ms.is_some() {
            steps.push((timing.card_ms, OpenDamage));
        }
        steps.push((timing.card_ms + 350, ReadCard));
        if let Some(at) = timing.damage_settled_ms {
            steps.push((at, SettleDamage));
        }
        steps.extend([(timing.closing_ms, Close), (timing.hidden_ms, Hide)]);
        steps.sort_by_key(|(at, _)| *at);
        steps
    }
}

pub(crate) fn committed_roll_card(
    outcome: &rpc::CombatOutcome,
    include_damage: bool,
) -> df_render::battle::RollCard {
    let faces = if outcome.faces.is_empty() {
        outcome.attack_die.to_string()
    } else {
        outcome
            .faces
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(" / ")
    };
    let label = outcome
        .display_metadata
        .as_ref()
        .map(|metadata| metadata.ability_name.clone())
        .filter(|name| !name.is_empty())
        .or_else(|| (!outcome.weapon.is_empty()).then(|| outcome.weapon.clone()))
        .unwrap_or_else(|| "Attack".to_owned());
    let metadata = outcome.display_metadata.as_ref();
    let parts = metadata.map(|metadata| {
        [
            ("ability", metadata.ability_modifier),
            ("proficiency", metadata.proficiency_modifier),
            ("other", metadata.other_modifier),
        ]
    });
    let bonus =
        if let Some(parts) = parts.filter(|parts| parts.iter().all(|(_, value)| value.is_some())) {
            parts
                .iter()
                .filter_map(|(name, value)| value.map(|value| format!(" {value:+} {name}")))
                .collect::<String>()
        } else {
            format!(" {:+} bonus", outcome.attack_modifier)
        };
    let mut explanation = format!(
        "{} d20 {}{} = {} vs {} AC {}",
        outcome.actor_name,
        faces,
        bonus,
        outcome.attack_total,
        outcome.target_name,
        outcome.target_armor_class
    );
    if let Some(metadata) = metadata {
        if let Some(parts) = parts.filter(|parts| parts.iter().any(|(_, value)| value.is_none())) {
            for (name, value) in parts {
                if let Some(value) = value {
                    explanation.push_str(&format!(" | {name} {value:+}"));
                }
            }
        }
        let attack_type = match metadata.attack_type {
            1 => "melee",
            2 => "ranged",
            3 => "thrown",
            _ => "",
        };
        if !attack_type.is_empty() {
            explanation.push_str(&format!(" | {attack_type}"));
        }
        if let Some(mastery) = &metadata.mastery {
            explanation.push_str(&format!(" | mastery: {mastery}"));
        }
    }
    if !outcome.reasons.is_empty() {
        explanation.push_str(&format!(" | {}", outcome.reasons.join("; ")));
    }
    if include_damage && (outcome.hit || outcome.grazed) {
        let faces: Vec<_> = if outcome.damage_rolls.is_empty() {
            outcome.damage_dice.clone()
        } else {
            outcome
                .damage_rolls
                .iter()
                .filter(|roll| roll.kept)
                .flat_map(|roll| roll.dice.iter().copied())
                .collect()
        };
        let damage_faces = faces
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(" + ");
        explanation.push_str(&format!(
            " | damage {} {:+} = {} {}",
            damage_faces, outcome.damage_modifier, outcome.damage, outcome.damage_type
        ));
    }
    let verdict = if outcome.hit && outcome.critical {
        "CRITICAL HIT"
    } else if outcome.hit {
        "HIT"
    } else if outcome.grazed {
        "GRAZE"
    } else if outcome.attack_die == 1 {
        "FUMBLE"
    } else {
        "MISS"
    };
    df_render::battle::RollCard {
        value: outcome.attack_total.to_string(),
        label: format!(
            "{} {} -> {}",
            metadata
                .and_then(|metadata| metadata.icon.as_deref())
                .unwrap_or(""),
            label,
            outcome.target_name
        ),
        explanation,
        verdict: verdict.to_owned(),
        tone: if outcome.hit { "hit" } else { "miss" }.to_owned(),
    }
}

/// A complete server technique owns its single dice seat. Partial/legacy metadata leaves the
/// ordinary clip presenter in charge so an incomplete optional extension cannot hide a roll.
pub(crate) fn technique_owns_roll(technique: &rpc::BattleTechnique) -> bool {
    use rpc::BattleTechniqueKind;
    let count = technique.strikes.len();
    let supported = match BattleTechniqueKind::try_from(technique.kind) {
        Ok(
            BattleTechniqueKind::BattleTechniqueFireBolt
            | BattleTechniqueKind::BattleTechniqueShield,
        ) => count == 1,
        Ok(BattleTechniqueKind::BattleTechniqueMagicMissile) => count == 3,
        Ok(BattleTechniqueKind::BattleTechniqueFlurry) => (1..=4).contains(&count),
        Ok(BattleTechniqueKind::BattleTechniqueFireball) => (1..=16).contains(&count),
        _ => false,
    };
    supported && technique.strikes.iter().all(|strike| strike.roll.is_some())
}

/// The display's dice live under this path, extensionless like every other extra asset.
pub(crate) const DICE_URL_PREFIX: &str = "/assets/ui/generated/dice/";
pub(crate) const FALLBACK_CLIP_COUNT: usize = 3;

/// One roll clip from the manifest: file names, the rest time the number and the land
/// cue follow, and the upward face geometry in 0-to-1 coordinates of the square.
#[derive(Clone, Debug)]
pub(crate) struct DiceClip {
    pub(crate) webm: String,
    #[cfg(target_arch = "wasm32")]
    pub(crate) key_mp4: String,
    #[cfg(target_arch = "wasm32")]
    pub(crate) rest_image: String,
    pub(crate) rest_ms: u64,
    #[cfg(target_arch = "wasm32")]
    pub(crate) face_center: [f64; 2],
    #[cfg(target_arch = "wasm32")]
    pub(crate) face_corners: [[f64; 2]; 3],
    #[cfg(target_arch = "wasm32")]
    pub(crate) face_rotation_deg: f64,
}

/// The manifest the server routes at `<DICE_URL_PREFIX>dice-rolls`.
#[derive(Clone, Debug)]
pub(crate) struct DiceManifest {
    pub(crate) clips: Vec<DiceClip>,
    pub(crate) key_color: String,
}

/// Reads the manifest; `None` when it is missing or names no playable clip, in
/// which case the display falls back to the roll card and never waits. Parsed by hand:
/// `serde_json` is a native-only dependency (another worker owns `Cargo.toml`), and this
/// module compiles for the browser too.
pub(crate) fn parse_manifest(text: &str) -> Option<DiceManifest> {
    let value = Json::parse(text)?;
    let clips = value.field("clips")?.array()?;
    let mut parsed = Vec::with_capacity(clips.len());
    for clip in clips {
        let key_mp4 = clip.field("key_mp4")?.string()?;
        let rest_image = clip.field("rest_image")?.string()?;
        let face_center = point(clip.field("face_center")?)?;
        let face_corners = {
            let corners = clip.field("face_corners")?.array()?;
            if corners.len() != 3 {
                return None;
            }
            [
                point(&corners[0])?,
                point(&corners[1])?,
                point(&corners[2])?,
            ]
        };
        let face_rotation_deg = clip.field("face_rotation_deg")?.number()?;
        #[cfg(not(target_arch = "wasm32"))]
        let _ = (
            &key_mp4,
            &rest_image,
            face_center,
            face_corners,
            face_rotation_deg,
        );
        parsed.push(DiceClip {
            webm: clip.field("webm")?.string()?,
            #[cfg(target_arch = "wasm32")]
            key_mp4,
            #[cfg(target_arch = "wasm32")]
            rest_image,
            rest_ms: {
                let rest_ms = clip.field("rest_ms")?.number()?;
                if rest_ms <= 0.0 || rest_ms.fract() != 0.0 || rest_ms > u64::MAX as f64 {
                    return None;
                }
                rest_ms as u64
            },
            #[cfg(target_arch = "wasm32")]
            face_center,
            #[cfg(target_arch = "wasm32")]
            face_corners,
            #[cfg(target_arch = "wasm32")]
            face_rotation_deg,
        });
    }
    if parsed.is_empty() || parsed.iter().any(|clip| clip.rest_ms == 0) {
        return None;
    }
    Some(DiceManifest {
        clips: parsed,
        key_color: value.field("key_color")?.string()?,
    })
}

fn point(value: &Json) -> Option<[f64; 2]> {
    let pair = value.array()?;
    if pair.len() != 2 {
        return None;
    }
    Some([pair[0].number()?, pair[1].number()?])
}

/// The manifest's JSON, parsed by hand: objects keep their members in order, numbers
/// land on `f64`, and anything malformed (including trailing garbage) is `None`.
#[derive(Debug)]
enum Json {
    Null,
    Number(f64),
    Text(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    fn parse(text: &str) -> Option<Self> {
        let mut cursor = Cursor {
            bytes: text.as_bytes(),
            position: 0,
        };
        let value = cursor.value()?;
        cursor.whitespace();
        if cursor.position != cursor.bytes.len() {
            return None;
        }
        Some(value)
    }

    fn field(&self, name: &str) -> Option<&Self> {
        match self {
            Self::Object(members) => members
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    fn array(&self) -> Option<&[Self]> {
        match self {
            Self::Array(items) => Some(items),
            _ => None,
        }
    }

    fn number(&self) -> Option<f64> {
        match self {
            Self::Number(number) => Some(*number),
            _ => None,
        }
    }

    fn string(&self) -> Option<String> {
        match self {
            Self::Text(text) => Some(text.clone()),
            _ => None,
        }
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl Cursor<'_> {
    fn whitespace(&mut self) {
        while self.position < self.bytes.len() && self.bytes[self.position].is_ascii_whitespace() {
            self.position += 1;
        }
    }

    fn literal(&mut self, word: &str) -> bool {
        let end = self.position + word.len();
        if self.bytes.get(self.position..end) == Some(word.as_bytes()) {
            self.position = end;
            true
        } else {
            false
        }
    }

    fn value(&mut self) -> Option<Json> {
        self.whitespace();
        match self.bytes.get(self.position)? {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => Some(Json::Text(self.text()?)),
            b't' => self.literal("true").then_some(Json::Null),
            b'f' => self.literal("false").then_some(Json::Null),
            b'n' => self.literal("null").then_some(Json::Null),
            _ => Some(Json::Number(self.number()?)),
        }
    }

    fn object(&mut self) -> Option<Json> {
        self.position += 1;
        let mut members = Vec::new();
        self.whitespace();
        if self.bytes.get(self.position) == Some(&b'}') {
            self.position += 1;
            return Some(Json::Object(members));
        }
        loop {
            self.whitespace();
            if self.bytes.get(self.position) != Some(&b'"') {
                return None;
            }
            let key = self.text()?;
            self.whitespace();
            if self.bytes.get(self.position) != Some(&b':') {
                return None;
            }
            self.position += 1;
            members.push((key, self.value()?));
            self.whitespace();
            match self.bytes.get(self.position)? {
                b',' => self.position += 1,
                b'}' => {
                    self.position += 1;
                    return Some(Json::Object(members));
                }
                _ => return None,
            }
        }
    }

    fn array(&mut self) -> Option<Json> {
        self.position += 1;
        let mut items = Vec::new();
        self.whitespace();
        if self.bytes.get(self.position) == Some(&b']') {
            self.position += 1;
            return Some(Json::Array(items));
        }
        loop {
            items.push(self.value()?);
            self.whitespace();
            match self.bytes.get(self.position)? {
                b',' => self.position += 1,
                b']' => {
                    self.position += 1;
                    return Some(Json::Array(items));
                }
                _ => return None,
            }
        }
    }

    fn text(&mut self) -> Option<String> {
        // The opening quote is the byte the caller just matched.
        self.position += 1;
        let mut out = String::new();
        loop {
            let byte = *self.bytes.get(self.position)?;
            self.position += 1;
            match byte {
                b'"' => return Some(out),
                b'\\' => {
                    let escape = *self.bytes.get(self.position)?;
                    self.position += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000C}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let digits = self.bytes.get(self.position..self.position + 4)?;
                            let code =
                                u32::from_str_radix(core::str::from_utf8(digits).ok()?, 16).ok()?;
                            self.position += 4;
                            out.push(char::from_u32(code)?);
                        }
                        _ => return None,
                    }
                }
                0x00..=0x1F => return None,
                _ => {
                    // Multi-byte UTF-8 rides through on the slice below.
                    let start = self.position - 1;
                    let mut end = start + 1;
                    while end < self.bytes.len() && self.bytes[end] >= 0x80 {
                        end += 1;
                    }
                    out.push_str(core::str::from_utf8(self.bytes.get(start..end)?).ok()?);
                }
            }
        }
    }

    fn number(&mut self) -> Option<f64> {
        let start = self.position;
        if self.bytes.get(self.position) == Some(&b'-') {
            self.position += 1;
        }
        let digits = |cursor: &mut Self| {
            let begin = cursor.position;
            while cursor.position < cursor.bytes.len()
                && cursor.bytes[cursor.position].is_ascii_digit()
            {
                cursor.position += 1;
            }
            cursor.position - begin
        };
        if digits(self) == 0 {
            return None;
        }
        if self.bytes.get(self.position) == Some(&b'.') {
            self.position += 1;
            if digits(self) == 0 {
                return None;
            }
        }
        if matches!(self.bytes.get(self.position), Some(b'e' | b'E')) {
            self.position += 1;
            if matches!(self.bytes.get(self.position), Some(b'-' | b'+')) {
                self.position += 1;
            }
            if digits(self) == 0 {
                return None;
            }
        }
        core::str::from_utf8(self.bytes.get(start..self.position)?)
            .ok()?
            .parse()
            .ok()
    }
}

/// The extensionless display URL for a manifest file name.
pub(crate) fn fallback_rest_url(index: usize) -> String {
    clip_url(&format!(
        "roll-{}-rest.webp",
        index % FALLBACK_CLIP_COUNT + 1
    ))
}

pub(crate) fn clip_url(file: &str) -> String {
    let stem = file.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(file);
    format!("{DICE_URL_PREFIX}{stem}")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RollMode {
    Plain,
    Advantage,
    Disadvantage,
}

/// The individual server-rolled damage faces, in their authoritative order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DamagePlay {
    pub(crate) faces: Vec<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Natural {
    Twenty,
    One,
}

/// What the display rolls: the server's kept face, the discarded draw when the server rolled
/// two dice, and whether the kept face is natural.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DicePlay {
    pub(crate) kept: u8,
    pub(crate) other: Option<u8>,
    pub(crate) mode: RollMode,
    pub(crate) natural: Option<Natural>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DiceTiming {
    pub(crate) land_ms: u64,
    pub(crate) clear_ms: u64,
}

/// Settled-face dwell before a damage result is readable.
const RESULT_READ_MS: u64 = df_render::battle::ROLL_CARD_HIT_HOLD.as_millis() as u64;
pub(crate) const DICE_FADE_MS: u64 = 320;
const DAMAGE_ROLL_MS: u64 = df_render::battle::DAMAGE_DICE_TUMBLE.as_millis() as u64;
pub(crate) const FIGHT_RESULT_HOLD_MS: u64 =
    df_render::battle::ROLL_CARD_MISS_HOLD.as_millis() as u64;
pub(crate) const ATTACK_D20_TUMBLE_MS: u64 =
    df_render::battle::ATTACK_D20_TUMBLE.as_millis() as u64;
pub(crate) const ATTACK_FACE_DWELL_MS: u64 =
    df_render::battle::ATTACK_FACE_DWELL.as_millis() as u64;
pub(crate) const FIGHT_REST_BUDGET_MS: u64 = ATTACK_D20_TUMBLE_MS;

/// Display-only ordering of already committed results, in milliseconds from roll start.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AttackTiming {
    pub(crate) settled_ms: u64,
    pub(crate) card_ms: u64,
    pub(crate) damage_settled_ms: Option<u64>,
    pub(crate) closing_ms: u64,
    pub(crate) hidden_ms: u64,
}

pub(crate) const fn attack_timing(hit: bool, damage_dice: bool) -> AttackTiming {
    let card_ms = ATTACK_D20_TUMBLE_MS + ATTACK_FACE_DWELL_MS;
    let damage_settled_ms = if damage_dice {
        Some(card_ms + DAMAGE_ROLL_MS)
    } else {
        None
    };
    let closing_ms = card_ms
        + if damage_dice { DAMAGE_ROLL_MS } else { 0 }
        + if hit {
            RESULT_READ_MS
        } else {
            FIGHT_RESULT_HOLD_MS
        };
    AttackTiming {
        settled_ms: ATTACK_D20_TUMBLE_MS,
        card_ms,
        damage_settled_ms,
        closing_ms,
        hidden_ms: closing_ms + DICE_FADE_MS,
    }
}

/// Reserve the longer hit sequence during catch-up too; never truncate a readable result.
pub(crate) const fn fight_beat_ms(_catching_up: bool) -> u64 {
    attack_timing(true, true).hidden_ms + 200
}

pub(crate) fn d20_timing(_rest_ms: u64, _catching_up: bool) -> DiceTiming {
    DiceTiming {
        land_ms: ATTACK_D20_TUMBLE_MS,
        clear_ms: fight_beat_ms(false),
    }
}

pub(crate) fn damage_timing() -> DiceTiming {
    let land_ms = DAMAGE_ROLL_MS;
    DiceTiming {
        land_ms,
        clear_ms: land_ms + RESULT_READ_MS + DICE_FADE_MS,
    }
}

pub(crate) fn damage_play_of(faces: &[u32]) -> Option<DamagePlay> {
    (!faces.is_empty()).then(|| DamagePlay {
        faces: faces.to_vec(),
    })
}

/// Where the visible dice are hosted: the 3D battlefield paints over the whole stage, so
/// while its container is mounted the dice live inside it (above its canvas, below its
/// HUD); on the flat stage they join the roll panel in flow (never covering its text, never
/// hidden behind it), or stay on the stage root when the panel is absent. The full stacking
/// order is stated where it is set, in `live_tv.css`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DiceHost {
    Battlefield,
    Panel,
    Stage,
}

/// Which clip plays this roll: a hash of the server's own numbers, so a replay shows
/// the same clip, skipping the clip that just played so two rolls in a row never look
/// like one clip. Pure in `last`: the caller keeps the previously played index.
pub(crate) fn choose_clip(play: &DicePlay, clips: usize, last: Option<usize>) -> usize {
    if clips == 0 {
        return 0;
    }
    let mut hash = u64::from(play.kept);
    hash = hash
        .wrapping_mul(31)
        .wrapping_add(u64::from(play.other.unwrap_or(0)));
    hash = hash.wrapping_mul(31).wrapping_add(match play.mode {
        RollMode::Plain => 0,
        RollMode::Advantage => 1,
        RollMode::Disadvantage => 2,
    });
    hash ^= hash >> 29;
    hash = hash.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    hash ^= hash >> 32;
    let mut index = (hash % clips as u64) as usize;
    if clips > 1 && Some(index) == last {
        index = (index + 1) % clips;
    }
    index
}

/// The pair die's clip: a different clip from the first, deterministic for the roll.
pub(crate) fn pair_clip(first: usize, clips: usize) -> usize {
    if clips < 2 {
        first
    } else {
        (first + 1) % clips
    }
}

/// The host for the dice: the battlefield container when it is mounted, else the flat roll
/// panel when it is present, else the stage root.
pub(crate) fn dice_host(field_mounted: bool, panel_present: bool) -> DiceHost {
    if field_mounted {
        DiceHost::Battlefield
    } else if panel_present {
        DiceHost::Panel
    } else {
        DiceHost::Stage
    }
}

fn face(value: u32) -> Option<u8> {
    u8::try_from(value)
        .ok()
        .filter(|face| (1..=20).contains(face))
}

/// The display plan for the outcome's three carried numbers, or `None` when they carry no
/// d20 test (a move, a rest, a recovery). Faces come only from what the server sent: the
/// kept face is `attack_die` and the discarded one is the server's other draw.
///
/// SRD 5.2.1 pp. 6-7 (D20 Tests, Advantage/Disadvantage): “roll two d20s, but you use the
/// number from only one of them.” The browser marks the server-selected face as kept.
pub(crate) fn play_of(attack_die: u32, faces: &[u32], roll_mode: i32) -> Option<DicePlay> {
    let kept = face(attack_die)?;
    let mode = match roll_mode {
        0 => RollMode::Plain,
        _ if roll_mode < 0 => RollMode::Disadvantage,
        _ => RollMode::Advantage,
    };
    let other = match mode {
        RollMode::Plain => None,
        RollMode::Advantage | RollMode::Disadvantage => faces
            .iter()
            .filter_map(|draw| face(*draw))
            .enumerate()
            .filter(|(position, draw)| *draw != kept || *position > 0)
            .map(|(_, draw)| draw)
            .next(),
    };
    let natural = match kept {
        20 => Some(Natural::Twenty),
        1 => Some(Natural::One),
        _ => None,
    };
    Some(DicePlay {
        kept,
        other,
        mode,
        natural,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(die: u32, mode: i32, faces: &[u32]) -> Option<DicePlay> {
        let wire = rpc::CombatOutcome {
            attack_die: die,
            roll_mode: mode,
            faces: faces.to_vec(),
            ..Default::default()
        };
        // The plan reads only the three numbers the server carries on the outcome.
        play_of(wire.attack_die, &wire.faces, wire.roll_mode)
    }

    #[test]
    fn a_plain_hit_shows_one_die_with_the_server_face() {
        let play = plan(14, 0, &[14]).expect("a d20 test has a plan");
        assert_eq!(play.kept, 14);
        assert_eq!(play.other, None);
        assert_eq!(play.mode, RollMode::Plain);
        assert_eq!(play.natural, None);
    }

    #[test]
    fn advantage_shows_two_dice_and_marks_the_kept_one() {
        let play = plan(17, 1, &[17, 9]).expect("a d20 test has a plan");
        assert_eq!(play.mode, RollMode::Advantage);
        assert_eq!(play.kept, 17);
        assert_eq!(play.other, Some(9));
    }

    #[test]
    fn disadvantage_marks_the_lower_kept_face() {
        let play = plan(6, -1, &[6, 19]).expect("a d20 test has a plan");
        assert_eq!(play.mode, RollMode::Disadvantage);
        assert_eq!(play.kept, 6);
        assert_eq!(play.other, Some(19));
    }

    #[test]
    fn a_pair_of_equal_faces_still_shows_two_dice() {
        let play = plan(11, 1, &[11, 11]).expect("a d20 test has a plan");
        assert_eq!(play.kept, 11);
        assert_eq!(play.other, Some(11));
    }

    #[test]
    fn faces_the_server_never_sent_are_never_shown() {
        // A second draw outside 1..=20 falls back to a single die; the kept face is intact.
        let play = plan(13, 1, &[13, 99]).expect("a d20 test has a plan");
        assert_eq!(play.kept, 13);
        assert_eq!(play.other, None);
    }

    #[test]
    fn natural_faces_are_named() {
        assert_eq!(
            plan(20, 0, &[20]).expect("a plan").natural,
            Some(Natural::Twenty)
        );
        assert_eq!(
            plan(1, 0, &[1]).expect("a plan").natural,
            Some(Natural::One)
        );
    }

    #[test]
    fn outcomes_without_a_d20_have_no_die() {
        assert_eq!(plan(0, 0, &[]), None);
        assert_eq!(plan(27, 0, &[]), None);
    }

    #[test]
    fn damage_plan_keeps_the_server_faces_in_order() {
        let play = damage_play_of(&[6, 5, 1, 4]).expect("hit damage carries dice");

        assert_eq!(play.faces, [6, 5, 1, 4]);
        assert_eq!(damage_play_of(&[]), None);
    }

    #[test]
    fn d20_result_uses_the_fixed_attack_tumble_and_hold() {
        let timing = d20_timing(1_200, false);

        assert_eq!(timing.land_ms, ATTACK_D20_TUMBLE_MS);
        assert_eq!(timing.clear_ms, fight_beat_ms(false));

        let catching_up = d20_timing(1_200, true);
        assert_eq!(
            catching_up.land_ms, ATTACK_D20_TUMBLE_MS,
            "the die still tumbles to its rest"
        );
        assert_eq!(catching_up.clear_ms, fight_beat_ms(true));
    }

    #[test]
    fn the_next_fight_die_waits_for_the_previous_one_to_clear() {
        // The director holds a blow at least this long, so two dice are never on screen
        // together: the slowest clip rest the manifest names (2.5 s clamp) is covered by the
        // budget the clips actually use (1.2-1.4 s).
        for rest in [1_200, 1_300, 1_400, FIGHT_REST_BUDGET_MS] {
            assert!(fight_beat_ms(false) >= d20_timing(rest, false).clear_ms);
            assert!(fight_beat_ms(true) >= d20_timing(rest, true).clear_ms);
        }
        assert_eq!(fight_beat_ms(true), fight_beat_ms(false));
        assert!(std::hint::black_box(RESULT_READ_MS) >= 2_000);
    }

    #[test]
    fn technique_dice_ownership_requires_a_complete_supported_plan() {
        let mut technique = rpc::BattleTechnique {
            kind: rpc::BattleTechniqueKind::BattleTechniqueFlurry as i32,
            strikes: vec![
                rpc::BattleTechniqueStrike::default(),
                rpc::BattleTechniqueStrike::default(),
            ],
            ..Default::default()
        };
        assert!(!technique_owns_roll(&technique));
        technique.strikes[0].roll = Some(rpc::BattleTechniqueRoll {
            d20: Some(12),
            ..Default::default()
        });
        assert!(
            !technique_owns_roll(&technique),
            "partial extension keeps the ordinary presenter"
        );
        technique.strikes[1].roll = Some(rpc::BattleTechniqueRoll {
            d20: Some(9),
            ..Default::default()
        });
        assert!(
            technique_owns_roll(&technique),
            "complete extension has one timeline seat"
        );
        technique.kind = 999;
        assert!(
            !technique_owns_roll(&technique),
            "unsupported extension retains the fallback"
        );
    }

    #[test]
    fn committed_card_preserves_authority_and_hides_damage_until_settled() {
        let mut outcome = rpc::CombatOutcome {
            actor_name: "Aster".to_owned(),
            target_name: "Skeleton".to_owned(),
            weapon: "Longsword".to_owned(),
            attack_die: 16,
            faces: vec![16, 4],
            attack_modifier: 7,
            attack_total: 23,
            target_armor_class: 15,
            hit: true,
            damage_dice: vec![5],
            damage: 8,
            damage_modifier: 3,
            damage_type: "slashing".to_owned(),
            ..Default::default()
        };
        let initial = committed_roll_card(&outcome, false);
        assert!(initial.explanation.contains("16 / 4 +7 bonus = 23"));
        assert!(!initial.explanation.contains("damage"));
        assert!(
            committed_roll_card(&outcome, true)
                .explanation
                .contains("damage 5 +3 = 8 slashing")
        );
        outcome.display_metadata = Some(rpc::CombatDisplayMetadata {
            icon: Some("\u{2694}".to_owned()),
            ability_modifier: Some(4),
            proficiency_modifier: Some(3),
            other_modifier: Some(0),
            ..Default::default()
        });
        assert!(
            committed_roll_card(&outcome, false)
                .explanation
                .contains("+4 ability +3 proficiency +0 other")
        );
        assert!(
            committed_roll_card(&outcome, false)
                .label
                .starts_with("\u{2694}")
        );
        outcome.damage_rolls = vec![
            rpc::DamageRoll {
                dice: vec![1],
                kept: false,
            },
            rpc::DamageRoll {
                dice: vec![6],
                kept: true,
            },
        ];
        assert!(
            committed_roll_card(&outcome, true)
                .explanation
                .contains("damage 6 +3")
        );
        outcome.hit = false;
        outcome.attack_die = 1;
        assert_eq!(committed_roll_card(&outcome, true).verdict, "FUMBLE");
        assert!(
            !committed_roll_card(&outcome, true)
                .explanation
                .contains("damage")
        );
        outcome.attack_die = 9;
        assert_eq!(committed_roll_card(&outcome, false).verdict, "MISS");
        outcome.hit = true;
        outcome.critical = true;
        assert_eq!(committed_roll_card(&outcome, false).verdict, "CRITICAL HIT");
    }

    #[test]
    fn attack_plan_orders_face_dwell_damage_and_full_read_hold() {
        let hit = attack_timing(true, true);
        assert_eq!(hit.settled_ms, 1100);
        assert_eq!(hit.card_ms, 1500);
        assert_eq!(hit.damage_settled_ms, Some(2300));
        assert_eq!(hit.closing_ms, 4300);
        assert_eq!(hit.hidden_ms, 4620);
        let miss = attack_timing(false, false);
        assert_eq!(miss.card_ms, 1500);
        assert_eq!(miss.damage_settled_ms, None);
        assert_eq!(miss.closing_ms, 4000);
        assert!(fight_beat_ms(true) >= miss.hidden_ms);
    }

    #[test]
    fn damage_dice_roll_tumbles_then_holds_the_result() {
        let timing = damage_timing();

        assert_eq!(timing.land_ms, 800);
        assert_eq!(timing.clear_ms, 3_120);
    }

    #[test]
    fn dice_live_in_the_battlefield_while_it_is_mounted() {
        // The regressions this pins: dice hosted on the stage root hide behind the
        // battlefield container, which paints over the whole stage; dice floating over
        // the flat stage cover the roll panel's text (or hide behind it), so they join
        // the panel in flow instead.
        assert_eq!(dice_host(true, false), DiceHost::Battlefield);
        assert_eq!(dice_host(true, true), DiceHost::Battlefield);
        assert_eq!(dice_host(false, true), DiceHost::Panel);
        assert_eq!(dice_host(false, false), DiceHost::Stage);
    }

    #[test]
    fn fallback_stills_use_a_known_rest_asset() {
        let play = plan(14, 0, &[14]).expect("a d20 test has a plan");
        let rest = fallback_rest_url(choose_clip(&play, FALLBACK_CLIP_COUNT, None));
        let selected = choose_clip(&play, FALLBACK_CLIP_COUNT, None);
        assert_eq!(rest, clip_url(&format!("roll-{}-rest.webp", selected + 1)));
    }

    #[test]
    fn clip_choice_is_deterministic_for_a_roll_and_never_repeats() {
        // A replay shows the same clip: the choice is a pure hash of the server's numbers.
        let first = plan(14, 0, &[14]).expect("a d20 test has a plan");
        let again = plan(14, 0, &[14]).expect("a d20 test has a plan");
        assert_eq!(choose_clip(&first, 3, None), choose_clip(&again, 3, None));
        // ... but never the clip that just played, so two in a row differ.
        for kept in 1..=20u32 {
            let play = plan(kept, 0, &[kept]).expect("a d20 test has a plan");
            for last in 0..3usize {
                assert_ne!(choose_clip(&play, 3, Some(last)), last);
            }
        }
        // Different rolls spread across the clips instead of piling on one.
        let mut seen = [false; 3];
        for kept in 1..=20u32 {
            let play = plan(kept, 0, &[kept]).expect("a d20 test has a plan");
            seen[choose_clip(&play, 3, None)] = true;
        }
        assert!(seen.iter().all(|seen| *seen));
    }

    #[test]
    fn a_pair_plays_two_different_clips() {
        for first in 0..3usize {
            assert_ne!(pair_clip(first, 3), first);
        }
        assert_eq!(pair_clip(0, 1), 0);
    }

    #[test]
    fn the_manifest_parses_and_invalid_ones_do_not() {
        let manifest = parse_manifest(
            r##"{"key_color":"#ff00ff","clips":[
                {"webm":"roll-1.webm","key_mp4":"roll-1-key.mp4","rest_image":"roll-1-rest.webp",
                 "rest_ms":1200,"face_center":[0.5,0.5],
                 "face_corners":[[0.4,0.4],[0.6,0.4],[0.5,0.6]],"face_rotation_deg":8.0},
                {"webm":"roll-2.webm","key_mp4":"roll-2-key.mp4","rest_image":"roll-2-rest.webp",
                 "rest_ms":1400,"face_center":[0.5,0.5],
                 "face_corners":[[0.4,0.4],[0.6,0.4],[0.5,0.6]],"face_rotation_deg":-14.0}
            ]}"##,
        )
        .expect("the contract manifest parses");
        assert_eq!(manifest.clips.len(), 2);
        assert_eq!(manifest.clips[0].rest_ms, 1200);
        assert_eq!(manifest.key_color, "#ff00ff");
        assert_eq!(
            clip_url(&manifest.clips[0].webm),
            "/assets/ui/generated/dice/roll-1"
        );
        assert!(parse_manifest(r##"{"key_color":"#ff00ff","clips":[]}"##).is_none());
        assert!(parse_manifest("{nope}").is_none());
    }
}
