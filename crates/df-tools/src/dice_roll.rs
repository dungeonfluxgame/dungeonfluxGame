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
#[cfg(test)]
use df_protocol::common as rpc;

/// The display's dice live under this path, extensionless like every other extra asset.
pub(crate) const DICE_URL_PREFIX: &str = "/assets/ui/generated/dice/";

/// One roll clip from the manifest: file names, the rest time the number and the land
/// cue follow, and the upward face geometry in 0-to-1 coordinates of the square.
#[derive(Clone, Debug)]
pub(crate) struct DiceClip {
    pub(crate) webm: String,
    pub(crate) key_mp4: String,
    pub(crate) rest_image: String,
    pub(crate) rest_ms: u64,
    pub(crate) face_center: [f64; 2],
    pub(crate) face_corners: [[f64; 2]; 3],
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
        parsed.push(DiceClip {
            webm: clip.field("webm")?.string()?,
            key_mp4: clip.field("key_mp4")?.string()?,
            rest_image: clip.field("rest_image")?.string()?,
            rest_ms: {
                let rest = clip.field("rest_ms")?.number()?;
                if rest <= 0.0 || rest.fract() != 0.0 || rest > u64::MAX as f64 {
                    return None;
                }
                rest as u64
            },
            face_center: point(clip.field("face_center")?)?,
            face_corners: {
                let corners = clip.field("face_corners")?.array()?;
                if corners.len() != 3 {
                    return None;
                }
                [
                    point(&corners[0])?,
                    point(&corners[1])?,
                    point(&corners[2])?,
                ]
            },
            face_rotation_deg: clip.field("face_rotation_deg")?.number()?,
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
