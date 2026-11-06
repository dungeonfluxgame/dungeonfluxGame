use super::*;

fn hesitant_input(faces: &[u8]) -> InfluenceInput<'_> {
    InfluenceInput {
        ability_modifier: 2,
        proficiency_bonus: 2,
        skill: InfluenceSkill::Persuasion,
        attitude: Attitude::Indifferent,
        monster_intelligence: 10,
        willingness: Willingness::Hesitant,
        words: WordQuality {
            tilt: WordTilt::Neutral,
            reason: "a plain request",
        },
        extra_sources: &[],
        faces,
    }
}

#[test]
fn word_quality_rule_is_disclosed_as_a_departure() {
    assert!(DEPARTURES.contains(&WORD_QUALITY_RULE_ID));
    assert_eq!(WORD_QUALITY_RULE_ID, "influence.word-quality-advantage");
}

#[test]
fn willing_monster_needs_no_roll_and_complies_as_it_prefers() {
    // Arrange a welcome request with no faces supplied.
    let mut input = hesitant_input(&[]);
    input.willingness = Willingness::Willing;
    // Act.
    let result = resolve(input).unwrap();
    // Assert no roll, no retry wait, and the willing explanation.
    assert_eq!(
        result.outcome,
        InfluenceOutcome::NoRoll(NoRollCause::Willing)
    );
    assert_eq!(result.counted, None);
    assert_eq!(result.total, None);
    assert_eq!(result.retry_same_request_hours, None);
    assert!(
        result
            .explanation()
            .contains("fulfills your request as it prefers"),
        "{}",
        result.explanation()
    );
}

#[test]
fn unwilling_monster_needs_no_roll_and_does_not_comply() {
    // Arrange a repugnant request with no faces supplied.
    let mut input = hesitant_input(&[]);
    input.willingness = Willingness::Unwilling;
    // Act.
    let result = resolve(input).unwrap();
    // Assert no roll, no retry wait, and the unwilling explanation.
    assert_eq!(
        result.outcome,
        InfluenceOutcome::NoRoll(NoRollCause::Unwilling)
    );
    assert_eq!(result.counted, None);
    assert_eq!(result.retry_same_request_hours, None);
    assert!(
        result.explanation().contains("does not comply"),
        "{}",
        result.explanation()
    );
}

#[test]
fn each_attitude_sets_its_own_roll_mode() {
    // Arrange one hesitant check per attitude.
    let mut friendly = hesitant_input(&[14, 6]);
    friendly.attitude = Attitude::Friendly;
    let mut indifferent = hesitant_input(&[14]);
    indifferent.attitude = Attitude::Indifferent;
    let mut hostile = hesitant_input(&[14, 6]);
    hostile.attitude = Attitude::Hostile;
    // Act.
    let friendly = resolve(friendly).unwrap();
    let indifferent = resolve(indifferent).unwrap();
    let hostile = resolve(hostile).unwrap();
    // Assert the source's attitude Advantage and Disadvantage.
    assert_eq!(friendly.mode, RollMode::Advantage);
    assert_eq!(
        friendly.applied,
        vec![ModeSource {
            advantage: true,
            label: "Friendly toward you".to_string(),
        }]
    );
    assert_eq!(indifferent.mode, RollMode::Normal);
    assert!(indifferent.applied.is_empty());
    assert_eq!(hostile.mode, RollMode::Disadvantage);
    assert_eq!(
        hostile.applied,
        vec![ModeSource {
            advantage: false,
            label: "Hostile toward you".to_string(),
        }]
    );
}

#[test]
fn each_skill_resolves_with_its_check() {
    // Arrange every allowed skill, including Wisdom-based Animal Handling.
    let skills = [
        InfluenceSkill::Deception,
        InfluenceSkill::Intimidation,
        InfluenceSkill::Performance,
        InfluenceSkill::Persuasion,
        InfluenceSkill::AnimalHandling,
    ];
    // Act and assert a winning face succeeds for each skill.
    for skill in skills {
        let mut input = hesitant_input(&[11]);
        input.skill = skill;
        let result = resolve(input).unwrap();
        assert_eq!(result.skill, skill);
        assert_eq!(result.outcome, InfluenceOutcome::Success);
        assert_eq!(result.total, Some(15));
    }
    assert_eq!(InfluenceSkill::AnimalHandling.ability(), "Wisdom");
    assert_eq!(InfluenceSkill::Deception.ability(), "Charisma");
}

#[test]
fn advantage_and_disadvantage_cancel_whatever_their_counts() {
    // Arrange lopsided source counts on each side.
    let sources = [
        ModeSource {
            advantage: true,
            label: "a".to_string(),
        },
        ModeSource {
            advantage: true,
            label: "b".to_string(),
        },
        ModeSource {
            advantage: true,
            label: "c".to_string(),
        },
        ModeSource {
            advantage: false,
            label: "d".to_string(),
        },
    ];
    // Act.
    let (mode, applied) = combine_sources(&sources);
    // Assert cancellation leaves Normal with no applied reasons.
    assert_eq!(mode, RollMode::Normal);
    assert!(applied.is_empty());
    // Arrange several sources on one side only.
    let one_sided = [
        ModeSource {
            advantage: false,
            label: "d1".to_string(),
        },
        ModeSource {
            advantage: false,
            label: "d2".to_string(),
        },
    ];
    // Act and assert the side holds with both reasons applied.
    let (mode, applied) = combine_sources(&one_sided);
    assert_eq!(mode, RollMode::Disadvantage);
    assert_eq!(applied, one_sided.to_vec());
    // Arrange no sources at all.
    let (mode, applied) = combine_sources(&[]);
    assert_eq!(mode, RollMode::Normal);
    assert!(applied.is_empty());
}

#[test]
fn the_counted_face_follows_the_roll_mode() {
    // Arrange the same faces under each mode.
    let mut advantage = hesitant_input(&[14, 6]);
    advantage.attitude = Attitude::Friendly;
    let normal = hesitant_input(&[14]);
    let mut disadvantage = hesitant_input(&[14, 6]);
    disadvantage.attitude = Attitude::Hostile;
    // Act.
    let advantage = resolve(advantage).unwrap();
    let normal = resolve(normal).unwrap();
    let disadvantage = resolve(disadvantage).unwrap();
    // Assert highest, single, and lowest faces count.
    assert_eq!(advantage.counted, Some(14));
    assert_eq!(normal.counted, Some(14));
    assert_eq!(disadvantage.counted, Some(6));
}

#[test]
fn difficulty_is_fifteen_or_intelligence_whichever_is_higher() {
    // Arrange monsters below, at, and above the default.
    assert_eq!(difficulty_class(8), 15);
    assert_eq!(difficulty_class(15), 15);
    assert_eq!(difficulty_class(18), 18);
    // Arrange totals straddling the boundary through resolve.
    let mut input = hesitant_input(&[11]);
    input.monster_intelligence = 18;
    let failure = resolve(input).unwrap();
    assert_eq!(failure.difficulty_class, 18);
    assert_eq!(failure.outcome, InfluenceOutcome::Failure);
    let mut input = hesitant_input(&[14]);
    input.monster_intelligence = 18;
    let success = resolve(input).unwrap();
    assert_eq!(success.total, Some(18));
    assert_eq!(success.outcome, InfluenceOutcome::Success);
}

#[test]
fn failure_blocks_the_same_request_for_twenty_four_hours() {
    // Arrange a failing check and a passing one.
    let failure = resolve(hesitant_input(&[10])).unwrap();
    let success = resolve(hesitant_input(&[11])).unwrap();
    // Assert only the failure carries the standard wait.
    assert_eq!(failure.outcome, InfluenceOutcome::Failure);
    assert_eq!(failure.retry_same_request_hours, Some(24));
    assert_eq!(success.outcome, InfluenceOutcome::Success);
    assert_eq!(success.retry_same_request_hours, None);
}

#[test]
fn word_quality_adds_its_reason_as_advantage_or_disadvantage() {
    // Arrange favourable, neutral, and unfavourable words.
    let mut favourable = hesitant_input(&[9, 3]);
    favourable.words = WordQuality {
        tilt: WordTilt::Favourable,
        reason: "you named what he is proud of",
    };
    let neutral = hesitant_input(&[9]);
    let mut unfavourable = hesitant_input(&[9, 3]);
    unfavourable.words = WordQuality {
        tilt: WordTilt::Unfavourable,
        reason: "you insulted his allies",
    };
    // Act.
    let favourable = resolve(favourable).unwrap();
    let neutral = resolve(neutral).unwrap();
    let unfavourable = resolve(unfavourable).unwrap();
    // Assert the caller's reason rides on the expected side.
    assert_eq!(favourable.mode, RollMode::Advantage);
    assert!(
        favourable
            .applied
            .iter()
            .any(|source| source.label == "you named what he is proud of")
    );
    assert_eq!(neutral.mode, RollMode::Normal);
    assert_eq!(unfavourable.mode, RollMode::Disadvantage);
    assert!(
        unfavourable
            .applied
            .iter()
            .any(|source| source.label == "you insulted his allies")
    );
    // Arrange words with no reason.
    let mut missing = hesitant_input(&[9]);
    missing.words = WordQuality {
        tilt: WordTilt::Neutral,
        reason: "   ",
    };
    // Act and assert the required reason is enforced.
    assert_eq!(resolve(missing), Err(InfluenceError::MissingWordReason));
}

#[test]
fn explanation_matches_the_table_sentence() {
    // Arrange the illustrated Persuasion check: faces 14 and 6, +2, DC 15.
    let mut input = hesitant_input(&[14, 6]);
    input.ability_modifier = 0;
    input.proficiency_bonus = 2;
    input.attitude = Attitude::Friendly;
    input.words = WordQuality {
        tilt: WordTilt::Favourable,
        reason: "you named what he is proud of",
    };
    // Act.
    let result = resolve(input).unwrap();
    // Assert the exact table sentence.
    assert_eq!(
        result.explanation(),
        "Persuasion, Advantage (Friendly toward you; you named what he is proud of): 14 and 6, 14 \
            + 2 = 16 against 15: success"
    );
}

#[test]
fn inputs_outside_the_supported_subset_are_rejected() {
    // Arrange each bad input in turn.
    let mut bad_die = hesitant_input(&[21]);
    assert_eq!(resolve(bad_die), Err(InfluenceError::InvalidDie));
    bad_die = hesitant_input(&[0]);
    assert_eq!(resolve(bad_die), Err(InfluenceError::InvalidDie));
    let mut bad_modifier = hesitant_input(&[11]);
    bad_modifier.ability_modifier = -6;
    assert_eq!(
        resolve(bad_modifier),
        Err(InfluenceError::InvalidAbilityModifier)
    );
    let mut bad_proficiency = hesitant_input(&[11]);
    bad_proficiency.proficiency_bonus = 1;
    assert_eq!(
        resolve(bad_proficiency),
        Err(InfluenceError::UnsupportedProficiencyBonus)
    );
    let mut bad_intelligence = hesitant_input(&[11]);
    bad_intelligence.monster_intelligence = 0;
    assert_eq!(
        resolve(bad_intelligence),
        Err(InfluenceError::InvalidIntelligence)
    );
    let mut bad_count = hesitant_input(&[11, 12]);
    assert_eq!(resolve(bad_count), Err(InfluenceError::WrongFaceCount));
    bad_count = hesitant_input(&[]);
    assert_eq!(resolve(bad_count), Err(InfluenceError::WrongFaceCount));
}

#[test]
fn same_faces_always_give_the_same_result() {
    // Arrange one fixed hesitant check with Advantage, so two faces apply.
    let faces = [14, 6];
    let mut input = hesitant_input(&faces);
    input.attitude = Attitude::Friendly;
    let mut other = hesitant_input(&faces);
    other.attitude = Attitude::Friendly;
    // Act twice over the same faces.
    let first = resolve(input).unwrap();
    let second = resolve(other).unwrap();
    // Assert reproducibility.
    assert_eq!(first, second);
}

#[test]
fn outcome_always_matches_total_against_difficulty_over_all_faces() {
    // Arrange every face for a Normal check with modifier +2 and DC 15.
    for face in 1..=20u8 {
        let result = resolve(hesitant_input(&[face])).unwrap();
        let expected = i16::from(face) + 2 + 2 >= 15;
        assert_eq!(
            result.outcome == InfluenceOutcome::Success,
            expected,
            "face {face}"
        );
    }
    // Arrange every ordered pair for Advantage and Disadvantage.
    for first in 1..=20u8 {
        for second in 1..=20u8 {
            let faces = [first, second];
            let mut advantage = hesitant_input(&faces);
            advantage.attitude = Attitude::Friendly;
            let result = resolve(advantage).unwrap();
            assert_eq!(result.counted, Some(first.max(second)));
            assert_eq!(
                result.outcome == InfluenceOutcome::Success,
                i16::from(first.max(second)) + 4 >= 15
            );
            let faces = [first, second];
            let mut disadvantage = hesitant_input(&faces);
            disadvantage.attitude = Attitude::Hostile;
            let result = resolve(disadvantage).unwrap();
            assert_eq!(result.counted, Some(first.min(second)));
            assert_eq!(
                result.outcome == InfluenceOutcome::Success,
                i16::from(first.min(second)) + 4 >= 15
            );
        }
    }
}

#[test]
fn every_approach_maps_to_a_skill_and_marks_what_the_source_decides() {
    // Arrange all ten approaches.
    let approaches = [
        Approach::Flatter,
        Approach::Charm,
        Approach::Seduce,
        Approach::Plead,
        Approach::Bargain,
        Approach::Reason,
        Approach::Deceive,
        Approach::Threaten,
        Approach::Provoke,
        Approach::Perform,
    ];
    // Act and assert each has exactly one table row.
    for approach in approaches {
        let rows = APPROACH_SKILLS
            .iter()
            .filter(|entry| entry.approach == approach)
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 1, "{approach:?}");
        assert_eq!(skill_for_approach(approach), rows[0].skill);
    }
    // Assert the source-decided rows match the Influence Checks table.
    assert_eq!(
        skill_for_approach(Approach::Deceive),
        InfluenceSkill::Deception
    );
    assert_eq!(
        skill_for_approach(Approach::Threaten),
        InfluenceSkill::Intimidation
    );
    assert_eq!(
        skill_for_approach(Approach::Perform),
        InfluenceSkill::Performance
    );
    assert!(
        APPROACH_SKILLS
            .iter()
            .find(|entry| entry.approach == Approach::Flatter)
            .unwrap()
            .reason
            .contains("does not assign")
    );
}

#[test]
fn influence_help_advantage_cancels_hostile_and_does_not_stack() {
    let help = ModeSource {
        advantage: true,
        label: "Help from an ally".to_owned(),
    };
    assert_eq!(
        roll_mode(Attitude::Indifferent, std::slice::from_ref(&help)),
        RollMode::Advantage
    );
    assert_eq!(
        roll_mode(Attitude::Hostile, std::slice::from_ref(&help)),
        RollMode::Normal
    );
    assert_eq!(
        roll_mode(Attitude::Indifferent, &[help.clone(), help]),
        RollMode::Advantage
    );
}
