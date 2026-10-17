//! Health-appropriate art for the characters of a run: which variants exist, which one a
//! character shows now, and when to ask the image service for one.
//!
//! A character's art has one variant per tier below Healthy (Healthy is the authored art, which
//! is always available and free). Variants are cached forever per source art and tier, so a run
//! pays for them once ever: this module only decides when a variant is asked for and what shows.
//!
//! * **On demand.** A variant is asked for only when a character is at that tier in a view: the
//!   first time a hero or a cast member reaches the tier, in a fight or out of one. Nothing is
//!   made ahead of time, and a character at full health asks for nothing.
//! * **Looked for, never made.** When a character first joins a view its cache is looked through
//!   once, so a variant made earlier shows at once.
//! * **Fallback while it is made.** A character shows the nearest variant already cached: an
//!   uncached tier keeps the closest cached one (ties go to the less severe), and the authored
//!   art when none is closer. The bar and motion cues carry the rest. Combat never waits on
//!   generation.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    imagery::{CharacterFacts, ImageKey},
    scene_mood::HealthTier,
};

/// The tiers below Healthy that a character can have a variant for, from least to most severe.
pub(super) const GENERATED_TIERS: [HealthTier; 4] = [
    HealthTier::Bloodied,
    HealthTier::Critical,
    HealthTier::Hurt,
    HealthTier::Down,
];

/// How many times a variant is tried before it is given up for this run.
pub(super) const MAXIMUM_ATTEMPTS: u8 = 2;

/// One character whose art has health variants: a hero of the party or a named member of the
/// scene's cast.
#[derive(Clone, Debug)]
pub(super) struct Character {
    /// Stable within the run: the hero's name or the cast member's id.
    pub(super) id: String,
    /// The served art the stage shows for this character; the variants are made from it.
    pub(super) art: String,
    pub(super) facts: CharacterFacts,
}

/// The variants known to exist in the cache, and what has been tried.
#[derive(Debug, Default)]
pub(super) struct Book {
    known: BTreeMap<(String, HealthTier), ImageKey>,
    /// Characters whose cache has been looked through once.
    peeked: BTreeSet<String>,
    /// How many attempts failed, by variant.
    failed: BTreeMap<(String, HealthTier), u8>,
    in_flight: BTreeSet<(String, HealthTier)>,
}

impl Book {
    pub(super) fn learn(&mut self, id: &str, tier: HealthTier, key: ImageKey) {
        self.in_flight.remove(&(id.to_owned(), tier));
        self.known.insert((id.to_owned(), tier), key);
    }

    pub(super) fn fail(&mut self, id: &str, tier: HealthTier) {
        self.in_flight.remove(&(id.to_owned(), tier));
        *self.failed.entry((id.to_owned(), tier)).or_default() += 1;
    }

    /// Whether this variant is neither known, out of attempts nor already being made.
    pub(super) fn is_open(&self, id: &str, tier: HealthTier) -> bool {
        let key = (id.to_owned(), tier);
        !self.known.contains_key(&key)
            && self.attempts(&key) < MAXIMUM_ATTEMPTS
            && !self.in_flight.contains(&key)
    }

    fn attempts(&self, key: &(String, HealthTier)) -> u8 {
        self.failed.get(key).copied().unwrap_or(0)
    }

    pub(super) fn mark_peeked(&mut self, id: &str) {
        self.peeked.insert(id.to_owned());
    }

    pub(super) fn needs_peek(&self, id: &str) -> bool {
        !self.peeked.contains(id)
    }

    pub(super) fn start_warming(&mut self, id: &str, tier: HealthTier) {
        self.in_flight.insert((id.to_owned(), tier));
    }

    /// The variant to show for `wanted`: the cached one of that tier, otherwise the nearest
    /// cached tier (ties go to the less severe). `None` means the authored art: either `wanted`
    /// is Healthy or nothing is cached closer than the authored art itself.
    pub(super) fn nearest(&self, id: &str, wanted: HealthTier) -> Option<(HealthTier, &ImageKey)> {
        if wanted == HealthTier::Healthy {
            return None;
        }
        let authored_distance = wanted.distance(HealthTier::Healthy);
        HealthTier::ALL
            .into_iter()
            .filter(|tier| *tier != HealthTier::Healthy)
            .filter_map(|tier| {
                self.known
                    .get(&(id.to_owned(), tier))
                    .map(|key| (tier, key))
            })
            .min_by_key(|(tier, _)| (wanted.distance(*tier), tier.severity()))
            .filter(|(tier, _)| wanted.distance(*tier) < authored_distance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(byte: char) -> ImageKey {
        ImageKey::parse(&byte.to_string().repeat(64)).unwrap()
    }

    #[test]
    fn a_variant_is_asked_for_once_at_a_time_and_at_most_twice_until_it_is_known() {
        let mut book = Book::default();
        assert!(book.is_open("hero-1", HealthTier::Critical));
        book.start_warming("hero-1", HealthTier::Critical);
        assert!(
            !book.is_open("hero-1", HealthTier::Critical),
            "one in flight at a time"
        );
        book.fail("hero-1", HealthTier::Critical);
        assert!(
            book.is_open("hero-1", HealthTier::Critical),
            "a failure is tried once more"
        );
        book.start_warming("hero-1", HealthTier::Critical);
        book.fail("hero-1", HealthTier::Critical);
        assert!(
            !book.is_open("hero-1", HealthTier::Critical),
            "given up after two attempts"
        );
        book.learn("hero-1", HealthTier::Hurt, key('d'));
        assert!(
            !book.is_open("hero-1", HealthTier::Hurt),
            "a known variant is never asked for again"
        );
    }

    #[test]
    fn the_nearest_cached_tier_stands_in_for_an_uncached_one_ties_to_the_less_severe() {
        let mut book = Book::default();
        assert_eq!(
            book.nearest("hero-1", HealthTier::Critical),
            None,
            "nothing cached: authored"
        );
        book.learn("hero-1", HealthTier::Bloodied, key('a'));
        book.learn("hero-1", HealthTier::Down, key('d'));
        let shown = |tier| book.nearest("hero-1", tier).map(|(tier, _)| tier);
        assert_eq!(shown(HealthTier::Bloodied), Some(HealthTier::Bloodied));
        // Critical is one from Bloodied and one from Down: the less severe wins the tie.
        assert_eq!(shown(HealthTier::Critical), Some(HealthTier::Bloodied));
        assert_eq!(shown(HealthTier::Down), Some(HealthTier::Down));
        // Hurt is one from the authored art and one from Bloodied: the tie goes to the less severe,
        // which is the authored art.
        assert_eq!(shown(HealthTier::Hurt), None);
        assert_eq!(shown(HealthTier::Healthy), None);
    }

    #[test]
    fn a_cached_variant_farther_than_the_authored_art_is_not_used() {
        let mut book = Book::default();
        book.learn("hero-1", HealthTier::Down, key('d'));
        // Hurt is 1 from Healthy and 3 from Down: the authored art is closer.
        assert_eq!(book.nearest("hero-1", HealthTier::Hurt), None);
        // Bloodied is 2 from Healthy and 2 from Down: a tie is not closer than authored.
        assert_eq!(book.nearest("hero-1", HealthTier::Bloodied), None);
        // Critical is 3 from Healthy and 1 from Down.
        assert_eq!(
            book.nearest("hero-1", HealthTier::Critical)
                .map(|(tier, _)| tier),
            Some(HealthTier::Down)
        );
    }

    #[test]
    fn a_character_is_looked_through_once() {
        let mut book = Book::default();
        assert!(book.needs_peek("innkeeper"));
        book.mark_peeked("innkeeper");
        assert!(!book.needs_peek("innkeeper"));
    }
}
