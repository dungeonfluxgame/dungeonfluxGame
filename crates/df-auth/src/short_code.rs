//! A short, speakable code (`planning/campaigns-and-table-research.md` 4.2): eight symbols of the
//! Crockford base-32 alphabet (digits and letters without I, L, O, U), 40 bits, shown as
//! `ABCD-EFGH`. It serves the room code and the display pairing code.
//!
//! The code is discovery, never authority. This module only builds and normalizes the text; it
//! interprets no player words (the look-alike mapping is syntax normalization of a code). The value
//! has no `Display` and its `Debug` hides it, so it cannot reach a log by accident.

use std::fmt;

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const LENGTH: usize = 8;

/// Why typed text is not a code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShortCodeRefusal {
    WrongLength,
    UnknownSymbol,
}

/// A 40-bit short code in canonical form.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct ShortCode([u8; LENGTH]);

impl ShortCode {
    /// Builds a code from 40 bits of caller-supplied entropy (five bytes, big-endian).
    pub fn from_entropy(entropy: [u8; 5]) -> Self {
        let bits = entropy
            .iter()
            .fold(0_u64, |acc, byte| (acc << 8) | u64::from(*byte));
        let mut symbols = [0_u8; LENGTH];
        for (index, symbol) in symbols.iter_mut().enumerate() {
            let shift = 5 * (LENGTH - 1 - index);
            // The mask keeps five bits, so the index is below 32.
            *symbol = ALPHABET[usize::try_from((bits >> shift) & 31).unwrap_or(0)];
        }
        Self(symbols)
    }

    /// Normalizes typed text: trims, drops dashes and spaces, upper-cases, and maps the look-alikes
    /// (`O` to `0`, `I` and `L` to `1`).
    pub fn parse(text: &str) -> Result<Self, ShortCodeRefusal> {
        let mut symbols = [0_u8; LENGTH];
        let mut count = 0;
        for character in text.trim().chars().filter(|c| !matches!(c, '-' | ' ')) {
            if count == LENGTH {
                return Err(ShortCodeRefusal::WrongLength);
            }
            let upper = character.to_ascii_uppercase();
            let mapped = match upper {
                'O' => '0',
                'I' | 'L' => '1',
                other => other,
            };
            let byte = u8::try_from(mapped).map_err(|_| ShortCodeRefusal::UnknownSymbol)?;
            if !ALPHABET.contains(&byte) {
                return Err(ShortCodeRefusal::UnknownSymbol);
            }
            symbols[count] = byte;
            count += 1;
        }
        if count != LENGTH {
            return Err(ShortCodeRefusal::WrongLength);
        }
        Ok(Self(symbols))
    }

    /// The eight canonical symbols, for digests and storage.
    pub fn canonical(&self) -> &[u8; LENGTH] {
        &self.0
    }

    /// The text shown to a person: `ABCD-EFGH`.
    pub fn display_form(&self) -> String {
        let text: String = self.0.iter().map(|byte| char::from(*byte)).collect();
        format!("{}-{}", &text[..4], &text[4..])
    }

    /// The eight canonical symbols as text, for storage columns.
    pub fn canonical_text(&self) -> String {
        self.0.iter().map(|byte| char::from(*byte)).collect()
    }
}

impl fmt::Debug for ShortCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ShortCode(..)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entropy_maps_to_eight_symbols_of_the_alphabet() {
        let code = ShortCode::from_entropy([0, 0, 0, 0, 0]);
        assert_eq!(code.canonical_text(), "00000000");
        let code = ShortCode::from_entropy([0xff; 5]);
        assert_eq!(code.canonical_text(), "ZZZZZZZZ");
        let code = ShortCode::from_entropy([0x12, 0x34, 0x56, 0x78, 0x9a]);
        assert_eq!(code.canonical_text().len(), 8);
        assert!(code.canonical().iter().all(|byte| ALPHABET.contains(byte)));
    }

    #[test]
    fn the_alphabet_has_thirty_two_distinct_symbols_without_look_alikes() {
        let mut seen = ALPHABET.to_vec();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), 32);
        for banned in *b"ILOU" {
            assert!(!ALPHABET.contains(&banned));
        }
    }

    #[test]
    fn parsing_normalizes_case_dashes_spaces_and_look_alikes() {
        let canonical = ShortCode::parse("AB01-CDEF").unwrap();
        assert_eq!(ShortCode::parse(" ab01 cdef ").unwrap(), canonical);
        assert_eq!(ShortCode::parse("abOl-cdef").unwrap(), canonical);
        assert_eq!(
            ShortCode::parse("abIL-cdef").unwrap().canonical_text(),
            "AB11CDEF"
        );
    }

    #[test]
    fn display_form_round_trips() {
        let code = ShortCode::from_entropy([1, 2, 3, 4, 5]);
        assert_eq!(ShortCode::parse(&code.display_form()).unwrap(), code);
        assert_eq!(code.display_form().len(), 9);
    }

    #[test]
    fn wrong_length_and_unknown_symbols_are_refused() {
        assert_eq!(
            ShortCode::parse("ABCD-EFG"),
            Err(ShortCodeRefusal::WrongLength)
        );
        assert_eq!(
            ShortCode::parse("ABCD-EFGHJ"),
            Err(ShortCodeRefusal::WrongLength)
        );
        assert_eq!(
            ShortCode::parse("ABCD-EFGU"),
            Err(ShortCodeRefusal::UnknownSymbol)
        );
        assert_eq!(
            ShortCode::parse("ABCD-EF!H"),
            Err(ShortCodeRefusal::UnknownSymbol)
        );
        assert_eq!(ShortCode::parse(""), Err(ShortCodeRefusal::WrongLength));
    }

    #[test]
    fn debug_never_shows_the_code() {
        let code = ShortCode::from_entropy([9, 8, 7, 6, 5]);
        assert_eq!(format!("{code:?}"), "ShortCode(..)");
    }
}
