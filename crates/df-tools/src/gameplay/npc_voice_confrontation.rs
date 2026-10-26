//! A committed confrontation reuses the ordinary NPC publication and voice path.
use crate::gameplay::{
    actor,
    journey::living_scene::readiness::Confrontation,
    npc_reply::{Exchange, NpcReplyContext, NpcVoice},
    table_log::TableLog,
};

pub(in crate::gameplay) async fn reply(
    voice: &NpcVoice,
    mut context: NpcReplyContext,
    log: &TableLog,
    confrontation: &Confrontation,
) -> Option<Exchange> {
    if confrontation.npc_id.is_empty()
        || context
            .actor_id
            .strip_prefix("npc:")
            .unwrap_or(&context.actor_id)
            != confrontation.npc_id
    {
        actor::record_audit_line(
            "gameplay.npc_confrontation",
            "CONFRONTATION reply skipped reason=actor_changed".into(),
        );
        return None;
    }
    // The confrontation's outcome is committed: the reply voices it and rolls nothing new.
    context.confrontation_committed = true;
    // Dossier reselection carries `now` into the final listener-safe expression facts.
    context.facts["now"]["committed_confrontation"] = serde_json::json!({
        "npc_id": confrontation.npc_id,
        "observed_action": confrontation.reason,
        "response": "Answer the confrontation in this NPC's own voice: fear, defiance or a warning as their character warrants. The recorded outcome already happened; this reply changes no item, check or fight outcome.",
    });
    actor::record_audit_line(
        "gameplay.npc_confrontation",
        format!(
            "CONFRONTATION dispatch npc={} public_facts_bytes={}",
            confrontation.npc_id,
            context.facts.to_string().len()
        ),
    );
    let exchange = voice.converse_report(context, log).await;
    actor::record_audit_diagnostic(
        "gameplay.npc_confrontation",
        format!(
            "CONFRONTATION reply npc={} outcome={}",
            confrontation.npc_id,
            exchange
                .as_ref()
                .map_or("unavailable", |exchange| exchange.outcome.label())
        ),
        serde_json::Map::from_iter([
            ("npc_id".into(), serde_json::json!(confrontation.npc_id)),
            ("reason".into(), serde_json::json!(confrontation.reason)),
            (
                "reply_produced".into(),
                serde_json::json!(exchange.as_ref().is_some_and(|exchange| {
                    exchange.outcome == crate::gameplay::npc_reply::ExchangeOutcome::Spoken
                        && !exchange.text.is_empty()
                })),
            ),
        ]),
        &[],
    );
    exchange
}

#[cfg(test)]
pub(in crate::gameplay) fn confrontation_judge() -> crate::gameplay::storyteller::tests::FakeEndpoint
{
    use crate::gameplay::storyteller::tests::{FakeEndpoint, Reply};
    use std::sync::Arc;
    let answer = Arc::new(|body: &serde_json::Value| {
        let criteria = body["questions"]["q1"]["criteria"].as_object().unwrap();
        let choice = ["recorded", "approve", "other", "none", "small_talk"]
            .into_iter()
            .find_map(|label| {
                criteria
                    .iter()
                    .find(|(_, text)| {
                        text.as_str()
                            .is_some_and(|text| text.starts_with(&format!("{label}:")))
                    })
                    .map(|(key, _)| key.clone())
            })
            .or_else(|| criteria.keys().next().cloned())
            .expect("the provider supplies at least one closed choice");
        crate::gameplay::actor::record_audit_line(
            "gameplay.test.confrontation_judge",
            format!(
                "CONFRONTATION judge request_bytes={} choice={} fact_decision={}",
                body.to_string().len(),
                choice,
                criteria.values().any(|text| text
                    .as_str()
                    .is_some_and(|text| text.starts_with("recorded:")))
            ),
        );
        (
            200,
            serde_json::json!({"answers":{"q1":{"type":"choice","choice":choice,"confidence":0.99}}}),
        )
    });
    FakeEndpoint::start(
        (0..24)
            .map(|_| Reply::ComputeResponse(answer.clone()))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gameplay::{
        beat_script_tests::{input, prepared_story},
        journey,
        npc_reply::StoryFacts,
        pack,
        social::SocialLedger,
        storyteller::tests::{FakeEndpoint, Reply, cache_directory, teller},
        wire,
    };
    use df_persistence::local_demo_scope::LocalDemoRole;
    use df_types::MemberId;
    use std::{sync::Arc, time::Duration};

    #[tokio::test(flavor = "current_thread")]
    async fn hs41_actor_confrontation_publishes_an_npc_speech_line_through_existing_voice_path() {
        let member = MemberId::from_bytes(&[0x61; 16]).unwrap();
        let current = prepared_story();
        let current = journey::stage_with_supplier(
            &current,
            &input(&current, member, 40, "begin-story", vec![]),
            &mut |_| panic!("opening is certain"),
        )
        .unwrap();
        let mut view = wire::journey_view(&current, LocalDemoRole::Player, member).unwrap();
        let content: pack::PackFile = serde_json::from_slice(include_bytes!(
            "../../../../content/harbor-journey/pack.json"
        ))
        .unwrap();
        let authored = content
            .scenes
            .iter()
            .find(|scene| scene.key == pack::SceneKey::Opening)
            .unwrap();
        let Some(df_protocol::common::view_message::Audience::Player(player)) =
            view.audience.as_mut()
        else {
            panic!("player projection");
        };
        player.scene = Some(df_protocol::common::GameplayScene {
            title: authored.title.clone(),
            scene_asset: authored.asset.clone(),
            cast: wire::scene_cast(authored),
            ..Default::default()
        });
        let facts = StoryFacts::from_pack(&content);
        let context = NpcReplyContext::for_target(
            &view,
            member,
            "I draw my sword.",
            &facts,
            &SocialLedger::none(),
            "Opening",
            Some("courier"),
        )
        .unwrap();
        assert_eq!(context.actor_id, "courier");
        let confrontation = Confrontation {
            npc_id: context
                .actor_id
                .strip_prefix("npc:")
                .unwrap_or(&context.actor_id)
                .into(),
            reason: "Brynn draws her greatsword.".into(),
        };
        let endpoint = FakeEndpoint::start(vec![Reply::Output(serde_json::json!({
            "reply": "Keep that blade away from me."
        }))]);
        let cache = cache_directory("hs41-confrontation");
        let judge = confrontation_judge();
        let voice = NpcVoice::new(Arc::new(teller(
            endpoint.address,
            1_000,
            Duration::from_secs(5),
            &cache,
        )))
        .with_approach_model(crate::gameplay::approach_model::tests::model(&judge));
        let log = TableLog::new();
        let exchange = reply(&voice, context, &log, &confrontation).await.unwrap();
        assert_eq!(exchange.text, "Keep that blade away from me.");
        let spoken = log.lines();
        assert!(spoken.iter().any(
            |line| line.role == df_protocol::common::TableRole::Npc as i32
                && line.kind == df_protocol::common::TableKind::Speech as i32
                && line.actor_id == "courier"
                && line.text == "Keep that blade away from me."
        ));
        assert!(
            endpoint
                .bodies()
                .iter()
                .any(|body| body.to_string().contains("committed_confrontation"))
        );
    }
}
