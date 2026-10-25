use super::*;

#[test]
fn hs53_measure_six_hero_reply_projection() {
    let loaded = super::super::pack::load_from_bytes(
        include_bytes!("../../../../content/free-roam/pack.json"),
        include_bytes!("../../../../content/free-roam/tuning.json"),
    )
    .unwrap();
    super::super::pack::with_test_pack(loaded, || {
        let started = Instant::now();
        let current = six_hero_opening();
        println!("HS53 six-hero setup_ms={}", started.elapsed().as_millis());
        let member = current.state().characters[0].owner;
        let started = Instant::now();
        assert!(audit_combat_view(&current, member).is_none());
        println!("HS53 audit_combat_ms={}", started.elapsed().as_millis());
        let started = Instant::now();
        assert!(
            audit_journey(&current, member)
                .and_then(|journey| journey.own_character)
                .is_some()
        );
        println!("HS53 audit_hero_ms={}", started.elapsed().as_millis());
    });
}

/// Six joined heroes past the opening. Creation and the opening beat name the authored script's
/// exact choice when the loaded script is not the canonical graph, as `beat_script_tests` does.
fn six_hero_opening() -> Checkpoint {
    use crate::gameplay::beat_script::with_script;
    use crate::gameplay::beat_script_runtime::CHOICE_GROUP;
    use crate::gameplay::beat_script_tests::{build, input};
    use crate::gameplay::{journey, model};

    const NAMES: [&str; 6] = ["Brynn", "Vale", "Orla", "Pip", "Vesper", "Thorin"];
    let mut current = journey::initial().unwrap();
    for index in 0..NAMES.len() {
        current = journey::stage_join(
            &current,
            &input(
                &current,
                journey::bootstrap_member().unwrap(),
                index as u8 + 1,
                "join-room",
                vec![],
            ),
        )
        .unwrap();
    }
    let members: Vec<_> = journey::participants(&current)
        .iter()
        .map(|link| link.member)
        .collect();
    let canonical = with_script(|script| script.is_canonical());
    for (index, member) in members.iter().enumerate() {
        let mut choices = build(NAMES[index]);
        if !canonical {
            choices.push((
                model::label(CHOICE_GROUP).unwrap(),
                model::label("create-character").unwrap(),
            ));
        }
        current = journey::stage_with_supplier(
            &current,
            &input(
                &current,
                *member,
                index as u8 + 20,
                "create-character",
                choices,
            ),
            &mut |_| panic!("creation draws no dice"),
        )
        .unwrap();
    }
    let begin = if canonical {
        vec![]
    } else {
        vec![(
            model::label(CHOICE_GROUP).unwrap(),
            model::label("begin-story").unwrap(),
        )]
    };
    journey::stage_with_supplier(
        &current,
        &input(&current, members[0], 40, "begin-story", begin),
        &mut |_| panic!("opening draws no dice"),
    )
    .unwrap()
}
