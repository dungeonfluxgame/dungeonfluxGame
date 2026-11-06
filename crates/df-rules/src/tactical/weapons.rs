use crate::listed::listed_enum;
use serde::{Deserialize, Serialize};
listed_enum! {
    keyed
    /// SRD Weapons table (p91); only carried weapons and the admitted stat block actions.
    #[derive(Deserialize, Serialize, Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
    #[serde(rename_all = "snake_case")]
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
        Greatclub,
        Rend,
        TreeClub,
        NecroticSword,
        NecroticBow,
    }
}
listed_enum! {
    keyed
    /// A weapon mastery property (SRD p. 90).
    #[derive(Deserialize, Serialize, Clone, Copy, Debug, Eq, PartialEq)]
    #[serde(rename_all = "snake_case")]
    pub enum Mastery {
        Graze,
        Sap,
        Slow,
        Vex,
        Nick,
    }
}
impl Mastery {
    /// The stable key of this mastery property, as it is serialized.
    pub fn key(self) -> &'static str {
        match self {
            Self::Graze => "graze",
            Self::Sap => "sap",
            Self::Slow => "slow",
            Self::Vex => "vex",
            Self::Nick => "nick",
        }
    }
}
listed_enum! {
    keyed
    /// A damage type (SRD p. 16).
    #[derive(Deserialize, Serialize, Clone, Copy, Debug, Eq, PartialEq)]
    #[serde(rename_all = "snake_case")]
    pub enum DamageType {
        Slashing,
        Piercing,
        Bludgeoning,
        Necrotic,
        Poison,
        Acid,
        Cold,
        Fire,
        Lightning,
        Thunder,
    }
}

impl DamageType {
    /// The stable key of this type, as it is serialized: the 2024 rules' name, lower case.
    pub fn key(self) -> &'static str {
        match self {
            Self::Slashing => "slashing",
            Self::Piercing => "piercing",
            Self::Bludgeoning => "bludgeoning",
            Self::Necrotic => "necrotic",
            Self::Poison => "poison",
            Self::Acid => "acid",
            Self::Cold => "cold",
            Self::Fire => "fire",
            Self::Lightning => "lightning",
            Self::Thunder => "thunder",
        }
    }
}

impl Weapon {
    /// The stable key of this weapon, as it is serialized.
    pub fn key(self) -> &'static str {
        match self {
            Self::Greatsword => "greatsword",
            Self::Flail => "flail",
            Self::Javelin => "javelin",
            Self::Spear => "spear",
            Self::Longbow => "longbow",
            Self::Shortbow => "shortbow",
            Self::Scimitar => "scimitar",
            Self::Shortsword => "shortsword",
            Self::LightCrossbow => "light_crossbow",
            Self::Dagger => "dagger",
            Self::Sickle => "sickle",
            Self::Club => "club",
            Self::Bite => "bite",
            Self::Slam => "slam",
            Self::UnarmedStrike => "unarmed_strike",
            Self::Greatclub => "greatclub",
            Self::Rend => "rend",
            Self::TreeClub => "tree_club",
            Self::NecroticSword => "necrotic_sword",
            Self::NecroticBow => "necrotic_bow",
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
            Self::Bite
            | Self::Slam
            | Self::UnarmedStrike
            | Self::Greatclub
            | Self::Rend
            | Self::TreeClub
            | Self::NecroticSword
            | Self::NecroticBow => None,
        }
    }
    /// Normal and long range in feet, for ranged and thrown weapons.
    pub fn range(self) -> Option<(u16, u16)> {
        match self {
            Self::Longbow | Self::NecroticBow => Some((150, 600)),
            Self::Shortbow | Self::LightCrossbow => Some((80, 320)),
            Self::Javelin => Some((30, 120)),
            Self::Spear | Self::Dagger => Some((20, 60)),
            _ => None,
        }
    }
    pub fn ranged_weapon(self) -> bool {
        matches!(
            self,
            Self::Longbow | Self::Shortbow | Self::LightCrossbow | Self::NecroticBow
        )
    }
    pub fn ammunition(self) -> bool {
        matches!(self, Self::Longbow | Self::Shortbow | Self::LightCrossbow)
    }
    pub fn thrown(self) -> bool {
        matches!(self, Self::Javelin | Self::Spear | Self::Dagger)
    }
    pub fn natural(self) -> bool {
        matches!(
            self,
            Self::Bite
                | Self::Slam
                | Self::Rend
                | Self::TreeClub
                | Self::NecroticSword
                | Self::NecroticBow
        )
    }
    pub fn two_handed(self) -> bool {
        matches!(
            self,
            Self::Greatsword
                | Self::Longbow
                | Self::Shortbow
                | Self::LightCrossbow
                | Self::NecroticBow
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
            Self::Greatclub => (2, 8),
            Self::TreeClub => (3, 8),
            Self::Rend => (2, 6),
            Self::NecroticSword | Self::NecroticBow => (1, 8),
            Self::Flail | Self::Longbow | Self::LightCrossbow | Self::Slam => (1, 8),
            Self::Spear if two_hands => (1, 8),
            Self::Dagger | Self::Sickle => (1, 4),
            _ => (1, 6),
        }
    }
    pub fn damage_type(self) -> DamageType {
        match self {
            Self::Greatsword | Self::Scimitar | Self::Sickle | Self::Rend | Self::NecroticSword => {
                DamageType::Slashing
            }
            Self::Flail | Self::Slam | Self::UnarmedStrike | Self::Greatclub | Self::TreeClub => {
                DamageType::Bludgeoning
            }
            _ => DamageType::Piercing,
        }
    }
}
