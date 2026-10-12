//! Health-appropriate art for the characters of a run: which variants exist, which one a
//! character shows now, and which one to ask the image service for next.
//!
//! A character's art has one variant per tier below Healthy (Healthy is the authored art, which
//! is always available and free). Variants are cached forever per source art and tier, so a run
//! pays for them once ever: this module only decides order and permission.
//!
//! * **Pre-warming** asks for the variants of every hero and every named cast member in the
//!   background, lowest tier distance first (Bloodied, then Critical, then Hurt, then Down), one
//!   at a time, only while budget beyond [`PREWARM_RESERVE`] calls remains and never during a
//!   fight.
//! * **In a fight nothing is requested.** A character shows the nearest variant already cached:
//!   an uncached tier keeps the closest cached one (ties go to the less severe) and the bar and
//!   motion cues carry the rest. Combat never waits on generation.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    imagery::{CharacterFacts, ImageKey},
    scene_mood::HealthTier,
};

/// The order variants are warmed in: the tiers a fight is most likely to reach first.
pub(super) const PREWARM_ORDER: [HealthTier; 4] = [
    HealthTier::Bloodied,
    HealthTier::Critical,
    HealthTier::Hurt,
    HealthTier::Down,
];

/// Image-service calls kept back for mood art; pre-warming stops when no more than this remain.
pub(super) const PREWARM_RESERVE: u32 = 10;

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
    failed: BTreeSet<(String, HealthTier)>,
    in_flight: BTreeSet<(String, HealthTier)>,
}

impl Book {
    pub(super) fn learn(&mut self, id: &str, tier: HealthTier, key: ImageKey) {
        self.in_flight.remove(&(id.to_owned(), tier));
        self.known.insert((id.to_owned(), tier), key);
    }

    pub(super) fn fail(&mut self, id: &str, tier: HealthTier) {
        self.in_flight.remove(&(id.to_owned(), tier));
        self.failed.insert((id.to_owned(), tier));
    }

    /// Gives up an attempt without remembering it as a failure (the budget ran out).
    pub(super) fn release(&mut self, id: &str, tier: HealthTier) {
        self.in_flight.remove(&(id.to_owned(), tier));
    }

    /// Whether this variant is neither known, failed nor already being made.
    pub(super) fn is_open(&self, id: &str, tier: HealthTier) -> bool {
        let key = (id.to_owned(), tier);
        !self.known.contains_key(&key)
            && !self.failed.contains(&key)
            && !self.in_flight.contains(&key)
    }

    pub(super) fn mark_peeked(&mut self, id: &str) {
        self.peeked.insert(id.to_owned());
    }

    pub(super) fn needs_peek(&self, id: &str) -> bool {
        !self.peeked.contains(id)
    }

    pub(super) fn is_warming(&self) -> bool {
        !self.in_flight.is_empty()
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

    /// The next variant to warm, or `None`: never in a fight, never while one is in flight,
    /// never when the budget has only the reserve left. Characters are taken in the given order
    /// (heroes first), tiers in [`PREWARM_ORDER`], one tier across all characters before the
    /// next.
    pub(super) fn next_warm(
        &self,
        characters: &[Character],
        budget_remaining: u32,
        in_combat: bool,
    ) -> Option<(usize, HealthTier)> {
        if in_combat || self.is_warming() || budget_remaining <= PREWARM_RESERVE {
            return None;
        }
        PREWARM_ORDER.into_iter().find_map(|tier| {
            characters
                .iter()
                .enumerate()
                .find_map(|(index, character)| {
                    let key = (character.id.clone(), tier);
                    (!self.known.contains_key(&key) && !self.failed.contains(&key))
                        .then_some((index, tier))
                })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        super::imagery::{CharacterRole, LifeStage},
        *,
    };

    fn key(byte: char) -> ImageKey {
        ImageKey::parse(&byte.to_string().repeat(64)).unwrap()
    }

    fn character(id: &str) -> Character {
        Character {
            id: id.to_owned(),
            art: format!("assets/ui/generated/sprite-{id}"),
            facts: CharacterFacts {
                role: CharacterRole::Contact,
                life_stage: LifeStage::Adult,
            },
        }
    }

    fn cast() -> Vec<Character> {
        vec![
            character("hero-1"),
            character("hero-2"),
            character("innkeeper"),
        ]
    }

    #[test]
    fn warming_goes_bloodied_critical_hurt_down_one_tier_across_everyone_first() {
        let mut book = Book::default();
        let characters = cast();
        let mut order = Vec::new();
        while let Some((index, tier)) = book.next_warm(&characters, 40, false) {
            order.push((characters[index].id.clone(), tier.label()));
            book.learn(&characters[index].id, tier, key('a'));
        }
        let tiers: Vec<&str> = order.iter().map(|(_, tier)| *tier).collect();
        assert_eq!(
            tiers,
            [
                "bloodied", "bloodied", "bloodied", "critical", "critical", "critical", "hurt",
                "hurt", "hurt", "down", "down", "down"
            ]
        );
        // Heroes before the cast within a tier.
        assert_eq!(order[0].0, "hero-1");
        assert_eq!(order[2].0, "innkeeper");
        assert_eq!(order.len(), 12);
    }

    #[test]
    fn warming_never_happens_in_a_fight_or_while_one_is_in_flight_or_inside_the_reserve() {
        let characters = cast();
        let mut book = Book::default();
        assert!(
            book.next_warm(&characters, 40, true).is_none(),
            "combat never requests"
        );
        assert!(
            book.next_warm(&characters, PREWARM_RESERVE, false)
                .is_none()
        );
        assert!(
            book.next_warm(&characters, PREWARM_RESERVE + 1, false)
                .is_some()
        );
        book.start_warming("hero-1", HealthTier::Bloodied);
        assert!(
            book.next_warm(&characters, 40, false).is_none(),
            "one at a time"
        );
        book.learn("hero-1", HealthTier::Bloodied, key('b'));
        assert!(book.next_warm(&characters, 40, false).is_some());
    }

    #[test]
    fn a_failed_variant_is_not_asked_for_again_and_a_known_one_is_skipped() {
        let characters = vec![character("hero-1")];
        let mut book = Book::default();
        book.learn("hero-1", HealthTier::Bloodied, key('a'));
        book.start_warming("hero-1", HealthTier::Critical);
        book.fail("hero-1", HealthTier::Critical);
        assert_eq!(
            book.next_warm(&characters, 40, false),
            Some((0, HealthTier::Hurt))
        );
        book.fail("hero-1", HealthTier::Hurt);
        book.learn("hero-1", HealthTier::Down, key('c'));
        assert_eq!(book.next_warm(&characters, 40, false), None);
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
