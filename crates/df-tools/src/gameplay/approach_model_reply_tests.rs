use super::*;
use crate::gameplay::approach_model::tests::{answer, model};

#[tokio::test(flavor = "current_thread")]
async fn jev_threats_lower_attitude_before_reply_and_ignore_the_reply_models_bribe_label() {
    for (case, words) in [
        ("indirect", "It would be a shame if this place burned."),
        (
            "coin",
            "Hand over the coin you owe us or I will break your arm.",
        ),
    ] {
        let decision = FakeEndpoint::start(vec![answer("o7")]);
        let reply = FakeEndpoint::start(vec![Reply::Output(json!({
            "reply": "Keep your voice down. Dangerous people are listening.",
            "approach": "bribe"
        }))]);
        let (voice, cache) = voice(&reply, &format!("jev-threat-{case}"));
        let voice = voice.with_approach_model(model(&decision));
        let mut adventure = crate::gameplay::npc_dossier_tests::adventure(1);
        let patron = adventure
            .dossiers
            .iter_mut()
            .find(|dossier| dossier.role == NpcRole::Patron)
            .unwrap();
        patron.starting_attitude = Attitude::Friendly;
        let patron = patron.clone();
        let social = SocialLedger::open(adventure.dossiers, "jev-threat", None).unwrap();
        let mut projected = view("", "Perrin");
        let Some(rpc::view_message::Audience::Player(player)) = projected.audience.as_mut() else {
            unreachable!();
        };
        player.narration = "The party meets Perrin in a tavern.".to_owned();
        player.scene.as_mut().unwrap().cast = vec![rpc::SceneActor {
            id: patron.npc_id.strip_prefix("patron:").unwrap().to_owned(),
            name: patron.name.clone(),
            role: rpc::SceneRole::Patron as i32,
            speaking: true,
            ..Default::default()
        }];
        let context =
            NpcReplyContext::for_cast(&projected, member(1), words, &story(), &social, "Opening")
                .unwrap();
        let log = TableLog::new();

        let exchange = voice.converse_in_character(context, &log).await.unwrap();

        assert_eq!(exchange.approach, Approach::Intimidation, "{case}");
        assert!(exchange.from_model, "{case}: {:?}", exchange.fallback);
        assert_eq!(
            social.standing(member(1), &patron).attitude,
            Attitude::Indifferent
        );
        let bodies = reply.bodies();
        let facts = bodies[0]["input"].as_str().unwrap();
        assert!(facts.contains("\"attitude_toward_hero\":\"indifferent\""));
        assert!(facts.contains("\"approach\":\"intimidation\""));
        assert_eq!(
            social.latest_exchange().unwrap().approach,
            Approach::Intimidation
        );
        assert!(
            log.lines()
                .iter()
                .any(|line| line.text == "cools toward Brannoch")
        );
        assert_eq!(decision.connections(), 1);
        assert_eq!(
            decision.bodies()[0]["state"]["public_scene"],
            "The party meets Perrin in a tavern."
        );
        std::fs::remove_dir_all(cache).unwrap();
    }
}

#[tokio::test(flavor = "current_thread")]
async fn a_reply_models_label_cannot_override_a_keyless_keyword_reading() {
    let endpoint = FakeEndpoint::start(vec![Reply::Output(json!({
        "reply": "Keep your voice down. Dangerous people are listening.",
        "approach": "intimidation"
    }))]);
    let (voice, cache) = voice(&endpoint, "jev-off-reply-label");
    let adventure = crate::gameplay::npc_dossier_tests::adventure(1);
    let patron = adventure
        .dossiers
        .iter()
        .find(|dossier| dossier.role == NpcRole::Patron)
        .unwrap()
        .clone();
    let social = SocialLedger::open(adventure.dossiers, "jev-off", None).unwrap();
    let mut projected = view("", "Perrin");
    let Some(rpc::view_message::Audience::Player(player)) = projected.audience.as_mut() else {
        unreachable!();
    };
    player.scene.as_mut().unwrap().cast = vec![rpc::SceneActor {
        id: patron.npc_id.strip_prefix("patron:").unwrap().to_owned(),
        name: patron.name,
        role: rpc::SceneRole::Patron as i32,
        speaking: true,
        ..Default::default()
    }];
    let context = NpcReplyContext::for_cast(
        &projected,
        member(1),
        "Good morning.",
        &story(),
        &social,
        "Opening",
    )
    .unwrap();

    let exchange = voice
        .converse_in_character(context, &TableLog::new())
        .await
        .unwrap();

    assert_eq!(exchange.approach, Approach::Other);
    assert!(exchange.from_model);
    std::fs::remove_dir_all(cache).unwrap();
}
