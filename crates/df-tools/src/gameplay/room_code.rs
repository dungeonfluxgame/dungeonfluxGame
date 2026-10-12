//! Each server run's own room code.
//!
//! A run shows its code on the display and players type it (or follow the `?room=` link) to
//! join, so two runs never share one. The code is derived from the run's identity (its database
//! name) rather than drawn at random: a restart of the same run keeps the code its players
//! already know. It is built from consonant-vowel syllables of unambiguous letters, so it can be
//! read aloud and cannot be mistaken for digits or for another letter.
//!
//! `DF_GAMEPLAY_ROOM_CODE` pins a code (for tests and demonstrations).

use std::{env::VarError, io};

use sha2::{Digest, Sha256};

/// Letters that are never confused with each other or with a digit, and sound clear aloud.
const CONSONANTS: &[u8] = b"BDFGKMNPRSTVZ";
const VOWELS: &[u8] = b"AEOU";
const SYLLABLES: usize = 3;
const MAXIMUM_PINNED_CHARACTERS: usize = 32;

/// The code of the run named `run`: the same run always yields the same code.
pub(super) fn generated(run: &str) -> String {
    let digest = Sha256::new()
        .chain_update(b"dungeonflux room code v1\n")
        .chain_update(run.as_bytes())
        .finalize();
    let mut code = String::with_capacity(SYLLABLES * 2);
    for syllable in 0..SYLLABLES {
        let (consonant, vowel) = (digest[syllable * 2], digest[syllable * 2 + 1]);
        code.push(char::from(
            CONSONANTS[usize::from(consonant) % CONSONANTS.len()],
        ));
        code.push(char::from(VOWELS[usize::from(vowel) % VOWELS.len()]));
    }
    code
}

/// The pinned code, upper-cased, or `None` when the setting is absent. A present but invalid
/// value stops startup without being echoed.
pub(super) fn pinned(input: Result<String, VarError>) -> Result<Option<String>, io::Error> {
    let value = match input {
        Ok(value) => value,
        Err(VarError::NotPresent) => return Ok(None),
        Err(VarError::NotUnicode(_)) => return Err(refused()),
    };
    let value = value.trim();
    if value.is_empty()
        || value.len() > MAXIMUM_PINNED_CHARACTERS
        || !value.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return Err(refused());
    }
    Ok(Some(value.to_ascii_uppercase()))
}

fn refused() -> io::Error {
    io::Error::other("DF_GAMEPLAY_ROOM_CODE refused: 1 to 32 ASCII letters or digits expected")
}

/// The code this run uses: the pinned one when set, otherwise the one derived from `run`.
pub(super) fn select(pin: Option<String>, run: &str) -> String {
    pin.unwrap_or_else(|| generated(run))
}

/// A public name for one server run: its database and the pack it plays. A page that keeps a
/// player's seat records it, and a seat saved under another run's name is discarded rather than
/// presented to a server that cannot know it.
pub(super) fn run_identity(run: &str, pack_revision: &str) -> String {
    let digest = Sha256::new()
        .chain_update(b"dungeonflux run identity v1\n")
        .chain_update(run.as_bytes())
        .chain_update(b"\n")
        .chain_update(pack_revision.as_bytes())
        .finalize();
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Whether the code a player supplied names this room. Case and surrounding space do not
/// matter; anything else must match exactly.
pub(super) fn matches(room: &str, supplied: &str) -> bool {
    room.eq_ignore_ascii_case(supplied.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_keeps_its_code_and_different_runs_differ() {
        let first = generated("df_gameplay_demo_play_20261011101500");
        assert_eq!(first, generated("df_gameplay_demo_play_20261011101500"));
        let others: Vec<String> = (0..40)
            .map(|number| generated(&format!("df_gameplay_demo_play_{number}")))
            .collect();
        let distinct: std::collections::BTreeSet<&String> = others.iter().collect();
        assert!(distinct.len() >= 38, "codes collide too often: {others:?}");
    }

    #[test]
    fn a_generated_code_is_short_pronounceable_and_unambiguous() {
        for number in 0..200 {
            let code = generated(&format!("run {number}"));
            assert_eq!(code.len(), SYLLABLES * 2);
            for (position, letter) in code.bytes().enumerate() {
                let allowed = if position % 2 == 0 {
                    CONSONANTS
                } else {
                    VOWELS
                };
                assert!(allowed.contains(&letter), "{code} breaks the pattern");
            }
        }
        assert!(
            !CONSONANTS
                .iter()
                .chain(VOWELS)
                .any(|letter| b"ILQ01".contains(letter))
        );
    }

    #[test]
    fn a_pinned_code_is_validated_and_wins() {
        assert_eq!(pinned(Err(VarError::NotPresent)).unwrap(), None);
        assert_eq!(
            pinned(Ok(" lantern ".to_owned())).unwrap().as_deref(),
            Some("LANTERN")
        );
        for bad in ["", "   ", "two words", "LANTERN!", &"A".repeat(33), "ÄÖÜ"] {
            assert!(pinned(Ok(bad.to_owned())).is_err(), "{bad:?} was accepted");
        }
        assert_eq!(select(Some("LANTERN".to_owned()), "any run"), "LANTERN");
        assert_eq!(select(None, "any run"), generated("any run"));
    }

    #[test]
    fn a_run_identity_names_the_database_and_the_pack() {
        let identity = run_identity("df_gameplay_demo_a", "abc");
        assert_eq!(identity.len(), 16);
        assert_eq!(identity, run_identity("df_gameplay_demo_a", "abc"));
        assert_ne!(identity, run_identity("df_gameplay_demo_b", "abc"));
        assert_ne!(identity, run_identity("df_gameplay_demo_a", "abd"));
    }

    #[test]
    fn a_supplied_code_matches_ignoring_case_and_space_only() {
        assert!(matches("BADOKU", "badoku"));
        assert!(matches("BADOKU", "  BADOKU "));
        assert!(!matches("BADOKU", "BADOKUS"));
        assert!(!matches("BADOKU", "BAD OKU"));
        assert!(!matches("BADOKU", ""));
    }
}
