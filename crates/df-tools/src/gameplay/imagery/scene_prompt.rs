//! Photographic prompts from the same accepted scene snapshot used by narration.

use super::{AssembledPrompt, ImageFailure, prompt::Part};
use crate::gameplay::{
    adventure::scene_model::{SceneModel, SceneThing},
    startup_configuration::ContentRating,
};

const PHOTO: &str = "One photographic frame from a live-action fantasy film, shot with a real \
camera and a prime lens, natural depth of field. Real worn materials, pores, sewn wool, linen \
and creased leather. Motivated uneven practical light, deep shadows, muted colour grade, \
fine film grain. No painted or illustrated appearance. No text, logos or watermarks. \
Every person is an adult; no real person's likeness.";

fn assembled(rating: ContentRating, text: String) -> Result<AssembledPrompt, ImageFailure> {
    let rating = match rating {
        ContentRating::Mature => "Mature story content is not softened.",
        ContentRating::Restrained => "No gore, nudity or sexual content.",
    };
    let prompt = AssembledPrompt::from_parts(vec![
        Part::Fact(text),
        Part::Fixed(PHOTO),
        Part::Fixed(rating),
    ]);
    if prompt.as_str().chars().count() > 4000 {
        return Err(ImageFailure::InvalidRequest);
    }
    Ok(prompt)
}

pub(in crate::gameplay) fn model_plate(
    rating: ContentRating,
    model: &SceneModel,
    place: &str,
    heroes: &[String],
) -> Result<AssembledPrompt, ImageFailure> {
    let state = model
        .picture_state()
        .map_err(|_| ImageFailure::InvalidRequest)?;
    assembled(
        rating,
        format!(
            "Compose this exact place: {place}. Wide 28mm view from among the party. \
Only the listed people, things and ways out exist; do not add extras or props from the \
reference painting. Follow each person's position and activity and each object's state. \
Keep the identities, faces and clothing of the supplied cast portraits and party sheet. \
Party heroes (in party-sheet order): {}. Heroes remain small at the foreground edges. \
The scene as it is now (JSON): {state}",
            heroes.join(", ")
        ),
    )
}

pub(in crate::gameplay) fn state_edit(
    rating: ContentRating,
    model: &SceneModel,
    change: &str,
) -> Result<AssembledPrompt, ImageFailure> {
    let state = model
        .picture_state()
        .map_err(|_| ImageFailure::InvalidRequest)?;
    assembled(
        rating,
        format!(
            "Image 1 is the previous film still of this exact scene. Edit it for this visible \
change: {change}. Keep the place, cast identities, faces, costumes, camera and framing. \
Change only what the current state requires; invent no people or objects. \
The scene as it is now (JSON): {state}"
        ),
    )
}

pub(in crate::gameplay) fn item_picture(
    rating: ContentRating,
    thing: &SceneThing,
    observed_detail: Option<&str>,
) -> Result<AssembledPrompt, ImageFailure> {
    let trace = observed_detail.map_or(String::new(), |detail| {
        format!("Show this observed trace plainly and physically: {detail}.")
    });
    assembled(
        rating,
        format!(
            "A close-up of this object alone: {}. Detailed description: {}. \
Current object state: open={}, lit={}, broken={}. {trace} \
Image 1, when supplied, is a scene still for light, materials and photographic style only. \
Show no people, hands, faces, figures or silhouettes. Fill the card with the object.",
            thing.picture_description, thing.detail, thing.open, thing.lit, thing.broken,
        ),
    )
}

pub(in crate::gameplay) fn graph_edit(
    rating: ContentRating,
    graph: &crate::gameplay::shot::scene_graph::SceneGraph,
    change: &str,
) -> Result<AssembledPrompt, ImageFailure> {
    let actors = graph
        .actors
        .iter()
        .map(|actor| {
            serde_json::json!({
                "name": actor.name, "position": actor.spot, "pose": actor.pose,
                "expression": actor.expression,
                "anchor": [actor.anchor_x_permille, actor.anchor_y_permille],
            })
        })
        .collect::<Vec<_>>();
    let actors = serde_json::to_string(&actors).map_err(|_| ImageFailure::InvalidRequest)?;
    assembled(
        rating,
        format!(
            "Image 1 is the previous film still of this exact scene. Edit for this visible change: \
{change}. Keep the place, cast identities, faces, costumes, lens and framing. Add no extras. \
The scene as it is now (JSON): {}. Actors in graph revision {} (JSON): {actors}",
            graph.visible_state, graph.revision,
        ),
    )
}
