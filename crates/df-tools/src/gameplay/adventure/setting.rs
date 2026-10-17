//! Explicit scene geography and depicted-art contracts (adventure-generator §20).
use super::{
    GenerateError, Tables,
    generate::Roller,
    render,
    schema::{Destination, Location, Table, TextEntry},
    tables::{Art, TablesError, intersects},
};
use crate::gameplay::pack::SceneKey;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SettingKind {
    Inn,
    Temple,
    Archive,
    FerryOffice,
    Shop,
    HerbShed,
    LampRoom,
    Sheepfold,
    Guardhouse,
    BellChamber,
    Quay,
    ForestTrail,
    MountainPass,
    MarketStreet,
    GraveyardGate,
    MarshBoardwalk,
    Farmyard,
    LibraryHall,
    LibraryForecourt,
    NetLoft,
    ChandlersRoom,
    Boathouse,
    CharcoalHut,
    WoodlandLodge,
    HermitsCabin,
    PilgrimsHall,
    WaystationHall,
    Bothy,
    RefugeHall,
    TeaHouse,
    Apothecary,
    Theater,
    LanternShop,
    CaretakersLodge,
    Chapel,
    UndercroftStudy,
    Vestry,
    StiltHouse,
    Shrine,
    EelCatchersHall,
    FerrymansHut,
    FarmKitchen,
    Hayloft,
    MillHouse,
    CiderCellar,
    MapRoom,
    Bindery,
    ArchivistsStudy,
    ReadingGallery,
}
impl SettingKind {
    /// Whether a place of this kind is outdoors.
    pub(crate) fn is_exterior(self) -> bool {
        self.exterior()
    }
    fn exterior(self) -> bool {
        matches!(
            self,
            Self::Sheepfold
                | Self::Quay
                | Self::ForestTrail
                | Self::MountainPass
                | Self::MarketStreet
                | Self::GraveyardGate
                | Self::MarshBoardwalk
                | Self::Farmyard
                | Self::LibraryForecourt
        )
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct SceneSetting {
    pub(crate) place_id: String,
    pub(crate) name: String,
    pub(crate) short: String,
    pub(crate) kind: SettingKind,
    pub(crate) exterior: bool,
    pub(crate) art_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) model: Option<super::scene_model::SceneModel>,
}
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChapterSettings {
    pub(crate) meeting: SceneSetting,
    pub(crate) confrontation: SceneSetting,
    pub(crate) destination: SceneSetting,
}
impl ChapterSettings {
    pub(crate) fn scene(&self, key: SceneKey) -> &SceneSetting {
        match key {
            SceneKey::Opening | SceneKey::DialogueSeal | SceneKey::DialogueEscort => &self.meeting,
            SceneKey::Room | SceneKey::Combat | SceneKey::Defeat => &self.confrontation,
            SceneKey::Victory | SceneKey::Inn => &self.destination,
        }
    }
    pub(super) fn facts(
        &self,
        context: &mut BTreeMap<String, String>,
    ) -> Result<(), GenerateError> {
        for (prefix, place) in [
            ("meeting", &self.meeting),
            ("confrontation", &self.confrontation),
            ("destination", &self.destination),
        ] {
            for (field, value) in [
                ("name", place.name.clone()),
                ("short", place.short.clone()),
                ("art_key", place.art_key.clone()),
                ("place_id", place.place_id.clone()),
                (
                    "kind",
                    serde_json::to_value(place.kind)
                        .map_err(|_| GenerateError::Serialization)?
                        .as_str()
                        .ok_or(GenerateError::Serialization)?
                        .into(),
                ),
                ("exterior", place.exterior.to_string()),
            ] {
                context.insert(format!("setting.{prefix}.{field}"), value);
            }
        }
        Ok(())
    }
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Place {
    pub(super) kind: SettingKind,
    pub(super) name: String,
    pub(super) short: String,
    pub(super) exterior: bool,
    pub(super) art: String,
    pub(super) excluded_locations: Vec<String>,
    pub(super) details: Table<TextEntry>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Confrontation {
    pub(super) kind: SettingKind,
    pub(super) name: String,
    pub(super) short: String,
    pub(super) exterior: bool,
    pub(super) art: String,
    pub(super) bridges: Table<TextEntry>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DestinationPlace {
    pub(super) kind: SettingKind,
    pub(super) art: Option<String>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Places {
    pub(super) schema: String,
    pub(super) meeting_places: BTreeMap<String, Place>,
    pub(super) confrontation: BTreeMap<String, Confrontation>,
    pub(super) destinations: BTreeMap<String, DestinationPlace>,
    pub(super) contact_destinations: BTreeMap<String, Vec<SettingKind>>,
}
fn invalid(reason: impl Into<String>) -> TablesError {
    TablesError::Invalid {
        file: "places.json".into(),
        path: ".".into(),
        reason: reason.into(),
    }
}
impl Places {
    pub(super) fn patron_fits(&self, patron: &str, location: &Location) -> bool {
        self.meeting_places
            .get(patron)
            .is_some_and(|p| !p.excluded_locations.contains(&location.id))
    }
    pub(super) fn contact_fits(&self, contact: &str, destination: &str) -> bool {
        self.destinations.get(destination).is_some_and(|d| {
            self.contact_destinations
                .get(contact)
                .is_some_and(|k| k.contains(&d.kind))
        })
    }
    pub(super) fn contact_can_visit(
        &self,
        contact: &str,
        location: &Location,
        tables: &Tables,
    ) -> bool {
        tables.destinations.get(&location.id).is_some_and(|t| {
            t.entries
                .iter()
                .any(|d| self.available(&d.id) && self.contact_fits(contact, &d.id))
        })
    }
    pub(super) fn available(&self, destination: &str) -> bool {
        self.destinations
            .get(destination)
            .is_some_and(|d| d.art.is_some())
    }
    pub(super) fn validate(&self, tables: &Tables) -> Result<(), TablesError> {
        if self.schema != "df-adventure-places/1"
            || self.meeting_places.len() != tables.patrons.entries.len()
            || self.confrontation.len() != tables.locations.entries.len()
            || self.destinations.len()
                != tables
                    .destinations
                    .values()
                    .map(|t| t.entries.len())
                    .sum::<usize>()
            || self.contact_destinations.len() != tables.contacts.entries.len()
        {
            return Err(invalid("schema or table coverage"));
        }
        let depiction = |key: &str, kind: SettingKind| -> Result<(), TablesError> {
            if !tables.depicts.get(key).is_some_and(|k| k.contains(&kind)) {
                return Err(invalid(format!("art {key} does not depict {kind:?}")));
            }
            Ok(())
        };
        for npc in &tables.patrons.entries {
            let p = self
                .meeting_places
                .get(&npc.id)
                .ok_or_else(|| invalid("missing patron place"))?;
            depiction(&p.art, p.kind)?;
            if p.exterior != p.kind.exterior() {
                return Err(invalid("meeting exterior does not match kind"));
            }
            if p.details.die != 4
                || p.details.entries.len() != 4
                || p.excluded_locations
                    .iter()
                    .any(|id| !tables.locations.entries.iter().any(|l| &l.id == id))
            {
                return Err(invalid("meeting detail d4 or unknown excluded location"));
            }
        }
        for npc in &tables.contacts.entries {
            if self
                .contact_destinations
                .get(&npc.id)
                .is_none_or(Vec::is_empty)
            {
                return Err(invalid("missing contact compatibility"));
            }
        }
        for loc in &tables.locations.entries {
            let c = self
                .confrontation
                .get(&loc.id)
                .ok_or_else(|| invalid("missing confrontation"))?;
            depiction(&c.art, c.kind)?;
            if c.exterior != c.kind.exterior() {
                return Err(invalid("confrontation exterior does not match kind"));
            }
            // The fight's painting only has to depict its setting; the library, whose arrival
            // painting is its indoor hall, fights in its forecourt.
            if c.bridges.die != 4 || c.bridges.entries.len() != 4 {
                return Err(invalid("confrontation bridge d4"));
            }
            if !tables
                .patrons
                .entries
                .iter()
                .any(|p| intersects(&p.fits, &loc.tags) && self.patron_fits(&p.id, loc))
            {
                return Err(invalid("location has no patron meeting place"));
            }
            let destinations = tables
                .destinations
                .get(&loc.id)
                .ok_or_else(|| invalid("missing destinations"))?;
            for dest in &destinations.entries {
                let d = self
                    .destinations
                    .get(&dest.id)
                    .ok_or_else(|| invalid("missing destination setting"))?;
                if let Some(art) = &d.art {
                    depiction(art, d.kind)?;
                    if !tables.contacts.entries.iter().any(|c| {
                        intersects(&c.fits, &loc.tags) && self.contact_fits(&c.id, &dest.id)
                    }) {
                        return Err(invalid("destination has no compatible contact"));
                    }
                }
            }
            if !destinations.entries.iter().any(|d| self.available(&d.id)) {
                return Err(invalid("location has no depicted destination"));
            }
        }
        Ok(())
    }
}
pub(super) fn roll_settings(
    tables: &Tables,
    location: &Location,
    patron: &str,
    destination: &Destination,
    seed: u64,
    roller: &mut Roller<'_>,
    context: &mut BTreeMap<String, String>,
) -> Result<ChapterSettings, GenerateError> {
    let places = &tables.places;
    let meeting = places
        .meeting_places
        .get(patron)
        .ok_or_else(|| GenerateError::MissingTable("meeting place".into()))?;
    let confrontation = places
        .confrontation
        .get(&location.id)
        .ok_or_else(|| GenerateError::MissingTable("confrontation".into()))?;
    let dest = places
        .destinations
        .get(&destination.id)
        .ok_or_else(|| GenerateError::MissingTable("destination setting".into()))?;
    let meeting_detail = roller.text("setting.meeting.detail", &meeting.details, |_| true)?;
    let bridge = roller.text(
        "setting.confrontation.bridge",
        &confrontation.bridges,
        |_| true,
    )?;
    context.insert("setting.meeting.detail".into(), meeting_detail);
    context.insert("setting.confrontation.detail".into(), bridge);
    let mut settings = ChapterSettings {
        meeting: SceneSetting {
            place_id: meeting.art.clone(),
            name: render::render(&meeting.name, context)?,
            short: meeting.short.clone(),
            kind: meeting.kind,
            exterior: meeting.exterior,
            art_key: meeting.art.clone(),
            model: None,
        },
        confrontation: SceneSetting {
            place_id: location.id.clone(),
            name: confrontation.name.clone(),
            short: confrontation.short.clone(),
            kind: confrontation.kind,
            exterior: confrontation.exterior,
            art_key: confrontation.art.clone(),
            model: None,
        },
        destination: SceneSetting {
            place_id: destination.id.clone(),
            name: destination.name.clone(),
            short: destination.short.clone(),
            kind: dest.kind,
            exterior: false,
            art_key: dest
                .art
                .clone()
                .ok_or_else(|| GenerateError::Incompatible("destination art".into()))?,
            model: None,
        },
    };
    settings.meeting.model = Some(super::scene_model::SceneModel::fallback(
        &settings.meeting,
        super::scene_model::ScenePlace::Meeting,
        seed,
        context,
    )?);
    settings.confrontation.model = Some(super::scene_model::SceneModel::fallback(
        &settings.confrontation,
        super::scene_model::ScenePlace::Confrontation,
        seed,
        context,
    )?);
    settings.destination.model = Some(super::scene_model::SceneModel::fallback(
        &settings.destination,
        super::scene_model::ScenePlace::Destination,
        seed,
        context,
    )?);
    settings.facts(context)?;
    Ok(settings)
}
/// Validate declared art against the compiled manifest at the pack loading boundary.
pub(crate) fn validates(scene: &SceneSetting, asset: &str) -> bool {
    static ART: std::sync::OnceLock<Result<Vec<Art>, serde_json::Error>> =
        std::sync::OnceLock::new();
    let art = ART.get_or_init(|| {
        serde_json::from_slice(include_bytes!(
            "../../../../../content/adventure-tables/art-manifest.json"
        ))
    });
    !scene.place_id.is_empty()
        && !scene.name.is_empty()
        && !scene.short.is_empty()
        && scene.exterior == scene.kind.exterior()
        && scene
            .model
            .as_ref()
            .is_none_or(|model| model.validate_structure().is_ok())
        && art.as_ref().is_ok_and(|entries| {
            entries.iter().any(|a| {
                a.art_key == scene.art_key
                    && a.kind == "scene"
                    && a.depicts.contains(&scene.kind)
                    && asset == a.asset()
            })
        })
}
