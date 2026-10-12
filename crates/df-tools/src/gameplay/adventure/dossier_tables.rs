//! Typed loading and validation of the NPC dossier tables; governing design:
//! adventure-generator §19.
//!
//! The `npc-*.json` files are deliberately not listed in `tables.json`: listing them would change
//! the chapter tables revision and so the recorded bytes of every existing adventure. They carry
//! their own revision (SHA-256 over their bytes, in `FILES` order).
#![allow(
    dead_code,
    reason = "consumed by the adapter and the reply task, which land after this crate-internal contract"
)]
use super::{
    TablesError,
    dossier::{Attitude, Binding, Effect, Group, Length, NpcRole, Topic, sentence_count},
    mature_overlay::Overlay,
    schema::Table,
};
use serde::{Deserialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

/// Files in revision order.
pub(crate) const FILES: [&str; 6] = [
    "npc-background.json",
    "npc-character.json",
    "npc-ties.json",
    "npc-knowledge.json",
    "npc-social.json",
    "npc-personal.json",
];
const SCHEMA: &str = "df-npc-tables/1";
const DICE: [u8; 6] = [4, 6, 8, 10, 12, 20];
const MAX_FILE: usize = 1024 * 1024;
/// Characters in one template (a slot counts as written, before filling).
pub(crate) const MAX_TEMPLATE: usize = 240;
const MAX_SHORT: usize = 40;
/// Slots a trait, tie or knowledge template may use.
/// Errand lines a friendly speaker (patron or contact) says it knows.
pub(super) const ERRAND_LINES: usize = 2;
const BASE_SLOTS: [&str; 21] = [
    "name",
    "role",
    "role_a",
    "sub",
    "obj",
    "pos",
    "stake",
    "stake_full",
    "location",
    "destination",
    "antagonist",
    "antagonist_the",
    "villain",
    "faction",
    "other",
    "other_role",
    "partner",
    "child",
    "child2",
    "age",
    "age2",
];
/// Slots a prepared-answer frame may use; each names one line the NPC may say freely.
const FRAME_SLOTS: [&str; 13] = [
    "name",
    "role_a",
    "origin_line",
    "trade_line",
    "event_line",
    "ideal_line",
    "bond_line",
    "want_line",
    "tie_line",
    "motive_line",
    "place_line",
    "place_line2",
    "rumour_line",
];
/// Slots that are always filled for an NPC in a one-chapter adventure.
const ALWAYS_SLOTS: [&str; 12] = [
    "name",
    "role",
    "role_a",
    "sub",
    "obj",
    "pos",
    "stake",
    "stake_full",
    "location",
    "destination",
    "antagonist",
    "antagonist_the",
];
/// Frame lines that exist for every NPC.
const ALWAYS_LINES: [&str; 9] = [
    "name",
    "role_a",
    "origin_line",
    "trade_line",
    "event_line",
    "ideal_line",
    "bond_line",
    "want_line",
    "place_line",
];

pub(crate) trait Faced {
    fn faces(&self) -> [u8; 2];
    fn entry_id(&self) -> &str;
}

/// One rolled entry of almost every table; each table requires the text fields it uses.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Profile {
    pub(crate) faces: [u8; 2],
    pub(crate) id: String,
    /// DM-facing description, or the speakable line for the plain colour tables.
    pub(crate) text: String,
    /// First-person line anyone may hear.
    #[serde(default)]
    pub(crate) speak: Option<String>,
    /// First-person line said only once trusted.
    #[serde(default)]
    pub(crate) confide: Option<String>,
    /// First-person line the NPC never says.
    #[serde(default)]
    pub(crate) shame: Option<String>,
    #[serde(default)]
    pub(crate) shame_terms: Vec<String>,
    #[serde(default)]
    pub(crate) tags: Vec<String>,
    #[serde(default)]
    pub(crate) roles: Vec<NpcRole>,
    /// Creature groups of the speaker this entry suits.
    #[serde(default)]
    pub(crate) speakers: Vec<Group>,
    /// Groups of the chapter antagonist this entry may talk about.
    #[serde(default)]
    pub(crate) foes: Vec<Group>,
    /// Archetype ids this entry must not be given to.
    #[serde(default)]
    pub(crate) except: Vec<String>,
    /// For the temperament tables: how the reaction moves attitude.
    #[serde(default)]
    pub(crate) effect: Option<Effect>,
}
impl Faced for Profile {
    fn faces(&self) -> [u8; 2] {
        self.faces
    }
    fn entry_id(&self) -> &str {
        &self.id
    }
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Tie {
    pub(crate) faces: [u8; 2],
    pub(crate) id: String,
    pub(crate) text: String,
    pub(crate) speak: String,
    pub(crate) public: bool,
    pub(crate) binds: Binding,
    #[serde(default)]
    pub(crate) speakers: Vec<Group>,
    #[serde(default)]
    pub(crate) foes: Vec<Group>,
    /// Roles `{other}` may name; required exactly when the template uses `{other}`.
    #[serde(default)]
    pub(crate) others: Vec<NpcRole>,
}
impl Faced for Tie {
    fn faces(&self) -> [u8; 2] {
        self.faces
    }
    fn entry_id(&self) -> &str {
        &self.id
    }
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StyleEntry {
    pub(crate) faces: [u8; 2],
    pub(crate) id: String,
    pub(crate) length: Length,
    pub(crate) vocabulary: String,
    pub(crate) tic: String,
    pub(crate) opener: String,
    pub(crate) closer: String,
    #[serde(default)]
    pub(crate) tags: Vec<String>,
    #[serde(default)]
    pub(crate) speakers: Vec<Group>,
}
impl Faced for StyleEntry {
    fn faces(&self) -> [u8; 2] {
        self.faces
    }
    fn entry_id(&self) -> &str {
        &self.id
    }
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AttitudeEntry {
    pub(crate) faces: [u8; 2],
    pub(crate) attitude: Attitude,
}
impl Faced for AttitudeEntry {
    fn faces(&self) -> [u8; 2] {
        self.faces
    }
    fn entry_id(&self) -> &str {
        self.attitude.label()
    }
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TraitTag {
    pub(crate) contains: String,
    pub(crate) tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File<T> {
    schema: String,
    file: String,
    description: String,
    tables: T,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Background {
    pub(super) origins: Table<Profile>,
    pub(super) past_trades: Table<Profile>,
    pub(super) formative_events: Table<Profile>,
    pub(super) talents: Table<Profile>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Character {
    pub(super) ideals: Table<Profile>,
    pub(super) bonds: Table<Profile>,
    pub(super) flaws: Table<Profile>,
    pub(super) wants: Table<Profile>,
    pub(super) fears: Table<Profile>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Ties {
    pub(super) patron: Table<Tie>,
    pub(super) contact: Table<Tie>,
    pub(super) antagonist: Table<Tie>,
    pub(super) villain: Table<Tie>,
    pub(super) lieutenant: Table<Tie>,
    pub(super) motives: Table<Profile>,
    pub(super) stand_downs: Table<Profile>,
}
impl Ties {
    pub(super) fn for_role(&self, role: NpcRole) -> &Table<Tie> {
        match role {
            NpcRole::Patron => &self.patron,
            NpcRole::Contact => &self.contact,
            NpcRole::Antagonist => &self.antagonist,
            NpcRole::Villain => &self.villain,
            NpcRole::Lieutenant => &self.lieutenant,
        }
    }
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct KnowledgeTables {
    /// Four lines per location id.
    pub(super) place_specific: BTreeMap<String, Table<Profile>>,
    pub(super) place_generic: Table<Profile>,
    pub(super) rumours: Table<Profile>,
    pub(super) hints: Table<Profile>,
    pub(super) world: Table<Profile>,
    /// What each role says it knows of the errand: the patron's need, the contact's wait, the
    /// enemy's demand. Entries name the roles that may say them.
    pub(super) errand: Table<Profile>,
    /// Key phrases for each secret id in `secrets.json`, on top of the rendered secret text.
    pub(super) secret_keywords: BTreeMap<String, Vec<String>>,
}
/// Who the NPC is off duty: kin, home, service, opinions, courtship, lay lore about
/// the errand's own things, and how each other cast member is known. All plain
/// first-person lines; `given_names` and `child_ages` are word pools for the kin slots.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Personal {
    pub(super) partner: Table<Profile>,
    pub(super) children: Table<Profile>,
    pub(super) given_names: Table<Profile>,
    pub(super) child_ages: Table<Profile>,
    pub(super) homes: Table<Profile>,
    pub(super) service: Table<Profile>,
    pub(super) opinions: Table<Profile>,
    pub(super) courtship: Table<Profile>,
    pub(super) errand_lore: Table<Profile>,
    pub(super) cast_close: Table<Profile>,
    pub(super) cast_passing: Table<Profile>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Attitudes {
    pub(super) patron: Table<AttitudeEntry>,
    pub(super) contact: Table<AttitudeEntry>,
    pub(super) antagonist: Table<AttitudeEntry>,
    pub(super) villain: Table<AttitudeEntry>,
    pub(super) lieutenant: Table<AttitudeEntry>,
}
impl Attitudes {
    pub(super) fn for_role(&self, role: NpcRole) -> &Table<AttitudeEntry> {
        match role {
            NpcRole::Patron => &self.patron,
            NpcRole::Contact => &self.contact,
            NpcRole::Antagonist => &self.antagonist,
            NpcRole::Villain => &self.villain,
            NpcRole::Lieutenant => &self.lieutenant,
        }
    }
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Social {
    pub(super) flattery: Table<Profile>,
    pub(super) flirting: Table<Profile>,
    pub(super) threats: Table<Profile>,
    pub(super) lies: Table<Profile>,
    pub(super) bribes: Table<Profile>,
    pub(super) deflections: Table<Profile>,
    /// Short refusals a Hostile NPC gives to anything; a few are kept per dossier.
    pub(super) curt: Table<Profile>,
    pub(super) attitudes: Attitudes,
    pub(super) styles: Table<StyleEntry>,
    pub(super) frames: BTreeMap<Topic, Table<Profile>>,
    pub(super) tag_vocabulary: Vec<String>,
    /// Pairs of tone tags that cannot sit in one person.
    pub(super) incompatible: Vec<[String; 2]>,
    /// Keyword rules that read tone tags from the chapter generator's own trait text.
    pub(super) trait_tags: Vec<TraitTag>,
}
impl Social {
    /// Tone tags suggested by the given trait texts (mannerism, speech direction, voice tag).
    pub(super) fn tags_of<'a>(&self, texts: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
        let combined = texts
            .into_iter()
            .map(str::to_lowercase)
            .collect::<Vec<_>>()
            .join(" ");
        self.trait_tags
            .iter()
            .filter(|rule| combined.contains(&rule.contains))
            .flat_map(|rule| rule.tags.iter().cloned())
            .collect()
    }
}

/// Validated NPC dossier tables.
#[derive(Debug)]
pub(crate) struct DossierTables {
    pub(crate) revision: String,
    pub(super) background: Background,
    pub(super) character: Character,
    pub(super) ties: Ties,
    pub(super) knowledge: KnowledgeTables,
    pub(super) social: Social,
    pub(super) personal: Personal,
}

fn invalid(file: &str, path: &str, reason: impl Into<String>) -> TablesError {
    TablesError::Invalid {
        file: file.into(),
        path: path.into(),
        reason: reason.into(),
    }
}
fn read(directory: &Path, file: &str) -> Result<Vec<u8>, TablesError> {
    use std::io::Read;
    let mut bytes = Vec::new();
    fs::File::open(directory.join(file))
        .and_then(|file| file.take(MAX_FILE as u64 + 1).read_to_end(&mut bytes))
        .map_err(|e| TablesError::Io {
            file: file.into(),
            kind: e.kind(),
        })?;
    if bytes.len() > MAX_FILE {
        return Err(invalid(file, ".", "file exceeds one MiB"));
    }
    Ok(bytes)
}
fn parse<T: DeserializeOwned>(file: &str, bytes: &[u8]) -> Result<T, TablesError> {
    let mut parser = serde_json::Deserializer::from_slice(bytes);
    let result = serde_path_to_error::deserialize(&mut parser)
        .map_err(|e| invalid(file, &e.path().to_string(), e.inner().to_string()))?;
    parser
        .end()
        .map_err(|e| invalid(file, ".", e.to_string()))?;
    Ok(result)
}
fn document<T: DeserializeOwned>(
    files: &BTreeMap<String, Vec<u8>>,
    file: &str,
) -> Result<T, TablesError> {
    let bytes = files
        .get(file)
        .ok_or_else(|| invalid(file, ".", "file missing"))?;
    let document: File<T> = parse(file, bytes)?;
    if document.schema != SCHEMA || document.file != file.trim_end_matches(".json") {
        return Err(invalid(file, "schema", "unsupported schema or file marker"));
    }
    if document.description.trim().is_empty() {
        return Err(invalid(file, "description", "empty"));
    }
    Ok(document.tables)
}

/// Placeholder names used by a template, in order. `{name|cap}` yields `name`.
pub(crate) fn slots_of(text: &str) -> Result<Vec<&str>, String> {
    let mut slots = Vec::new();
    let mut tail = text;
    while let Some(open) = tail.find(['{', '}']) {
        if !tail[open..].starts_with('{') {
            return Err("unmatched closing brace".into());
        }
        let rest = &tail[open + 1..];
        let close = rest.find('}').ok_or("unclosed brace")?;
        let inner = &rest[..close];
        let name = inner.strip_suffix("|cap").unwrap_or(inner);
        if name.is_empty()
            || !name
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
        {
            return Err(format!("malformed slot {inner:?}"));
        }
        slots.push(name);
        tail = &rest[close + 1..];
    }
    Ok(slots)
}

fn check_template(file: &str, path: &str, text: &str, allowed: &[&str]) -> Result<(), TablesError> {
    let length = text.chars().count();
    if length == 0 || length > MAX_TEMPLATE {
        return Err(invalid(
            file,
            path,
            format!("{length} characters; 1 to {MAX_TEMPLATE} allowed"),
        ));
    }
    if text.chars().any(char::is_control) || text.trim() != text {
        return Err(invalid(file, path, "control characters or edge whitespace"));
    }
    for slot in slots_of(text).map_err(|reason| invalid(file, path, reason))? {
        if !allowed.contains(&slot) {
            return Err(invalid(file, path, format!("unknown slot {slot}")));
        }
    }
    Ok(())
}
fn check_short(file: &str, path: &str, text: &str) -> Result<(), TablesError> {
    let length = text.chars().count();
    if length == 0 || length > MAX_SHORT || text.chars().any(char::is_control) {
        return Err(invalid(file, path, "1 to 40 plain characters required"));
    }
    Ok(())
}
fn check_id(file: &str, path: &str, id: &str) -> Result<(), TablesError> {
    if id.is_empty()
        || id.len() > 48
        || !id
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
    {
        return Err(invalid(file, path, "id must be lowercase kebab-case"));
    }
    Ok(())
}
/// Dice are standard, entries fit the die, and faces tile `1..=die` in order without gaps.
fn check_table<T: Faced>(file: &str, name: &str, table: &Table<T>) -> Result<(), TablesError> {
    let path = format!("tables.{name}");
    if !DICE.contains(&table.die) {
        return Err(invalid(file, &path, "die must be 4, 6, 8, 10, 12 or 20"));
    }
    if table.entries.len() < 2 || table.entries.len() > usize::from(table.die) {
        return Err(invalid(
            file,
            &path,
            "entries must be 2 and no more than the die",
        ));
    }
    let mut next = 1u8;
    let mut ids = BTreeSet::new();
    for entry in &table.entries {
        let [first, last] = entry.faces();
        if first != next || last < first || last > table.die {
            return Err(invalid(
                file,
                &format!("{path}.{}", entry.entry_id()),
                "faces must continue the previous range without a gap or overlap",
            ));
        }
        next = last.saturating_add(1);
        check_id(file, &path, entry.entry_id())?;
        if !ids.insert(entry.entry_id()) {
            return Err(invalid(file, &path, "duplicate id"));
        }
    }
    if next != table.die + 1 {
        return Err(invalid(file, &path, "faces must cover the whole die"));
    }
    Ok(())
}

#[derive(Clone, Copy, Default)]
struct Needs {
    speak: bool,
    confide: bool,
    shame: bool,
}
fn check_profiles(
    file: &str,
    name: &str,
    table: &Table<Profile>,
    needs: Needs,
    tags: &BTreeSet<&str>,
) -> Result<(), TablesError> {
    check_table(file, name, table)?;
    for entry in &table.entries {
        let path = format!("tables.{name}.{}", entry.id);
        check_template(file, &path, &entry.text, &BASE_SLOTS)?;
        for (label, wanted, value) in [
            ("speak", needs.speak, &entry.speak),
            ("confide", needs.confide, &entry.confide),
            ("shame", needs.shame, &entry.shame),
        ] {
            match (wanted, value) {
                (true, Some(text)) => {
                    check_template(file, &format!("{path}.{label}"), text, &BASE_SLOTS)?;
                }
                (true, None) => {
                    return Err(invalid(file, &path, format!("{label} is required")));
                }
                (false, Some(_)) => {
                    return Err(invalid(file, &path, format!("{label} is not used here")));
                }
                (false, None) => {}
            }
        }
        if needs.shame {
            if entry.shame_terms.is_empty() {
                return Err(invalid(file, &path, "shame_terms is required"));
            }
            for term in &entry.shame_terms {
                check_short(file, &format!("{path}.shame_terms"), term)?;
            }
        } else if !entry.shame_terms.is_empty() {
            return Err(invalid(file, &path, "shame_terms is not used here"));
        }
        for tag in &entry.tags {
            if !tags.contains(tag.as_str()) {
                return Err(invalid(file, &path, format!("unknown tag {tag}")));
            }
        }
    }
    Ok(())
}
fn check_ties(
    file: &str,
    name: &str,
    table: &Table<Tie>,
    own_slots: &[&str],
) -> Result<(), TablesError> {
    check_table(file, name, table)?;
    for tie in &table.entries {
        let path = format!("tables.{name}.{}", tie.id);
        check_template(file, &format!("{path}.text"), &tie.text, &BASE_SLOTS)?;
        check_template(file, &format!("{path}.speak"), &tie.speak, &BASE_SLOTS)?;
        for (label, text) in [("text", &tie.text), ("speak", &tie.speak)] {
            let used = slots_of(text).map_err(|reason| invalid(file, &path, reason))?;
            if !tie.binds.slots().iter().any(|slot| used.contains(slot)) {
                return Err(invalid(
                    file,
                    &format!("{path}.{label}"),
                    "must name the fact it binds to",
                ));
            }
            if used.iter().any(|slot| own_slots.contains(slot)) {
                return Err(invalid(
                    file,
                    &format!("{path}.{label}"),
                    "a tie cannot name the role it belongs to",
                ));
            }
            let needs_other = used
                .iter()
                .any(|slot| matches!(*slot, "other" | "other_role"));
            if needs_other == tie.others.is_empty() {
                return Err(invalid(
                    file,
                    &format!("{path}.{label}"),
                    "others must be listed exactly when {other} is used",
                ));
            }
        }
    }
    Ok(())
}

fn requires_campaign(slots: &[&str]) -> bool {
    slots
        .iter()
        .any(|slot| matches!(*slot, "villain" | "faction" | "other" | "other_role"))
}

impl DossierTables {
    /// Read and validate the six restrained dossier files from a directory.
    pub(crate) fn load(directory: &Path) -> Result<Self, TablesError> {
        Self::load_variant(directory, None)
    }
    /// [`Self::load`], with the mature overlay's replacements applied when one is given. The
    /// mature revision is the restrained one plus the overlay's bytes.
    pub(super) fn load_variant(
        directory: &Path,
        overlay: Option<&Overlay>,
    ) -> Result<Self, TablesError> {
        let mut files = BTreeMap::new();
        for file in FILES {
            files.insert(file.to_owned(), read(directory, file)?);
        }
        Self::build(&files, overlay)
    }
    /// Validate dossier files already in memory, keyed by file name.
    pub(crate) fn from_files(files: &BTreeMap<String, Vec<u8>>) -> Result<Self, TablesError> {
        Self::build(files, None)
    }
    fn build(
        files: &BTreeMap<String, Vec<u8>>,
        overlay: Option<&Overlay>,
    ) -> Result<Self, TablesError> {
        let mut digest = Sha256::new();
        for file in FILES {
            digest.update(
                files
                    .get(file)
                    .ok_or_else(|| invalid(file, ".", "file missing"))?,
            );
        }
        let mut files = files.clone();
        if let Some(overlay) = overlay {
            digest.update(overlay.bytes());
            for file in FILES {
                let merged = overlay.merged(file, &files[file])?;
                files.insert(file.to_owned(), merged);
            }
        }
        let files = &files;
        let tables = Self {
            revision: format!("{:x}", digest.finalize()),
            background: document(files, FILES[0])?,
            character: document(files, FILES[1])?,
            ties: document(files, FILES[2])?,
            knowledge: document(files, FILES[3])?,
            social: document(files, FILES[4])?,
            personal: document(files, FILES[5])?,
        };
        tables.validate()?;
        Ok(tables)
    }

    fn validate(&self) -> Result<(), TablesError> {
        let tags: BTreeSet<&str> = self
            .social
            .tag_vocabulary
            .iter()
            .map(String::as_str)
            .collect();
        self.validate_social_vocabulary(&tags)?;
        let speak = Needs {
            speak: true,
            ..Needs::default()
        };
        let plain = Needs::default();
        let (background, character, ties, knowledge) = (FILES[0], FILES[1], FILES[2], FILES[3]);
        let b = &self.background;
        check_profiles(background, "origins", &b.origins, plain, &tags)?;
        check_profiles(background, "past_trades", &b.past_trades, speak, &tags)?;
        check_profiles(
            background,
            "formative_events",
            &b.formative_events,
            speak,
            &tags,
        )?;
        check_profiles(background, "talents", &b.talents, plain, &tags)?;
        let c = &self.character;
        check_profiles(character, "ideals", &c.ideals, speak, &tags)?;
        check_profiles(character, "bonds", &c.bonds, speak, &tags)?;
        check_profiles(
            character,
            "flaws",
            &c.flaws,
            Needs {
                shame: true,
                ..Needs::default()
            },
            &tags,
        )?;
        check_profiles(character, "wants", &c.wants, speak, &tags)?;
        check_profiles(
            character,
            "fears",
            &c.fears,
            Needs {
                confide: true,
                ..Needs::default()
            },
            &tags,
        )?;
        let t = &self.ties;
        check_ties(ties, "patron", &t.patron, &[])?;
        check_ties(ties, "contact", &t.contact, &[])?;
        check_ties(
            ties,
            "antagonist",
            &t.antagonist,
            &["antagonist", "antagonist_the"],
        )?;
        check_ties(ties, "villain", &t.villain, &["villain"])?;
        check_ties(ties, "lieutenant", &t.lieutenant, &[])?;
        check_profiles(ties, "motives", &t.motives, speak, &tags)?;
        check_profiles(ties, "stand_downs", &t.stand_downs, speak, &tags)?;
        let mut tie_ids = BTreeSet::new();
        for table in [
            &t.patron,
            &t.contact,
            &t.antagonist,
            &t.villain,
            &t.lieutenant,
        ] {
            for tie in &table.entries {
                if !tie_ids.insert(tie.id.as_str()) {
                    return Err(invalid(
                        ties,
                        "tables",
                        format!("tie id repeated: {}", tie.id),
                    ));
                }
            }
        }
        let k = &self.knowledge;
        if k.place_specific.is_empty() {
            return Err(invalid(knowledge, "tables.place_specific", "empty"));
        }
        for (location, table) in &k.place_specific {
            check_id(knowledge, "tables.place_specific", location)?;
            check_profiles(
                knowledge,
                &format!("place_specific.{location}"),
                table,
                plain,
                &tags,
            )?;
        }
        check_profiles(knowledge, "place_generic", &k.place_generic, plain, &tags)?;
        check_profiles(knowledge, "rumours", &k.rumours, plain, &tags)?;
        check_profiles(knowledge, "hints", &k.hints, plain, &tags)?;
        check_profiles(knowledge, "world", &k.world, plain, &tags)?;
        check_profiles(knowledge, "errand", &k.errand, plain, &tags)?;
        for role in [
            NpcRole::Patron,
            NpcRole::Contact,
            NpcRole::Antagonist,
            NpcRole::Villain,
            NpcRole::Lieutenant,
        ] {
            let wanted = if role.opposes_party() {
                1
            } else {
                ERRAND_LINES
            };
            let fitting = k
                .errand
                .entries
                .iter()
                .filter(|entry| entry.roles.contains(&role))
                .count();
            if fitting < wanted {
                return Err(invalid(
                    knowledge,
                    "tables.errand",
                    format!("{} needs {wanted} lines", role.label()),
                ));
            }
            // A campaign can make the destination, the place or the stake a secret term, so each
            // role keeps lines that name none of them.
            let plain_lines = k
                .errand
                .entries
                .iter()
                .filter(|entry| entry.roles.contains(&role))
                .filter(|entry| slots_of(&entry.text).is_ok_and(|slots| slots.is_empty()))
                .count();
            if plain_lines < wanted {
                return Err(invalid(
                    knowledge,
                    "tables.errand",
                    format!(
                        "{} needs {wanted} lines that name nothing of the adventure",
                        role.label()
                    ),
                ));
            }
        }
        if k.secret_keywords.is_empty() {
            return Err(invalid(knowledge, "tables.secret_keywords", "empty"));
        }
        for (secret, keywords) in &k.secret_keywords {
            check_id(knowledge, "tables.secret_keywords", secret)?;
            if keywords.is_empty() {
                return Err(invalid(knowledge, "tables.secret_keywords", "no keywords"));
            }
            for keyword in keywords {
                check_short(knowledge, "tables.secret_keywords", keyword)?;
            }
        }
        let personal = "npc-personal";
        let p = &self.personal;
        check_profiles(personal, "partner", &p.partner, plain, &tags)?;
        check_profiles(personal, "children", &p.children, plain, &tags)?;
        check_profiles(personal, "given_names", &p.given_names, plain, &tags)?;
        check_profiles(personal, "child_ages", &p.child_ages, plain, &tags)?;
        check_profiles(personal, "homes", &p.homes, plain, &tags)?;
        check_profiles(personal, "service", &p.service, plain, &tags)?;
        check_profiles(personal, "opinions", &p.opinions, plain, &tags)?;
        check_profiles(personal, "courtship", &p.courtship, plain, &tags)?;
        check_profiles(personal, "errand_lore", &p.errand_lore, plain, &tags)?;
        check_profiles(personal, "cast_close", &p.cast_close, plain, &tags)?;
        check_profiles(personal, "cast_passing", &p.cast_passing, plain, &tags)?;
        self.validate_social(&tags)?;
        self.validate_coverage()
    }

    fn validate_social_vocabulary(&self, tags: &BTreeSet<&str>) -> Result<(), TablesError> {
        let file = FILES[4];
        if tags.len() != self.social.tag_vocabulary.len() || tags.is_empty() {
            return Err(invalid(
                file,
                "tables.tag_vocabulary",
                "empty or repeated tags",
            ));
        }
        for pair in &self.social.incompatible {
            if pair.iter().any(|tag| !tags.contains(tag.as_str())) || pair[0] == pair[1] {
                return Err(invalid(
                    file,
                    "tables.incompatible",
                    "unknown or equal tags",
                ));
            }
        }
        for rule in &self.social.trait_tags {
            check_short(file, "tables.trait_tags", &rule.contains)?;
            if rule.contains != rule.contains.to_lowercase()
                || rule.tags.iter().any(|tag| !tags.contains(tag.as_str()))
            {
                return Err(invalid(
                    file,
                    "tables.trait_tags",
                    "keyword must be lower case with known tags",
                ));
            }
        }
        Ok(())
    }

    fn validate_social(&self, tags: &BTreeSet<&str>) -> Result<(), TablesError> {
        let file = FILES[4];
        let s = &self.social;
        let reaction = Needs {
            speak: true,
            ..Needs::default()
        };
        for (name, table) in [
            ("flattery", &s.flattery),
            ("flirting", &s.flirting),
            ("threats", &s.threats),
            ("lies", &s.lies),
            ("bribes", &s.bribes),
            ("deflections", &s.deflections),
            ("curt", &s.curt),
        ] {
            check_profiles(file, name, table, reaction, tags)?;
            let moves = matches!(
                name,
                "flattery" | "flirting" | "threats" | "lies" | "bribes"
            );
            if table
                .entries
                .iter()
                .any(|entry| entry.effect.is_some() != moves)
            {
                return Err(invalid(
                    file,
                    &format!("tables.{name}"),
                    "effect is required on every temperament entry and used nowhere else",
                ));
            }
            // Spoken as a prepared answer, so it must stay within three sentences.
            for entry in &table.entries {
                if entry
                    .speak
                    .as_deref()
                    .is_some_and(|text| sentence_count(text) > 3)
                {
                    return Err(invalid(
                        file,
                        &format!("tables.{name}.{}", entry.id),
                        "a spoken reaction is at most three sentences",
                    ));
                }
            }
            if !table.entries.iter().any(|entry| entry.tags.is_empty()) {
                return Err(invalid(
                    file,
                    &format!("tables.{name}"),
                    "an untagged entry is required as an agreement fallback",
                ));
            }
        }
        for (name, table) in [
            ("patron", &s.attitudes.patron),
            ("contact", &s.attitudes.contact),
            ("antagonist", &s.attitudes.antagonist),
            ("villain", &s.attitudes.villain),
            ("lieutenant", &s.attitudes.lieutenant),
        ] {
            check_table(file, &format!("attitudes.{name}"), table)?;
        }
        check_table(file, "styles", &s.styles)?;
        for style in &s.styles.entries {
            let path = format!("tables.styles.{}", style.id);
            for (label, text) in [
                ("vocabulary", &style.vocabulary),
                ("tic", &style.tic),
                ("opener", &style.opener),
                ("closer", &style.closer),
            ] {
                check_template(file, &format!("{path}.{label}"), text, &[])?;
            }
            for (label, text) in [("opener", &style.opener), ("closer", &style.closer)] {
                if !text.ends_with(['.', '!', '?']) || text.chars().count() > MAX_SHORT {
                    return Err(invalid(
                        file,
                        &format!("{path}.{label}"),
                        "one short sentence required",
                    ));
                }
            }
            if style.tags.iter().any(|tag| !tags.contains(tag.as_str())) {
                return Err(invalid(file, &path, "unknown tag"));
            }
        }
        for speaker in [Group::Person, Group::Goblinoid] {
            let covered = s.styles.entries.iter().any(|style| {
                style.tags.is_empty()
                    && (style.speakers.is_empty() || style.speakers.contains(&speaker))
            });
            if !covered {
                return Err(invalid(
                    file,
                    "tables.styles",
                    "an untagged style is required per speaker group",
                ));
            }
        }
        let expected = [
            Topic::WhoAreYou,
            Topic::YourPast,
            Topic::WhyThisMatters,
            Topic::TheAntagonist,
            Topic::ThePlace,
        ];
        if s.frames.len() != expected.len()
            || expected.iter().any(|topic| !s.frames.contains_key(topic))
        {
            return Err(invalid(
                file,
                "tables.frames",
                "exactly the five spoken topics are required",
            ));
        }
        for (topic, table) in &s.frames {
            check_table(file, &format!("frames.{}", topic.label()), table)?;
            for frame in &table.entries {
                let path = format!("tables.frames.{}.{}", topic.label(), frame.id);
                check_template(file, &path, &frame.text, &FRAME_SLOTS)?;
                if frame.speak.is_some() || frame.confide.is_some() || frame.shame.is_some() {
                    return Err(invalid(file, &path, "frames use text only"));
                }
            }
        }
        Ok(())
    }

    /// Prove that, whatever the campaign layer supplies, each role can always be served: for
    /// every role and speaker group there is a fitting entry that needs only always-present
    /// facts. Without this a rare combination could fail an adventure at run time.
    fn validate_coverage(&self) -> Result<(), TablesError> {
        const FOES: [Group; 4] = [Group::Person, Group::Goblinoid, Group::Beast, Group::Undead];
        const ROLES: [(NpcRole, &[Group]); 5] = [
            (NpcRole::Patron, &[Group::Person]),
            (NpcRole::Contact, &[Group::Person]),
            (NpcRole::Antagonist, &[Group::Person, Group::Goblinoid]),
            (NpcRole::Villain, &[Group::Person]),
            (NpcRole::Lieutenant, &[Group::Person, Group::Goblinoid]),
        ];
        let adm = |list: &[Group], group: Group| list.is_empty() || list.contains(&group);
        for (role, speakers) in ROLES {
            let own: &[&str] = match role {
                NpcRole::Antagonist => &["antagonist", "antagonist_the"],
                _ => &[],
            };
            let usable = |slots: &[&str]| {
                !requires_campaign(slots)
                    && slots
                        .iter()
                        .all(|slot| ALWAYS_SLOTS.contains(slot) && !own.contains(slot))
            };
            let profile_ok = |entry: &Profile, speaker: Group, foe: Group| {
                (entry.roles.is_empty() || entry.roles.contains(&role))
                    && adm(&entry.speakers, speaker)
                    && adm(&entry.foes, foe)
                    && [
                        Some(&entry.text),
                        entry.speak.as_ref(),
                        entry.confide.as_ref(),
                        entry.shame.as_ref(),
                    ]
                    .into_iter()
                    .flatten()
                    .all(|text| slots_of(text).is_ok_and(|slots| usable(&slots)))
            };
            let mut profile_tables: Vec<(&str, &Table<Profile>)> = vec![
                ("origins", &self.background.origins),
                ("past_trades", &self.background.past_trades),
                ("formative_events", &self.background.formative_events),
                ("talents", &self.background.talents),
                ("ideals", &self.character.ideals),
                ("bonds", &self.character.bonds),
                ("flaws", &self.character.flaws),
                ("wants", &self.character.wants),
                ("fears", &self.character.fears),
                ("flattery", &self.social.flattery),
                ("flirting", &self.social.flirting),
                ("threats", &self.social.threats),
                ("lies", &self.social.lies),
                ("bribes", &self.social.bribes),
                ("deflections", &self.social.deflections),
                ("place_generic", &self.knowledge.place_generic),
            ];
            if role.opposes_party() {
                profile_tables.push(("motives", &self.ties.motives));
                profile_tables.push(("stand_downs", &self.ties.stand_downs));
            } else {
                profile_tables.push(("rumours", &self.knowledge.rumours));
                profile_tables.push(("hints", &self.knowledge.hints));
            }
            for &speaker in speakers {
                for foe in FOES {
                    for (name, table) in &profile_tables {
                        // Wants and trades also filter by speaker and foe; rumours by foe only.
                        if !table
                            .entries
                            .iter()
                            .any(|entry| profile_ok(entry, speaker, foe))
                        {
                            return Err(invalid(
                                FILES[0],
                                &format!("coverage.{}.{name}", role.label()),
                                format!(
                                    "no always-usable entry for {:?} speaking of {foe:?}",
                                    speaker
                                ),
                            ));
                        }
                        let fitting = table
                            .entries
                            .iter()
                            .filter(|entry| profile_ok(entry, speaker, foe))
                            .count();
                        if *name == "past_trades" && fitting < 2 {
                            return Err(invalid(
                                FILES[0],
                                &format!("coverage.{}.{name}", role.label()),
                                "at least two past trades are needed so one can differ from the job",
                            ));
                        }
                    }
                    let tie_table = self.ties.for_role(role);
                    let tie_ok = tie_table.entries.iter().any(|tie| {
                        adm(&tie.speakers, speaker)
                            && adm(&tie.foes, foe)
                            && [&tie.text, &tie.speak]
                                .into_iter()
                                .all(|text| slots_of(text).is_ok_and(|slots| usable(&slots)))
                    });
                    if !tie_ok {
                        return Err(invalid(
                            FILES[2],
                            &format!("coverage.{}.ties", role.label()),
                            format!("no always-usable tie for {speaker:?} speaking of {foe:?}"),
                        ));
                    }
                }
                self.validate_frames(role, speaker)?;
            }
        }
        let faction_only = self.knowledge.world.entries.iter().any(|entry| {
            slots_of(&entry.text).is_ok_and(|slots| {
                slots.contains(&"faction") && !slots.contains(&"villain") && !requires_other(&slots)
            })
        });
        let villain_only = self.knowledge.world.entries.iter().any(|entry| {
            slots_of(&entry.text).is_ok_and(|slots| {
                slots.contains(&"villain") && !slots.contains(&"faction") && !requires_other(&slots)
            })
        });
        if !faction_only || !villain_only {
            return Err(invalid(
                FILES[3],
                "tables.world",
                "needs an entry for a faction alone and one for a villain alone",
            ));
        }
        Ok(())
    }

    /// Each spoken topic needs a frame for this role that uses only lines every such NPC has.
    fn validate_frames(&self, role: NpcRole, _speaker: Group) -> Result<(), TablesError> {
        let lines: Vec<&str> = ALWAYS_LINES
            .iter()
            .copied()
            .chain(role.opposes_party().then_some("motive_line"))
            .chain((!role.opposes_party()).then_some("rumour_line"))
            .chain(Some("place_line2"))
            .collect();
        for (topic, table) in &self.social.frames {
            let ok = table.entries.iter().any(|frame| {
                (frame.roles.is_empty() || frame.roles.contains(&role))
                    && slots_of(&frame.text)
                        .is_ok_and(|slots| slots.iter().all(|slot| lines.contains(slot)))
            });
            if !ok {
                return Err(invalid(
                    FILES[4],
                    &format!("coverage.{}.frames.{}", role.label(), topic.label()),
                    "no frame the role can always fill",
                ));
            }
        }
        Ok(())
    }
}
fn requires_other(slots: &[&str]) -> bool {
    slots
        .iter()
        .any(|slot| matches!(*slot, "other" | "other_role"))
}
