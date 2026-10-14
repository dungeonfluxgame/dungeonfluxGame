//! The audience-safe composition for one story scene.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{HeroSlot, ShotSpec};
use crate::gameplay::{
    imagery::{
        CharacterFacts, CharacterRole, ImageFailure, ImageKind, ImageRequest, OutputSpec,
        ReferenceImage, cast_expression,
    },
    pack::SceneKey,
    scene_mood::{Expression, HealthTier},
    shot::PlateLayout,
    startup_configuration::ContentRating,
};

#[cfg(test)]
#[path = "../gameplay_browser/scene_placement.rs"]
mod scene_placement;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::gameplay) enum CameraFraming {
    Wide,
    TwoShot,
    Speaker,
}

impl CameraFraming {
    fn label(self) -> &'static str {
        match self {
            Self::Wide => "wide",
            Self::TwoShot => "two-shot",
            Self::Speaker => "speaker",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::gameplay) enum Weather {
    Clear,
    Fog,
    Rain,
    Snow,
    Wind,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(in crate::gameplay) struct SceneGraph {
    pub(in crate::gameplay) revision: u64,
    pub(in crate::gameplay) scene_key: String,
    pub(in crate::gameplay) set: SceneSet,
    pub(in crate::gameplay) camera: CameraFraming,
    pub(in crate::gameplay) actors: Vec<SceneActor>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(in crate::gameplay) struct SceneSet {
    pub(in crate::gameplay) place_id: String,
    pub(in crate::gameplay) place_name: String,
    pub(in crate::gameplay) asset: String,
    pub(in crate::gameplay) fallback_asset: String,
    pub(in crate::gameplay) exterior: bool,
    pub(in crate::gameplay) time_of_day: String,
    pub(in crate::gameplay) weather: Option<Weather>,
    pub(in crate::gameplay) lights: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(in crate::gameplay) struct SceneActor {
    pub(in crate::gameplay) id: String,
    pub(in crate::gameplay) name: String,
    pub(in crate::gameplay) art: String,
    pub(in crate::gameplay) fallback_art: String,
    pub(in crate::gameplay) spot: String,
    pub(in crate::gameplay) anchor_x_permille: u32,
    pub(in crate::gameplay) anchor_y_permille: u32,
    pub(in crate::gameplay) depth_permille: u32,
    pub(in crate::gameplay) pose: String,
    pub(in crate::gameplay) expression: String,
    pub(in crate::gameplay) look_at: Option<String>,
    pub(in crate::gameplay) speaking: bool,
}

/// Public actor material only. The graph builder cannot accept dialogue or private clue text.
#[derive(Clone, Copy, Debug)]
pub(in crate::gameplay) struct ActorInput<'a> {
    pub(in crate::gameplay) id: &'a str,
    pub(in crate::gameplay) name: &'a str,
    pub(in crate::gameplay) fallback_art: &'a str,
    pub(in crate::gameplay) live_art: Option<&'a str>,
    pub(in crate::gameplay) expression: Option<Expression>,
    pub(in crate::gameplay) hero_art: Option<u8>,
    pub(in crate::gameplay) speaking: bool,
}

/// Inputs are server-owned scene data and already measured shot placement.
#[derive(Clone, Copy, Debug)]
pub(in crate::gameplay) struct SceneGraphInput<'a> {
    pub(in crate::gameplay) scene_key: SceneKey,
    pub(in crate::gameplay) place_id: &'a str,
    pub(in crate::gameplay) place_name: &'a str,
    pub(in crate::gameplay) fallback_set: &'a str,
    pub(in crate::gameplay) refreshed_set: Option<&'a str>,
    pub(in crate::gameplay) exterior: bool,
    pub(in crate::gameplay) weather: Option<Weather>,
    pub(in crate::gameplay) shot: &'a ShotSpec,
    pub(in crate::gameplay) actors: &'a [ActorInput<'a>],
    pub(in crate::gameplay) speaker_id: Option<&'a str>,
    pub(in crate::gameplay) listener_id: Option<&'a str>,
    pub(in crate::gameplay) measured_speaker_layout: Option<PlateLayout>,
    pub(in crate::gameplay) scene_open: bool,
    pub(in crate::gameplay) narrator_speaks: bool,
    pub(in crate::gameplay) line_length: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum SceneGraphError {
    MissingSceneData,
    MissingActor,
    MissingHeroPlacement,
    DuplicateActor,
    InvalidActor,
    Serialization,
}

/// Builds a stable graph from public story state. Its revision identifies visible graph content.
pub(in crate::gameplay) fn build(
    input: &SceneGraphInput<'_>,
) -> Result<SceneGraph, SceneGraphError> {
    if input.place_id.trim().is_empty()
        || input.place_name.trim().is_empty()
        || input.fallback_set.trim().is_empty()
    {
        return Err(SceneGraphError::MissingSceneData);
    }
    if input.actors.is_empty() {
        return Err(SceneGraphError::MissingActor);
    }
    for (index, actor) in input.actors.iter().enumerate() {
        if actor.id.trim().is_empty()
            || actor.name.trim().is_empty()
            || actor.fallback_art.trim().is_empty()
        {
            return Err(SceneGraphError::InvalidActor);
        }
        if input.actors[..index]
            .iter()
            .any(|earlier| earlier.id == actor.id)
        {
            return Err(SceneGraphError::DuplicateActor);
        }
    }
    if input
        .speaker_id
        .is_some_and(|id| !input.actors.iter().any(|actor| actor.id == id))
        || input
            .listener_id
            .is_some_and(|id| !input.actors.iter().any(|actor| actor.id == id))
    {
        return Err(SceneGraphError::MissingActor);
    }

    let speaker_layout = input
        .measured_speaker_layout
        .or_else(|| input.shot.speaker.as_ref().map(|speaker| speaker.layout));
    let mut other_cast_index = 0usize;
    let mut actors = Vec::with_capacity(input.actors.len());
    for actor in input.actors {
        let (spot, anchor_x, anchor_y, depth) = if let Some(hero_art) = actor.hero_art {
            let placement = input
                .shot
                .heroes
                .iter()
                .find(|hero| hero.art == hero_art)
                .ok_or(SceneGraphError::MissingHeroPlacement)?;
            hero_spot(placement.slot)
        } else if Some(actor.id) == input.speaker_id {
            let layout = speaker_layout.ok_or(SceneGraphError::MissingActor)?;
            (
                "speaker".to_owned(),
                layout.head_x_permille,
                (layout.head_y_permille + 540).min(960),
                620,
            )
        } else {
            let spot = cast_spot(other_cast_index, input.actors.len());
            other_cast_index += 1;
            spot
        };
        let live_art = actor.live_art.filter(|art| !art.trim().is_empty());
        let look_at = if Some(actor.id) == input.speaker_id {
            input.listener_id
        } else {
            input.speaker_id
        };
        actors.push(SceneActor {
            id: actor.id.to_owned(),
            name: actor.name.to_owned(),
            art: live_art.unwrap_or(actor.fallback_art).to_owned(),
            fallback_art: actor.fallback_art.to_owned(),
            spot,
            anchor_x_permille: u32::from(anchor_x),
            anchor_y_permille: u32::from(anchor_y),
            depth_permille: u32::from(depth),
            pose: "standing".to_owned(),
            expression: actor
                .expression
                .map(Expression::label)
                .unwrap_or("neutral")
                .to_owned(),
            look_at: look_at.map(str::to_owned),
            speaking: actor.speaking,
        });
    }
    actors.sort_by_key(|actor| actor.depth_permille);

    let camera = if input.scene_open || input.narrator_speaks || input.speaker_id.is_none() {
        CameraFraming::Wide
    } else if input.line_length >= 160 {
        CameraFraming::Speaker
    } else if input.listener_id.is_some() {
        CameraFraming::TwoShot
    } else {
        CameraFraming::Speaker
    };
    let set = SceneSet {
        place_id: input.place_id.to_owned(),
        place_name: input.place_name.to_owned(),
        asset: input
            .refreshed_set
            .filter(|asset| !asset.trim().is_empty())
            .unwrap_or(input.fallback_set)
            .to_owned(),
        fallback_asset: input.fallback_set.to_owned(),
        exterior: input.exterior,
        time_of_day: time_label(input.shot.time_of_day).to_owned(),
        weather: input.weather,
        lights: input
            .shot
            .lights
            .iter()
            .map(|light| (*light).to_owned())
            .collect(),
    };
    let scene_key = scene_key_label(input.scene_key).to_owned();
    let revision_bytes = serde_json::to_vec(&(&scene_key, &set, camera, &actors))
        .map_err(|_| SceneGraphError::Serialization)?;
    let digest = Sha256::digest(revision_bytes);
    let first_eight = digest
        .get(..8)
        .ok_or(SceneGraphError::Serialization)?
        .try_into()
        .map_err(|_| SceneGraphError::Serialization)?;
    let revision = u64::from_be_bytes(first_eight);

    Ok(SceneGraph {
        revision,
        scene_key,
        set,
        camera,
        actors,
    })
}

/// Makes the transparent standing hero cut-out request from that hero's party-sheet reference.
pub(in crate::gameplay) fn hero_cutout_request(
    rating: ContentRating,
    party_sheet_figure: ReferenceImage,
    figure_index: usize,
    figure_count: usize,
) -> Result<ImageRequest, ImageFailure> {
    let prompt = cast_expression(
        rating,
        CharacterFacts {
            role: CharacterRole::Hero,
            life_stage: super::LifeStage::Adult,
        },
        Expression::Neutral,
        HealthTier::Healthy,
        false,
    )
    .map_err(|_| ImageFailure::InvalidRequest)?
    .with_development_direction(&format!(
        "The party sheet has {figure_count} figures from left to right. Isolate only figure {} from the left as a full-body standing transparent cut-out; preserve the figure's identity and clothing.",
        figure_index + 1
    ));
    Ok(ImageRequest {
        kind: ImageKind::CastExpression,
        references: vec![party_sheet_figure],
        prompt,
        output: OutputSpec::CUTOUT,
    })
}

fn hero_spot(slot: HeroSlot) -> (String, u16, u16, u16) {
    match slot {
        HeroSlot::LeftForeground => ("hero-left".to_owned(), 140, 870, 930),
        HeroSlot::RightForeground => ("hero-right".to_owned(), 860, 870, 930),
        HeroSlot::LeftMid => ("hero-mid-left".to_owned(), 240, 760, 650),
        HeroSlot::RightMid => ("hero-mid-right".to_owned(), 760, 760, 650),
        HeroSlot::BackLeft => ("hero-back-left".to_owned(), 170, 650, 380),
        HeroSlot::BackRight => ("hero-back-right".to_owned(), 830, 650, 380),
    }
}

fn cast_spot(index: usize, total_actors: usize) -> (String, u16, u16, u16) {
    let cast_count = total_actors.saturating_sub(1).max(1);
    let spread = 700usize.saturating_mul(index + 1) / (cast_count + 1);
    let anchor_x = 150usize.saturating_add(spread).min(850) as u16;
    let depth = 300usize.saturating_add((index % 5) * 65).min(560) as u16;
    let anchor_y = 690u16.saturating_sub(((index % 3) as u16) * 45);
    (format!("cast-{}", index + 1), anchor_x, anchor_y, depth)
}

fn time_label(time: super::TimeOfDay) -> &'static str {
    match time {
        super::TimeOfDay::Dawn => "dawn",
        super::TimeOfDay::Morning => "morning",
        super::TimeOfDay::Midday => "midday",
        super::TimeOfDay::Afternoon => "afternoon",
        super::TimeOfDay::Dusk => "dusk",
        super::TimeOfDay::Night => "night",
    }
}

fn scene_key_label(key: SceneKey) -> &'static str {
    match key {
        SceneKey::Room => "room",
        SceneKey::Opening => "opening",
        SceneKey::DialogueSeal => "dialogue-seal",
        SceneKey::DialogueEscort => "dialogue-escort",
        SceneKey::Combat => "combat",
        SceneKey::Inn => "inn",
        SceneKey::Victory => "victory",
        SceneKey::Defeat => "defeat",
    }
}

/// Fields for the diagnostic event when the session owner observes a new graph revision.
pub(in crate::gameplay) fn change_log(graph: &SceneGraph) -> [(&'static str, String); 4] {
    let actors = graph
        .actors
        .iter()
        .map(|actor| format!("{}@{}", actor.name, actor.spot))
        .collect::<Vec<_>>()
        .join(",");
    [
        ("scene", graph.scene_key.clone()),
        ("revision", graph.revision.to_string()),
        ("actors", format!("[{actors}]")),
        ("camera", graph.camera.label().to_owned()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gameplay::shot::{self, ShotInput, SpeakerInput};

    struct TestActor {
        id: String,
        name: String,
        fallback_art: String,
        hero_art: Option<u8>,
        speaking: bool,
    }

    fn graph_for(
        scene: crate::gameplay::pack::Scene,
        refreshed_set: Option<&str>,
        live_hero_art: Option<&str>,
        line_length: usize,
    ) -> (SceneGraph, String, String) {
        let scene_key = scene.key;
        let setting = scene.setting.as_ref().expect("story fixture has a setting");
        let place_kind = serde_json::to_value(setting.kind)
            .expect("setting kind serializes")
            .as_str()
            .expect("setting kind is a string")
            .to_owned();
        let speaker = scene
            .cast
            .iter()
            .find(|actor| !scene.speaker.is_empty() && actor.name == scene.speaker);
        let other_sprites: Vec<&str> = scene
            .cast
            .iter()
            .filter(|actor| speaker.is_none_or(|speaker| actor.id != speaker.id))
            .map(|actor| actor.sprite.as_str())
            .collect();
        let party_art = [1, 2];
        let story = format!("{} {}", scene.narration, scene.dialogue);
        let shot = shot::direct(&ShotInput {
            scene_key,
            place_id: &setting.place_id,
            place_kind: &place_kind,
            exterior: setting.exterior,
            title: &scene.title,
            story_text: &story,
            speaker: speaker.map(|actor| SpeakerInput {
                sprite: &actor.sprite,
                gender: actor.gender,
                age: actor.age,
            }),
            others: &other_sprites,
            heroes: &party_art,
        })
        .expect("story fixture stages");
        let mut owned = vec![
            TestActor {
                id: "hero-1".to_owned(),
                name: "Brakka".to_owned(),
                fallback_art: "/assets/ui/generated/figure-hero-1.webp".to_owned(),
                hero_art: Some(1),
                speaking: false,
            },
            TestActor {
                id: "hero-2".to_owned(),
                name: "Sigrun".to_owned(),
                fallback_art: "/assets/ui/generated/figure-hero-2.webp".to_owned(),
                hero_art: Some(2),
                speaking: false,
            },
        ];
        owned.extend(scene.cast.iter().map(|actor| TestActor {
            id: actor.id.clone(),
            name: actor.name.clone(),
            fallback_art: format!("/assets/ui/generated/{}", actor.sprite),
            hero_art: None,
            speaking: !scene.speaker.is_empty() && actor.name == scene.speaker,
        }));
        let speaker_id = owned
            .iter()
            .find(|actor| actor.speaking)
            .map(|actor| actor.id.as_str());
        let actor_inputs: Vec<ActorInput<'_>> = owned
            .iter()
            .enumerate()
            .map(|(index, actor)| ActorInput {
                id: &actor.id,
                name: &actor.name,
                fallback_art: &actor.fallback_art,
                live_art: if index == 0 { live_hero_art } else { None },
                expression: Some(Expression::Neutral),
                hero_art: actor.hero_art,
                speaking: actor.speaking,
            })
            .collect();
        let input = SceneGraphInput {
            scene_key,
            place_id: &setting.place_id,
            place_name: &setting.name,
            fallback_set: &scene.asset,
            refreshed_set,
            exterior: setting.exterior,
            weather: None,
            shot: &shot,
            actors: &actor_inputs,
            speaker_id,
            listener_id: speaker_id.map(|_| "hero-1"),
            measured_speaker_layout: shot.speaker.as_ref().map(|speaker| speaker.layout),
            scene_open: scene_key == SceneKey::Opening,
            narrator_speaks: speaker_id.is_none(),
            line_length,
        };
        let graph = build(&input).expect("public story scene graph builds");
        (graph, scene.narration, scene.dialogue)
    }

    fn scene_from_pack(
        pack: &serde_json::Value,
        key: SceneKey,
    ) -> Option<crate::gameplay::pack::Scene> {
        pack["scenes"]
            .as_array()?
            .iter()
            .chain(
                pack["beat_script"]["scenes"]
                    .as_object()
                    .into_iter()
                    .flat_map(|scenes| scenes.values()),
            )
            .find_map(|value| {
                let scene: crate::gameplay::pack::Scene =
                    serde_json::from_value(value.clone()).ok()?;
                (scene.key == key).then_some(scene)
            })
    }

    fn saved_pack_paths() -> Vec<std::path::PathBuf> {
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        for ancestor in manifest.ancestors().take(7) {
            let Ok(entries) = std::fs::read_dir(ancestor.join("development/runs")) else {
                continue;
            };
            let mut packs: Vec<std::path::PathBuf> = entries
                .flatten()
                .map(|entry| entry.path().join("pack.json"))
                .filter(|path| path.is_file())
                .collect();
            packs.sort();
            if !packs.is_empty() {
                return packs;
            }
        }
        Vec::new()
    }

    fn three_adventure_pairs() -> Vec<(crate::gameplay::pack::Scene, crate::gameplay::pack::Scene)>
    {
        let mut pairs = Vec::new();
        let mut adventure_ids = std::collections::HashSet::new();
        for path in saved_pack_paths() {
            let Ok(text) = std::fs::read_to_string(path) else {
                continue;
            };
            let Ok(pack) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            let Some(adventure_id) = pack["id"].as_str() else {
                continue;
            };
            if !adventure_ids.insert(adventure_id.to_owned()) {
                continue;
            }
            let Some(opening) = scene_from_pack(&pack, SceneKey::Opening) else {
                continue;
            };
            let second = scene_from_pack(&pack, SceneKey::DialogueSeal)
                .or_else(|| scene_from_pack(&pack, SceneKey::DialogueEscort));
            if let Some(second) = second {
                pairs.push((opening, second));
            }
            if pairs.len() == 3 {
                return pairs;
            }
        }
        for fixture in [
            "harbor-journey-pack-baseline.json",
            "harbor-journey-pack-integration-baseline.json",
            "harbor-journey-tuning-baseline.json",
        ] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src/gameplay/fixtures")
                .join(fixture);
            let Ok(text) = std::fs::read_to_string(path) else {
                continue;
            };
            let Ok(pack) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            let Some(adventure_id) = pack["id"].as_str() else {
                continue;
            };
            if !adventure_ids.insert(adventure_id.to_owned()) {
                continue;
            }
            let Some(opening) = scene_from_pack(&pack, SceneKey::Opening) else {
                continue;
            };
            let second = scene_from_pack(&pack, SceneKey::DialogueSeal)
                .or_else(|| scene_from_pack(&pack, SceneKey::DialogueEscort));
            if let Some(second) = second {
                pairs.push((opening, second));
            }
            if pairs.len() == 3 {
                break;
            }
        }
        pairs
    }

    #[test]
    fn opening_and_second_scenes_of_three_adventures_have_safe_ordered_graphs() {
        let pairs = three_adventure_pairs();
        assert_eq!(pairs.len(), 3, "three adventure fixtures are required");
        for (opening, second) in pairs {
            let second_key = second.key;
            for (scene, scene_key) in [(opening, SceneKey::Opening), (second, second_key)] {
                let setting = scene.setting.as_ref().expect("story scene has a setting");
                let (graph, narration, dialogue) =
                    graph_for(scene.clone(), Some("/gameplay/imagery/empty-set"), None, 40);
                assert_eq!(graph.scene_key, scene_key_label(scene_key));
                if scene_key == SceneKey::Opening {
                    assert_eq!(graph.camera, CameraFraming::Wide);
                }
                assert!(graph.actors.len() >= 2);
                assert!(
                    graph
                        .actors
                        .windows(2)
                        .all(|pair| { pair[0].depth_permille <= pair[1].depth_permille })
                );
                assert!(graph.actors.iter().all(|actor| {
                    actor.anchor_x_permille <= 1000
                        && actor.anchor_y_permille <= 1000
                        && actor.depth_permille <= 1000
                        && !actor.spot.is_empty()
                        && actor.pose == "standing"
                }));
                assert!(graph.actors.iter().any(|actor| actor.id == "hero-1"));
                assert!(graph.actors.iter().any(|actor| actor.id == "hero-2"));
                if let Some(speaker) = graph.actors.iter().find(|actor| actor.speaking) {
                    if scene_key != SceneKey::Opening {
                        assert_eq!(graph.camera, CameraFraming::TwoShot);
                        let long_line = graph_for(
                            scene.clone(),
                            Some("/gameplay/imagery/empty-set"),
                            None,
                            180,
                        )
                        .0;
                        assert_eq!(long_line.camera, CameraFraming::Speaker);
                    }
                    assert_eq!(speaker.look_at.as_deref(), Some("hero-1"));
                    assert!(
                        graph
                            .actors
                            .iter()
                            .filter(|actor| actor.id.starts_with("hero-"))
                            .all(|actor| actor.look_at.as_deref() == Some(speaker.id.as_str()))
                    );
                }
                let graph_json = serde_json::to_string(&graph).expect("graph serializes");
                assert!(!graph_json.contains("private_clue"));
                assert!(!graph_json.contains("secret"));
                assert!(narration.is_empty() || !graph_json.contains(&narration));
                assert!(dialogue.is_empty() || !graph_json.contains(&dialogue));
                assert_eq!(graph.set.place_id, setting.place_id);
                assert_eq!(graph.set.place_name, setting.name);
                let diagnostic = change_log(&graph);
                assert_eq!(diagnostic[0], ("scene", graph.scene_key.clone()));
                assert_eq!(diagnostic[1], ("revision", graph.revision.to_string()));
                assert_eq!(diagnostic[2].0, "actors");
                assert!(diagnostic[2].1.starts_with('[') && diagnostic[2].1.ends_with(']'));
                assert!(graph.actors.iter().all(|actor| {
                    diagnostic[2]
                        .1
                        .contains(&format!("{}@{}", actor.name, actor.spot))
                }));
                assert_eq!(
                    diagnostic[3],
                    (
                        "camera",
                        match graph.camera {
                            CameraFraming::Wide => "wide",
                            CameraFraming::TwoShot => "two-shot",
                            CameraFraming::Speaker => "speaker",
                        }
                        .to_owned(),
                    )
                );
                let repeated = graph_for(scene, Some("/gameplay/imagery/empty-set"), None, 40).0;
                assert_eq!(graph.revision, repeated.revision);
            }
        }
    }

    #[test]
    fn actor_art_prefers_a_live_cutout_and_keeps_the_painted_sprite_fallback() {
        let scene = three_adventure_pairs()
            .into_iter()
            .next()
            .expect("opening adventure fixture")
            .0;
        let (graph, _, _) = graph_for(
            scene.clone(),
            Some("/gameplay/imagery/empty-set"),
            Some("/gameplay/imagery/hero-cutout"),
            40,
        );
        let hero = graph
            .actors
            .iter()
            .find(|actor| actor.id == "hero-1")
            .expect("first hero is present");
        assert_eq!(hero.art, "/gameplay/imagery/hero-cutout");
        assert!(hero.fallback_art.ends_with("figure-hero-1.webp"));
        assert!(graph.set.asset.ends_with("empty-set"));
        let (fallback_graph, _, _) =
            graph_for(scene, Some("/gameplay/imagery/empty-set"), None, 40);
        let fallback = fallback_graph
            .actors
            .iter()
            .find(|actor| actor.id == "hero-1")
            .expect("first hero is present");
        assert_eq!(fallback.art, fallback.fallback_art);
    }

    #[test]
    fn hero_cutout_uses_the_existing_transparent_cast_expression_recipe() {
        let reference = ReferenceImage::new(
            df_providers::ImageFormat::Webp,
            bytes::Bytes::from_static(b"one-hero-party-sheet"),
        );
        let request = hero_cutout_request(ContentRating::Mature, reference.clone(), 0, 1)
            .expect("neutral adult hero cutout is allowed");
        assert_eq!(request.kind, ImageKind::CastExpression);
        assert_eq!(request.references.len(), 1);
        assert_eq!(request.references[0].sha256, reference.sha256);
        assert_eq!(request.output, OutputSpec::CUTOUT);
        assert!(request.output.transparent);
    }
}
