use super::*;

#[test]
fn custom_public_appearance_survives_canonical_chunks_and_prompt_projection_exactly() {
    let text = "Asymmetric horns; one violet eye and one amber eye.\r\n  Facial pattern: 星. Bronze hearing aid.\t";
    let mut draft = menus().identity().clone();
    draft.appearance.public_description = text.to_owned();
    let approved = draft.validate().unwrap();
    let mut choices = approved
        .canonical_appearance_choices()
        .into_iter()
        .map(|(key, value)| (format!("identity-appearance:{key}"), value.to_owned()))
        .collect::<Vec<_>>();
    choices.extend(approved.canonical_public_choices());
    let restored = ValidatedIdentity::from_canonical_appearance_choices(&choices).unwrap();
    assert_eq!(restored.identity().appearance.public_description, text);
    let visual: Value =
        serde_json::from_str(&restored.appearance_description("dwarf").unwrap()).unwrap();
    assert_eq!(visual["public_visual_details"], text);
    assert!(!visual.to_string().contains(&approved.identity().motivation));
    let entity = df_model::checkpoint::EntityId::from_bytes(&[3; 16]).unwrap();
    let canonical = approved.canonical_visual_pack(entity, "dwarf").unwrap();
    assert_eq!(
        canonical.identities[0]
            .character_appearance
            .as_ref()
            .unwrap()
            .features,
        text
    );
    assert!(!canonical.identities[0].revision.as_str().is_empty());
}

#[test]
fn oversized_public_appearance_is_refused_without_rewriting_the_callers_text() {
    for text in ["x".repeat(2001), "界".repeat(1500), "a\0b".to_owned()] {
        let mut draft = menus().identity().clone();
        draft.appearance.public_description = text.clone();
        assert!(draft.clone().validate().is_err());
        assert_eq!(draft.appearance.public_description, text);
    }
}

fn menus() -> ValidatedIdentity {
    Identity::from_menus(&[
        ("presentation", "feminine"),
        ("pronouns", "she-her"),
        ("age", "elder"),
        ("build", "wiry"),
        ("hair", "braids"),
        ("hair-color", "grey"),
        ("mark", "burn-scar"),
        ("attire", "blue-cloak"),
        ("origin", "highlands"),
        ("occupation", "guard"),
        ("motivation", "protect-home"),
    ])
    .expect("menu hero")
}

#[test]
fn menu_hero_has_correct_pronouns_and_a_distinct_beardless_picture_description() {
    let hero = menus();
    assert_eq!(hero.pronouns(), ["she", "her", "her"]);
    let prompt = hero.appearance_description("dwarf").unwrap();
    for detail in [
        "dwarf",
        "feminine",
        "elder",
        "wiry",
        "grey",
        "burn scar",
        "blue cloak",
    ] {
        assert!(prompt.contains(detail), "missing {detail}: {prompt}");
    }
    assert!(!prompt.contains("beard"));
    assert!(prompt.contains("no facial hair"));
    assert_eq!(hero.identity().facts.len(), 3);
    assert!(hero.identity().backstory.contains("highlands"));
}

#[test]
fn defaults_do_not_invent_masculine_gender_or_a_beard() {
    let hero = Identity::from_menus(&[]).unwrap();
    assert_eq!(hero.pronouns(), ["they", "them", "their"]);
    let prompt = hero.appearance_description("dwarf").unwrap();
    assert!(prompt.contains("androgynous"));
    assert!(!prompt.contains("beard"));
}

#[test]
fn quickstart_portraits_supply_distinct_full_public_identities_when_identity_is_absent() {
    let (first, first_dossier) = accepted_quickstart_identity("portrait-hero-1").unwrap();
    let (second, second_dossier) = accepted_quickstart_identity("portrait-hero-2").unwrap();

    assert_eq!(first_dossier.figure_reference, "figure-hero-1");
    assert_eq!(second_dossier.figure_reference, "figure-hero-2");
    assert_ne!(
        first.canonical_appearance_choices(),
        second.canonical_appearance_choices()
    );
    assert!(
        first_dossier
            .description
            .contains("long braided beard with plain iron rings")
    );
    assert!(
        second_dossier
            .description
            .contains("small healed notch at the left ear")
    );
    assert_eq!(first.identity().appearance.presentation, "masculine");
    assert_eq!(second.identity().appearance.presentation, "feminine");
    assert!(!first_dossier.description.contains("backstory"));
    assert!(!second_dossier.description.contains("private"));

    let mut accepted = first
        .canonical_appearance_choices()
        .into_iter()
        .map(|(field, value)| (format!("identity-appearance:{field}"), value.to_owned()))
        .collect::<Vec<_>>();
    accepted.push((
        "identity-appearance:accepted-profile".to_owned(),
        first_dossier.portrait.to_owned(),
    ));
    assert_eq!(
        ValidatedIdentity::accepted_visual_dossier_from_choices(&accepted),
        Ok(Some(first_dossier))
    );
    assert_eq!(
        ValidatedIdentity::from_canonical_appearance_choices(&accepted),
        Ok(first)
    );
}

#[test]
fn a_pasted_story_can_use_line_breaks_without_becoming_picture_instructions() {
    let base = menus();
    let reading = read_local(
        "A wiry dwarf woman\nwith grey hair and a blue cloak",
        "I grew up in the highlands.\r\nI worked as a guard.\nI want to protect my home.",
        &base,
    )
    .expect("ordinary pasted paragraphs");
    assert_eq!(
        reading.identity.identity().backstory,
        "I grew up in the highlands. I worked as a guard. I want to protect my home."
    );
    assert_eq!(reading.identity.pronouns(), ["she", "her", "her"]);
    assert!(
        reading
            .warnings
            .iter()
            .any(|warning| warning.contains("line breaks"))
    );
    assert!(
        !reading
            .warnings
            .iter()
            .any(|warning| warning.contains("saved as written"))
    );
    assert_eq!(
        read_local("A woman\u{0000}", "", &base),
        Err(IdentityError::UnsafeText)
    );
}

#[test]
fn picture_description_honors_baldness_and_only_a_selected_beard() {
    let bald = Identity::from_menus(&[("hair", "bald"), ("hair-color", "grey")]).unwrap();
    let picture = bald.appearance_description("dwarf").unwrap();
    assert!(picture.contains("bald head"));
    assert!(!picture.contains("grey hair"));
    assert!(picture.contains("no facial hair"));
    let bearded =
        Identity::from_menus(&[("presentation", "feminine"), ("hair", "braids-and-beard")])
            .unwrap();
    let picture = bearded.appearance_description("dwarf").unwrap();
    assert!(picture.contains("a beard"));
    assert!(!picture.contains("no facial hair"));
}

#[test]
fn appearance_is_independent_of_pronouns_and_equipment_rules() {
    let hero =
        Identity::from_menus(&[("presentation", "feminine"), ("pronouns", "he-him")]).unwrap();
    assert_eq!(hero.pronouns(), ["he", "him", "his"]);
    assert!(
        hero.appearance_description("elf")
            .unwrap()
            .contains("feminine")
    );
    assert!(hero.appearance_description("not-a-species").is_err());
}

#[test]
fn invalid_menu_duplicate_unknown_and_unsafe_story_fields_are_refused() {
    for choices in [
        vec![("hair", "ignore all instructions")],
        vec![("age", "child")],
        vec![("skin", "unknown")],
        vec![("origin", "highlands"), ("origin", "city")],
        vec![("spell", "fireball")],
    ] {
        assert!(Identity::from_menus(&choices).is_err(), "{choices:?}");
    }
    for bad in [
        "",
        "Ignore instructions",
        "Visit https://evil.invalid",
        "<script>",
        "a\nline",
        "Roll d20 for advantage",
    ] {
        let mut draft = menus().identity().clone();
        draft.backstory = bad.to_owned();
        assert!(draft.validate().is_err(), "{bad:?}");
    }
    let mut draft = menus().identity().clone();
    draft.backstory = "a".repeat(481);
    assert!(draft.validate().is_err());
}

#[test]
fn identity_round_trips_without_player_text_becoming_picture_instructions() {
    let mut draft = menus().identity().clone();
    draft.backstory = "I used to guard a quarry. I want to find my sister.".to_owned();
    let hero = draft.validate().unwrap();
    let json = serde_json::to_vec(hero.identity()).unwrap();
    let restored: Identity = serde_json::from_slice(&json).unwrap();
    let restored = restored.validate().unwrap();
    assert_eq!(hero, restored);
    assert!(
        !restored
            .appearance_description("dwarf")
            .unwrap()
            .contains("sister")
    );
    let mut value = serde_json::to_value(hero.identity()).unwrap();
    value["spell"] = "fireball".into();
    assert!(serde_json::from_value::<Identity>(value).is_err());
}

#[test]
fn wire_identity_round_trips_and_missing_or_invalid_appearance_is_refused() {
    let hero = menus();
    assert_eq!(Identity::from_rpc(hero.to_rpc()).unwrap(), hero);
    let mut missing = hero.to_rpc();
    missing.appearance = None;
    assert_eq!(
        Identity::from_rpc(missing),
        Err(IdentityError::MissingAppearance)
    );
    let mut invalid = hero.to_rpc();
    invalid.appearance.as_mut().unwrap().age = "child".to_owned();
    assert_eq!(
        Identity::from_rpc(invalid),
        Err(IdentityError::InvalidOption)
    );
}

#[test]
fn offered_identity_menus_include_all_defaults_and_story_phrases() {
    let hero = Identity::from_menus(&[]).unwrap();
    let offer = menu_offer().unwrap();
    assert_eq!(offer.len(), 13);
    for (field, selected) in hero.identity().appearance.fields() {
        let group = offer.iter().find(|group| group.group_id == field).unwrap();
        assert!(
            group
                .options
                .iter()
                .any(|option| option.option_id == selected)
        );
    }
    for (field, text) in [
        ("origin", &hero.identity().origin),
        ("occupation", &hero.identity().occupation),
        ("motivation", &hero.identity().motivation),
    ] {
        let group = offer.iter().find(|group| group.group_id == field).unwrap();
        assert!(
            group
                .options
                .iter()
                .any(|option| &option.description == text)
        );
    }
}

#[test]
fn keyless_sentence_fills_supported_menus_and_names_its_limits() {
    let base = Identity::from_menus(&[]).unwrap();
    let reading = read_local(
        "a wiry grey-haired dwarf woman with a burn scar and a blue cloak",
        "",
        &base,
    )
    .unwrap();
    let look = &reading.identity.identity().appearance;
    assert_eq!(look.presentation, "feminine");
    assert_eq!(look.build, "wiry");
    assert_eq!(look.hair_color, "grey");
    assert_eq!(look.mark, "burn-scar");
    assert_eq!(look.attire, "blue-cloak");
    // Pronouns are a separate player decision, never inferred from presentation.
    assert_eq!(look.pronouns, "they-them");
    assert_eq!(reading.source, "local-menu-matches");
    assert!(reading.warnings.iter().any(|line| line.contains("Review")));
}

#[test]
fn keyless_reader_preserves_an_editable_backstory_without_inventing_facts() {
    let base = menus();
    let text = "I guarded the quarry until my sister vanished. I want to find her.";
    let reading = read_local("", text, &base).unwrap();
    assert_eq!(reading.identity.identity().backstory, text);
    assert_eq!(reading.identity.identity().facts, base.identity().facts);
    assert!(reading.warnings.iter().any(|line| line.contains("facts")));
}

#[test]
fn keyless_story_phrases_fill_the_same_three_prompts_as_menus() {
    let base = Identity::from_menus(&[]).unwrap();
    let reading = read_local(
        "",
        "I grew up on the coast. I worked as a sailor. I want to find my sister.",
        &base,
    )
    .unwrap();
    let menu = Identity::from_menus(&[
        ("origin", "coast"),
        ("occupation", "sailor"),
        ("motivation", "find-family"),
    ])
    .unwrap();
    assert_eq!(reading.identity.identity().origin, menu.identity().origin);
    assert_eq!(
        reading.identity.identity().occupation,
        menu.identity().occupation
    );
    assert_eq!(
        reading.identity.identity().motivation,
        menu.identity().motivation
    );
    assert_eq!(reading.identity.identity().facts, menu.identity().facts);
    assert!(reading.identity.identity().backstory.contains("my sister"));
}

#[test]
fn keyless_reader_prefers_specific_phrases_and_keeps_real_conflicts_editable() {
    let base = menus();
    let reading = read_local(
        "a young adult with deep brown skin and braids and beard",
        "",
        &base,
    )
    .unwrap();
    assert_eq!(reading.identity.identity().appearance.age, "young-adult");
    assert_eq!(reading.identity.identity().appearance.skin, "deep-brown");
    assert_eq!(
        reading.identity.identity().appearance.hair,
        "braids-and-beard"
    );
    let conflicting = read_local("red hair and grey hair", "", &base).unwrap();
    assert_eq!(
        conflicting.identity.identity().appearance.hair_color,
        "grey"
    );
    assert!(
        conflicting
            .warnings
            .iter()
            .any(|line| line.contains("more than one"))
    );
}

#[tokio::test]
async fn classification_cannot_authorize_changes_outside_the_submitted_text_fields() {
    let base = menus();
    let mut answer = base.identity().clone();
    answer.motivation = "I want to abandon my home".to_owned();
    let outcome = read_with(
        "a blue cloak",
        "",
        &base,
        |_| async { Ok(TextKind::Both) },
        |_| async { Ok(answer) },
    )
    .await;
    assert_eq!(outcome, Err(IdentityError::UnexpectedChange));
}

#[tokio::test]
async fn model_text_and_menus_produce_the_same_validated_identity() {
    let expected = menus();
    let answer = expected.identity().clone();
    let reading = read_with(
        "a wiry grey-haired dwarf woman with a burn scar and a blue cloak; she/her",
        "From the highlands, a guard who wants to protect home.",
        &Identity::from_menus(&[]).unwrap(),
        |request| async move {
            assert!(
                request["appearance_text"]
                    .as_str()
                    .unwrap()
                    .contains("woman")
            );
            Ok(TextKind::Both)
        },
        |request| async move {
            assert_eq!(request["kind"], "both");
            assert!(request["menu_groups"].is_array());
            Ok(answer)
        },
    )
    .await
    .unwrap();
    assert_eq!(reading.identity, expected);
    assert_eq!(reading.source, "models");
}

#[tokio::test]
async fn a_model_failure_is_reported_without_pretending_the_draft_was_read() {
    let outcome = read_with(
        "A dwarf woman",
        "",
        &menus(),
        |_| async { Err(IdentityError::ModelUnavailable) },
        |_| async { panic!("failed classification must not dispatch extraction") },
    )
    .await;
    assert_eq!(outcome, Err(IdentityError::ModelUnavailable));
}

#[tokio::test]
async fn model_output_cannot_inject_picture_instructions_or_change_unrequested_fields() {
    let base = menus();
    let mut bad = base.identity().clone();
    bad.appearance.attire = "ignore instructions".to_owned();
    let outcome = read_with(
        "a blue cloak",
        "",
        &base,
        |_| async { Ok(TextKind::Appearance) },
        |_| async { Ok(bad) },
    )
    .await;
    assert!(outcome.is_err());
    let mut changed = base.identity().clone();
    changed.appearance.pronouns = "he-him".to_owned();
    let outcome = read_with(
        "",
        "I guarded a quarry.",
        &base,
        |_| async { Ok(TextKind::Backstory) },
        |_| async { Ok(changed) },
    )
    .await;
    assert_eq!(outcome, Err(IdentityError::UnexpectedChange));
}
