use super::super::{
    pack::{self, SceneKey},
    startup_configuration::ContentRating,
};
use super::{
    FactSheet, ForcedEntries, GenerateError, GenerateOptions, GeneratedAdventure, PatternProse,
    ProseSource, Roll, RollStatus, RulesMode, Tables, render,
    rng::Dice,
    schema::*,
    tables::{group, intersects},
};
use std::collections::{BTreeMap, BTreeSet};

trait Entry {
    fn faces(&self) -> [u8; 2];
    fn id(&self) -> &str;
}
macro_rules! identified { ($($kind:ty),*)=>{$(impl Entry for $kind {fn faces(&self)->[u8;2]{self.faces} fn id(&self)->&str{&self.id}})*}; }
identified!(
    Location,
    NpcArchetype,
    Hook,
    Stake,
    Antagonist,
    Complication,
    Secret,
    Destination
);
impl Entry for TextEntry {
    fn faces(&self) -> [u8; 2] {
        self.faces
    }
    fn id(&self) -> &str {
        &self.text
    }
}
fn recorded_id<T: Entry>(table: &str, entries: &[T], entry: &T) -> String {
    if table.starts_with("prose.")
        || table.starts_with("trait.")
        || table.starts_with("atmosphere.")
        || ["reward", "antagonist.arrival", "antagonist.warning"].contains(&table)
    {
        let index = entries
            .iter()
            .position(|candidate| std::ptr::eq(candidate, entry))
            .unwrap_or(0);
        format!("variant {}", index + 1)
    } else {
        entry.id().to_owned()
    }
}
pub(super) struct Roller<'a> {
    dice: Dice,
    rolls: Vec<Roll>,
    forced: &'a ForcedEntries,
    used: BTreeSet<String>,
}
impl Roller<'_> {
    fn record(&mut self, table: &str, die: u8, natural: u8, id: &str, status: RollStatus) {
        self.rolls.push(Roll {
            n: self.rolls.len() + 1,
            table: table.into(),
            die,
            natural,
            entry_id: id.into(),
            status,
        });
    }
    fn select<'a, T: Entry>(
        &mut self,
        table: &str,
        die: u8,
        entries: &'a [T],
        compatible: impl Fn(&T) -> bool,
    ) -> Result<&'a T, GenerateError> {
        if let Some(id) = self.forced.get(table) {
            let entry = entries
                .iter()
                .find(|e| (e.id() == id || recorded_id(table, entries, *e) == *id) && compatible(e))
                .ok_or_else(|| GenerateError::UnknownForced(table.into()))?;
            self.used.insert(table.into());
            self.record(
                table,
                die,
                0,
                &recorded_id(table, entries, entry),
                RollStatus::Forced,
            );
            return Ok(entry);
        }
        if entries.len() == 1 && die == 1 {
            return entries
                .first()
                .filter(|e| compatible(e))
                .ok_or_else(|| GenerateError::Incompatible(table.into()));
        }
        for _ in 0..64 {
            let natural = self.dice.roll(die);
            let entry = entries
                .iter()
                .find(|e| {
                    let [first, last] = e.faces();
                    (first..=last).contains(&natural)
                })
                .ok_or_else(|| GenerateError::MissingTable(table.into()))?;
            let accepted = compatible(entry);
            self.record(
                table,
                die,
                natural,
                &recorded_id(table, entries, entry),
                if accepted {
                    RollStatus::Ok
                } else {
                    RollStatus::Rejected
                },
            );
            if accepted {
                return Ok(entry);
            }
        }
        let entry = entries
            .iter()
            .find(|e| compatible(e))
            .ok_or_else(|| GenerateError::Incompatible(table.into()))?;
        self.record(
            table,
            die,
            0,
            &recorded_id(table, entries, entry),
            RollStatus::Fallback,
        );
        Ok(entry)
    }
    pub(super) fn text(
        &mut self,
        table: &str,
        choices: &Table<TextEntry>,
        compatible: impl Fn(&TextEntry) -> bool,
    ) -> Result<String, GenerateError> {
        Ok(self
            .select(table, choices.die, &choices.entries, compatible)?
            .text
            .clone())
    }
    fn pattern(&mut self, table: &str, choices: &PatternTable) -> Result<String, GenerateError> {
        Ok(self
            .select(table, choices.die.unwrap_or(1), &choices.variants, |_| true)?
            .text
            .clone())
    }
}
fn named<'a, T>(tables: &'a BTreeMap<String, T>, name: &str) -> Result<&'a T, GenerateError> {
    tables
        .get(name)
        .ok_or_else(|| GenerateError::MissingTable(name.into()))
}
fn put(context: &mut BTreeMap<String, String>, prefix: &str, fields: &[(&str, &str)]) {
    for (key, value) in fields {
        context.insert(format!("{prefix}.{key}"), (*value).into());
    }
}
fn pronouns(gender: &str) -> Result<[&str; 4], GenerateError> {
    match gender {
        "he" => Ok(["he", "him", "his", "himself"]),
        "she" => Ok(["she", "her", "her", "herself"]),
        "it" => Ok(["it", "it", "its", "itself"]),
        _ => Err(GenerateError::Pattern("gender".into())),
    }
}
fn npc(
    context: &mut BTreeMap<String, String>,
    prefix: &str,
    archetype: &NpcArchetype,
    first: &str,
    surname: &str,
) -> Result<(), GenerateError> {
    put(
        context,
        prefix,
        &[
            ("first", first),
            ("full", &format!("{first} {surname}")),
            ("role", &archetype.role),
            ("desc", &archetype.desc),
            ("voice", &archetype.voice),
            ("gender", &archetype.gender),
            ("age", &archetype.age),
            ("id", &archetype.id),
        ],
    );
    let [sub, obj, pos, reflexive] = pronouns(&archetype.gender)?;
    put(
        context,
        prefix,
        &[
            ("sub", sub),
            ("obj", obj),
            ("pos", pos),
            ("self", reflexive),
        ],
    );
    Ok(())
}
/// The tone sentence of the fact sheets and the model's facts for `rating`.
pub(super) fn tone_of(rating: ContentRating) -> &'static str {
    match rating {
        ContentRating::Restrained => "heroic fantasy for a living-room table",
        ContentRating::Mature => {
            "heroic fantasy for an adult table, dark where the story calls for it"
        }
    }
}
/// Generate a complete adventure using the shipped pattern expression source.
/// Pure function of validated tables, seed, options and forced entries; no I/O or global state.
pub(crate) fn generate(
    tables: &Tables,
    seed: u64,
    options: GenerateOptions,
    forced: &ForcedEntries,
) -> Result<GeneratedAdventure, GenerateError> {
    super::generate_with_prose(tables, seed, options, forced, &PatternProse)
}
/// Expression substitution point: the source sees a bounded sheet for every story slot.
/// Mechanics are appended afterward and never passed to the source.
pub(crate) fn generate_with_prose(
    tables: &Tables,
    seed: u64,
    options: GenerateOptions,
    forced: &ForcedEntries,
    source: &dyn ProseSource,
) -> Result<GeneratedAdventure, GenerateError> {
    let tables = tables.for_rating(options.rating);
    let mut roller = Roller {
        dice: Dice::new(seed),
        rolls: Vec::new(),
        forced,
        used: BTreeSet::new(),
    };
    let chapter_type = forced
        .get(super::beat_tables::CHAPTER_TYPE)
        .map(String::as_str)
        .unwrap_or("courier");
    let beat_template = tables.beat_scripts.chapter(chapter_type)?;
    if forced.contains_key(super::beat_tables::CHAPTER_TYPE) {
        roller.used.insert(super::beat_tables::CHAPTER_TYPE.into());
    }
    let loc = roller.select(
        "location",
        tables.locations.die,
        &tables.locations.entries,
        |_| true,
    )?;
    let hook = roller.select("hook", tables.hooks.die, &tables.hooks.entries, |e| {
        intersects(&e.fits, &loc.tags)
    })?;
    let patron = roller.select("patron", tables.patrons.die, &tables.patrons.entries, |e| {
        intersects(&e.fits, &loc.tags) && tables.places.patron_fits(&e.id, loc)
    })?;
    let stake = roller.select("stake", tables.stakes.die, &tables.stakes.entries, |e| {
        intersects(&e.tags, &hook.stake_tags)
    })?;
    let ant = roller.select(
        "antagonist",
        tables.antagonists.die,
        &tables.antagonists.entries,
        |e| {
            intersects(&e.fits, &loc.tags)
                && (e.fidelity == "exact" || options.rules_mode == RulesMode::CustomAllowed)
        },
    )?;
    let complication = roller.select(
        "complication",
        tables.complications.die,
        &tables.complications.entries,
        |e| e.groups.iter().any(|g| g == group(ant)),
    )?;
    let secret = roller.select("secret", tables.secrets.die, &tables.secrets.entries, |e| {
        e.kinds.contains(&hook.kind)
    })?;
    let destinations = named(&tables.destinations, &loc.id)?;
    let dest = roller.select(
        "destination",
        destinations.die,
        &destinations.entries,
        |e| {
            tables.places.available(&e.id)
                && tables.contacts.entries.iter().any(|c| {
                    intersects(&c.fits, &loc.tags)
                        && tables.places.contact_fits(&c.id, &e.id)
                        && forced.get("contact").is_none_or(|id| id == &c.id)
                })
        },
    )?;
    let contact = roller.select(
        "contact",
        tables.contacts.die,
        &tables.contacts.entries,
        |e| intersects(&e.fits, &loc.tags) && tables.places.contact_fits(&e.id, &dest.id),
    )?;
    let reward = roller.text("reward", &tables.rewards, |_| true)?;
    let patron_first = roller.text(
        "name.patron.given",
        named(&tables.names, &patron.gender)?,
        |_| true,
    )?;
    let patron_surname = roller.text(
        "name.patron.surname",
        named(&tables.names, "surnames")?,
        |_| true,
    )?;
    let contact_first = roller.text(
        "name.contact.given",
        named(&tables.names, &contact.gender)?,
        |e| e.text != patron_first,
    )?;
    let contact_surname = roller.text(
        "name.contact.surname",
        named(&tables.names, "surnames")?,
        |e| e.text != patron_surname,
    )?;
    let ant_name = match ant.name_form.as_str() {
        "person" => {
            let first = roller.text(
                "name.antagonist.given",
                named(&tables.names, &ant.gender)?,
                |e| e.text != patron_first && e.text != contact_first,
            )?;
            let surname = roller.text(
                "name.antagonist.surname",
                named(&tables.names, "surnames")?,
                |_| true,
            )?;
            format!("{first} {surname}")
        }
        "goblinoid" => {
            let pool = if ant.id == "goblin-warrior" {
                "goblin"
            } else {
                "kobold"
            };
            format!(
                "{} the {}",
                roller.text("name.antagonist", named(&tables.names, pool)?, |_| true)?,
                ant.noun
            )
        }
        "creature" | "undead" => {
            let pool = if ant.name_form == "undead" {
                "undead_adj"
            } else {
                "creature_adj"
            };
            format!(
                "the {} {}",
                roller.text(
                    "name.antagonist.adjective",
                    named(&tables.names, pool)?,
                    |_| true
                )?,
                ant.noun
            )
        }
        _ => return Err(GenerateError::Pattern("ant.name_form".into())),
    };
    let mut context = BTreeMap::new();
    context.insert("chapter.type".into(), chapter_type.into());
    npc(
        &mut context,
        "patron",
        patron,
        &patron_first,
        &patron_surname,
    )?;
    npc(
        &mut context,
        "contact",
        contact,
        &contact_first,
        &contact_surname,
    )?;
    put(
        &mut context,
        "loc",
        &[
            ("name", &loc.name),
            ("title", &loc.title),
            ("short", &loc.short),
            ("id", &loc.id),
        ],
    );
    put(
        &mut context,
        "hook",
        &[("id", &hook.id), ("kind", &hook.kind)],
    );
    put(
        &mut context,
        "stake",
        &[
            ("short", &stake.short),
            ("full", &stake.full),
            ("title", &stake.title),
            ("id", &stake.id),
        ],
    );
    put(
        &mut context,
        "dest",
        &[
            ("name", &dest.name),
            ("short", &dest.short),
            ("title", &dest.title),
            ("id", &dest.id),
        ],
    );
    put(
        &mut context,
        "ant",
        &[
            ("short", &ant_name),
            ("full", &render::capitalize(&ant_name)),
            ("the", &ant.the),
            ("id", &ant.id),
            ("group", group(ant)),
            ("voice", &ant.voice),
            ("gender", &ant.gender),
            ("age", &ant.age),
        ],
    );
    let encounter = encounter_details(ant)?;
    let [sub, obj, pos, reflexive] = pronouns(&ant.gender)?;
    put(
        &mut context,
        "ant",
        &[
            ("sub", sub),
            ("obj", obj),
            ("pos", pos),
            ("self", reflexive),
        ],
    );
    for prefix in ["patron", "contact"] {
        let mannerism = roller.text(
            &format!("trait.{prefix}.mannerism"),
            named(&tables.traits, "mannerisms")?,
            |_| true,
        )?;
        let possession = context
            .get(&format!("{prefix}.pos"))
            .ok_or_else(|| GenerateError::Pattern(prefix.into()))?;
        context.insert(
            format!("{prefix}.mannerism"),
            mannerism.replace("{pos}", possession),
        );
        let speech = roller.text(
            &format!("trait.{prefix}.speech"),
            named(&tables.traits, "speech")?,
            |_| true,
        )?;
        context.insert(format!("{prefix}.delivery"), speech);
    }
    if ant.speaks {
        let menace = roller.text(
            "trait.antagonist.menace",
            named(&tables.traits, "menace")?,
            |_| true,
        )?;
        context.insert("ant.delivery".into(), menace);
    }
    let mut atmos = BTreeSet::new();
    for letter in ["a", "b", "c"] {
        let text = roller.text(&format!("atmosphere.{letter}"), &loc.atmosphere, |e| {
            !atmos.contains(&e.text)
        })?;
        atmos.insert(text.clone());
        context.insert(format!("loc.atmos_{letter}"), text);
    }
    let settings = super::setting::roll_settings(
        tables,
        loc,
        &patron.id,
        dest,
        seed,
        &mut roller,
        &mut context,
    )?;
    context.insert("pos".into(), pronouns(&patron.gender)?[2].into());
    let entrance = render::render(&hook.entrance, &context)?;
    context.insert("entrance".into(), entrance);
    let secret_fact = render::render(&secret.fact, &context)?;
    let secret_confirm = render::render(&secret.confirm, &context)?;
    put(
        &mut context,
        "secret",
        &[
            ("fact", &secret_fact),
            ("confirm", &secret_confirm),
            ("id", &secret.id),
        ],
    );
    put(&mut context, "reward", &[("line", &reward)]);
    let complication_text = render::render(&complication.text, &context)?;
    context.insert("complication".into(), complication_text.clone());
    context.insert("complication.id".into(), complication.id.clone());
    // Roll prose in exactly Appendix A order before resolving dependent slots.
    let mut patterns = BTreeMap::new();
    let group_key = group(ant);
    let order = [
        "begin_label".into(),
        "room_title".into(),
        "room_narration".into(),
        "opening_narration".into(),
        format!("foreshadow_{group_key}"),
        "ask_label".into(),
        "help_label".into(),
        "stand_label".into(),
        "continue_label".into(),
        "ask_title".into(),
        "ask_narration".into(),
        "@arrival".into(),
        "help_title".into(),
        "help_narration".into(),
        format!("route_{}", hook.kind),
        "clue".into(),
        "combat_title".into(),
        format!("combat_open_{group_key}"),
        "@warning".into(),
        "victory_title".into(),
        "victory_lead".into(),
        "victory_dialogue".into(),
        "defeat_title".into(),
        format!("defeat_{}", hook.kind),
        "inn_title".into(),
        "inn_intro".into(),
        "inn_dialogue".into(),
    ];
    for key in order {
        let text = match key.as_str() {
            "@arrival" => roller.pattern("antagonist.arrival", &ant.arrivals)?,
            "@warning" => {
                if !ant.speaks {
                    continue;
                }
                roller.pattern(
                    "antagonist.warning",
                    ant.warnings
                        .as_ref()
                        .ok_or_else(|| GenerateError::MissingTable("ant.warnings".into()))?,
                )?
            }
            _ => roller.pattern(&format!("prose.{key}"), named(&tables.prose, &key)?)?,
        };
        patterns.insert(key, text);
    }
    for (slot, key) in [
        ("foreshadow", format!("foreshadow_{group_key}")),
        ("arrival", "@arrival".into()),
        ("route", format!("route_{}", hook.kind)),
    ] {
        let text = render::render(named(&patterns, &key)?, &context)?;
        context.insert(slot.into(), text);
    }
    context.insert("name".into(), "{name}".into());
    let mut sheets = Vec::new();
    let mut expression = |scene_key: Option<SceneKey>,
                          beat: &str,
                          slot: &str,
                          speaker: &str,
                          pattern: &str,
                          bound: usize|
     -> Result<String, GenerateError> {
        let place = scene_key
            .map(|key| settings.scene(key))
            .unwrap_or(&settings.meeting);
        let mut scene_context = context.clone();
        scene_context.insert("place".into(), place.name.clone());
        scene_context.insert("place_short".into(), place.short.clone());
        if let (Some(key), Some(model)) = (scene_key, place.model.as_ref()) {
            scene_context.insert(
                "scene.model".into(),
                super::scene_model::prompt_context(model, key)?,
            );
        }
        if scene_key == Some(SceneKey::Combat) {
            scene_context.insert("encounter.count".into(), encounter.count.to_string());
            scene_context.insert("encounter.creatures".into(), encounter.creatures.clone());
            scene_context.insert("encounter.weapons".into(), encounter.weapons.clone());
            scene_context.insert("encounter.fallback".into(), encounter.fallback.clone());
        }
        let detail = match scene_key {
            Some(SceneKey::Room | SceneKey::Defeat) => context
                .get("loc.atmos_a")
                .ok_or_else(|| GenerateError::Pattern("loc.atmos_a".into()))?
                .clone(),
            Some(SceneKey::Combat) => context
                .get("setting.confrontation.detail")
                .ok_or_else(|| GenerateError::Pattern("setting.confrontation.detail".into()))?
                .clone(),
            Some(SceneKey::Victory | SceneKey::Inn) => dest.arrival.clone(),
            _ => context
                .get("setting.meeting.detail")
                .ok_or_else(|| GenerateError::Pattern("setting.meeting.detail".into()))?
                .clone(),
        };
        scene_context.insert("place_detail".into(), detail);
        let baseline = render::render(pattern, &scene_context)?;
        let mut facts = scene_context.clone();
        let forbidden = if slot == "clue" {
            Vec::new()
        } else {
            facts.remove("secret.fact");
            facts.remove("secret.id");
            if beat == "scene-flow" {
                facts.retain(|key, _| !key.starts_with("secret."));
            } else if beat != "resolution" {
                facts.remove("secret.confirm");
            }
            vec![secret_fact.clone()]
        };
        let mut must_mention = Vec::new();
        for key in [
            "place_short",
            "loc.name",
            "patron.first",
            "patron.full",
            "contact.first",
            "contact.full",
            "stake.short",
            "dest.name",
            "ant.short",
        ] {
            if let Some(value) = scene_context.get(key)
                && baseline.to_lowercase().contains(&value.to_lowercase())
                && !must_mention.contains(value)
            {
                must_mention.push(value.clone());
            }
        }
        let sheet = FactSheet {
            scene: scene_key.and_then(|key| {
                let value = serde_json::to_value(key).ok()?;
                value.as_str().map(str::to_owned)
            }),
            beat: beat.into(),
            slot: slot.into(),
            speaker: context
                .get(&format!("{speaker}.first"))
                .or_else(|| context.get(&format!("{speaker}.full")))
                .cloned()
                .unwrap_or_default(),
            voice: context
                .get(&format!("{speaker}.voice"))
                .cloned()
                .unwrap_or_default(),
            delivery: context
                .get(&format!("{speaker}.delivery"))
                .cloned()
                .unwrap_or_default(),
            facts,
            must_mention,
            forbidden,
            bounds: bound,
            baseline,
            tone: tone_of(options.rating).into(),
        };
        let text = source.express(&sheet)?;
        check(slot, &text, bound)?;
        if !source.validate(&sheet, &text)
            || sheet
                .must_mention
                .iter()
                .any(|term| !text.to_lowercase().contains(&term.to_lowercase()))
            || sheet
                .forbidden
                .iter()
                .any(|term| text.to_lowercase().contains(&term.to_lowercase()))
            || (slot == "line" && speaker.is_empty() && !text.is_empty())
        {
            return Err(GenerateError::Pattern(format!(
                "unvalidated expression for {beat}.{slot}"
            )));
        }
        sheets.push(sheet);
        Ok(text)
    };
    let template =
        |key: &str| -> Result<&str, GenerateError> { Ok(named(&patterns, key)?.as_str()) };
    let mut pack: pack::PackFile = serde_json::from_slice(include_bytes!(
        "../fixtures/harbor-journey-pack-baseline.json"
    ))
    .map_err(|_| GenerateError::Serialization)?;
    let mut tuning: pack::TuningFile = serde_json::from_slice(include_bytes!(
        "../../../../../content/harbor-journey/tuning.json"
    ))
    .map_err(|_| GenerateError::Serialization)?;
    pack.id = format!("adv-{seed:016x}");
    tuning.pack = pack.id.clone();
    tuning.tempo = pack::TempoTuning::default();
    tuning.tactical = ant.tactical.clone();
    pack.bandit.name = render::capitalize(ant_name.strip_prefix("the ").unwrap_or(&ant_name));
    pack.bandit.srd_id = Some(ant.id.clone());
    pack.destination_label = Some(dest.name.clone());
    pack.secret_terms = super::listener::secret_terms(secret, dest, &context);
    pack.speakers = super::listener::speakers(&context, ant, &pack.bandit.name);
    tuning.bandit = pack::BanditTuning {
        hit_points: ant.stats.hit_points,
        armor_class: ant.stats.armor_class,
        abilities: ant.stats.abilities,
        attack_bonus: ant.stats.attack_bonus,
        damage_modifier: ant.stats.damage_modifier,
        weapon: pack::AntagonistWeaponTuning {
            item: ant.stats.weapon.label.clone(),
            damage_dice: ant.stats.weapon.damage_dice,
            damage_sides: ant.stats.weapon.damage_sides,
        },
    };
    pack.offer_labels.begin_story = expression(
        None,
        "gathering",
        "begin_label",
        "",
        template("begin_label")?,
        40,
    )?;
    pack.offer_labels.ask_courier = expression(
        None,
        "roleplay",
        "ask_label",
        "",
        template("ask_label")?,
        40,
    )?;
    pack.offer_labels.escort_courier = expression(
        None,
        "roleplay",
        "help_label",
        "",
        template("help_label")?,
        40,
    )?;
    pack.offer_labels.defend_courier = expression(
        None,
        "confrontation",
        "stand_label",
        "",
        template("stand_label")?,
        40,
    )?;
    pack.offer_labels.choose_harbor_scene = expression(
        None,
        "resolution",
        "continue_label",
        "",
        template("continue_label")?,
        40,
    )?;
    pack.courier_clue = expression(None, "roleplay", "clue", "", template("clue")?, 280)?;
    pack.escort_support_suffix = render::render(
        &named(&tables.prose, "escort_support_suffix")?
            .variants
            .first()
            .ok_or_else(|| GenerateError::MissingTable("escort_support_suffix".into()))?
            .text,
        &context,
    )?;
    use super::super::pack::CastRole;
    let patron_actor = super::listener::cast_member(
        CastRole::Patron,
        &patron.id,
        &patron_first,
        &tables.asset(&patron.art)?,
        (&patron.voice, &patron.gender, &patron.age),
    );
    let contact_actor = super::listener::cast_member(
        CastRole::Contact,
        &contact.id,
        &contact_first,
        &tables.asset(&contact.art)?,
        (&contact.voice, &contact.gender, &contact.age),
    );
    let antagonist_actor = super::listener::cast_member(
        CastRole::Antagonist,
        &ant.id,
        &pack.bandit.name,
        &tables.asset(&ant.art)?,
        (&ant.voice, &ant.gender, &ant.age),
    );
    for scene in &mut pack.scenes {
        let (beat, title, narration, speaker, line, portrait) = match scene.key {
            SceneKey::Room => (
                "gathering",
                template("room_title")?.to_owned(),
                template("room_narration")?.to_owned(),
                "",
                "".into(),
                String::new(),
            ),
            SceneKey::Opening => (
                "hook",
                hook.title.clone(),
                template("opening_narration")?.to_owned(),
                "patron",
                hook.pitch.clone(),
                tables.asset(&patron.art)?,
            ),
            SceneKey::DialogueSeal => (
                "roleplay",
                template("ask_title")?.to_owned(),
                template("ask_narration")?.to_owned(),
                "patron",
                hook.answer.clone(),
                tables.asset(&patron.art)?,
            ),
            SceneKey::DialogueEscort => (
                "roleplay",
                template("help_title")?.to_owned(),
                template("help_narration")?.to_owned(),
                "patron",
                hook.accept.clone(),
                tables.asset(&patron.art)?,
            ),
            SceneKey::Combat => (
                "confrontation",
                template("combat_title")?.to_owned(),
                template(&format!("combat_open_{group_key}"))?.to_owned(),
                if ant.speaks { "ant" } else { "" },
                if ant.speaks {
                    template("@warning")?.to_owned()
                } else {
                    String::new()
                },
                tables.asset(&ant.art)?,
            ),
            SceneKey::Victory => (
                "aftermath",
                template("victory_title")?.to_owned(),
                format!("{} {}", template("victory_lead")?, complication_text),
                "patron",
                template("victory_dialogue")?.to_owned(),
                tables.asset(&patron.art)?,
            ),
            SceneKey::Defeat => (
                "aftermath",
                template("defeat_title")?.to_owned(),
                template(&format!("defeat_{}", hook.kind))?.to_owned(),
                "",
                String::new(),
                String::new(),
            ),
            SceneKey::Inn => (
                "resolution",
                template("inn_title")?.to_owned(),
                format!("{} {}", dest.arrival, template("inn_intro")?),
                "contact",
                template("inn_dialogue")?.to_owned(),
                tables.asset(&contact.art)?,
            ),
        };
        scene.title = expression(Some(scene.key), beat, "title", speaker, &title, 32)?;
        let story = expression(Some(scene.key), beat, "narration", speaker, &narration, 220)?;
        scene.speaker = if speaker == "ant" {
            pack.bandit.name.clone()
        } else {
            context
                .get(&format!("{speaker}.first"))
                .cloned()
                .unwrap_or_default()
        };
        check("speaker", &scene.speaker, 28)?;
        scene.dialogue = expression(Some(scene.key), beat, "line", speaker, &line, 160)?;
        let setting = settings.scene(scene.key);
        scene.asset = tables.asset(&setting.art_key)?;
        let mut pack_setting = setting.clone();
        // Chapter settings persist the shared model once; repeating it on every pack scene exceeds the document bound.
        pack_setting.model = None;
        scene.setting = Some(pack_setting);
        scene.speaker_portrait = portrait;
        // Who is on stage: the patron through the story, the antagonist from the beat that
        // brings it in, the contact at the destination, nobody but the heroes in the lobby.
        scene.cast = match scene.key {
            SceneKey::Opening | SceneKey::Victory => vec![patron_actor.clone()],
            SceneKey::DialogueSeal | SceneKey::DialogueEscort => {
                vec![patron_actor.clone(), antagonist_actor.clone()]
            }
            SceneKey::Combat => vec![antagonist_actor.clone()],
            SceneKey::Inn => vec![contact_actor.clone()],
            SceneKey::Room | SceneKey::Defeat => Vec::new(),
        };
        let mechanic = match scene.key {
            SceneKey::Combat => Some("combat_mech"),
            SceneKey::Victory => Some("victory_mech"),
            SceneKey::Defeat => Some("defeat_mech"),
            _ => None,
        };
        scene.narration = story;
        if let Some(key) = mechanic {
            // The pinned rules sentences stay out of the story text: they are their own field.
            let mut mechanical_context = context.clone();
            mechanical_context.insert("story".into(), String::new());
            let text = &named(&tables.prose, key)?
                .variants
                .first()
                .ok_or_else(|| GenerateError::MissingTable(key.into()))?
                .text;
            scene.rules_note = render::render(text, &mechanical_context)?
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
        }
        if matches!(scene.key, SceneKey::Victory | SceneKey::Defeat) {
            let key = if scene.key == SceneKey::Victory {
                "victory_rested"
            } else {
                "defeat_rested"
            };
            scene.narration_rested = Some(render::render(
                &named(&tables.prose, key)?
                    .variants
                    .first()
                    .ok_or_else(|| GenerateError::MissingTable(key.into()))?
                    .text,
                &context,
            )?);
        }
    }
    let errand_truth = format!("The errand's hidden detail is: {}", pack.courier_clue);
    let enemy_position = format!(
        "{} has {} waiting at {}.",
        pack.bandit.name, encounter.creatures, settings.confrontation.name
    );
    // SRD 5.2.1 p. 6 lists "Medium 15" among typical ability-check DCs.
    let findings = || {
        vec![
            pack::Finding {
                id: "errand-truth".into(),
                sentence: errand_truth.clone(),
                skill: pack::FindingSkill::Investigation,
                dc: 15,
                private: false,
                consequence: None,
            },
            pack::Finding {
                id: "enemy-position".into(),
                sentence: enemy_position.clone(),
                skill: pack::FindingSkill::Perception,
                dc: 15,
                private: false,
                consequence: None,
            },
        ]
    };
    for scene in &mut pack.scenes {
        if matches!(
            scene.key,
            SceneKey::Room | SceneKey::Opening | SceneKey::DialogueSeal | SceneKey::DialogueEscort
        ) {
            scene.findings = findings();
        }
    }
    pack.beat_script = Some(beat_template.assemble(&pack, &mut expression)?);
    for key in forced.keys() {
        if !roller.used.contains(key) {
            return Err(GenerateError::UnknownForced(key.clone()));
        }
    }
    let pack_bytes = serde_json::to_vec(&pack).map_err(|_| GenerateError::Serialization)?;
    let tuning_bytes = serde_json::to_vec(&tuning).map_err(|_| GenerateError::Serialization)?;
    pack::load_from_bytes(&pack_bytes, &tuning_bytes).map_err(GenerateError::Pack)?;
    let mut adventure = GeneratedAdventure {
        seed,
        tables_revision: tables.revision.clone(),
        rules_mode: options.rules_mode,
        rating: options.rating,
        facts: context,
        fact_sheet: sheets,
        rolls: roller.rolls,
        pack,
        tuning,
        settings,
        dossiers: Vec::new(),
    };
    adventure.dossiers = super::dossier_cast::roll_for(
        tables,
        &adventure,
        &super::dossier_cast::PublicStory::default(),
    )?;
    Ok(adventure)
}
fn check(slot: &str, text: &str, maximum: usize) -> Result<(), GenerateError> {
    let actual = text.chars().count();
    if actual > maximum {
        return Err(GenerateError::Bounds {
            slot: slot.into(),
            actual,
            maximum,
        });
    }
    if text.contains(['{', '}']) {
        return Err(GenerateError::Pattern(slot.into()));
    }
    Ok(())
}

struct EncounterDetails {
    count: u16,
    creatures: String,
    weapons: String,
    fallback: String,
}

fn encounter_details(ant: &Antagonist) -> Result<EncounterDetails, GenerateError> {
    let tactical = ant
        .tactical
        .as_ref()
        .ok_or_else(|| GenerateError::Pattern("ant.tactical".into()))?;
    let mut count = 0_u16;
    let mut creatures = Vec::new();
    let mut weapons = BTreeSet::new();
    for enemy in &tactical.enemies {
        count += u16::from(enemy.count);
        let creature = df_rules::SRD_521_CREATURES
            .iter()
            .find(|creature| creature.id == enemy.srd_id)
            .ok_or_else(|| GenerateError::Pattern("tactical creature source".into()))?;
        let spec = df_rules::tactical::CombatantSpec::creature(
            1,
            &enemy.srd_id,
            df_rules::tactical::Cell(0, 0),
            enemy.ammunition,
        )
        .map_err(|_| GenerateError::Pattern("tactical creature weapons".into()))?;
        for weapon in spec.weapons {
            weapons.insert(weapon.id().replace('-', " "));
        }
        creatures.push((enemy.count, creature.name.to_lowercase()));
    }
    if count == 0 {
        return Err(GenerateError::Pattern("empty tactical encounter".into()));
    }
    let creatures = creatures
        .iter()
        .map(|(number, name)| {
            if *number == 1 {
                format!("one {name}")
            } else {
                format!("{} {}", number_word(u16::from(*number)), pluralize(name))
            }
        })
        .collect::<Vec<_>>()
        .join(" and ");
    let weapons = weapons.into_iter().collect::<Vec<_>>().join(", ");
    let noun = if count == 1 { "takes" } else { "take" };
    let fallback = format!("{creatures} {noun} the field with {weapons}.");
    Ok(EncounterDetails {
        count,
        creatures,
        weapons,
        fallback,
    })
}

fn number_word(number: u16) -> String {
    match number {
        2 => "two".into(),
        3 => "three".into(),
        4 => "four".into(),
        _ => number.to_string(),
    }
}

fn pluralize(noun: &str) -> String {
    if let Some(stem) = noun.strip_suffix("lf") {
        format!("{stem}lves")
    } else if let Some(stem) = noun
        .strip_suffix('y')
        .filter(|stem| !stem.ends_with(['a', 'e', 'i', 'o', 'u']))
    {
        format!("{stem}ies")
    } else if noun.ends_with('s') {
        format!("{noun}es")
    } else {
        format!("{noun}s")
    }
}
