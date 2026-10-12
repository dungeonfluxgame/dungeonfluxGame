//! The voice roster file and the pure casting of speakers to roster voices.
//!
//! Gender is a hard constraint. Among the voices of the speaker's gender, a voice not yet
//! used by another speaker of the same adventure always beats a used one; after that a
//! matching age and then more tone words in common win, and a hash of the run identifier,
//! the speaker and the voice settles what is left, so the same inputs always give the same
//! cast while different adventures vary.
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
    fmt, io,
    path::Path,
};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{Age, Gender, SpeakerCasting};
use crate::gameplay::startup_configuration::is_voice_identifier;

const ROSTER_SCHEMA: &str = "df-voice-casting/1";
const MAXIMUM_ROSTER_BYTES: usize = 64 * 1024;
/// Fewest roster voices per gender, so every kind of speaker can be cast.
const REQUIRED_VOICES: [(Gender, usize); 4] = [
    (Gender::She, 4),
    (Gender::He, 4),
    (Gender::They, 1),
    (Gender::It, 2),
];

/// Why a roster entry was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RosterProblem {
    InvalidIdentifier,
    UnknownValue,
    Duplicate,
    Empty,
    Mismatch,
    NarratorVoice,
    TooFew,
    Missing,
}

impl fmt::Display for RosterProblem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidIdentifier => "is not a valid voice id",
            Self::UnknownValue => "has an unknown value",
            Self::Duplicate => "is a duplicate",
            Self::Empty => "is empty",
            Self::Mismatch => "does not match the provider metadata or its role",
            Self::NarratorVoice => "must not be the narrator voice",
            Self::TooFew => "has too few voices",
            Self::Missing => "is not in the roster",
        })
    }
}

/// A voice roster that could not be loaded; startup refuses with it.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum RosterError {
    Unreadable(io::ErrorKind),
    TooLarge,
    /// The file is not JSON of the roster's shape; `path` locates the offending field.
    Malformed {
        path: String,
        detail: String,
    },
    Invalid {
        field: &'static str,
        problem: RosterProblem,
    },
}

impl fmt::Display for RosterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable(kind) => write!(formatter, "voice roster unreadable: {kind}"),
            Self::TooLarge => formatter.write_str("voice roster file is too large"),
            Self::Malformed { path, detail } => {
                write!(formatter, "voice roster malformed at {path}: {detail}")
            }
            Self::Invalid { field, problem } => write!(formatter, "voice roster {field} {problem}"),
        }
    }
}

impl std::error::Error for RosterError {}

fn invalid(field: &'static str, problem: RosterProblem) -> RosterError {
    RosterError::Invalid { field, problem }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawNarrator {
    voice_id: String,
    name: String,
    provider_gender: String,
    provider_age: String,
    verified_on: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawVoice {
    voice_id: String,
    name: String,
    gender: String,
    age: String,
    tones: Vec<String>,
    provider_gender: String,
    provider_age: String,
    verified_on: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRoster {
    schema: String,
    narrator: RawNarrator,
    fallback_voice: String,
    npc_voices: Vec<RawVoice>,
}

/// One provider voice an NPC may be cast with.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RosterVoice {
    pub(super) voice_id: String,
    pub(super) name: String,
    pub(super) gender: Gender,
    pub(super) age: Age,
    /// Lowercase tone words.
    pub(super) tones: Vec<String>,
}

/// The narrator voice plus the NPC voices, validated as a whole.
#[derive(Clone, Debug)]
pub(super) struct Roster {
    pub(super) narrator_voice_id: String,
    pub(super) narrator_name: String,
    pub(super) voices: Vec<RosterVoice>,
    /// A `they` voice for speakers nobody cast; it is never gendered.
    pub(super) fallback: RosterVoice,
}

impl Roster {
    pub(super) fn load(path: &Path) -> Result<Self, RosterError> {
        let bytes = std::fs::read(path).map_err(|error| RosterError::Unreadable(error.kind()))?;
        Self::parse(&bytes)
    }

    pub(super) fn parse(bytes: &[u8]) -> Result<Self, RosterError> {
        if bytes.len() > MAXIMUM_ROSTER_BYTES {
            return Err(RosterError::TooLarge);
        }
        let mut deserializer = serde_json::Deserializer::from_slice(bytes);
        let raw: RawRoster =
            serde_path_to_error::deserialize(&mut deserializer).map_err(|error| {
                RosterError::Malformed {
                    path: error.path().to_string(),
                    detail: error.inner().to_string(),
                }
            })?;
        Self::validated(raw)
    }

    fn validated(raw: RawRoster) -> Result<Self, RosterError> {
        if raw.schema != ROSTER_SCHEMA {
            return Err(invalid("schema", RosterProblem::Mismatch));
        }
        let narrator = &raw.narrator;
        if !is_voice_identifier(&narrator.voice_id) {
            return Err(invalid(
                "narrator.voice_id",
                RosterProblem::InvalidIdentifier,
            ));
        }
        if narrator.name.trim().is_empty() || narrator.verified_on.trim().is_empty() {
            return Err(invalid("narrator", RosterProblem::Empty));
        }
        provider_gender(&narrator.provider_gender, "narrator.provider_gender")?;
        provider_age(&narrator.provider_age, "narrator.provider_age")?;

        let mut known = BTreeSet::new();
        let mut voices = Vec::with_capacity(raw.npc_voices.len());
        for entry in raw.npc_voices {
            let voice = validated_voice(entry)?;
            if voice.voice_id == narrator.voice_id {
                return Err(invalid(
                    "npc_voices[].voice_id",
                    RosterProblem::NarratorVoice,
                ));
            }
            if !known.insert(voice.voice_id.clone()) {
                return Err(invalid("npc_voices[].voice_id", RosterProblem::Duplicate));
            }
            voices.push(voice);
        }
        for (gender, minimum) in REQUIRED_VOICES {
            if voices.iter().filter(|voice| voice.gender == gender).count() < minimum {
                return Err(invalid(gender.roster_group_field(), RosterProblem::TooFew));
            }
        }
        let fallback = voices
            .iter()
            .find(|voice| voice.voice_id == raw.fallback_voice)
            .ok_or_else(|| invalid("fallback_voice", RosterProblem::Missing))?
            .clone();
        if fallback.gender != Gender::They {
            return Err(invalid("fallback_voice", RosterProblem::Mismatch));
        }
        Ok(Self {
            narrator_voice_id: raw.narrator.voice_id,
            narrator_name: raw.narrator.name,
            voices,
            fallback,
        })
    }

    pub(super) fn voice(&self, voice_id: &str) -> Option<&RosterVoice> {
        self.voices.iter().find(|voice| voice.voice_id == voice_id)
    }
}

/// The provider's own gender label as the roster gender it implies; `None` for `it`, which
/// the provider has no label for.
fn provider_gender(label: &str, field: &'static str) -> Result<Gender, RosterError> {
    match label {
        "male" => Ok(Gender::He),
        "female" => Ok(Gender::She),
        "neutral" => Ok(Gender::They),
        _ => Err(invalid(field, RosterProblem::UnknownValue)),
    }
}

fn provider_age(label: &str, field: &'static str) -> Result<Age, RosterError> {
    match label {
        "young" => Ok(Age::Young),
        "middle_aged" => Ok(Age::Adult),
        "old" => Ok(Age::Elder),
        _ => Err(invalid(field, RosterProblem::UnknownValue)),
    }
}

fn validated_voice(entry: RawVoice) -> Result<RosterVoice, RosterError> {
    if !is_voice_identifier(&entry.voice_id) {
        return Err(invalid(
            "npc_voices[].voice_id",
            RosterProblem::InvalidIdentifier,
        ));
    }
    if entry.name.trim().is_empty() || entry.verified_on.trim().is_empty() {
        return Err(invalid("npc_voices[]", RosterProblem::Empty));
    }
    let gender = Gender::parse(&entry.gender)
        .ok_or_else(|| invalid("npc_voices[].gender", RosterProblem::UnknownValue))?;
    let age = Age::parse(&entry.age)
        .ok_or_else(|| invalid("npc_voices[].age", RosterProblem::UnknownValue))?;
    let from_provider_gender =
        provider_gender(&entry.provider_gender, "npc_voices[].provider_gender")?;
    let from_provider_age = provider_age(&entry.provider_age, "npc_voices[].provider_age")?;
    // `it` is a project category for creatures; every other gender must be the provider's.
    if gender != Gender::It && gender != from_provider_gender {
        return Err(invalid("npc_voices[].gender", RosterProblem::Mismatch));
    }
    if age != from_provider_age {
        return Err(invalid("npc_voices[].age", RosterProblem::Mismatch));
    }
    let tones: Vec<String> = entry
        .tones
        .iter()
        .map(|tone| tone.trim().to_lowercase())
        .collect();
    if tones.is_empty() || tones.iter().any(String::is_empty) {
        return Err(invalid("npc_voices[].tones", RosterProblem::Empty));
    }
    Ok(RosterVoice {
        voice_id: entry.voice_id,
        name: entry.name,
        gender,
        age,
        tones,
    })
}

fn tone_words(phrase: &str) -> BTreeSet<String> {
    phrase
        .split(|character: char| !character.is_alphabetic())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn tie_break(run_identifier: &str, speaker: &str, voice_id: &str) -> u64 {
    let mut hasher = Sha256::new();
    for part in [run_identifier, speaker, voice_id] {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    let digest = hasher.finalize();
    digest
        .iter()
        .take(8)
        .fold(0, |value, byte| (value << 8) | u64::from(*byte))
}

/// Chooses a roster voice for every speaker whose gender has a voice; the narrator voice is
/// never chosen. Speakers are cast in name order, so the input order does not matter.
/// Returns speaker name to voice id.
pub(super) fn cast_speakers(
    roster: &Roster,
    narrator_voice: &str,
    speakers: &[SpeakerCasting],
    run_identifier: &str,
) -> BTreeMap<String, String> {
    let mut ordered: Vec<&SpeakerCasting> = speakers.iter().collect();
    ordered.sort();
    ordered.dedup_by(|later, earlier| later.name == earlier.name);

    let mut uses: BTreeMap<&str, usize> = BTreeMap::new();
    let mut cast = BTreeMap::new();
    for speaker in ordered {
        let words = tone_words(&speaker.tone);
        let chosen = roster
            .voices
            .iter()
            .filter(|voice| voice.gender == speaker.gender && voice.voice_id != narrator_voice)
            .min_by_key(|voice| {
                (
                    uses.get(voice.voice_id.as_str()).copied().unwrap_or(0),
                    // No voice of the speaker's age: the nearest age, never the far end.
                    speaker
                        .age
                        .map_or(0, |age| (age as i8 - voice.age as i8).unsigned_abs()),
                    Reverse(
                        voice
                            .tones
                            .iter()
                            .filter(|tone| words.contains(*tone))
                            .count(),
                    ),
                    tie_break(run_identifier, &speaker.name, &voice.voice_id),
                )
            });
        if let Some(voice) = chosen {
            *uses.entry(&voice.voice_id).or_insert(0) += 1;
            cast.insert(speaker.name.clone(), voice.voice_id.clone());
        }
    }
    cast
}

/// A small valid roster with known ids shared by this module's and the voice tests.
#[cfg(test)]
pub(super) mod fixtures {
    use super::Roster;

    /// A valid roster with known ids: `she1`..`she4` (young, adult, elder, adult), `he1`..`he4`
    /// (young, adult, elder, adult), `they1`, `it1`, `it2`.
    pub(in crate::gameplay::voice) fn roster_json() -> String {
        let mut voices = Vec::new();
        let ages = [
            ("young", "young"),
            ("middle_aged", "adult"),
            ("old", "elder"),
        ];
        for (gender, provider, tones) in [
            ("she", "female", ["warm", "gentle", "brisk", "cold"]),
            ("he", "male", ["gruff", "jovial", "anxious", "dry"]),
        ] {
            for index in 0..4 {
                let (provider_age, age) = ages[[0, 1, 2, 1][index]];
                voices.push(format!(
                    r#"{{"voice_id":"{gender}{n}","name":"{gender}{n}name","gender":"{gender}","age":"{age}","tones":["{tone}"],"provider_gender":"{provider}","provider_age":"{provider_age}","verified_on":"2026-10-11"}}"#,
                    n = index + 1,
                    tone = tones[index],
                ));
            }
        }
        voices.push(voice_json(
            "they1",
            "they",
            "neutral",
            "adult",
            "middle_aged",
            "calm",
        ));
        voices.push(voice_json(
            "it1",
            "it",
            "male",
            "adult",
            "middle_aged",
            "raspy",
        ));
        voices.push(voice_json("it2", "it", "male", "young", "young", "sharp"));
        format!(
            r#"{{"schema":"df-voice-casting/1","narrator":{{"voice_id":"narr","name":"Narr","provider_gender":"male","provider_age":"middle_aged","verified_on":"2026-10-11"}},"fallback_voice":"they1","npc_voices":[{}]}}"#,
            voices.join(",")
        )
    }

    fn voice_json(
        id: &str,
        gender: &str,
        provider_gender: &str,
        age: &str,
        provider_age: &str,
        tone: &str,
    ) -> String {
        format!(
            r#"{{"voice_id":"{id}","name":"{id}name","gender":"{gender}","age":"{age}","tones":["{tone}"],"provider_gender":"{provider_gender}","provider_age":"{provider_age}","verified_on":"2026-10-11"}}"#
        )
    }

    pub(in crate::gameplay::voice) fn test_roster() -> Roster {
        Roster::parse(roster_json().as_bytes()).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        fixtures::{roster_json, test_roster},
        *,
    };

    fn speaker(name: &str, gender: Gender, age: Option<Age>, tone: &str) -> SpeakerCasting {
        SpeakerCasting {
            name: name.to_owned(),
            gender,
            age,
            tone: tone.to_owned(),
        }
    }

    fn gender_of(roster: &Roster, voice_id: &str) -> Gender {
        roster.voice(voice_id).unwrap().gender
    }

    #[test]
    fn gender_is_a_hard_constraint_for_every_speaker() {
        let roster = test_roster();
        let speakers = [
            speaker("Wren", Gender::She, Some(Age::Elder), "warm"),
            speaker("Garrick", Gender::He, Some(Age::Adult), "gruff"),
            speaker("Marl", Gender::They, None, ""),
            speaker("Skrit", Gender::It, None, "raspy"),
        ];

        let cast = cast_speakers(&roster, "narr", &speakers, "0x1");

        for speaker in &speakers {
            assert_eq!(gender_of(&roster, &cast[&speaker.name]), speaker.gender);
        }
    }

    #[test]
    fn gender_beats_age_and_tone() {
        let roster = test_roster();
        // Only `he1` and `he3` share the speaker's age, but `she` voices match tone and age too.
        let cast = cast_speakers(
            &roster,
            "narr",
            &[speaker(
                "Pell",
                Gender::He,
                Some(Age::Elder),
                "warm gentle brisk cold",
            )],
            "0x1",
        );

        assert_eq!(cast["Pell"], "he3");
    }

    #[test]
    fn matching_age_is_preferred() {
        let roster = test_roster();
        for (age, expected) in [(Age::Young, "she1"), (Age::Elder, "she3")] {
            let cast = cast_speakers(
                &roster,
                "narr",
                &[speaker("Ila", Gender::She, Some(age), "")],
                "0x1",
            );

            assert_eq!(cast["Ila"], expected);
        }
    }

    #[test]
    fn the_nearest_age_is_used_when_no_voice_has_the_speakers_age() {
        // The shipped roster has no elder woman: an elder woman gets an adult voice, not a
        // young one, whatever the run and whatever her tone.
        let roster = Roster::parse(SHIPPED.as_bytes()).unwrap();
        for run in ["seed-a", "seed-b", "seed-c", "seed-d"] {
            let cast = cast_speakers(
                &roster,
                "narr",
                &[speaker("Gwenna", Gender::She, Some(Age::Elder), "warm")],
                run,
            );
            let voice = roster
                .voices
                .iter()
                .find(|voice| voice.voice_id == cast["Gwenna"])
                .unwrap();
            assert_eq!(voice.age, Age::Adult, "{run}");
        }
    }

    #[test]
    fn tone_overlap_is_preferred_among_voices_of_the_same_age() {
        let roster = test_roster();
        // `she2` and `she4` are both adult; their tones are warm-less `gentle` and `cold`.
        let gentle = cast_speakers(
            &roster,
            "narr",
            &[speaker(
                "Ila",
                Gender::She,
                Some(Age::Adult),
                "a Gentle, soft voice",
            )],
            "0x1",
        );
        let cold = cast_speakers(
            &roster,
            "narr",
            &[speaker(
                "Ila",
                Gender::She,
                Some(Age::Adult),
                "precise, COLD woman",
            )],
            "0x1",
        );

        assert_eq!(gentle["Ila"], "she2");
        assert_eq!(cold["Ila"], "she4");
    }

    #[test]
    fn speakers_of_one_adventure_get_different_voices_while_the_roster_allows() {
        let roster = test_roster();
        let speakers: Vec<_> = ["A", "B", "C", "D"]
            .into_iter()
            .map(|name| speaker(name, Gender::She, Some(Age::Young), "warm"))
            .collect();

        let cast = cast_speakers(&roster, "narr", &speakers, "0x1");

        let distinct: BTreeSet<_> = cast.values().collect();
        assert_eq!(cast.len(), 4);
        assert_eq!(distinct.len(), 4);
    }

    #[test]
    fn a_full_roster_group_reuses_voices_instead_of_dropping_speakers() {
        let roster = test_roster();
        let speakers: Vec<_> = ["A", "B", "C"]
            .into_iter()
            .map(|name| speaker(name, Gender::They, None, ""))
            .collect();

        let cast = cast_speakers(&roster, "narr", &speakers, "0x1");

        assert_eq!(cast.len(), 3);
        assert!(cast.values().all(|voice| voice == "they1"));
    }

    #[test]
    fn casting_is_deterministic_and_ignores_input_order() {
        let roster = test_roster();
        let speakers = [
            speaker("Wren", Gender::She, None, ""),
            speaker("Brisa", Gender::She, None, ""),
            speaker("Odo", Gender::He, None, ""),
        ];
        let reversed: Vec<_> = speakers.iter().rev().cloned().collect();

        let first = cast_speakers(&roster, "narr", &speakers, "0xabc");

        assert_eq!(first, cast_speakers(&roster, "narr", &speakers, "0xabc"));
        assert_eq!(first, cast_speakers(&roster, "narr", &reversed, "0xabc"));
    }

    #[test]
    fn the_run_identifier_varies_the_cast_between_adventures() {
        let roster = test_roster();
        let speakers = [speaker("Wren", Gender::He, None, "")];

        let voices: BTreeSet<_> = (0..16)
            .map(|run| {
                cast_speakers(&roster, "narr", &speakers, &format!("0x{run}"))["Wren"].clone()
            })
            .collect();

        assert!(voices.len() > 1);
    }

    #[test]
    fn the_narrator_voice_is_never_cast() {
        let roster = test_roster();
        let speakers = [speaker("Wren", Gender::She, None, "")];

        for index in 1..=4 {
            let narrator = format!("she{index}");
            let cast = cast_speakers(&roster, &narrator, &speakers, "0x1");

            assert_ne!(cast["Wren"], narrator);
        }
    }

    #[test]
    fn a_gender_without_a_usable_voice_is_not_cast() {
        let roster = test_roster();
        let speakers = [speaker("Marl", Gender::They, None, "")];

        let cast = cast_speakers(&roster, "they1", &speakers, "0x1");

        assert!(cast.is_empty());
    }

    fn replaced(from: &str, to: &str) -> Vec<u8> {
        let json = roster_json();
        assert!(json.contains(from), "{from}");
        json.replacen(from, to, 1).into_bytes()
    }

    #[test]
    fn invalid_rosters_are_refused_naming_the_field() {
        let cases: [(Vec<u8>, &str); 11] = [
            (
                replaced(r#""gender":"she""#, r#""gender":"shee""#),
                "npc_voices[].gender",
            ),
            (
                replaced(r#""age":"young""#, r#""age":"tiny""#),
                "npc_voices[].age",
            ),
            (
                replaced(r#""voice_id":"she2""#, r#""voice_id":"she1""#),
                "npc_voices[].voice_id",
            ),
            (
                replaced(r#""voice_id":"she2""#, r#""voice_id":"narr""#),
                "npc_voices[].voice_id",
            ),
            (
                replaced(r#""voice_id":"she2""#, r#""voice_id":"a b""#),
                "npc_voices[].voice_id",
            ),
            (
                replaced(r#""gender":"he""#, r#""gender":"she""#),
                "npc_voices[].gender",
            ),
            (
                replaced(r#""fallback_voice":"they1""#, r#""fallback_voice":"he1""#),
                "fallback_voice",
            ),
            (
                replaced(
                    r#""fallback_voice":"they1""#,
                    r#""fallback_voice":"nobody""#,
                ),
                "fallback_voice",
            ),
            (
                replaced(r#""tones":["warm"]"#, r#""tones":[]"#),
                "npc_voices[].tones",
            ),
            (
                replaced("df-voice-casting/1", "df-voice-casting/2"),
                "schema",
            ),
            (
                replaced(
                    r#""provider_gender":"neutral""#,
                    r#""provider_gender":"robot""#,
                ),
                "npc_voices[].provider_gender",
            ),
        ];

        for (bytes, field) in cases {
            let error = Roster::parse(&bytes).unwrap_err();

            assert!(
                matches!(&error, RosterError::Invalid { field: named, .. } if *named == field),
                "{field}: {error}"
            );
            assert!(error.to_string().contains(field));
        }
    }

    #[test]
    fn a_roster_missing_a_required_gender_group_is_refused() {
        for gender in [Gender::She, Gender::He, Gender::They, Gender::It] {
            let mut json: serde_json::Value = serde_json::from_str(&roster_json()).unwrap();
            let voices = json["npc_voices"].as_array_mut().unwrap();
            voices.retain(|voice| voice["gender"] != gender.label());

            let error = Roster::parse(&serde_json::to_vec(&json).unwrap()).unwrap_err();

            assert_eq!(
                error,
                invalid(gender.roster_group_field(), RosterProblem::TooFew)
            );
            assert!(error.to_string().contains(gender.label()));
        }
    }

    #[test]
    fn malformed_and_unreadable_rosters_are_typed_errors() {
        let missing_field = replaced(r#""age":"young","#, "");
        let unknown_field = replaced(r#""schema""#, r#""extra":1,"schema""#);

        assert!(matches!(
            Roster::parse(&missing_field),
            Err(RosterError::Malformed { path, .. }) if path.contains("npc_voices")
        ));
        assert!(matches!(
            Roster::parse(&unknown_field),
            Err(RosterError::Malformed { .. })
        ));
        assert!(matches!(
            Roster::parse(b"not json"),
            Err(RosterError::Malformed { .. })
        ));
        assert_eq!(
            Roster::load(Path::new("/nonexistent/voice-casting.json")).unwrap_err(),
            RosterError::Unreadable(io::ErrorKind::NotFound)
        );
        assert_eq!(
            Roster::parse(&vec![b' '; MAXIMUM_ROSTER_BYTES + 1]).unwrap_err(),
            RosterError::TooLarge
        );
    }

    const SHIPPED: &str = include_str!("../../../../../content/voice-casting.json");
    const SHIPPED_SOURCE: &str = include_str!("../../../../../content/voice-casting.source.json");

    #[test]
    fn the_shipped_roster_loads_and_covers_every_required_group() {
        let roster = Roster::parse(SHIPPED.as_bytes()).unwrap();

        assert!(roster.voices.len() >= 12);
        for (gender, minimum) in REQUIRED_VOICES {
            let count = roster
                .voices
                .iter()
                .filter(|voice| voice.gender == gender)
                .count();
            assert!(count >= minimum, "{gender:?}");
        }
        for age in [Age::Young, Age::Adult, Age::Elder] {
            assert!(
                roster
                    .voices
                    .iter()
                    .any(|voice| voice.gender == Gender::He && voice.age == age),
                "he {age:?}"
            );
        }
        assert_eq!(roster.fallback.gender, Gender::They);
        assert_eq!(
            roster.narrator_voice_id,
            crate::gameplay::startup_configuration::DEFAULT_NARRATOR_VOICE
        );
    }

    #[test]
    fn every_shipped_roster_entry_matches_the_saved_provider_metadata() {
        let roster: serde_json::Value = serde_json::from_str(SHIPPED).unwrap();
        let source: serde_json::Value = serde_json::from_str(SHIPPED_SOURCE).unwrap();
        let provider = |voice_id: &str| {
            source["voices"]
                .as_array()
                .unwrap()
                .iter()
                .find(|voice| voice["voice_id"] == voice_id)
                .unwrap_or_else(|| panic!("{voice_id} absent from the provider listing"))
                .clone()
        };

        let entries = roster["npc_voices"]
            .as_array()
            .unwrap()
            .iter()
            .chain([&roster["narrator"]]);
        for entry in entries {
            let listed = provider(entry["voice_id"].as_str().unwrap());
            assert_eq!(listed["labels"]["gender"], entry["provider_gender"]);
            assert_eq!(listed["labels"]["age"], entry["provider_age"]);
            assert_eq!(listed["category"], "premade");
            if entry.get("gender").is_some_and(|gender| gender == "it") {
                assert_eq!(listed["labels"]["use_case"], "characters_animation");
            }
        }
    }
}
