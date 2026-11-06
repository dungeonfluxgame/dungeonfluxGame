use df_table::identity::SeatId;

use super::error::AdapterError;
use super::hero_profile::{
    Ability, HeroProfile, MAXIMUM_ABILITY_SCORE, MINIMUM_ABILITY_SCORE, SeatRoster, Skill,
    SkillBonus,
};

fn hero(scores: [u8; 6], skills: &[Skill]) -> HeroProfile {
    HeroProfile::new(scores, 2, skills.iter().copied()).expect("the hero is valid")
}

#[test]
fn every_skill_uses_the_ability_the_srd_skills_table_gives_it() {
    let expected = [
        (Skill::Acrobatics, Ability::Dexterity),
        (Skill::AnimalHandling, Ability::Wisdom),
        (Skill::Arcana, Ability::Intelligence),
        (Skill::Athletics, Ability::Strength),
        (Skill::Deception, Ability::Charisma),
        (Skill::History, Ability::Intelligence),
        (Skill::Insight, Ability::Wisdom),
        (Skill::Intimidation, Ability::Charisma),
        (Skill::Investigation, Ability::Intelligence),
        (Skill::Medicine, Ability::Wisdom),
        (Skill::Nature, Ability::Intelligence),
        (Skill::Perception, Ability::Wisdom),
        (Skill::Performance, Ability::Charisma),
        (Skill::Persuasion, Ability::Charisma),
        (Skill::Religion, Ability::Intelligence),
        (Skill::SleightOfHand, Ability::Dexterity),
        (Skill::Stealth, Ability::Dexterity),
        (Skill::Survival, Ability::Wisdom),
    ];
    assert_eq!(expected.len(), Skill::ALL.len());
    for (skill, ability) in expected {
        assert_eq!(skill.ability(), ability, "{}", skill.key());
    }
}

#[test]
fn a_skill_key_round_trips_through_its_parse_and_its_serde_name() {
    for skill in Skill::ALL {
        assert_eq!(Skill::from_key(skill.key()), Some(skill));
        let written = serde_json::to_string(&skill).expect("a skill serializes");
        assert_eq!(written, format!("\"{}\"", skill.key()));
    }
    assert_eq!(Skill::from_key("Perception"), None);
    assert_eq!(Skill::from_key("sleight-of-hand"), None);
}

#[test]
fn ability_scores_become_the_modifiers_of_the_srd_table() {
    let scores_and_modifiers = [
        (1, -5),
        (8, -1),
        (9, -1),
        (10, 0),
        (11, 0),
        (15, 2),
        (30, 10),
    ];
    for (score, modifier) in scores_and_modifiers {
        let profile = hero([score; 6], &[]);
        let bonus = profile.skill_bonus(Skill::Athletics);
        assert_eq!(bonus.ability_modifier, modifier, "score {score}");
    }
}

#[test]
fn a_score_outside_one_to_thirty_is_refused_and_names_the_ability() {
    for score in [0, MAXIMUM_ABILITY_SCORE + 1] {
        let error = HeroProfile::new([10, 10, 10, 10, score, 10], 2, []).expect_err("refused");
        assert_eq!(
            error,
            AdapterError::AbilityScoreOutOfRange {
                ability: Ability::Wisdom,
                score
            }
        );
    }
    assert!(HeroProfile::new([MINIMUM_ABILITY_SCORE; 6], 2, []).is_ok());
}

#[test]
fn a_proficiency_bonus_outside_what_the_rules_accept_is_refused() {
    for bonus in [0, 1, 7] {
        assert_eq!(
            HeroProfile::new([10; 6], bonus, []).expect_err("refused"),
            AdapterError::ProficiencyBonusOutOfRange { bonus }
        );
    }
    assert!(HeroProfile::new([10; 6], 6, []).is_ok());
}

#[test]
fn the_proficiency_bonus_applies_only_to_a_proficient_skill() {
    let profile = hero([10, 14, 10, 10, 12, 10], &[Skill::Stealth]);

    assert_eq!(
        profile.skill_bonus(Skill::Stealth),
        SkillBonus {
            ability_modifier: 2,
            proficiency_bonus: 2
        }
    );
    assert_eq!(
        profile.skill_bonus(Skill::Acrobatics),
        SkillBonus {
            ability_modifier: 2,
            proficiency_bonus: 0
        }
    );
    assert_eq!(
        profile.skill_bonus(Skill::Perception),
        SkillBonus {
            ability_modifier: 1,
            proficiency_bonus: 0
        }
    );
}

#[test]
fn a_seat_holds_one_hero_and_a_second_is_refused() {
    let roster = SeatRoster::default()
        .with_hero(SeatId::new(1), hero([10; 6], &[]))
        .expect("the first hero seats");

    assert!(roster.hero(SeatId::new(1)).is_some());
    assert!(roster.hero(SeatId::new(2)).is_none());
    assert_eq!(
        roster
            .with_hero(SeatId::new(1), hero([12; 6], &[]))
            .expect_err("refused"),
        AdapterError::SeatAlreadyHasHero {
            seat: SeatId::new(1)
        }
    );
}
