//! Pure campaign frame, chapter decisions and continuity; adventure-generator §§13–16.
use super::campaign_tables::*;
use super::{
    ForcedEntries, GenerateError, GenerateOptions, Roll, RollStatus, RulesMode,
    rng::Dice,
    schema::{Antagonist, Hook, Location, NpcArchetype, Stake, Table, TextEntry},
    tables::intersects,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

const STEP: u64 = 0x9e37_79b9_7f4a_7c15;
const FRAME_STREAM: u64 = 0xca3d_a16b_5eed;
#[derive(Debug)]
pub(crate) enum CampaignError {
    Missing(String),
    Invalid(String),
    Complete,
    Serialization,
    Generate(GenerateError),
}
impl fmt::Display for CampaignError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(s) => write!(f, "campaign table missing: {s}"),
            Self::Invalid(s) => write!(f, "campaign refused: {s}"),
            Self::Complete => f.write_str("campaign complete"),
            Self::Serialization => f.write_str("campaign serialization failed"),
            Self::Generate(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for CampaignError {}
impl From<GenerateError> for CampaignError {
    fn from(e: GenerateError) -> Self {
        Self::Generate(e)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CampaignFrame {
    pub(crate) schema: String,
    pub(crate) generator_version: String,
    pub(crate) seed: u64,
    pub(crate) tables_revision: String,
    pub(crate) rules_mode: RulesMode,
    pub(crate) title: String,
    pub(super) premise: Premise,
    pub(super) villain: Villain,
    pub(super) artifact: Artifact,
    pub(crate) region: String,
    pub(super) region_short: String,
    pub(crate) sites: Vec<CampaignSite>,
    pub(super) factions: Vec<CampaignFaction>,
    pub(super) patron: CampaignAlly,
    pub(super) contact: CampaignAlly,
    pub(crate) themes: [String; 2],
    pub(crate) tone: String,
    pub(crate) clock: Vec<String>,
    pub(super) lieutenant_fates: Table<LieutenantFate>,
    pub(crate) acts: Vec<String>,
    pub(crate) outline: Vec<ChapterSlot>,
    pub(super) twists: [Twist; 2],
    pub(super) milestones: [String; 3],
    pub(crate) rolls: Vec<Roll>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Villain {
    pub(super) kind: VillainType,
    pub(super) name: String,
    pub(super) title: String,
    pub(super) motive: String,
    pub(super) scheme: String,
    pub(super) method: String,
    pub(super) resources: String,
    pub(super) weakness: Weakness,
    pub(super) signature: String,
    pub(super) lieutenants: Vec<Lieutenant>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CampaignSite {
    pub(crate) role: String,
    pub(crate) location: String,
    pub(crate) name: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CampaignFaction {
    pub(super) faction: Faction,
    pub(super) party: Attitude,
    pub(super) villain: Attitude,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CampaignAlly {
    pub(super) archetype: String,
    pub(super) first: String,
    pub(super) surname: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChapterSlot {
    pub(crate) number: u8,
    pub(crate) act: u8,
    pub(crate) role: String,
    pub(crate) chapter_type: String,
    pub(crate) pillars: [u8; 3],
    pub(crate) allowed: Vec<String>,
    pub(crate) skipped: bool,
    pub(crate) needs: Vec<String>,
    pub(crate) nodes: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ChapterResult {
    Victory,
    Defeat,
    Abandoned,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Approach {
    Asked,
    Escorted,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChapterOutcome {
    pub(crate) result: ChapterResult,
    pub(crate) approach: Approach,
    pub(crate) member: String,
    pub(crate) rested: bool,
    pub(crate) destination_reached: bool,
    pub(crate) antagonist_spared: bool,
    pub(crate) campaign_time: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "member", rename_all = "snake_case")]
pub(crate) enum Audience {
    Public,
    Private(String),
    Dm,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EstablishedFact {
    pub(crate) id: String,
    pub(crate) text: String,
    pub(crate) audience: Audience,
    pub(crate) established_in: u8,
    pub(crate) revealed: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClueDelivery {
    pub(crate) node: String,
    pub(crate) clue_id: String,
    pub(crate) text: String,
    pub(crate) vehicle: String,
    pub(crate) chapter: u8,
    pub(crate) audience: Audience,
    pub(crate) delivered: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NpcState {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) attitude: String,
    pub(crate) status: String,
    pub(crate) last_seen: u8,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LieutenantState {
    pub(crate) id: String,
    pub(crate) fate: String,
    pub(crate) returned: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClockAdvance {
    pub(crate) chapter: u8,
    pub(crate) delta: u8,
    pub(crate) reason: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChapterRecord {
    pub(crate) plan: ChapterPlan,
    pub(crate) run_database: String,
    pub(crate) adventure_digest: String,
    pub(crate) outcome: Option<ChapterOutcome>,
    pub(crate) lieutenant_fate: Option<String>,
}
/// Canonical continuity authority. The frame is immutable; append outcomes in chapter order.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CampaignRecord {
    pub(crate) schema: String,
    pub(crate) frame: CampaignFrame,
    pub(crate) frame_digest: String,
    pub(crate) clock_position: u8,
    pub(crate) clock_history: Vec<ClockAdvance>,
    pub(crate) chapters: Vec<ChapterRecord>,
    pub(crate) facts: Vec<EstablishedFact>,
    pub(crate) clues: Vec<ClueDelivery>,
    pub(crate) npcs: Vec<NpcState>,
    pub(crate) lieutenants: Vec<LieutenantState>,
    pub(crate) rolls: Vec<Roll>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StorySoFar {
    pub(crate) campaign: String,
    pub(crate) region: String,
    pub(crate) tone: String,
    pub(crate) teaser: String,
    pub(crate) chapters: Vec<String>,
    pub(crate) public_facts: Vec<String>,
    pub(crate) private_facts: BTreeMap<String, Vec<String>>,
    pub(crate) open_leads: Vec<String>,
    pub(crate) latest_sign: String,
    pub(crate) npcs: Vec<NpcState>,
    pub(crate) hint_only: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChapterPlan {
    pub(crate) number: u8,
    pub(crate) act: u8,
    pub(crate) chapter_type: String,
    pub(crate) title: String,
    pub(crate) site: String,
    pub(crate) seed: u64,
    pub(crate) outcome_digest_in: String,
    pub(crate) forced: ForcedEntries,
    pub(crate) leads: Vec<ClueDelivery>,
    pub(crate) rescued: Vec<ClueDelivery>,
    pub(crate) twist: Option<String>,
    pub(crate) milestone: Option<String>,
    pub(crate) hints: Vec<String>,
    pub(crate) recap: StorySoFar,
    pub(super) patron: CampaignAlly,
    pub(super) contact: CampaignAlly,
    pub(crate) hook_context: String,
    pub(crate) desperate: bool,
    pub(crate) returning_lieutenant: Option<String>,
    pub(crate) rolls: Vec<Roll>,
}
trait Entry {
    fn faces(&self) -> [u8; 2];
    fn id(&self) -> &str;
}
macro_rules! identified {($($t:ty),*)=>{$(impl Entry for $t{fn faces(&self)->[u8;2]{self.faces} fn id(&self)->&str{&self.id}})*};}
identified!(
    Premise,
    Artifact,
    SiteRole,
    VillainType,
    Scheme,
    Weakness,
    Lieutenant,
    Faction,
    Attitude,
    ChapterType,
    Twist,
    Location,
    NpcArchetype,
    Hook,
    Antagonist,
    Stake
);
impl Entry for TextEntry {
    fn faces(&self) -> [u8; 2] {
        self.faces
    }
    fn id(&self) -> &str {
        &self.text
    }
}
impl Entry for RegionName {
    fn faces(&self) -> [u8; 2] {
        self.faces
    }
    fn id(&self) -> &str {
        &self.name
    }
}
impl Entry for ChapterCount {
    fn faces(&self) -> [u8; 2] {
        self.faces
    }
    fn id(&self) -> &str {
        "chapter-count"
    }
}
impl Entry for Clue {
    fn faces(&self) -> [u8; 2] {
        self.faces
    }
    fn id(&self) -> &str {
        &self.p
    }
}
impl Entry for LieutenantFate {
    fn faces(&self) -> [u8; 2] {
        self.faces
    }
    fn id(&self) -> &str {
        &self.id
    }
}
struct Roller {
    dice: Dice,
    rolls: Vec<Roll>,
}
impl Roller {
    fn new(seed: u64) -> Self {
        Self {
            dice: Dice::new(seed),
            rolls: Vec::new(),
        }
    }
    fn record(&mut self, name: &str, die: u8, natural: u8, id: &str, status: RollStatus) {
        self.rolls.push(Roll {
            n: self.rolls.len() + 1,
            table: name.into(),
            die,
            natural,
            entry_id: id.into(),
            status,
        });
    }
    fn roll(&mut self, name: &str, die: u8) -> u8 {
        let n = self.dice.roll(die);
        self.record(name, die, n, &n.to_string(), RollStatus::Ok);
        n
    }
    fn select<'a, T: Entry>(
        &mut self,
        name: &str,
        table: &'a Table<T>,
        accept: impl Fn(&T) -> bool,
    ) -> Result<&'a T, CampaignError> {
        for _ in 0..64 {
            let natural = self.dice.roll(table.die);
            let entry = table
                .entries
                .iter()
                .find(|e| {
                    let [a, b] = e.faces();
                    (a..=b).contains(&natural)
                })
                .ok_or_else(|| CampaignError::Missing(name.into()))?;
            let valid = accept(entry);
            self.record(
                name,
                table.die,
                natural,
                &if entry.id().contains('{') {
                    format!("variant {}", entry.faces()[0])
                } else {
                    entry.id().to_owned()
                },
                if valid {
                    RollStatus::Ok
                } else {
                    RollStatus::Rejected
                },
            );
            if valid {
                return Ok(entry);
            }
        }
        let e = table
            .entries
            .iter()
            .find(|e| accept(e))
            .ok_or_else(|| CampaignError::Invalid(format!("no compatible {name}")))?;
        self.record(
            name,
            table.die,
            0,
            &if e.id().contains('{') {
                format!("variant {}", e.faces()[0])
            } else {
                e.id().to_owned()
            },
            RollStatus::Fallback,
        );
        Ok(e)
    }
    fn text(&mut self, name: &str, table: &Table<TextEntry>) -> Result<String, CampaignError> {
        Ok(self.select(name, table, |_| true)?.text.clone())
    }
}
fn named<'a, T>(map: &'a BTreeMap<String, T>, key: &str) -> Result<&'a T, CampaignError> {
    map.get(key)
        .ok_or_else(|| CampaignError::Missing(key.into()))
}
fn finalize(mut v: u64) -> u64 {
    v = (v ^ (v >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    v = (v ^ (v >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    v ^ (v >> 31)
}
/// Sorted JSON object keys, compact UTF-8, no trailing newline; arrays retain semantic order.
pub(crate) fn canonical_bytes<T: Serialize>(record: &T) -> Result<Vec<u8>, CampaignError> {
    let value = serde_json::to_value(record).map_err(|_| CampaignError::Serialization)?;
    serde_json::to_vec(&value).map_err(|_| CampaignError::Serialization)
}
pub(crate) fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn ally(
    roller: &mut Roller,
    tables: &CampaignTables,
    prefix: &str,
    table: &Table<NpcArchetype>,
    location: &Location,
    other: Option<&CampaignAlly>,
) -> Result<CampaignAlly, CampaignError> {
    let a = roller.select(&format!("ally.{prefix}"), table, |a| {
        intersects(&a.fits, &location.tags)
    })?;
    let first = roller
        .select(
            &format!("ally.{prefix}.given"),
            named(&tables.chapter.names, &a.gender)?,
            |e| other.is_none_or(|o| o.first != e.text),
        )?
        .text
        .clone();
    let surname = roller
        .select(
            &format!("ally.{prefix}.surname"),
            named(&tables.chapter.names, "surnames")?,
            |entry| other.is_none_or(|ally| ally.surname != entry.text),
        )?
        .text
        .clone();
    Ok(CampaignAlly {
        archetype: a.id.clone(),
        first,
        surname,
    })
}
fn frame_region_fit(f: &Faction, sites: &[CampaignSite], tables: &CampaignTables) -> bool {
    sites.iter().any(|s| {
        tables
            .chapter
            .locations
            .entries
            .iter()
            .any(|l| l.id == s.location && intersects(&f.fits, &l.tags))
    })
}
/// Roll an immutable, coherent frame using the independent campaign stream. No I/O.
pub(crate) fn generate_frame(
    tables: &CampaignTables,
    seed: u64,
    options: GenerateOptions,
) -> Result<CampaignFrame, CampaignError> {
    let mut r = Roller::new(seed ^ FRAME_STREAM);
    let p = &tables.premises.tables;
    let v = &tables.villains.tables;
    let premise = r.select("premise", &p.premises, |_| true)?.clone();
    let kind = r
        .select("villain.type", &v.types, |t| {
            premise.villain_types.contains(&t.id)
        })?
        .clone();
    let name = r.text(
        "villain.name",
        named(&tables.chapter.names, &format!("villain_{}", kind.gender))?,
    )?;
    let adjective = r.text(
        "villain.title",
        named(&tables.chapter.names, "villain_adjective")?,
    )?;
    let villain_title = format!(
        "{adjective} {}",
        named(&tables.villains.title_nouns, &kind.id)?
    );
    let motive = r.text("villain.motive", &v.motives)?;
    let scheme = r
        .select("villain.scheme", &v.schemes, |s| {
            s.families.contains(&premise.family)
        })?
        .text
        .clone();
    let method = r.text("villain.method", &v.methods)?;
    let resources = r.text("villain.resources", &v.resources)?;
    let weakness = r
        .select("villain.weakness", &v.weaknesses, |_| true)?
        .clone();
    let signature = r.text("villain.signature", &v.signatures)?;
    let artifact = r.select("artifact", &p.artifacts, |_| true)?.clone();
    let region = r.select("region.name", &p.region_names, |_| true)?.clone();
    let site_count = 4 + usize::from(r.roll("region.sites", 4));
    let mut sites = Vec::new();
    let mut used = BTreeSet::new();
    let roles = [
        "home-base",
        "first-clue",
        "lieutenant-lair",
        "stronghold",
        "rumor-source",
        "ally-haven",
        "old-secret",
        "neutral-ground",
    ];
    for role in roles.into_iter().take(site_count) {
        let preferences = &p
            .site_roles
            .entries
            .iter()
            .find(|e| e.id == role)
            .ok_or_else(|| CampaignError::Missing(role.into()))?
            .prefer;
        let has_preferred = tables.chapter.locations.entries.iter().any(|l| {
            !used.contains(&l.id) && (preferences.is_empty() || intersects(preferences, &l.tags))
        });
        let loc = r.select(&format!("site.{role}"), &tables.chapter.locations, |l| {
            !used.contains(&l.id)
                && (!has_preferred || preferences.is_empty() || intersects(preferences, &l.tags))
        })?;
        used.insert(loc.id.clone());
        sites.push(CampaignSite {
            role: role.into(),
            location: loc.id.clone(),
            name: loc.name.clone(),
        });
    }
    let faction_count = 2 + usize::from((r.roll("factions.count", 6) - 1) / 2);
    let mut factions = Vec::new();
    let mut used = BTreeSet::new();
    for n in 0..faction_count {
        let faction = r
            .select(
                &format!("faction.{n}"),
                &tables.factions.tables.factions,
                |f| !used.contains(&f.id) && frame_region_fit(f, &sites, tables),
            )?
            .clone();
        used.insert(faction.id.clone());
        let party = r
            .select(
                &format!("faction.{n}.party"),
                &tables.factions.tables.attitude_toward_party,
                |_| true,
            )?
            .clone();
        let villain = r
            .select(
                &format!("faction.{n}.villain"),
                &tables.factions.tables.attitude_toward_villain,
                |_| true,
            )?
            .clone();
        factions.push(CampaignFaction {
            faction,
            party,
            villain,
        });
    }
    let home = sites
        .first()
        .ok_or_else(|| CampaignError::Invalid("no home".into()))?;
    let home_location = tables
        .chapter
        .locations
        .entries
        .iter()
        .find(|l| l.id == home.location)
        .ok_or_else(|| CampaignError::Missing(home.location.clone()))?;
    let patron = ally(
        &mut r,
        tables,
        "patron",
        &tables.chapter.patrons,
        home_location,
        None,
    )?;
    let contact = ally(
        &mut r,
        tables,
        "contact",
        &tables.chapter.contacts,
        home_location,
        Some(&patron),
    )?;
    let count = r
        .select("chapters", &tables.acts.tables.chapter_count, |_| true)?
        .chapters;
    let mut lieutenants = Vec::new();
    for n in 0..if count >= 9 { 2 } else { 1 } {
        let l = r
            .select(&format!("lieutenant.{n}"), &v.lieutenants, |l| {
                !lieutenants.iter().any(|old: &Lieutenant| old.id == l.id)
                    && tables.chapter.antagonists.entries.iter().any(|a| {
                        a.id == l.antagonist
                            && (options.rules_mode != RulesMode::Standard || a.fidelity == "exact")
                    })
            })?
            .clone();
        lieutenants.push(l);
    }
    let theme_a = r.text("theme.a", &p.themes)?;
    let theme_b = r
        .select("theme.b", &p.themes, |e| e.text != theme_a)?
        .text
        .clone();
    let tone = r.text("tone", &p.tones)?;
    let templates = named(&tables.acts.slots, &count.to_string())?;
    let mut outline = Vec::new();
    for (index, slot) in templates.iter().enumerate() {
        let t = r.select(
            &format!("slot.{}", index + 1),
            &tables.types.tables.chapter_types,
            |t| slot.allowed.contains(&t.id),
        )?;
        outline.push(ChapterSlot {
            number: index as u8 + 1,
            act: match slot.act.as_str() {
                "act-1" => 1,
                "act-2" => 2,
                "act-3" => 3,
                _ => return Err(CampaignError::Invalid("act".into())),
            },
            role: slot.role.clone(),
            chapter_type: t.id.clone(),
            pillars: t.intended,
            allowed: slot.allowed.clone(),
            skipped: t.v1 == "no",
            needs: t.needs.clone(),
            nodes: tables
                .acts
                .tables
                .nodes
                .entries
                .iter()
                .filter(|n| n.needed_by == slot.role)
                .map(|n| n.id.clone())
                .collect(),
        });
    }
    // Repair bounded mutable slots, recording all rejected rolls; never change act/lieutenant/finale contracts.
    for attempt in 0..4096 {
        if balanced(&outline) {
            break;
        }
        let mutable: Vec<usize> = outline
            .iter()
            .enumerate()
            .filter(|(_, s)| s.allowed.len() > 1)
            .map(|(i, _)| i)
            .collect();
        if mutable.is_empty() {
            return Err(CampaignError::Invalid("no balance repair slot".into()));
        }
        let index = mutable[attempt % mutable.len()];
        let allowed = &outline[index].allowed;
        let t = r.select(
            &format!("repair.{attempt}.slot{}", index + 1),
            &tables.types.tables.chapter_types,
            |t| allowed.contains(&t.id),
        )?;
        outline[index].chapter_type = t.id.clone();
        outline[index].pillars = t.intended;
        outline[index].needs = t.needs.clone();
    }
    if !balanced(&outline) {
        return Err(CampaignError::Invalid("balance repair exhausted".into()));
    }
    let twist_a = r
        .select("twist.act1", &tables.twists.tables.twists, |t| {
            t.families.contains(&premise.family)
        })?
        .clone();
    let twist_b = r
        .select("twist.act2", &tables.twists.tables.twists, |t| {
            t.families.contains(&premise.family) && t.id != twist_a.id
        })?
        .clone();
    let milestone_a = r.text("milestone.act1", &tables.rewards.tables.milestones)?;
    let milestone_b = r.text("milestone.act2", &tables.rewards.tables.milestones)?;
    let milestone_c = r.text("milestone.act3", &tables.rewards.tables.milestones)?;
    let title_table = Table {
        die: 4,
        entries: premise
            .titles
            .iter()
            .enumerate()
            .map(|(n, text)| TextEntry {
                faces: [n as u8 + 1, n as u8 + 1],
                text: text.clone(),
            })
            .collect(),
    };
    let title = r
        .select("campaign.title", &title_table, |e| {
            e.text
                .replace("{region.short}", &region.short)
                .replace("{artifact.short}", &artifact.short)
                .chars()
                .count()
                <= 32
        })?
        .text
        .clone();
    let clock = named(&tables.clocks.tables.clocks, &premise.clock)?
        .steps
        .iter()
        .map(|s| s.text.clone())
        .collect();
    let mut frame = CampaignFrame {
        schema: "df-campaign-frame/1".into(),
        generator_version: "splitmix64-campaign-1".into(),
        seed,
        tables_revision: tables.chapter.revision.clone(),
        rules_mode: options.rules_mode,
        title,
        premise,
        villain: Villain {
            kind,
            name,
            title: villain_title,
            motive,
            scheme,
            method,
            resources,
            weakness,
            signature,
            lieutenants,
        },
        artifact,
        region: region.name,
        region_short: region.short,
        sites,
        factions,
        patron,
        contact,
        themes: [theme_a, theme_b],
        tone,
        clock,
        lieutenant_fates: tables.acts.tables.lieutenant_fate.clone(),
        acts: tables
            .acts
            .tables
            .acts
            .entries
            .iter()
            .map(|a| a.name.clone())
            .collect(),
        outline,
        twists: [twist_a, twist_b],
        milestones: [milestone_a, milestone_b, milestone_c],
        rolls: r.rolls,
    };
    super::campaign_render::resolve_frame(&mut frame)?;
    Ok(frame)
}
pub(super) fn balanced(outline: &[ChapterSlot]) -> bool {
    let playable: Vec<_> = outline.iter().filter(|s| !s.skipped).collect();
    !playable
        .windows(2)
        .any(|w| w[0].chapter_type == w[1].chapter_type)
        && (0..3).all(|i| {
            playable
                .iter()
                .map(|s| u32::from(s.pillars[i]))
                .sum::<u32>()
                >= 25 * playable.len() as u32
        })
}
impl CampaignRecord {
    pub(crate) fn new(frame: CampaignFrame) -> Result<Self, CampaignError> {
        let frame_digest = digest(&canonical_bytes(&frame)?);
        let mut npcs = vec![
            NpcState {
                id: "patron".into(),
                name: format!("{} {}", frame.patron.first, frame.patron.surname),
                attitude: "neutral".into(),
                status: "active".into(),
                last_seen: 0,
            },
            NpcState {
                id: "contact".into(),
                name: format!("{} {}", frame.contact.first, frame.contact.surname),
                attitude: "neutral".into(),
                status: "active".into(),
                last_seen: 0,
            },
        ];
        npcs.extend(frame.factions.iter().map(|f| NpcState {
            id: f.faction.id.clone(),
            name: f.faction.name.clone(),
            attitude: f.party.id.clone(),
            status: "active".into(),
            last_seen: 0,
        }));
        Ok(Self {
            schema: "df-campaign-record/1".into(),
            frame,
            frame_digest,
            clock_position: 1,
            clock_history: Vec::new(),
            chapters: Vec::new(),
            facts: Vec::new(),
            clues: Vec::new(),
            npcs,
            lieutenants: Vec::new(),
            rolls: Vec::new(),
        })
    }
    pub(crate) fn canonical_bytes(&self) -> Result<Vec<u8>, CampaignError> {
        canonical_bytes(self)
    }
    pub(crate) fn digest(&self) -> Result<String, CampaignError> {
        Ok(digest(&self.canonical_bytes()?))
    }
    pub(crate) fn validate(&self) -> Result<(), CampaignError> {
        if self.schema != "df-campaign-record/1"
            || self.frame.schema != "df-campaign-frame/1"
            || self.frame.generator_version != "splitmix64-campaign-1"
            || self.frame_digest != digest(&canonical_bytes(&self.frame)?)
            || !(1..=7).contains(&self.clock_position)
            || !balanced(&self.frame.outline)
        {
            return Err(CampaignError::Invalid(
                "record schema, immutable frame or clock".into(),
            ));
        }
        let known_types = [
            "courier",
            "escort",
            "warning",
            "retrieval",
            "mystery",
            "delve",
            "defence",
            "heist",
            "hunt",
            "parley",
            "race-clock",
            "rescue",
            "escape",
            "lieutenant",
            "finale",
            "rest-interlude",
        ];
        if !(6..=10).contains(&self.frame.outline.len())
            || self.frame.clock.len() != 7
            || self.frame.acts.len() != 3
            || !(5..=8).contains(&self.frame.sites.len())
            || !(2..=4).contains(&self.frame.factions.len())
            || self.frame.title.is_empty()
            || self.frame.title.chars().count() > 32
            || self.frame.villain.lieutenants.is_empty()
            || self.frame.villain.lieutenants.len() > 2
            || self.frame.outline.first().is_none_or(|s| s.act != 1)
            || self
                .frame
                .outline
                .last()
                .is_none_or(|s| s.act != 3 || s.chapter_type != "finale")
            || self.frame.outline.iter().enumerate().any(|(index, slot)| {
                slot.number as usize != index + 1
                    || !(1..=3).contains(&slot.act)
                    || !known_types.contains(&slot.chapter_type.as_str())
                    || slot.skipped != (slot.chapter_type == "rest-interlude")
                    || slot.pillars.iter().map(|p| u16::from(*p)).sum::<u16>() != 100
            })
            || self
                .frame
                .outline
                .windows(2)
                .any(|slots| slots[1].act < slots[0].act || slots[1].act > slots[0].act + 1)
        {
            return Err(CampaignError::Invalid(
                "frame outline, sites, cast, title or clock bounds".into(),
            ));
        }
        for role in ["home-base", "first-clue", "lieutenant-lair", "stronghold"] {
            if self
                .frame
                .sites
                .iter()
                .filter(|site| site.role == role)
                .count()
                != 1
            {
                return Err(CampaignError::Invalid("region core roles".into()));
            }
        }
        for (index, chapter) in self.chapters.iter().enumerate() {
            let slot = self
                .frame
                .outline
                .iter()
                .filter(|slot| !slot.skipped)
                .nth(index)
                .ok_or_else(|| CampaignError::Invalid("too many chapters".into()))?;
            let prior_clock = self
                .clock_history
                .iter()
                .filter(|clock| clock.chapter < chapter.plan.number)
                .cloned()
                .collect::<Vec<_>>();
            let (seed, input) = chapter_binding(
                self.frame.seed,
                chapter.plan.number,
                &self.chapters[..index],
                &prior_clock,
            )?;
            if chapter.plan.number != slot.number
                || chapter.plan.act != slot.act
                || !known_types[..15].contains(&chapter.plan.chapter_type.as_str())
                || chapter.plan.seed != seed
                || chapter.plan.outcome_digest_in != input
                || chapter.plan.desperate != (slot.role == "finale" && prior_clock.len() + 1 >= 6)
                || chapter.plan.leads.len() != 3
                || chapter
                    .plan
                    .leads
                    .iter()
                    .map(|c| c.vehicle.as_str())
                    .collect::<BTreeSet<_>>()
                    != BTreeSet::from(["private", "victory", "contact"])
                || chapter
                    .plan
                    .leads
                    .iter()
                    .any(|lead| lead.chapter != slot.number || lead.delivered)
            {
                return Err(CampaignError::Invalid(
                    "chapter plan identity, semantic history, seed or leads".into(),
                ));
            }
            for (key, value) in [
                ("patron", &chapter.plan.patron.archetype),
                ("contact", &chapter.plan.contact.archetype),
                ("name.patron.given", &chapter.plan.patron.first),
                ("name.patron.surname", &chapter.plan.patron.surname),
                ("name.contact.given", &chapter.plan.contact.first),
                ("name.contact.surname", &chapter.plan.contact.surname),
            ] {
                if chapter.plan.forced.get(key) != Some(value) {
                    return Err(CampaignError::Invalid("forced chapter cast".into()));
                }
            }
            for key in ["location", "hook", "stake", "antagonist"] {
                if chapter.plan.forced.get(key).is_none_or(String::is_empty) {
                    return Err(CampaignError::Invalid("missing forced chapter fact".into()));
                }
            }
            if !self.frame.sites.iter().any(|site| {
                site.name == chapter.plan.site
                    && chapter.plan.forced.get("location") == Some(&site.location)
            }) {
                return Err(CampaignError::Invalid("chapter site outside region".into()));
            }
            if chapter.outcome.as_ref().is_some_and(|outcome| {
                outcome.result != ChapterResult::Victory
                    && (outcome.destination_reached || outcome.antagonist_spared)
            }) {
                return Err(CampaignError::Invalid(
                    "defeat cannot reach destination or spare antagonist".into(),
                ));
            }
            if chapter.lieutenant_fate.as_ref().is_some_and(|fate| {
                !self
                    .frame
                    .lieutenant_fates
                    .entries
                    .iter()
                    .any(|entry| entry.id == *fate)
            }) {
                return Err(CampaignError::Invalid("unknown lieutenant fate".into()));
            }
        }
        let mut previous = 0;
        for c in &self.chapters {
            if c.plan.number <= previous
                || c.plan.number as usize > self.frame.outline.len()
                || !self
                    .frame
                    .outline
                    .iter()
                    .any(|s| s.number == c.plan.number && !s.skipped)
            {
                return Err(CampaignError::Invalid("chapter sequence".into()));
            }
            previous = c.plan.number;
        }
        if self
            .chapters
            .iter()
            .rev()
            .skip(1)
            .any(|c| c.outcome.is_none())
        {
            return Err(CampaignError::Invalid("unfinished earlier chapter".into()));
        }
        let mut expected_history = Vec::new();
        for chapter in self.chapters.iter().filter(|c| c.outcome.is_some()) {
            let outcome = chapter
                .outcome
                .as_ref()
                .ok_or_else(|| CampaignError::Invalid("outcome".into()))?;
            if outcome.member.is_empty()
                || outcome.member.len() > 64
                || outcome.member.chars().any(char::is_control)
            {
                return Err(CampaignError::Invalid("outcome member".into()));
            }
            if outcome.result == ChapterResult::Defeat {
                expected_history.push((chapter.plan.number, "defeat"));
            }
            if self
                .frame
                .outline
                .iter()
                .find(|s| s.number > chapter.plan.number)
                .is_some_and(|s| s.act > chapter.plan.act)
            {
                expected_history.push((chapter.plan.number, "act break"));
            }
            if chapter
                .lieutenant_fate
                .as_deref()
                .is_some_and(|f| ["escapes", "escapes-wounded", "rescued"].contains(&f))
            {
                expected_history.push((chapter.plan.number, "lieutenant escaped"));
            }
        }
        if self.clock_history.len() != expected_history.len()
            || self.clock_history.iter().zip(&expected_history).any(
                |(advance, (chapter, reason))| {
                    advance.chapter != *chapter || advance.delta != 1 || advance.reason != *reason
                },
            )
            || self.clock_position != (1 + expected_history.len()).min(7) as u8
        {
            return Err(CampaignError::Invalid(
                "clock history does not match outcomes".into(),
            ));
        }
        Ok(())
    }
    pub(crate) fn begin(
        &mut self,
        plan: ChapterPlan,
        run_database: String,
        adventure_digest: String,
    ) -> Result<(), CampaignError> {
        if self.chapters.last().is_some_and(|c| c.outcome.is_none()) {
            return Err(CampaignError::Invalid("current chapter unfinished".into()));
        }
        if self.chapters.iter().any(|c| c.plan.number == plan.number) {
            return Err(CampaignError::Invalid("chapter already exists".into()));
        }
        self.chapters.push(ChapterRecord {
            plan,
            run_database,
            adventure_digest,
            outcome: None,
            lieutenant_fate: None,
        });
        Ok(())
    }
    fn advance(&mut self, chapter: u8, reason: &str) {
        self.clock_position = self.clock_position.saturating_add(1).min(7);
        self.clock_history.push(ClockAdvance {
            chapter,
            delta: 1,
            reason: reason.into(),
        });
    }
    /// Identical completion replay is a no-op; conflicting outcomes refuse without mutation.
    pub(crate) fn complete(
        &mut self,
        number: u8,
        outcome: ChapterOutcome,
    ) -> Result<bool, CampaignError> {
        let current = self
            .chapters
            .last()
            .ok_or_else(|| CampaignError::Invalid("missing current chapter".into()))?;
        if current.plan.number != number {
            return Err(CampaignError::Invalid("completion chapter differs".into()));
        }
        if let Some(existing) = &current.outcome {
            return if *existing == outcome {
                Ok(false)
            } else {
                Err(CampaignError::Invalid(
                    "conflicting completion replay".into(),
                ))
            };
        }
        let plan = current.plan.clone();
        let mut staged = self.clone();
        for clue in &plan.rescued {
            let mut clue = clue.clone();
            clue.delivered = true;
            staged.establish_clue(clue);
        }
        for clue in &plan.leads {
            let mut clue = clue.clone();
            clue.audience = if clue.vehicle == "private" {
                Audience::Private(outcome.member.clone())
            } else {
                Audience::Public
            };
            clue.delivered = match clue.vehicle.as_str() {
                "private" => outcome.approach == Approach::Asked,
                "victory" => outcome.result == ChapterResult::Victory,
                "contact" => outcome.destination_reached,
                _ => false,
            };
            staged.establish_clue(clue);
        }
        for (n, hint) in plan.hints.iter().enumerate() {
            staged.facts.push(EstablishedFact {
                id: format!("hint.{number}.{n}"),
                text: hint.clone(),
                audience: Audience::Public,
                established_in: number,
                revealed: true,
            });
        }
        if let Some(twist) = &plan.twist {
            staged.facts.push(EstablishedFact {
                id: format!("twist.{number}"),
                text: twist.clone(),
                audience: Audience::Public,
                established_in: number,
                revealed: true,
            });
        }
        if let Some(milestone) = &plan.milestone {
            staged.facts.push(EstablishedFact {
                id: format!("milestone.{number}"),
                text: milestone.clone(),
                audience: Audience::Public,
                established_in: number,
                revealed: true,
            });
        }
        if plan.chapter_type == "finale" {
            staged.facts.push(EstablishedFact {
                id: "campaign.epilogue".into(),
                text: super::campaign_render::epilogue(
                    &self.frame,
                    &plan,
                    outcome.result == ChapterResult::Victory,
                    outcome.approach,
                ),
                audience: Audience::Public,
                established_in: number,
                revealed: true,
            });
            staged.facts.push(EstablishedFact {
                id: "milestone.final".into(),
                text: self.frame.milestones[2].clone(),
                audience: Audience::Public,
                established_in: number,
                revealed: true,
            });
        }
        staged.facts.push(EstablishedFact {
            id: format!("chapter.{number}.outcome"),
            text: format!(
                "{}: the party {} at {}.",
                plan.title,
                if outcome.result == ChapterResult::Victory {
                    "won"
                } else {
                    "lost"
                },
                plan.site
            ),
            audience: Audience::Public,
            established_in: number,
            revealed: true,
        });
        if number == 1
            && let Some(stake) = plan.forced.get("stake")
        {
            staged.facts.push(EstablishedFact {
                id: "keepsake".into(),
                text: format!("The party carried the {}.", stake.replace('-', " ")),
                audience: Audience::Public,
                established_in: 1,
                revealed: true,
            });
        }
        if outcome.result == ChapterResult::Defeat {
            staged.advance(number, "defeat");
        }
        if self
            .frame
            .outline
            .iter()
            .find(|s| s.number > number)
            .is_some_and(|s| s.act > plan.act)
        {
            staged.advance(number, "act break");
        }
        let mut fate = None;
        if plan.chapter_type == "lieutenant" && outcome.result == ChapterResult::Victory {
            let mut r = Roller::new(plan.seed ^ 0x1e17_ea17);
            let rolled = r.select("lieutenant.fate", &self.frame.lieutenant_fates, |_| true)?;
            fate = Some(rolled.id.clone());
            let lieutenant = plan
                .returning_lieutenant
                .clone()
                .or_else(|| {
                    self.frame
                        .villain
                        .lieutenants
                        .get(
                            if self
                                .chapters
                                .iter()
                                .filter(|c| c.plan.chapter_type == "lieutenant")
                                .count()
                                > 1
                            {
                                1
                            } else {
                                0
                            },
                        )
                        .map(|l| l.id.clone())
                })
                .ok_or_else(|| CampaignError::Invalid("lieutenant identity".into()))?;
            staged.lieutenants.push(LieutenantState {
                id: lieutenant,
                fate: rolled.id.clone(),
                returned: false,
            });
            if ["escapes", "escapes-wounded", "rescued"].contains(&rolled.id.as_str()) {
                staged.advance(number, "lieutenant escaped");
            }
            if rolled.id == "informant" {
                staged.facts.push(EstablishedFact {
                    id: format!("obligation.{number}"),
                    text: "The party owes its informant a favor.".into(),
                    audience: Audience::Public,
                    established_in: number,
                    revealed: true,
                });
            }
            for mut roll in r.rolls {
                roll.n = staged.rolls.len() + 1;
                staged.rolls.push(roll);
            }
        }
        if let Some(id) = &plan.returning_lieutenant {
            for state in &mut staged.lieutenants {
                if state.id == *id {
                    state.returned = true;
                }
            }
        }
        for (role, ally) in [("patron", &plan.patron), ("contact", &plan.contact)] {
            let name = format!("{} {}", ally.first, ally.surname);
            let id = if staged
                .npcs
                .iter()
                .any(|npc| npc.id == role && npc.name == name)
            {
                role.to_owned()
            } else {
                format!("agent.{}.{}", ally.archetype, name)
            };
            if !staged.npcs.iter().any(|npc| npc.id == id) {
                staged.npcs.push(NpcState {
                    id: id.clone(),
                    name,
                    attitude: "neutral".into(),
                    status: "active".into(),
                    last_seen: 0,
                });
            }
            let npc = staged
                .npcs
                .iter_mut()
                .find(|npc| npc.id == id)
                .ok_or_else(|| CampaignError::Invalid("chapter cast".into()))?;
            npc.last_seen = number;
            npc.attitude = if outcome.result == ChapterResult::Victory
                && (outcome.approach == Approach::Escorted || role == "contact")
            {
                "friendly"
            } else {
                "wary"
            }
            .into();
            npc.status = if outcome.result == ChapterResult::Defeat && role == "contact" {
                "captured"
            } else {
                "active"
            }
            .into();
        }
        for mut roll in plan.rolls.clone() {
            roll.n = staged.rolls.len() + 1;
            staged.rolls.push(roll);
        }
        let current = staged
            .chapters
            .last_mut()
            .ok_or_else(|| CampaignError::Invalid("current chapter".into()))?;
        current.outcome = Some(outcome);
        current.lieutenant_fate = fate;
        staged.validate()?;
        *self = staged;
        Ok(true)
    }
    fn establish_clue(&mut self, clue: ClueDelivery) {
        if clue.delivered {
            self.facts.push(EstablishedFact {
                id: clue.clue_id.clone(),
                text: clue.text.clone(),
                audience: clue.audience.clone(),
                established_in: clue.chapter,
                revealed: clue.audience == Audience::Public,
            });
        }
        self.clues.push(clue);
    }
    fn node_resolved(&self, node: &str) -> bool {
        let clues: Vec<_> = self
            .clues
            .iter()
            .filter(|c| c.node == node && c.delivered)
            .collect();
        clues
            .iter()
            .map(|c| &c.clue_id)
            .collect::<BTreeSet<_>>()
            .len()
            >= 3
            && clues
                .iter()
                .map(|c| &c.vehicle)
                .collect::<BTreeSet<_>>()
                .len()
                >= 2
    }
}
fn chapter_type<'a>(
    tables: &'a CampaignTables,
    id: &str,
) -> Result<&'a ChapterType, CampaignError> {
    tables
        .types
        .tables
        .chapter_types
        .entries
        .iter()
        .find(|t| t.id == id)
        .ok_or_else(|| CampaignError::Missing(id.into()))
}
fn clue(
    roller: &mut Roller,
    tables: &CampaignTables,
    frame: &CampaignFrame,
    node: &str,
    vehicle: &str,
    number: u8,
    excluded: &BTreeSet<String>,
) -> Result<ClueDelivery, CampaignError> {
    let choices = named(&tables.clues.tables.clues, node)?;
    let available = choices
        .entries
        .iter()
        .any(|c| !excluded.contains(&format!("{node}.{}", c.faces[0])));
    let entry = roller.select(&format!("clue.{node}.{vehicle}"), choices, |c| {
        (!available || !excluded.contains(&format!("{node}.{}", c.faces[0])))
            && (number != 1 || node != "villain" || vehicle != "victory" || c.faces[0] == 1)
    })?;
    let text = super::campaign_render::render(
        frame,
        match vehicle {
            "private" => &entry.p,
            "victory" => &entry.r,
            _ => &entry.c,
        },
        false,
    )?;
    Ok(ClueDelivery {
        node: node.into(),
        clue_id: format!("{node}.{}", entry.faces[0]),
        text,
        vehicle: vehicle.into(),
        chapter: number,
        audience: Audience::Public,
        delivered: false,
    })
}
fn chapter_binding(
    frame_seed: u64,
    number: u8,
    chapters: &[ChapterRecord],
    clock_history: &[ClockAdvance],
) -> Result<(u64, String), CampaignError> {
    let bytes = canonical_bytes(&(
        chapters
            .iter()
            .map(|chapter| {
                (
                    chapter.plan.number,
                    &chapter.outcome,
                    &chapter.lieutenant_fate,
                    &chapter.plan.returning_lieutenant,
                )
            })
            .collect::<Vec<_>>(),
        clock_history,
    ))?;
    let (input, numeric) = if chapters.is_empty() {
        ("0".repeat(64), 0)
    } else {
        let hash = Sha256::digest(&bytes);
        let mut first = [0; 8];
        first.copy_from_slice(&hash[..8]);
        (digest(&bytes), u64::from_be_bytes(first))
    };
    // The semantic history digest's first eight bytes are interpreted big endian.
    Ok((
        finalize(
            frame_seed
                .wrapping_add(u64::from(number).wrapping_mul(STEP))
                .wrapping_add(finalize(numeric)),
        ),
        input,
    ))
}
/// Derive the next playable run from an immutable frame and committed continuity. No I/O.
pub(crate) fn next_chapter(
    tables: &CampaignTables,
    frame: &CampaignFrame,
    record: &CampaignRecord,
) -> Result<ChapterPlan, CampaignError> {
    record.validate()?;
    if frame.tables_revision != tables.chapter.revision
        || record.frame_digest != digest(&canonical_bytes(frame)?)
    {
        return Err(CampaignError::Invalid(
            "frame/table revision differs".into(),
        ));
    }
    if record.chapters.last().is_some_and(|c| c.outcome.is_none()) {
        return Err(CampaignError::Invalid("current chapter unfinished".into()));
    }
    let last = record.chapters.last();
    let previous_number = last.map_or(0, |c| c.plan.number);
    let slot = frame
        .outline
        .iter()
        .find(|s| s.number > previous_number && !s.skipped)
        .ok_or(CampaignError::Complete)?;
    let (seed, outcome_digest_in) = chapter_binding(
        frame.seed,
        slot.number,
        &record.chapters,
        &record.clock_history,
    )?;
    let mut r = Roller::new(seed ^ FRAME_STREAM);
    let provisional = chapter_type(tables, &slot.chapter_type)?;
    let mut selected = provisional;
    let viable = |t: &ChapterType| {
        let mut outline = frame.outline.clone();
        for chapter in &record.chapters {
            if let Some(slot) = outline.iter_mut().find(|s| s.number == chapter.plan.number)
                && let Ok(kind) = chapter_type(tables, &chapter.plan.chapter_type)
            {
                slot.chapter_type = kind.id.clone();
                slot.pillars = kind.intended;
            }
        }
        if let Some(slot) = outline.iter_mut().find(|s| s.number == slot.number) {
            slot.chapter_type = t.id.clone();
            slot.pillars = t.intended;
        }
        balanced(&outline)
    };
    if let Some(previous) = last {
        let previous_type = chapter_type(tables, &previous.plan.chapter_type)?;
        let outcome = previous
            .outcome
            .as_ref()
            .ok_or_else(|| CampaignError::Invalid("unfinished chapter".into()))?;
        let bias = if outcome.result == ChapterResult::Victory {
            &previous_type.on_victory
        } else {
            &previous_type.on_defeat
        };
        if slot.allowed.iter().any(|id| {
            bias.contains(id)
                && *id != previous.plan.chapter_type
                && chapter_type(tables, id).is_ok_and(&viable)
        }) {
            selected = r.select(
                "chapter.type.bias",
                &tables.types.tables.chapter_types,
                |t| {
                    slot.allowed.contains(&t.id)
                        && bias.contains(&t.id)
                        && t.id != previous.plan.chapter_type
                        && t.v1 != "no"
                        && viable(t)
                },
            )?;
        }
    }
    if last.is_some_and(|c| c.plan.chapter_type == selected.id) {
        selected = r.select(
            "chapter.type.adjacency",
            &tables.types.tables.chapter_types,
            |t| slot.allowed.contains(&t.id) && t.id != selected.id && t.v1 != "no",
        )?;
    }
    let beat = selected
        .beat
        .as_ref()
        .ok_or_else(|| CampaignError::Invalid("nonplayable chapter".into()))?;
    // faction-seat is an authored abstract role, realized by neutral-ground or ally-haven.
    let role = if slot.number == 1 {
        "home-base"
    } else if beat.location_role == "faction-seat" {
        "neutral-ground"
    } else {
        &beat.location_role
    };
    let preferred = frame
        .sites
        .iter()
        .find(|s| s.role == role)
        .or_else(|| frame.sites.iter().find(|s| s.role == "ally-haven"))
        .or_else(|| frame.sites.first())
        .ok_or_else(|| CampaignError::Invalid("site".into()))?;
    let lieutenant_index = usize::from(slot.role == "lieutenant-2");
    let lieutenant = frame.villain.lieutenants.get(lieutenant_index);
    let returning = if selected.id != "lieutenant" && selected.id != "finale" {
        record
            .lieutenants
            .iter()
            .find(|l| !l.returned && ["escapes", "escapes-wounded"].contains(&l.fate.as_str()))
            .map(|l| l.id.clone())
    } else {
        None
    };
    let returning_creature = returning
        .as_ref()
        .and_then(|id| frame.villain.lieutenants.iter().find(|l| l.id == *id))
        .map(|l| l.antagonist.as_str());
    let forced_creature = if selected.id == "lieutenant" {
        lieutenant.map(|l| l.antagonist.as_str())
    } else {
        returning_creature
    };
    let mut location = tables
        .chapter
        .locations
        .entries
        .iter()
        .find(|l| l.id == preferred.location)
        .ok_or_else(|| CampaignError::Missing(preferred.location.clone()))?;
    if let Some(creature) = forced_creature
        && !tables
            .chapter
            .antagonists
            .entries
            .iter()
            .any(|a| a.id == creature && intersects(&a.fits, &location.tags))
    {
        location = r.select("chapter.site.relocate", &tables.chapter.locations, |l| {
            frame.sites.iter().any(|s| s.location == l.id)
                && tables
                    .chapter
                    .antagonists
                    .entries
                    .iter()
                    .any(|a| a.id == creature && intersects(&a.fits, &l.tags))
        })?;
    }
    let hooks = named(&tables.acts.family_hooks, &frame.premise.family)?;
    let family_compatible = tables.chapter.hooks.entries.iter().any(|h| {
        h.kind == beat.hook_kind
            && intersects(&h.fits, &location.tags)
            && intersects(&h.stake_tags, &beat.stake_tags)
            && hooks.contains(&h.id)
    });
    let hook = r.select("chapter.hook", &tables.chapter.hooks, |h| {
        h.kind == beat.hook_kind
            && intersects(&h.fits, &location.tags)
            && intersects(&h.stake_tags, &beat.stake_tags)
            && (!family_compatible || hooks.contains(&h.id))
    })?;
    let stake = r.select("chapter.stake", &tables.chapter.stakes, |s| {
        intersects(&s.tags, &hook.stake_tags) && intersects(&s.tags, &beat.stake_tags)
    })?;
    let exact = |a: &Antagonist| frame.rules_mode != RulesMode::Standard || a.fidelity == "exact";
    let organization_available = tables.chapter.antagonists.entries.iter().any(|a| {
        exact(a)
            && intersects(&a.fits, &location.tags)
            && (frame.villain.kind.lieutenant_pref.contains(&a.id) || a.id == "bandit")
    });
    let strongest = tables
        .chapter
        .antagonists
        .entries
        .iter()
        .filter(|a| exact(a) && intersects(&a.fits, &location.tags))
        .map(|a| a.stats.hit_points)
        .max()
        .ok_or_else(|| CampaignError::Invalid("no exact antagonist".into()))?;
    let ant = r.select("chapter.antagonist", &tables.chapter.antagonists, |a| {
        exact(a)
            && intersects(&a.fits, &location.tags)
            && if let Some(id) = forced_creature {
                a.id == id
            } else if selected.id == "finale" {
                a.stats.hit_points == strongest
            } else {
                !organization_available
                    || frame.villain.kind.lieutenant_pref.contains(&a.id)
                    || a.id == "bandit"
            }
    })?;
    let patron = if tables
        .chapter
        .patrons
        .entries
        .iter()
        .any(|p| p.id == frame.patron.archetype && intersects(&p.fits, &location.tags))
    {
        frame.patron.clone()
    } else {
        ally(
            &mut r,
            tables,
            "faction-patron",
            &tables.chapter.patrons,
            location,
            None,
        )?
    };
    let mut contact = if tables
        .chapter
        .contacts
        .entries
        .iter()
        .any(|p| p.id == frame.contact.archetype && intersects(&p.fits, &location.tags))
    {
        frame.contact.clone()
    } else {
        ally(
            &mut r,
            tables,
            "faction-contact",
            &tables.chapter.contacts,
            location,
            Some(&patron),
        )?
    };
    let mut forced = ForcedEntries::from([
        ("location".into(), location.id.clone()),
        ("hook".into(), hook.id.clone()),
        ("stake".into(), stake.id.clone()),
        ("antagonist".into(), ant.id.clone()),
        ("patron".into(), patron.archetype.clone()),
        ("contact".into(), contact.archetype.clone()),
        ("name.patron.given".into(), patron.first.clone()),
        ("name.patron.surname".into(), patron.surname.clone()),
        ("name.contact.given".into(), contact.first.clone()),
        ("name.contact.surname".into(), contact.surname.clone()),
    ]);
    // If location requires two archetypes of the same gender, preserve chapter generator's distinct-name filter.
    if patron.first == contact.first {
        let archetype = tables
            .chapter
            .contacts
            .entries
            .iter()
            .find(|p| p.id == contact.archetype)
            .ok_or_else(|| CampaignError::Missing("contact".into()))?;
        let name = r.select(
            "chapter.contact.distinct",
            named(&tables.chapter.names, &archetype.gender)?,
            |e| e.text != patron.first,
        )?;
        forced.insert("name.contact.given".into(), name.text.clone());
        contact.first = name.text.clone();
    }
    if patron.surname == contact.surname {
        let name = r.select(
            "chapter.contact.surname",
            named(&tables.chapter.names, "surnames")?,
            |e| e.text != patron.surname,
        )?;
        forced.insert("name.contact.surname".into(), name.text.clone());
        contact.surname = name.text.clone();
    }
    // Deadline rescue is truthful public delivery before the chapter uses its conclusion.
    let mut rescued = Vec::new();
    if let Some(previous) = last
        && previous
            .lieutenant_fate
            .as_deref()
            .is_some_and(|f| ["captured-talks", "informant"].contains(&f))
        && let Some(clue) = previous.plan.leads.iter().find(|clue| {
            !record
                .clues
                .iter()
                .any(|old| old.clue_id == clue.clue_id && old.delivered)
        })
    {
        let mut clue = clue.clone();
        clue.chapter = slot.number;
        clue.audience = Audience::Public;
        clue.vehicle = "rescue-dialogue".into();
        rescued.push(clue);
    }

    for node in &tables.acts.tables.nodes.entries {
        let deadline = frame
            .outline
            .iter()
            .find(|s| s.role == node.needed_by)
            .ok_or_else(|| CampaignError::Missing(node.needed_by.clone()))?
            .number;
        if deadline > slot.number || record.node_resolved(&node.id) {
            continue;
        }
        let mut delivered: BTreeSet<String> = record
            .clues
            .iter()
            .filter(|c| c.node == node.id && c.delivered)
            .map(|c| c.clue_id.clone())
            .collect();
        let one_vehicle = record
            .clues
            .iter()
            .filter(|c| c.node == node.id && c.delivered)
            .map(|c| &c.vehicle)
            .collect::<BTreeSet<_>>()
            .len()
            < 2;
        if delivered.len() >= 3 && one_vehicle {
            let lead = clue(
                &mut r,
                tables,
                frame,
                &node.id,
                "rescue-narration",
                slot.number,
                &BTreeSet::new(),
            )?;
            rescued.push(lead);
        }
        while delivered.len() < 3 {
            let mut lead = clue(
                &mut r,
                tables,
                frame,
                &node.id,
                "rescue-narration",
                slot.number,
                &delivered,
            )?;
            delivered.insert(lead.clue_id.clone());
            lead.vehicle = if rescued.len() % 2 == 0 {
                "rescue-narration"
            } else {
                "rescue-dialogue"
            }
            .into();
            rescued.push(lead);
        }
    }
    let nodes: Vec<_> = tables
        .acts
        .tables
        .nodes
        .entries
        .iter()
        .filter(|n| {
            !record.node_resolved(&n.id)
                && frame
                    .outline
                    .iter()
                    .find(|s| s.role == n.needed_by)
                    .is_some_and(|s| s.number > slot.number)
        })
        .take(2)
        .collect();
    let nodes = if nodes.is_empty() {
        tables
            .acts
            .tables
            .nodes
            .entries
            .iter()
            .rev()
            .take(2)
            .collect()
    } else {
        nodes
    };
    let mut excluded: BTreeSet<String> = record
        .clues
        .iter()
        .filter(|c| c.delivered)
        .map(|c| c.clue_id.clone())
        .collect();
    let mut leads = Vec::new();
    for (n, vehicle) in ["private", "victory", "contact"].into_iter().enumerate() {
        let node = nodes
            .get(n % nodes.len())
            .ok_or_else(|| CampaignError::Invalid("no clue nodes".into()))?;
        let lead = clue(
            &mut r,
            tables,
            frame,
            &node.id,
            vehicle,
            slot.number,
            &excluded,
        )?;
        excluded.insert(lead.clue_id.clone());
        leads.push(lead);
    }
    let twist = if last.is_some_and(|c| c.plan.act < slot.act) {
        frame
            .twists
            .get(usize::from(slot.act - 2))
            .map(|t| super::campaign_render::public_text(frame, &t.reveal))
    } else {
        None
    };
    let milestone = if twist.is_some() {
        frame.milestones.get(usize::from(slot.act - 2)).cloned()
    } else {
        None
    };
    let act_slots: Vec<_> = frame
        .outline
        .iter()
        .filter(|s| s.act == slot.act && !s.skipped)
        .collect();
    let hints = if slot.act <= 2 {
        let twist = &frame.twists[usize::from(slot.act - 1)];
        let ordinal = act_slots
            .iter()
            .position(|s| s.number == slot.number)
            .ok_or_else(|| CampaignError::Invalid("act slot".into()))?;
        if ordinal < 2 {
            vec![super::campaign_render::public_text(
                frame,
                &twist.hints[ordinal],
            )]
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };
    let title = hook.title.replace("{patron.first}", &patron.first).replace(
        "{contact.first}",
        forced
            .get("name.contact.given")
            .ok_or_else(|| CampaignError::Missing("contact name".into()))?,
    );
    let mut recap = super::campaign_render::story_so_far(record);
    recap.open_leads = record
        .clues
        .iter()
        .rev()
        .filter(|c| c.delivered && c.audience == Audience::Public && !record.node_resolved(&c.node))
        .map(|c| c.text.clone())
        .collect();
    let hook_context = match last.and_then(|c| c.outcome.as_ref()) {
        Some(outcome) => format!(
            "After the party {} at {}, {}.",
            if outcome.result == ChapterResult::Victory {
                "won"
            } else {
                "lost"
            },
            last.map(|c| c.plan.site.as_str())
                .ok_or_else(|| CampaignError::Invalid("previous site".into()))?,
            if outcome.approach == Approach::Escorted {
                "an ally asks for their help again"
            } else {
                "the witness seeks them out again"
            }
        ),
        None => frame.premise.pitch.clone(),
    };
    Ok(ChapterPlan {
        number: slot.number,
        act: slot.act,
        chapter_type: selected.id.clone(),
        title,
        site: location.name.clone(),
        seed,
        outcome_digest_in,
        forced,
        leads,
        rescued,
        twist,
        milestone,
        hints,
        recap,
        patron,
        contact,
        hook_context,
        desperate: slot.role == "finale" && record.clock_position >= 6,
        returning_lieutenant: returning,
        rolls: r.rolls,
    })
}
