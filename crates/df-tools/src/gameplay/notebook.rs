//! Durable, audience-scoped notes for one adventure.

use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{self, Read, Write},
    mem::size_of,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
};

use df_types::MemberId;
use serde::{Deserialize, Serialize};

const SCHEMA: &str = "df-party-notebook/1";
const FILE_NAME: &str = "notebook.json";
const MAXIMUM_FILE_BYTES: u64 = 2 * 1024 * 1024;
pub(super) const MAXIMUM_ENTRIES: usize = 256;
pub(super) const MAXIMUM_NOTES_PER_PLAYER: usize = 64;
pub(super) const MAXIMUM_TITLE_BYTES: usize = 96;
pub(super) const MAXIMUM_TEXT_BYTES: usize = 4096;
pub(super) const MAXIMUM_SCENE_BYTES: usize = 64;
pub(super) const MAXIMUM_HERO_BYTES: usize = 96;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Kind {
    Person,
    Place,
    Task,
    Clue,
    Note,
}

impl Kind {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Place => "place",
            Self::Task => "task",
            Self::Clue => "clue",
            Self::Note => "note",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    kind: Kind,
    title: String,
    text: String,
    scene: String,
    order: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    owner_member: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    owner_hero: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    author_member: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    author_hero: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SavedFile {
    schema: String,
    adventure_seed: String,
    next_order: u64,
    entries: Vec<Entry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct EntryView {
    pub(super) kind: Kind,
    pub(super) title: String,
    pub(super) text: String,
    pub(super) scene: String,
    pub(super) order: u64,
    pub(super) owner: Option<String>,
    pub(super) author: Option<String>,
}

pub(super) fn validate_view_entries(entries: &[EntryView]) -> Result<(), NotebookError> {
    if entries.len() > MAXIMUM_ENTRIES {
        return Err(NotebookError::Bound("entry count"));
    }
    let mut orders = BTreeSet::new();
    for entry in entries {
        validate_fields(
            &entry.title,
            &entry.text,
            &entry.scene,
            entry.author.as_deref(),
            entry.owner.as_deref(),
        )?;
        if entry.order == 0 || !orders.insert(entry.order) {
            return Err(NotebookError::Invalid("entry order"));
        }
        if entry.owner.is_some() && !matches!(entry.kind, Kind::Clue | Kind::Note)
            || entry.owner.is_some() && entry.owner != entry.author
            || entry.author.is_some() && !matches!(entry.kind, Kind::Clue | Kind::Note)
            || matches!(entry.kind, Kind::Clue | Kind::Note) && entry.author.is_none()
        {
            return Err(NotebookError::Invalid("private entry kind"));
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Default)]
pub(super) struct NotebookSnapshot {
    entries: Vec<Entry>,
    next_order: u64,
}

impl NotebookSnapshot {
    pub(super) fn retained_heap_bytes(&self) -> Option<usize> {
        self.entries.iter().try_fold(
            self.entries.len().checked_mul(size_of::<Entry>())?,
            |bytes, entry| {
                bytes
                    .checked_add(entry.title.capacity())?
                    .checked_add(entry.text.capacity())?
                    .checked_add(entry.scene.capacity())?
                    .checked_add(entry.owner_member.as_ref().map_or(0, String::capacity))?
                    .checked_add(entry.owner_hero.as_ref().map_or(0, String::capacity))?
                    .checked_add(entry.author_member.as_ref().map_or(0, String::capacity))?
                    .checked_add(entry.author_hero.as_ref().map_or(0, String::capacity))
            },
        )
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum NotebookError {
    Io(io::ErrorKind),
    Invalid(&'static str),
    Bound(&'static str),
}

impl std::fmt::Display for NotebookError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(kind) => write!(formatter, "notebook file: {kind}"),
            Self::Invalid(reason) => write!(formatter, "notebook refused: {reason}"),
            Self::Bound(reason) => write!(formatter, "notebook bound exceeded: {reason}"),
        }
    }
}

impl std::error::Error for NotebookError {}

pub(super) struct NotebookLedger {
    adventure_seed: String,
    dossiers: Vec<super::adventure::dossier::Dossier>,
    fact_sheet: Vec<super::adventure::FactSheet>,
    state: Mutex<LedgerState>,
    file: Option<PathBuf>,
    writing: Mutex<()>,
}

#[derive(Clone, Debug)]
struct LedgerState {
    entries: Vec<Entry>,
    next_order: u64,
}

impl NotebookLedger {
    pub(super) fn none() -> Arc<Self> {
        Arc::new(Self {
            adventure_seed: String::new(),
            dossiers: Vec::new(),
            fact_sheet: Vec::new(),
            state: Mutex::new(LedgerState {
                entries: Vec::new(),
                next_order: 1,
            }),
            file: None,
            writing: Mutex::new(()),
        })
    }

    pub(super) fn open(
        adventure_seed: &str,
        dossiers: Vec<super::adventure::dossier::Dossier>,
        fact_sheet: Vec<super::adventure::FactSheet>,
        directory: Option<&Path>,
    ) -> Result<Arc<Self>, NotebookError> {
        let file = directory.map(|directory| directory.join(FILE_NAME));
        let saved = match &file {
            Some(file) => match read_saved(file) {
                Ok(saved) => Some(saved),
                Err(NotebookError::Io(io::ErrorKind::NotFound)) => None,
                Err(error) => return Err(error),
            },
            None => None,
        };
        let (entries, next_order) = match saved {
            Some(saved) => {
                if saved.schema != SCHEMA || saved.adventure_seed != adventure_seed {
                    return Err(NotebookError::Invalid("a different adventure's state"));
                }
                validate_entries(&saved.entries, saved.next_order)?;
                (saved.entries, saved.next_order)
            }
            None => (Vec::new(), 1),
        };
        Ok(Arc::new(Self {
            adventure_seed: adventure_seed.to_owned(),
            dossiers,
            fact_sheet,
            state: Mutex::new(LedgerState {
                entries,
                next_order,
            }),
            file,
            writing: Mutex::new(()),
        }))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, LedgerState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn add_fact(
        &self,
        kind: Kind,
        title: &str,
        text: &str,
        scene: &str,
    ) -> Result<Option<u64>, NotebookError> {
        if matches!(kind, Kind::Clue | Kind::Note) {
            return Err(NotebookError::Invalid("fact kind"));
        }
        validate_fields(title, text, scene, None, None)?;
        let _writing = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        let mut state = self.lock();
        if let Some(index) = state
            .entries
            .iter_mut()
            .position(|entry| entry.kind == kind && entry.title.eq_ignore_ascii_case(title))
        {
            if state.entries[index].text == text {
                return Ok(None);
            }
            let next_order = state
                .next_order
                .checked_add(1)
                .ok_or(NotebookError::Bound("entry order"))?;
            let mut candidate = state.entries.clone();
            candidate[index].text = text.to_owned();
            candidate[index].order = state.next_order;
            self.write_saved(&candidate, next_order)?;
            state.entries = candidate;
            state.next_order = next_order;
            return Ok(None);
        }
        let order =
            self.insert_locked(&mut state, kind, title, text, scene, None, None, None, None)?;
        Ok(Some(order))
    }

    pub(super) fn observe_scene(
        &self,
        member: MemberId,
        hero: &str,
        scene: &super::pack::Scene,
        private_clue: &str,
        adventure_ended: bool,
    ) -> Result<Vec<EntryView>, NotebookError> {
        let scene_key = scene_name(scene.key)?;
        let scene_title = scene.title.as_str();
        if let Some(setting) = &scene.setting {
            self.merge_place_alias(&setting.short, &setting.name)?;
        }
        let sheets = self
            .fact_sheet
            .iter()
            .filter(|sheet| sheet.scene.as_deref() == Some(scene_key.as_str()))
            .collect::<Vec<_>>();
        let mut added = Vec::new();
        let mut mentioned_people = BTreeSet::new();
        for sheet in &sheets {
            for (prefix, location_key) in [
                ("patron", "setting.meeting.name"),
                ("contact", "setting.destination.name"),
                ("ant", "setting.confrontation.name"),
            ] {
                let Some(name) = sheet.facts.get(&format!("{prefix}.full")) else {
                    continue;
                };
                let first = sheet.facts.get(&format!("{prefix}.first"));
                let short = sheet.facts.get(&format!("{prefix}.short"));
                if !sheet.must_mention.iter().any(|mentioned| {
                    mentioned.eq_ignore_ascii_case(name)
                        || first.is_some_and(|first| mentioned.eq_ignore_ascii_case(first))
                        || short.is_some_and(|short| mentioned.eq_ignore_ascii_case(short))
                }) {
                    continue;
                }
                if prefix == "ant"
                    && sheet
                        .facts
                        .get("ant.group")
                        .is_some_and(|group| matches!(group.as_str(), "beast" | "undead"))
                {
                    for name in [Some(name), first, short].into_iter().flatten() {
                        let name = person_name_key(name);
                        if !name.is_empty() {
                            mentioned_people.insert(name);
                        }
                    }
                    continue;
                }
                let role = sheet
                    .facts
                    .get(&format!("{prefix}.role"))
                    .or_else(|| sheet.facts.get(&format!("{prefix}.the")))
                    .map(String::as_str)
                    .unwrap_or("person");
                let place = sheet
                    .facts
                    .get(location_key)
                    .or_else(|| scene.setting.as_ref().map(|setting| &setting.name));
                let text = place.map_or_else(
                    || format!("{role}."),
                    |place| format!("{role}; found at {place}."),
                );
                if let Some(order) = self.add_fact(Kind::Person, name, &text, scene_title)? {
                    added.push(EntryView {
                        kind: Kind::Person,
                        title: name.clone(),
                        text,
                        scene: scene_title.to_owned(),
                        order,
                        owner: None,
                        author: None,
                    });
                }
                mentioned_people.insert(person_name_key(name));
                mentioned_people.insert(person_name_key(
                    name.split_whitespace().next().unwrap_or_default(),
                ));
                if let Some(first) = first {
                    mentioned_people.insert(person_name_key(first));
                }
                if let Some(short) = short {
                    mentioned_people.insert(person_name_key(short));
                }
            }
        }
        for dossier in &self.dossiers {
            let mentioned = scene.cast.iter().any(|actor| {
                actor.name.eq_ignore_ascii_case(&dossier.name)
                    || dossier.npc_id.ends_with(&format!(":{}", actor.id))
            }) || scene.speaker.eq_ignore_ascii_case(&dossier.name)
                || sheets.iter().any(|sheet| {
                    sheet
                        .must_mention
                        .iter()
                        .any(|name| matches_person_name(name, &dossier.name))
                });
            if !mentioned
                || mentioned_people
                    .iter()
                    .any(|name| matches_person_name(name, &dossier.name))
            {
                continue;
            }
            mentioned_people.insert(dossier.name.to_lowercase());
            let where_found = dossier_location(dossier.role, &sheets)
                .or_else(|| scene.setting.as_ref().map(|setting| setting.name.clone()));
            let text = match where_found {
                Some(place) => format!("{}; found at {}.", dossier.role_label, place),
                None => format!("{}.", dossier.role_label),
            };
            if let Some(order) = self.add_fact(Kind::Person, &dossier.name, &text, scene_title)? {
                added.push(EntryView {
                    kind: Kind::Person,
                    title: dossier.name.clone(),
                    text,
                    scene: scene_title.to_owned(),
                    order,
                    owner: None,
                    author: None,
                });
            }
        }
        for actor in &scene.cast {
            if actor.name.trim().is_empty()
                || mentioned_people.contains(&person_name_key(&actor.name))
                || self
                    .dossiers
                    .iter()
                    .any(|dossier| dossier.npc_id.ends_with(&format!(":{}", actor.id)))
            {
                continue;
            }
            let role = match actor.role {
                super::pack::CastRole::Patron => "patron",
                super::pack::CastRole::Contact => "contact",
                super::pack::CastRole::Antagonist => "antagonist",
                super::pack::CastRole::Bystander => "bystander",
            };
            let place = scene
                .setting
                .as_ref()
                .map(|setting| setting.name.as_str())
                .unwrap_or("the current scene");
            let text = format!("{role}; found at {place}.");
            if let Some(order) = self.add_fact(Kind::Person, &actor.name, &text, scene_title)? {
                added.push(EntryView {
                    kind: Kind::Person,
                    title: actor.name.clone(),
                    text,
                    scene: scene_title.to_owned(),
                    order,
                    owner: None,
                    author: None,
                });
            }
        }

        for sheet in &sheets {
            for key in ["place_short", "loc.name", "dest.name"] {
                let Some(name) = sheet.facts.get(key) else {
                    continue;
                };
                if name.trim().is_empty()
                    || !sheet
                        .must_mention
                        .iter()
                        .any(|mentioned| mentioned.eq_ignore_ascii_case(name))
                {
                    continue;
                }
                if key == "place_short"
                    && sheet
                        .facts
                        .get("loc.short")
                        .is_some_and(|short| short.eq_ignore_ascii_case(name))
                {
                    continue;
                }
                let name = if key == "place_short" {
                    scene
                        .setting
                        .as_ref()
                        .filter(|setting| setting.short.eq_ignore_ascii_case(name))
                        .map(|setting| &setting.name)
                        .unwrap_or(name)
                } else {
                    name
                };
                let detail = if key == "dest.name" {
                    self.dossiers
                        .iter()
                        .find(|dossier| dossier.role == super::adventure::dossier::NpcRole::Contact)
                        .map(|dossier| {
                            format!("{} is a {} here.", dossier.name, dossier.role_label)
                        })
                        .unwrap_or_else(|| "Named as a place in the current scene.".to_owned())
                } else if key == "loc.name" {
                    sheet
                        .facts
                        .get("loc.short")
                        .map(|short| format!("Also known as {short}."))
                        .unwrap_or_else(|| "Named in the current scene.".to_owned())
                } else {
                    sheet
                        .facts
                        .get("place_detail")
                        .cloned()
                        .unwrap_or_else(|| "Named in the current scene.".to_owned())
                };
                if let Some(order) = self.add_fact(Kind::Place, name, &detail, scene_title)? {
                    added.push(EntryView {
                        kind: Kind::Place,
                        title: name.clone(),
                        text: detail,
                        scene: scene_title.to_owned(),
                        order,
                        owner: None,
                        author: None,
                    });
                }
            }
        }

        if scene.key == super::pack::SceneKey::Opening
            && super::beat_script::with_script(|script| {
                script.mode == super::beat_script::ScriptMode::Story
            })
            && !scene.speaker.trim().is_empty()
            && !scene.dialogue.trim().is_empty()
            && scene.cast.iter().any(|actor| {
                actor.role == super::pack::CastRole::Patron
                    && actor.name.eq_ignore_ascii_case(&scene.speaker)
            })
        {
            let task = format!(
                "Status: given\n{} asked the party: {}",
                scene.speaker, scene.dialogue
            );
            if let Some(order) = self.add_fact(Kind::Task, "The errand", &task, scene_title)? {
                added.push(EntryView {
                    kind: Kind::Task,
                    title: "The errand".to_owned(),
                    text: task,
                    scene: scene_title.to_owned(),
                    order,
                    owner: None,
                    author: None,
                });
            }
        }
        if scene.key == super::pack::SceneKey::Inn {
            if let Some(reward) = sheets
                .iter()
                .find_map(|sheet| sheet.facts.get("reward.line"))
            {
                self.add_task_hook(reward)?;
            }
        }
        if adventure_ended && scene.key == super::pack::SceneKey::Defeat {
            self.mark_task_failed()?;
        }
        if !private_clue.trim().is_empty()
            && let Some(order) =
                self.add_clue(member, hero, "Private clue", private_clue, scene_title)?
        {
            added.push(EntryView {
                kind: Kind::Clue,
                title: "Private clue".to_owned(),
                text: private_clue.to_owned(),
                scene: scene_title.to_owned(),
                order,
                owner: Some(hero.to_owned()),
                author: Some(hero.to_owned()),
            });
        }
        Ok(added)
    }

    pub(super) fn mark_task_status(&self, status: &str) -> Result<bool, NotebookError> {
        let status = match status {
            "accepted" | "given" => "given",
            "done" | "failed" => status,
            _ => return Err(NotebookError::Invalid("task status")),
        };
        self.update_task(|text| format!("Status: {status}\n{}", task_details(text)))
    }

    fn mark_task_failed(&self) -> Result<bool, NotebookError> {
        self.update_task(|text| format!("Status: failed\n{}", task_details(text)))
    }

    fn merge_place_alias(&self, alias: &str, canonical: &str) -> Result<(), NotebookError> {
        if alias.eq_ignore_ascii_case(canonical) {
            return Ok(());
        }
        let _writing = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        let mut state = self.lock();
        let Some(index) = state
            .entries
            .iter()
            .position(|entry| entry.kind == Kind::Place && entry.title.eq_ignore_ascii_case(alias))
        else {
            return Ok(());
        };
        let mut candidate = state.entries.clone();
        if candidate
            .iter()
            .any(|entry| entry.kind == Kind::Place && entry.title.eq_ignore_ascii_case(canonical))
        {
            candidate.remove(index);
        } else {
            candidate[index].title = canonical.to_owned();
        }
        self.write_saved(&candidate, state.next_order)?;
        state.entries = candidate;
        Ok(())
    }

    pub(super) fn sync_task_reward(
        &self,
        gold_per_hero: u32,
        paid: bool,
    ) -> Result<bool, NotebookError> {
        let status = if paid { "paid" } else { "offered; not paid" };
        let reward = format!("Reward: {gold_per_hero} GP per hero ({status}).");
        self.update_task(|text| {
            let details = text
                .lines()
                .filter(|line| !line.starts_with("Reward: "))
                .collect::<Vec<_>>()
                .join("\n");
            format!("{details}\n{reward}")
        })
    }

    fn add_task_hook(&self, hook: &str) -> Result<(), NotebookError> {
        if hook.trim().is_empty() {
            return Ok(());
        }
        self.update_task(|text| {
            let next = format!("Next hook: {hook}");
            let old = format!("Reward: {hook}");
            let mut lines = text.lines().filter(|line| *line != old).collect::<Vec<_>>();
            if !lines.iter().any(|line| *line == next) {
                lines.push(&next);
            }
            lines.join("\n")
        })?;
        Ok(())
    }

    fn update_task(&self, update: impl FnOnce(&str) -> String) -> Result<bool, NotebookError> {
        let _writing = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        let mut state = self.lock();
        let Some(index) = state
            .entries
            .iter()
            .position(|entry| entry.kind == Kind::Task && entry.title == "The errand")
        else {
            return Ok(false);
        };
        let text = update(&state.entries[index].text);
        if text == state.entries[index].text {
            return Ok(false);
        }
        validate_fields("The errand", &text, &state.entries[index].scene, None, None)?;
        let mut candidate = state.entries.clone();
        candidate[index].text = text;
        candidate[index].order = state.next_order;
        let next_order = state
            .next_order
            .checked_add(1)
            .ok_or(NotebookError::Bound("entry order"))?;
        self.write_saved(&candidate, next_order)?;
        state.entries = candidate;
        state.next_order = next_order;
        Ok(true)
    }

    pub(super) fn add_clue(
        &self,
        member: MemberId,
        hero: &str,
        title: &str,
        text: &str,
        scene: &str,
    ) -> Result<Option<u64>, NotebookError> {
        validate_fields(title, text, scene, Some(hero), None)?;
        let member = hex(member.as_bytes());
        let _writing = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        let mut state = self.lock();
        if state.entries.iter().any(|entry| {
            entry.kind == Kind::Clue
                && entry.author_member.as_deref() == Some(member.as_str())
                && entry.title == title
                && entry.text == text
        }) {
            return Ok(None);
        }
        let order = self.insert_locked(
            &mut state,
            Kind::Clue,
            title,
            text,
            scene,
            Some(member.clone()),
            Some(hero),
            Some(member),
            Some(hero),
        )?;
        Ok(Some(order))
    }

    pub(super) fn add_note(
        &self,
        member: MemberId,
        hero: &str,
        text: &str,
        scene: &str,
    ) -> Result<u64, NotebookError> {
        let text = text.trim();
        validate_fields("Party note", text, scene, Some(hero), None)?;
        let member = hex(member.as_bytes());
        let _writing = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        let mut state = self.lock();
        if state
            .entries
            .iter()
            .filter(|entry| {
                entry.kind == Kind::Note && entry.author_member.as_deref() == Some(member.as_str())
            })
            .count()
            >= MAXIMUM_NOTES_PER_PLAYER
        {
            return Err(NotebookError::Bound("notes per player"));
        }
        self.insert_locked(
            &mut state,
            Kind::Note,
            "Party note",
            text,
            scene,
            None,
            None,
            Some(member),
            Some(hero),
        )
    }

    pub(super) fn record_never_forgives(
        &self,
        member: MemberId,
        hero: &str,
        npc: &str,
        reason: &str,
        scene: &str,
    ) -> Result<Option<u64>, NotebookError> {
        let title = format!("{npc}: permanent grievance");
        let text = format!("{npc} will never forgive {hero}: {reason}");
        validate_fields(&title, &text, scene, Some(hero), Some(hero))?;
        let member = hex(member.as_bytes());
        let _writing = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        let mut state = self.lock();
        if state.entries.iter().any(|entry| {
            entry.kind == Kind::Note
                && entry.owner_member.as_deref() == Some(member.as_str())
                && entry.title == title
        }) {
            return Ok(None);
        }
        if state
            .entries
            .iter()
            .filter(|entry| {
                entry.kind == Kind::Note && entry.author_member.as_deref() == Some(member.as_str())
            })
            .count()
            >= MAXIMUM_NOTES_PER_PLAYER
        {
            return Err(NotebookError::Bound("notes per player"));
        }
        let order = self.insert_locked(
            &mut state,
            Kind::Note,
            &title,
            &text,
            scene,
            Some(member.clone()),
            Some(hero),
            Some(member),
            Some(hero),
        )?;
        Ok(Some(order))
    }

    pub(super) fn share(
        &self,
        member: MemberId,
        order: u64,
    ) -> Result<Option<EntryView>, NotebookError> {
        let member = hex(member.as_bytes());
        let _writing = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        let mut state = self.lock();
        let Some(index) = state.entries.iter().position(|entry| entry.order == order) else {
            return Ok(None);
        };
        if state.entries[index].kind != Kind::Clue
            || state.entries[index].owner_member.as_deref() != Some(member.as_str())
        {
            return Ok(None);
        }
        let mut candidate = state.entries.clone();
        candidate[index].owner_member = None;
        candidate[index].owner_hero = None;
        candidate[index].order = state.next_order;
        let next_order = state
            .next_order
            .checked_add(1)
            .ok_or(NotebookError::Bound("entry order"))?;
        let shared = view(&candidate[index]);
        self.write_saved(&candidate, next_order)?;
        state.entries = candidate;
        state.next_order = next_order;
        Ok(Some(shared))
    }

    pub(super) fn visible_to(&self, member: MemberId) -> Vec<EntryView> {
        let member = hex(member.as_bytes());
        let mut entries = self
            .lock()
            .entries
            .iter()
            .filter(|entry| {
                entry
                    .owner_member
                    .as_deref()
                    .is_none_or(|owner| owner == member)
            })
            .map(view)
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| right.order.cmp(&left.order));
        entries
    }

    pub(super) fn public_entries(&self) -> Vec<EntryView> {
        let mut entries = self
            .lock()
            .entries
            .iter()
            .filter(|entry| entry.owner_member.is_none())
            .map(view)
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| right.order.cmp(&left.order));
        entries
    }

    pub(super) fn snapshot(&self) -> NotebookSnapshot {
        let state = self.lock();
        NotebookSnapshot {
            entries: state.entries.clone(),
            next_order: state.next_order,
        }
    }

    pub(super) fn restore_snapshot(
        &self,
        snapshot: &NotebookSnapshot,
    ) -> Result<(), NotebookError> {
        validate_entries(&snapshot.entries, snapshot.next_order)?;
        let _writing = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        self.write_saved(&snapshot.entries, snapshot.next_order)?;
        let mut state = self.lock();
        state.entries = snapshot.entries.clone();
        state.next_order = snapshot.next_order;
        Ok(())
    }

    fn insert_locked(
        &self,
        state: &mut LedgerState,
        kind: Kind,
        title: &str,
        text: &str,
        scene: &str,
        owner_member: Option<String>,
        owner_hero: Option<&str>,
        author_member: Option<String>,
        author_hero: Option<&str>,
    ) -> Result<u64, NotebookError> {
        if state.entries.len() >= MAXIMUM_ENTRIES {
            return Err(NotebookError::Bound("entries"));
        }
        let order = state.next_order;
        let next_order = state
            .next_order
            .checked_add(1)
            .ok_or(NotebookError::Bound("entry order"))?;
        let mut candidate = state.entries.clone();
        candidate.push(Entry {
            kind,
            title: title.to_owned(),
            text: text.to_owned(),
            scene: scene.to_owned(),
            order,
            owner_member,
            owner_hero: owner_hero.map(str::to_owned),
            author_member,
            author_hero: author_hero.map(str::to_owned),
        });
        self.write_saved(&candidate, next_order)?;
        state.entries = candidate;
        state.next_order = next_order;
        Ok(order)
    }

    fn write_saved(&self, entries: &[Entry], next_order: u64) -> Result<(), NotebookError> {
        let Some(file) = &self.file else {
            return Ok(());
        };
        let bytes = serde_json::to_vec_pretty(&SavedFile {
            schema: SCHEMA.to_owned(),
            adventure_seed: self.adventure_seed.clone(),
            next_order,
            entries: entries.to_vec(),
        })
        .map_err(|_| NotebookError::Invalid("serialization"))?;
        if u64::try_from(bytes.len()).map_or(true, |length| length > MAXIMUM_FILE_BYTES) {
            return Err(NotebookError::Bound("file size"));
        }
        let file = file.as_path();
        let directory = file
            .parent()
            .ok_or(NotebookError::Invalid("file directory"))?;
        let staging = directory.join(format!("{FILE_NAME}.tmp"));
        let mut staged = File::create(&staging).map_err(|error| NotebookError::Io(error.kind()))?;
        staged
            .write_all(&bytes)
            .and_then(|()| staged.sync_all())
            .map_err(|error| NotebookError::Io(error.kind()))?;
        fs::rename(&staging, file).map_err(|error| NotebookError::Io(error.kind()))?;
        crate::file_identity::sync_directory(directory)
            .map_err(|error| NotebookError::Io(error.kind()))
    }
}

fn read_saved(file: &Path) -> Result<SavedFile, NotebookError> {
    let mut bytes = Vec::new();
    File::open(file)
        .and_then(|opened| opened.take(MAXIMUM_FILE_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|error| NotebookError::Io(error.kind()))?;
    if u64::try_from(bytes.len()).map_or(true, |length| length > MAXIMUM_FILE_BYTES) {
        return Err(NotebookError::Invalid("file size"));
    }
    serde_json::from_slice(&bytes).map_err(|_| NotebookError::Invalid("file contents"))
}

pub(super) fn scene_name(key: super::pack::SceneKey) -> Result<String, NotebookError> {
    serde_json::to_value(key)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or(NotebookError::Invalid("scene key"))
}

fn matches_person_name(candidate: &str, full_name: &str) -> bool {
    let candidate = person_name_key(candidate);
    let full_name = person_name_key(full_name);
    let first_name = full_name.split_whitespace().next().unwrap_or_default();
    candidate.eq_ignore_ascii_case(&full_name) || candidate.eq_ignore_ascii_case(first_name)
}

fn person_name_key(name: &str) -> String {
    name.trim()
        .strip_prefix("The ")
        .or_else(|| name.trim().strip_prefix("the "))
        .unwrap_or(name.trim())
        .to_lowercase()
}

fn dossier_location(
    role: super::adventure::dossier::NpcRole,
    sheets: &[&super::adventure::FactSheet],
) -> Option<String> {
    let key = match role {
        super::adventure::dossier::NpcRole::Patron
        | super::adventure::dossier::NpcRole::Bystander => "setting.meeting.name",
        super::adventure::dossier::NpcRole::Contact => "setting.destination.name",
        super::adventure::dossier::NpcRole::Antagonist
        | super::adventure::dossier::NpcRole::Villain
        | super::adventure::dossier::NpcRole::Lieutenant => "setting.confrontation.name",
    };
    sheets
        .iter()
        .find_map(|sheet| sheet.facts.get(key).cloned())
}

fn task_details(text: &str) -> &str {
    text.strip_prefix("Status: ")
        .and_then(|status| status.split_once('\n').map(|(_, details)| details))
        .unwrap_or(text)
}

fn validate_fields(
    title: &str,
    text: &str,
    scene: &str,
    hero: Option<&str>,
    owner: Option<&str>,
) -> Result<(), NotebookError> {
    if title.trim().is_empty() || title.len() > MAXIMUM_TITLE_BYTES {
        return Err(NotebookError::Bound("title"));
    }
    if text.trim().is_empty() || text.len() > MAXIMUM_TEXT_BYTES {
        return Err(NotebookError::Bound("text"));
    }
    if scene.trim().is_empty() || scene.len() > MAXIMUM_SCENE_BYTES {
        return Err(NotebookError::Bound("scene"));
    }
    for name in [hero, owner].into_iter().flatten() {
        if name.trim().is_empty() || name.len() > MAXIMUM_HERO_BYTES {
            return Err(NotebookError::Bound("hero"));
        }
    }
    Ok(())
}

fn validate_entries(entries: &[Entry], next_order: u64) -> Result<(), NotebookError> {
    if entries.len() > MAXIMUM_ENTRIES || next_order == 0 {
        return Err(NotebookError::Invalid("entry count or order"));
    }
    let mut orders = BTreeSet::new();
    let mut note_counts = std::collections::BTreeMap::<&str, usize>::new();
    for entry in entries {
        validate_fields(
            &entry.title,
            &entry.text,
            &entry.scene,
            entry.owner_hero.as_deref(),
            entry.author_hero.as_deref(),
        )
        .map_err(|_| NotebookError::Invalid("entry fields"))?;
        if entry.order == 0 || entry.order >= next_order || !orders.insert(entry.order) {
            return Err(NotebookError::Invalid("entry order"));
        }
        if entry.owner_member.is_some() != entry.owner_hero.is_some()
            || entry.author_member.is_some() != entry.author_hero.is_some()
            || matches!(entry.kind, Kind::Clue | Kind::Note) != entry.author_member.is_some()
            || entry.owner_member.is_some() && !matches!(entry.kind, Kind::Clue | Kind::Note)
            || entry.owner_member.is_some()
                && entry.kind == Kind::Note
                && (entry.owner_member != entry.author_member
                    || entry.owner_hero != entry.author_hero)
        {
            return Err(NotebookError::Invalid("entry audience"));
        }
        if let Some(member) = entry.owner_member.as_deref() {
            validate_member_hex(member)?;
        }
        if let Some(member) = entry.author_member.as_deref() {
            validate_member_hex(member)?;
        }
        if entry.kind == Kind::Note {
            let member = entry.author_member.as_deref().unwrap_or_default();
            let count = note_counts.entry(member).or_default();
            *count += 1;
            if *count > MAXIMUM_NOTES_PER_PLAYER {
                return Err(NotebookError::Invalid("notes per player"));
            }
        }
    }
    Ok(())
}

fn validate_member_hex(member: &str) -> Result<(), NotebookError> {
    if MemberId::from_hex(member).is_err() {
        return Err(NotebookError::Invalid("member identity"));
    }
    Ok(())
}

fn view(entry: &Entry) -> EntryView {
    EntryView {
        kind: entry.kind,
        title: entry.title.clone(),
        text: entry.text.clone(),
        scene: entry.scene.clone(),
        order: entry.order,
        owner: entry.owner_hero.clone(),
        author: entry.author_hero.clone(),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
