use super::*;
use crate::gameplay::{
    approach_model::{LineIntent, LineReading, PhysicalActionKind},
    beat_script_variety_tests as native,
    journey::checkpoint_fight_tests::{fight, with_created_place},
    table_log::TableLog,
};
use df_model::checkpoint::Checkpoint;
use df_session::inbox::{AdmissionSequence, Reducer, bounded_inbox};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{Semaphore, watch};

struct RefusingOwner {
    checkpoint: Checkpoint,
    revision: rpc::SessionRevision,
    submissions: Arc<AtomicUsize>,
}

impl Reducer<actor::Call> for RefusingOwner {
    fn reduce(&mut self, _: AdmissionSequence, call: actor::Call) {
        match call {
            actor::Call::SpeechMember { reply, .. } => {
                reply.send(Ok(native::first())).unwrap();
            }
            actor::Call::FightAftermath {
                expected_basis,
                preparation: Some(preparation),
                reply,
                ..
            } => {
                assert_eq!(expected_basis, self.checkpoint.basis());
                let saved = journey::living_scene::prepared_fight(
                    self.checkpoint.state(),
                    &preparation.fight_id,
                )
                .unwrap()
                .unwrap();
                assert_eq!(
                    serde_json::to_value(saved).unwrap(),
                    serde_json::to_value(preparation).unwrap(),
                );
                reply.send(Ok(self.checkpoint.clone())).unwrap();
            }
            actor::Call::Submit { request, reply, .. } => {
                assert_eq!(
                    request.action_kind,
                    rpc::GameplayActionKind::SceneAssault as i32
                );
                self.submissions.fetch_add(1, Ordering::SeqCst);
                reply
                    .send(Ok(rpc::SubmitActionResponse {
                        outcome: Some(rpc::submit_action_response::Outcome::CommittedDecision(
                            Box::new(rpc::DecisionReceipt {
                                revision: Some(self.revision.clone()),
                                outcome: Some(rpc::decision_receipt::Outcome::Rejected(
                                    rpc::Rejection {
                                        code: rpc::RejectionCode::InvalidSelection as i32,
                                    },
                                )),
                                ..Default::default()
                            }),
                        )),
                    }))
                    .unwrap();
            }
            _ => panic!("unexpected refusal owner call"),
        }
    }
}

fn refuse(current: Checkpoint, target: &str, expected_reason: &str, submissions: usize) {
    let (_, updates) = watch::channel(current.clone());
    let (sender, inbox) = bounded_inbox();
    let table = TableLog::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut owner = RefusingOwner {
        checkpoint: current.clone(),
        revision: wire::revision(current.basis().revision),
        submissions: calls.clone(),
    };
    let thread = std::thread::spawn(move || inbox.run(&mut owner));
    let service = Service {
        portraits: crate::gameplay::creator_portraits::Portraits::disabled(),
        display_credential: [0x82; 32],
        layout: crate::gameplay::dev_control::layout::LayoutState::new(),
        actor: sender.clone(),
        updates,
        streams: Arc::new(Semaphore::new(2)),
        voice: crate::gameplay::voice::Voice::disabled(
            crate::gameplay::startup_configuration::VoiceOff::Qualification,
        ),
        npc_voice: Arc::new(NpcVoice::silent()),
        table_log: table.clone(),
        social: crate::gameplay::social::SocialLedger::none(),
        living: crate::gameplay::living_scenes::LivingScenes::disabled(),
    };
    let text = format!("I draw my greataxe and attack {target}!");
    let request = rpc::RawDialogueRequest {
        session_id: Some(rpc::SessionId {
            value: Some(current.basis().session.as_bytes().to_vec()),
        }),
        run_id: Some(rpc::RunId {
            value: Some(current.basis().run.as_bytes().to_vec()),
        }),
        observed_revision: Some(wire::revision(current.basis().revision)),
        input_id: Some(rpc::OperationId {
            value: Some(vec![90; 16]),
        }),
        text: text.clone(),
        ..Default::default()
    };
    let reading = LineReading {
        intent: LineIntent::PhysicalAction,
        skill: None,
        thing: target.into(),
        physical_kind: Some(PhysicalActionKind::Attack),
        source: "fixture",
        elapsed_ms: 0,
    };
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
        .block_on(service.physical_action_call([1; 32], &request, &reading, None));
    sender.stop().unwrap();
    thread.join().unwrap().unwrap();
    let DialogueReply::Raw(response, None) = result.unwrap() else {
        panic!("resolved Action reply");
    };
    assert_eq!(
        response.disposition,
        rpc::DialogueDisposition::Action as i32
    );
    assert_eq!(response.input_id, request.input_id);
    assert_eq!(response.revision, request.observed_revision);
    assert!(
        response.npc_reply.contains(expected_reason),
        "{}",
        response.npc_reply
    );
    assert!(!response.confirmation_required);
    assert_eq!(service.updates.borrow().basis(), current.basis());
    assert_eq!(calls.load(Ordering::SeqCst), submissions);
    let rows = table.lines();
    assert_eq!(
        rows.len(),
        2,
        "one hero attempt followed by one narrator refusal"
    );
    assert_eq!(rows[0].role, rpc::TableRole::Hero as i32);
    assert_eq!(rows[0].text, text);
    assert_eq!(rows[1].role, rpc::TableRole::Narrator as i32);
    assert_eq!(rows[1].text, response.npc_reply);
}

#[test]
fn c2_attack_unknown_target_is_refused_once_at_the_same_revision() {
    with_created_place(|_, street, _| {
        refuse(street, "Perrin", "no target named Perrin", 0);
    });
}

#[test]
fn c2_attack_already_down_target_is_refused_once_at_the_same_revision() {
    with_created_place(|_, street, _| {
        let initialized = fight(&street);
        let mut state = street.state().clone();
        state.entities = initialized.state().entities.clone();
        state.continuity.npcs = initialized.state().continuity.npcs.clone();
        state.resources = initialized.state().resources.clone();
        let target = journey::scene_fight::person_entity("market-street-wenna").unwrap();
        for resource in &mut state.resources {
            if resource.owner == target {
                match resource.resource.as_str() {
                    "hit-points" | "unconscious" => resource.value = 1,
                    _ => {}
                }
            }
        }
        let down = model::checkpoint(street.basis(), state).unwrap();
        refuse(down, "Wenna", "Wenna is already down", 0);
    });
}

#[test]
fn c2_attack_rejected_scene_submission_is_refused_once_at_the_same_revision() {
    with_created_place(|_, street, _| {
        // This refusal fixture starts after the separate, durable equipment preparation.
        let scene = crate::gameplay::beat_script_runtime::scene(street.state()).unwrap();
        let possessions = scene
            .cast
            .iter()
            .filter_map(|person| person.stat_block.as_ref().map(|profile| (person, profile)))
            .map(|(person, profile)| {
                (
                    person.id.clone(),
                    crate::gameplay::social::fight_witness::stat_block_possessions(profile)
                        .unwrap(),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let preparation = serde_json::from_value(serde_json::json!({
            "fight_id": journey::hex(&[90; 16]),
            "place": scene.setting.as_ref().unwrap().place_id,
            "scene": scene.key,
            "possessions": possessions,
        }))
        .unwrap();
        let street = journey::living_scene::commit_fight_preparation(
            &street,
            OperationId::from_bytes(&[89; 16]).unwrap(),
            preparation,
        )
        .unwrap();
        refuse(street, "Wenna", "attack was refused", 1);
    });
}
