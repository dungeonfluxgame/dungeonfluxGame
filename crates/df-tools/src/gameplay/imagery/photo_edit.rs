//! Edits a reviewed harbor photograph without asking the provider to stage a new key-art scene.

use super::{
    AssembledPrompt, ImageFailure,
    plate_prompt::expression_phrase_of,
    prompt::{FILM_LOOK, Part},
};
use crate::gameplay::{
    adventure::scene_model::SceneModel, shot::ShotSpec, startup_configuration::ContentRating,
};

const EDIT_PHOTOGRAPH: &str = "Edit image 1 as the actual photograph. Keep its camera position, \
    framing, perspective, optical softness, candid poses, imperfect materials, diffuse light \
    and restrained color unless canonical state changes them. Apply only listed scene facts. \
    No restaging, dramatic relighting, blue/amber grade, rim light, polished texture or poster \
    poses. Other images identify performers and clothes only, never a new composition.";

const TWO_TRAVELERS: &str = "Keep both foreground travelers from image 1: armored man at left \
    is party performer 1; adult woman at right is party performer 2. Match her red hair, armor \
    and blue cloak to the party sheet. Do not replace her with a man or porter, remove her, \
    or move either traveler.";

/// Uses the full-frame photograph as the edit base. The optional model remains the same
/// canonical picture state used by narration; rule-based shot embellishments are omitted.
pub(in crate::gameplay) fn harbor_photo_edit(
    rating: ContentRating,
    spec: &ShotSpec,
    model: Option<&SceneModel>,
    hero_names: &[String],
) -> Result<AssembledPrompt, ImageFailure> {
    let mut parts = vec![Part::Fixed(EDIT_PHOTOGRAPH), Part::Fixed(FILM_LOOK)];
    if spec.heroes.iter().any(|hero| hero.art == 1) && spec.heroes.iter().any(|hero| hero.art == 2)
    {
        parts.push(Part::Fixed(TWO_TRAVELERS));
    }
    if let Some(model) = model {
        let state = model
            .picture_state()
            .map_err(|_| ImageFailure::InvalidRequest)?;
        parts.push(Part::Fact(format!(
            "Canonical scene state (JSON): {state}. Only these people, objects and ways out \
             may appear. Apply their current positions, activities, states, time and weather; \
             remove any incompatible content from the base photograph. Party travelers in \
             party-sheet order: {}.",
            hero_names.join(", "),
        )));
    } else {
        parts.push(Part::Fact(format!(
            "The party has exactly {} travelers. Replace the foreground travelers with the \
             supplied party-sheet performers in sheet order; remove any surplus traveler. \
             Keep their cropped over-the-shoulder scale rather than moving the camera back.",
            spec.heroes.len(),
        )));
        for (index, hero) in spec.heroes.iter().enumerate() {
            parts.push(Part::Fact(format!("Traveler {}:", index + 1)));
            parts.push(Part::Fixed(hero.activity));
        }
        if let Some(speaker) = &spec.speaker {
            parts.extend([
                Part::Fixed("The speaker is"),
                Part::Fixed(speaker.look),
                Part::Fixed("Their current task is"),
                Part::Fixed(speaker.task),
                Part::Fixed("Their expression is"),
                Part::Fixed(expression_phrase_of(speaker.expression)),
            ]);
        }
        for other in &spec.others {
            parts.extend([
                Part::Fixed("Also present, at a natural distance behind the conversation:"),
                Part::Fixed(other),
            ]);
        }
    }
    parts.push(Part::Fixed(
        "Fictional adults only. No text, interface, borders or watermark.",
    ));
    parts.push(Part::Fixed(match rating {
        ContentRating::Mature => "Mature story content is not softened.",
        ContentRating::Restrained => "No gore, nudity or sexual content.",
    }));
    let prompt = AssembledPrompt::from_parts(parts);
    if prompt.as_str().chars().count() > 4000 {
        return Err(ImageFailure::InvalidRequest);
    }
    Ok(prompt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gameplay::{
        npc_dossier_tests::adventure,
        pack::SceneKey,
        shot::{self, ShotInput},
    };

    #[test]
    fn photograph_edit_keeps_canonical_object_states_time_and_weather() {
        let mut model = adventure(39)
            .settings
            .meeting
            .model
            .unwrap()
            .for_scene(SceneKey::Opening);
        model.time_of_day = "midday".to_owned();
        model.weather = "clear sky".to_owned();
        model.things[0].holder_id = Some("Brakka".to_owned());
        model.things[0].position = "held by Brakka".to_owned();
        model.ways_out[0].open = true;
        let spec = shot::direct(&ShotInput {
            scene_key: SceneKey::Opening,
            place_id: "salt-quay",
            place_kind: "quay",
            exterior: true,
            title: "The Broken Seal",
            story_text: "A courier waits.",
            speaker: None,
            others: &[],
            heroes: &[1, 2],
        })
        .unwrap();
        let prompt = harbor_photo_edit(
            ContentRating::Restrained,
            &spec,
            Some(&model),
            &["Brakka".into(), "Sigrun".into()],
        )
        .unwrap();
        assert!(prompt.as_str().contains(&model.picture_state().unwrap()));
        assert!(prompt.as_str().contains("Brakka, Sigrun"));
        assert!(
            prompt
                .as_str()
                .contains("adult woman at right is party performer 2")
        );
        assert!(prompt.as_str().chars().count() <= 4000);
    }
}
