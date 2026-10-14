use super::super::imagery::FILM_LOOK;
use super::super::shot::{HeroSlot, ShotSpec, SpeakerSlot};

const MAXIMUM_CHARACTERS: usize = 1500;

pub(in crate::gameplay) fn model_staging(
    model: &crate::gameplay::adventure::scene_model::SceneModel,
) -> String {
    let people = model
        .people
        .iter()
        .map(|person| format!("{} at {}", person.name, person.position))
        .collect::<Vec<_>>()
        .join("; ");
    let things = model
        .things
        .iter()
        .filter(|thing| thing.in_scene || thing.holder_id.is_some())
        .map(|thing| thing.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{}; {}; {people}. Existing objects: {things}. Continue only the activities visible in the still",
        model.time_of_day, model.weather
    )
}

const INSTRUCTIONS: [(&str, &str); 7] = [
    (
        "Keep the camera completely still: no pan, zoom, reframing, or cuts.",
        "Keeps the plate composition and the TV camera's own movement stable.",
    ),
    (
        "Keep every person's face, hair, clothing, body, and place in the frame unchanged.",
        "Preserves the identities and staging already in the plate.",
    ),
    (
        "No one enters or leaves; add no people, creatures, objects, signs, or text.",
        "Prevents new content from appearing in the scene.",
    ),
    (
        "People only breathe, blink, and shift their weight slightly.",
        "Adds life without changing a person's pose or expression.",
    ),
    (
        "Each person continues the small activity described for them, with only subtle hand motion.",
        "Makes movement fit the plate instead of inventing a different action.",
    ),
    (
        "Only fires, candles, lantern light, and other lights already visible may flicker; smoke may drift.",
        "Moves existing light and flame without changing the set.",
    ),
    (
        "Fine dust drifts slowly through the existing light.",
        "Adds restrained background motion while keeping the room intact.",
    ),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Refusal {
    EmptyDescription,
    InvalidDescription,
    PromptTooLong,
}

pub(super) fn ambient(description: &str) -> Result<String, Refusal> {
    let description = description.trim();
    if description.is_empty() {
        return Err(Refusal::EmptyDescription);
    }
    if description.chars().any(char::is_control) {
        return Err(Refusal::InvalidDescription);
    }

    let mut prompt = format!("The plate shows: {description}. {FILM_LOOK} ");
    for (instruction, _reason) in INSTRUCTIONS {
        prompt.push_str(instruction);
        prompt.push(' ');
    }
    if prompt.chars().count() > MAXIMUM_CHARACTERS {
        return Err(Refusal::PromptTooLong);
    }
    Ok(prompt)
}

/// Summarizes the exact staged positions and activities without copying player dialogue.
pub(super) fn staging(spec: &ShotSpec) -> String {
    let mut description = format!(
        "{} {}",
        spec.place_description,
        time_of_day(spec.time_of_day)
    );
    if let Some(speaker) = &spec.speaker {
        let side = match speaker.slot {
            SpeakerSlot::MidLeft => "mid-left",
            SpeakerSlot::MidRight => "mid-right",
        };
        description.push_str(&format!(
            "; the {} speaker stands {side} and is {}",
            speaker.look, speaker.task
        ));
    }
    for hero in &spec.heroes {
        let position = match hero.slot {
            HeroSlot::LeftForeground => "left foreground",
            HeroSlot::RightForeground => "right foreground",
            HeroSlot::LeftMid => "left middle distance",
            HeroSlot::RightMid => "right middle distance",
            HeroSlot::BackLeft => "back left",
            HeroSlot::BackRight => "back right",
        };
        description.push_str(&format!(
            "; hero {} is at the {position}, {}",
            hero.art, hero.activity
        ));
    }
    description
}

fn time_of_day(time: super::super::shot::TimeOfDay) -> &'static str {
    match time {
        super::super::shot::TimeOfDay::Dawn => "at dawn",
        super::super::shot::TimeOfDay::Morning => "in the morning",
        super::super::shot::TimeOfDay::Midday => "at midday",
        super::super::shot::TimeOfDay::Afternoon => "in the afternoon",
        super::super::shot::TimeOfDay::Dusk => "at dusk",
        super::super::shot::TimeOfDay::Night => "at night",
    }
}

#[cfg(test)]
mod tests {
    use super::{Refusal, ambient};

    #[test]
    fn ambient_prompt_keeps_scene_details_and_all_motion_instructions() {
        let prompt = ambient("the patron stands mid-left, rolling dice beside the hearth")
            .expect("a bounded scene description");
        assert!(prompt.contains("the patron stands mid-left, rolling dice beside the hearth"));
        assert!(prompt.contains("camera completely still"));
        assert!(prompt.contains("named film grade"));
        assert!(prompt.contains("fine film grain"));
        assert!(prompt.contains("no painting, illustration, concept art, CGI or game art"));
        assert!(prompt.contains("face, hair, clothing"));
        assert!(prompt.contains("No one enters or leaves"));
        assert!(prompt.contains("breathe, blink, and shift their weight"));
        assert!(prompt.contains("Each person continues the small activity"));
        assert!(prompt.contains("fires, candles, lantern light"));
        assert!(prompt.contains("dust drifts slowly"));
    }

    #[test]
    fn ambient_prompt_refuses_missing_controlled_or_oversized_descriptions() {
        assert_eq!(ambient("  "), Err(Refusal::EmptyDescription));
        assert_eq!(
            ambient("a scene\nwith a newline"),
            Err(Refusal::InvalidDescription)
        );
        assert_eq!(ambient(&"scene ".repeat(300)), Err(Refusal::PromptTooLong));
    }
}
