use serde::{Deserialize, Serialize};
/// SRD Weapons table (p91); only carried weapons and the admitted stat block actions.
#[derive(Deserialize, Serialize, Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Weapon {
    Greatsword,
    Flail,
    Javelin,
    Spear,
    Longbow,
    Shortbow,
    Scimitar,
    Shortsword,
    LightCrossbow,
    Dagger,
    Sickle,
    Club,
    Bite,
    Slam,
    UnarmedStrike,
}
#[derive(Deserialize, Serialize, Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mastery {
    Graze,
    Sap,
    Slow,
    Vex,
    Nick,
}
#[derive(Deserialize, Serialize, Clone, Copy, Debug, Eq, PartialEq)]
pub enum DamageType {
    Slashing,
    Piercing,
    Bludgeoning,
    Necrotic,
    Poison,
}

impl DamageType {
    /// The 2024 rules' name of the type, lower case.
    pub fn id(self) -> &'static str {
        match self {
            Self::Slashing => "slashing",
            Self::Piercing => "piercing",
            Self::Bludgeoning => "bludgeoning",
            Self::Necrotic => "necrotic",
            Self::Poison => "poison",
        }
    }
}

impl Weapon {
    pub fn id(self) -> &'static str {
        match self {
            Self::Greatsword => "greatsword",
            Self::Flail => "flail",
            Self::Javelin => "javelin",
            Self::Spear => "spear",
            Self::Longbow => "longbow",
            Self::Shortbow => "shortbow",
            Self::Scimitar => "scimitar",
            Self::Shortsword => "shortsword",
            Self::LightCrossbow => "light-crossbow",
            Self::Dagger => "dagger",
            Self::Sickle => "ritual-sickle",
            Self::Club => "club",
            Self::Bite => "bite",
            Self::Slam => "slam",
            Self::UnarmedStrike => "unarmed-strike",
        }
    }
    pub fn mastery(self) -> Option<Mastery> {
        match self {
            Self::Greatsword => Some(Mastery::Graze),
            Self::Flail | Self::Spear => Some(Mastery::Sap),
            Self::Javelin | Self::Longbow | Self::LightCrossbow => Some(Mastery::Slow),
            Self::Shortbow | Self::Shortsword => Some(Mastery::Vex),
            Self::Scimitar | Self::Dagger | Self::Sickle => Some(Mastery::Nick),
            Self::Club => Some(Mastery::Sap),
            Self::Bite | Self::Slam | Self::UnarmedStrike => None,
        }
    }
    pub fn range(self) -> Option<(u16, u16)> {
        match self {
            Self::Longbow => Some((150, 600)),
            Self::Shortbow | Self::LightCrossbow => Some((80, 320)),
            Self::Javelin => Some((30, 120)),
            Self::Spear | Self::Dagger => Some((20, 60)),
            _ => None,
        }
    }
    pub fn ranged_weapon(self) -> bool {
        matches!(self, Self::Longbow | Self::Shortbow | Self::LightCrossbow)
    }
    pub fn ammunition(self) -> bool {
        self.ranged_weapon()
    }
    pub fn two_handed(self) -> bool {
        matches!(
            self,
            Self::Greatsword | Self::Longbow | Self::Shortbow | Self::LightCrossbow
        )
    }
    pub fn heavy(self) -> bool {
        matches!(self, Self::Greatsword | Self::Longbow)
    }
    pub fn finesse(self) -> bool {
        matches!(self, Self::Scimitar | Self::Shortsword | Self::Dagger)
    }
    pub fn light(self) -> bool {
        matches!(
            self,
            Self::Scimitar | Self::Shortsword | Self::Dagger | Self::Sickle
        )
    }
    /// Source weapon damage dice, including a selected Versatile grip.
    pub fn dice(self, two_hands: bool) -> (usize, u32) {
        match self {
            Self::UnarmedStrike => (0, 6),
            Self::Greatsword => (2, 6),
            Self::Flail | Self::Longbow | Self::LightCrossbow | Self::Slam => (1, 8),
            Self::Spear if two_hands => (1, 8),
            Self::Dagger | Self::Sickle => (1, 4),
            _ => (1, 6),
        }
    }
    pub fn damage_type(self) -> DamageType {
        match self {
            Self::Greatsword | Self::Scimitar | Self::Sickle => DamageType::Slashing,
            Self::Flail | Self::Slam | Self::UnarmedStrike => DamageType::Bludgeoning,
            _ => DamageType::Piercing,
        }
    }
}
