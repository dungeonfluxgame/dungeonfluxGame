use super::*;

#[test]
fn sheet_projects_all_eighteen_skills_and_six_saves_from_accepted_abilities() {
    let current = tests::tuning_journey().remove(0);
    let hero = entity(ENTITIES[0]).unwrap();
    let sheet = character_sheet(current.state(), hero).unwrap();
    // SRD 5.2.1 pp. 6, 7, 9, 47: modifiers, saves, skills, Fighter proficiencies.
    for (label, expected) in [
        ("Strength saving throw", "+5"),
        ("Dexterity saving throw", "+1"),
        ("Constitution saving throw", "+4"),
        ("Intelligence saving throw", "+0"),
        ("Wisdom saving throw", "+1"),
        ("Charisma saving throw", "-1"),
        ("Acrobatics", "+1"),
        ("Animal Handling", "+1"),
        ("Arcana", "+0"),
        ("Athletics", "+5"),
        ("Deception", "-1"),
        ("History", "+0"),
        ("Insight", "+1"),
        ("Intimidation", "+1"),
        ("Investigation", "+0"),
        ("Medicine", "+1"),
        ("Nature", "+0"),
        ("Perception", "+3"),
        ("Performance", "-1"),
        ("Persuasion", "-1"),
        ("Religion", "+0"),
        ("Sleight of Hand", "+1"),
        ("Stealth", "+1"),
        ("Survival", "+3"),
    ] {
        let fact = sheet
            .facts
            .iter()
            .find(|fact| fact.label == label)
            .expect(label);
        assert_eq!(fact.value, expected, "{label}");
    }
}

#[test]
fn sheet_skill_proficiency_union_does_not_double_background_and_class_bonus() {
    let current = tests::tuning_journey().remove(0);
    let hero = entity(ENTITIES[0]).unwrap();
    let mut state = current.state().clone();
    let character = state
        .characters
        .iter_mut()
        .find(|character| character.entity == hero)
        .unwrap();
    character
        .choices
        .iter_mut()
        .find(|choice| choice.offer.as_str() == "skill-1")
        .unwrap()
        .selected = model::label("athletics").unwrap();
    // Current creation menus refuse duplicate background selections. The shared native
    // modifier still treats the two provenance paths as a union, never two bonuses.
    assert_eq!(
        super::super::beat_script_runtime::skill_check_modifiers(&state, hero, "athletics")
            .unwrap(),
        (3, 2)
    );
    state.characters = current.state().characters.clone();
    state
        .resources
        .retain(|resource| !(resource.owner == hero && resource.resource.as_str() == "wisdom"));
    assert_eq!(
        character_sheet(&state, hero),
        Err(RepositoryError::InvalidCandidate)
    );
}
