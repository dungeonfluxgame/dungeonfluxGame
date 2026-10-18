//! A narrative reveal reframes an existing public subject without performing an interaction.

use super::{
    AssembledPrompt, ImageFailure,
    prompt::{FILM_LOOK, Part},
};
use crate::gameplay::{
    adventure::scene_model::SceneModel, narrative_focus::NarrativeFocus,
    startup_configuration::ContentRating,
};

pub(in crate::gameplay) fn reveal(
    rating: ContentRating,
    model: &SceneModel,
    focus: &NarrativeFocus,
) -> Result<AssembledPrompt, ImageFailure> {
    if !focus.subject.remains_current(model) {
        return Err(ImageFailure::InvalidRequest);
    }
    let state = model
        .picture_state()
        .map_err(|_| ImageFailure::InvalidRequest)?;
    let subject = serde_json::to_string(focus).map_err(|_| ImageFailure::InvalidRequest)?;
    let prompt = AssembledPrompt::from_parts(vec![
        Part::Fixed(FILM_LOOK),
        Part::Fixed(
            "Image 1 is the currently displayed photograph. Image 2 is the canonical room photograph: anchor the room's architecture, geography, materials and light to it rather than recursively copying compression or edit artifacts from image 1. Reframe the same medieval live-action scene to make the one canonical narrative focus below plainly visible and useful for further inspection. Use a close, informative camera composition: the discussed goods must be large enough to distinguish their described form and material, rather than vague background dressing. Retain enough of the familiar seller, counter and nearby cast to make the goods' location clear. Preserve every canonical actor's exact identity, face, age, hair and costume using the identity sheet and speaker reference. Keep all people and held objects in their current positions and ownership. An offer is available seller stock, not a purchased or gifted item: show its actual described goods at the seller's display, with no ownership transfer, payment, newly held parcel, consumption or sale. For a scene thing, preserve its exact position, holder and owner. Do not add other goods, people, transactions or outcomes. A conversation, question or hypothetical only selects an existing subject; never show hypothetical actions, violence or future events. Do not invent a color, emblem or item property absent from the canonical description. Do not write labels, prices, dialogue or other text into the photograph. Leave caption space by the speaking face. Canonical current scene (JSON):",
        ),
        Part::Fact(state),
        Part::Fixed("Canonical public narrative focus (JSON):"),
        Part::Fact(subject),
        Part::Fixed(match rating {
            ContentRating::Mature => "Mature story content is not softened.",
            ContentRating::Restrained => "No gore, nudity or sexual content.",
        }),
    ]);
    if prompt.character_count() > df_providers::IMAGE_EDIT_MAXIMUM_PROMPT_CHARACTERS {
        return Err(ImageFailure::InvalidRequest);
    }
    Ok(prompt)
}
