//! Typed scene-flow authoring data. Templates sequence existing handlers; prose cannot
//! introduce checks, combat statistics or other mechanical authority.
use super::super::{
    beat_script::{BeatEffect, BeatKind, BeatScript, BeatTrigger, ScriptError, ScriptErrorReason},
    journey::tactical::FightOutcome,
    pack::{self, PackFile, SceneKey},
};
use super::{
    GenerateError,
    tables::{TablesError, invalid, parse},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
pub(super) const CHAPTER_TYPE: &str = "chapter.type";
const FILE: &str = "beat-scripts.json";
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BeatScripts {
    schema: String,
    file: String,
    pub(super) chapter_types: BTreeMap<String, BeatTemplate>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BeatTemplate {
    script: BeatScript,
    scenes: BTreeMap<String, ScenePattern>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ScenePattern {
    scene: SceneKey,
    title: String,
    narration: String,
    line: String,
}
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum SceneRole {
    Room,
    Opening,
    Followup,
    Observation,
    Contact,
}
/// Delivery roles derive from the graph and closed handlers, never variant names.
pub(super) fn scene_roles(script: &BeatScript) -> Result<BTreeMap<String, SceneRole>, ScriptError> {
    let refuse = |beat: &str| ScriptError {
        beat: beat.into(),
        reason: ScriptErrorReason::HandlerWiring,
    };
    let entry = script
        .find(&script.entry)
        .ok_or_else(|| refuse(&script.entry))?;
    let begin = entry
        .offers
        .iter()
        .find(|offer| offer.effect == BeatEffect::BeginStory)
        .ok_or_else(|| refuse(&entry.id))?;
    let trigger = BeatTrigger::Offer {
        offer: begin.id.clone(),
    };
    let opening = entry
        .transitions
        .iter()
        .find(|transition| transition.trigger == trigger)
        .and_then(|transition| script.find(&transition.target))
        .ok_or_else(|| refuse(&entry.id))?;
    let mut roles = BTreeMap::new();
    for beat in &script.beats {
        let Some(variant) = &beat.scene_variant else {
            continue;
        };
        let role = if beat.id == opening.id {
            SceneRole::Opening
        } else {
            match beat.kind {
                BeatKind::Gather => SceneRole::Room,
                BeatKind::Investigate | BeatKind::Converse => SceneRole::Observation,
                BeatKind::Narrate | BeatKind::Travel => SceneRole::Followup,
                BeatKind::Conclude | BeatKind::Ending => SceneRole::Contact,
                BeatKind::Fight | BeatKind::Rest => return Err(refuse(&beat.id)),
            }
        };
        if roles
            .insert(variant.clone(), role)
            .is_some_and(|previous| previous != role)
        {
            return Err(refuse(&beat.id));
        }
    }
    Ok(roles)
}
impl BeatScripts {
    pub(super) fn load(
        bytes: &[u8],
        types: &super::campaign_tables::ChapterFile,
    ) -> Result<Self, TablesError> {
        // Use the same beat-naming preflight as packs before typed deserialization.
        let value: serde_json::Value = parse(FILE, bytes)?;
        if let Some(entries) = value
            .get("chapter_types")
            .and_then(|value| value.as_object())
        {
            for (kind, entry) in entries {
                if let Some(script) = entry.get("script") {
                    let wrapped = serde_json::to_vec(&serde_json::json!({"beat_script":script}))
                        .map_err(|_| invalid(FILE, kind, "serialization"))?;
                    super::super::beat_script::preflight(&wrapped)
                        .map_err(|error| invalid(FILE, kind, error.to_string()))?;
                }
            }
        }
        let result: Self = parse(FILE, bytes)?;
        if result.schema != "df-adventure-tables/1" || result.file != "beat-scripts" {
            return Err(invalid(FILE, "schema", "unsupported beat script table"));
        }
        let expected = types
            .tables
            .chapter_types
            .entries
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<BTreeSet<_>>();
        let actual = result
            .chapter_types
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if actual != expected {
            return Err(invalid(
                FILE,
                "chapter_types",
                "every chapter type requires exactly one explicit script",
            ));
        }
        let content: PackFile = parse(
            FILE,
            include_bytes!("../../../../../content/harbor-journey/pack.json"),
        )?;
        for (kind, template) in &result.chapter_types {
            template.validate(kind, &content)?;
        }
        Ok(result)
    }
    pub(super) fn chapter(&self, kind: &str) -> Result<&BeatTemplate, GenerateError> {
        self.chapter_types.get(kind).ok_or_else(|| {
            GenerateError::MissingTable(format!("beat-scripts.chapter_types.{kind}"))
        })
    }
}
impl BeatTemplate {
    fn validate(&self, kind: &str, content: &PackFile) -> Result<(), TablesError> {
        let path = |slot: &str| format!("chapter_types.{kind}.{slot}");
        if !self.script.scenes.is_empty() {
            return Err(invalid(
                FILE,
                &path("script.scenes"),
                "scene templates inherit generated settings/art/cast; use scene patterns",
            ));
        }
        let mut referenced = self
            .script
            .beats
            .iter()
            .filter_map(|beat| beat.scene_variant.clone())
            .collect::<BTreeSet<_>>();
        // A fight's closing scenes come in one variant per way the fight can end.
        if self.script.uses_encounter() {
            for beat in &self.script.beats {
                if let (BeatKind::Ending, Some(variant)) = (beat.kind, &beat.scene_variant)
                    && let Some(stem) = variant.strip_suffix(FightOutcome::Won.slug())
                {
                    referenced.extend(
                        FightOutcome::ALL
                            .iter()
                            .map(|outcome| format!("{stem}{}", outcome.slug())),
                    );
                }
            }
        }
        let authored = self.scenes.keys().cloned().collect::<BTreeSet<_>>();
        if referenced != authored {
            return Err(invalid(
                FILE,
                &path("scenes"),
                "every scene variant must have one referenced pattern",
            ));
        }
        let mut script = self.script.clone();
        for (slot, label) in &mut script.labels {
            *label = pattern(label, 40).map_err(|error| invalid(FILE, &path(slot), error))?;
        }
        for (id, entry) in &self.scenes {
            if matches!(
                entry.scene,
                SceneKey::Combat | SceneKey::Victory | SceneKey::Defeat
            ) {
                return Err(invalid(
                    FILE,
                    &path(id),
                    "peaceful scene pattern cannot inherit a combat/rest scene",
                ));
            }
            let mut scene = content
                .scenes
                .iter()
                .find(|scene| scene.key == entry.scene)
                .cloned()
                .ok_or_else(|| invalid(FILE, &path(id), "unknown base scene"))?;
            scene.title =
                pattern(&entry.title, 30).map_err(|error| invalid(FILE, &path(id), error))?;
            scene.narration =
                pattern(&entry.narration, 220).map_err(|error| invalid(FILE, &path(id), error))?;
            scene.dialogue =
                pattern(&entry.line, 160).map_err(|error| invalid(FILE, &path(id), error))?;
            scene.rules_note.clear();
            scene.narration_rested = None;
            script.scenes.insert(id.clone(), scene);
        }
        script
            .validate(content)
            .map_err(|error| invalid(FILE, &path("script"), error.to_string()))?;
        if !script.uses_encounter() {
            for (id, role) in scene_roles(&script)
                .map_err(|error| invalid(FILE, &path("script"), error.to_string()))?
            {
                let key = script
                    .scenes
                    .get(&id)
                    .ok_or_else(|| invalid(FILE, &path(&id), "unknown variant"))?
                    .key;
                let compatible = match role {
                    SceneRole::Room => key == SceneKey::Room,
                    SceneRole::Opening => key == SceneKey::Opening,
                    SceneRole::Followup => {
                        matches!(key, SceneKey::DialogueSeal | SceneKey::DialogueEscort)
                    }
                    SceneRole::Contact => key == SceneKey::Inn,
                    SceneRole::Observation => true,
                };
                if !compatible {
                    return Err(invalid(
                        FILE,
                        &path(&id),
                        "scene is incompatible with campaign delivery role",
                    ));
                }
            }
        }
        Ok(())
    }
    pub(super) fn assemble(
        &self,
        content: &PackFile,
        expression: &mut impl FnMut(
            Option<SceneKey>,
            &str,
            &str,
            &str,
            &str,
            usize,
        ) -> Result<String, GenerateError>,
    ) -> Result<BeatScript, GenerateError> {
        let mut script = self.script.clone();
        for text in script.labels.values_mut() {
            *text = expression(None, "scene-flow", "beat_label", "", text, 40)?;
        }
        for (id, entry) in &self.scenes {
            let mut scene = content
                .scenes
                .iter()
                .find(|scene| scene.key == entry.scene)
                .cloned()
                .ok_or_else(|| GenerateError::MissingTable(format!("beat scene {id}")))?;
            let speaker = if scene.speaker.is_empty() {
                ""
            } else if entry.scene == SceneKey::Inn {
                "contact"
            } else {
                "patron"
            };
            scene.title = expression(
                Some(entry.scene),
                "scene-flow",
                "title",
                "",
                &entry.title,
                30,
            )?;
            scene.narration = expression(
                Some(entry.scene),
                "scene-flow",
                "narration",
                "",
                &entry.narration,
                220,
            )?;
            scene.dialogue = expression(
                Some(entry.scene),
                "scene-flow",
                "line",
                speaker,
                &entry.line,
                160,
            )?;
            scene.rules_note.clear();
            scene.narration_rested = None;
            script.scenes.insert(id.clone(), scene);
        }
        for beat in &mut script.beats {
            if !matches!(
                beat.kind,
                super::super::beat_script::BeatKind::Converse
                    | super::super::beat_script::BeatKind::Investigate
                    | super::super::beat_script::BeatKind::Narrate
            ) {
                continue;
            }
            if let Some(scene) = beat
                .scene_variant
                .as_ref()
                .and_then(|variant| script.scenes.get(variant))
                .or_else(|| content.scenes.iter().find(|scene| scene.key == beat.scene))
                && (!scene.findings.is_empty() || !scene.cast.is_empty())
                && !beat
                    .offers
                    .iter()
                    .any(|offer| offer.effect == super::super::beat_script::BeatEffect::Observe)
            {
                BeatScript::add_look_around_offer(beat);
            }
        }
        script
            .validate(content)
            .map_err(|error| GenerateError::Pack(pack::PackError::Script(error)))?;
        Ok(script)
    }
}
fn pattern(text: &str, maximum: usize) -> Result<String, String> {
    let facts = [
        "patron.first",
        "contact.first",
        "stake.short",
        "dest.name",
        "place_short",
        "place_detail",
    ]
    .into_iter()
    .map(|key| (key.into(), "Example".into()))
    .collect();
    let rendered = super::render::render(text, &facts).map_err(|error| error.to_string())?;
    if rendered.chars().count() > maximum || rendered.chars().any(char::is_control) {
        return Err(format!("bounded public prose required (maximum {maximum})"));
    }
    Ok(rendered)
}
