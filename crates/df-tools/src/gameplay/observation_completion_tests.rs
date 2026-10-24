use super::*;
use crate::gameplay::{
    approach_model::tests::{answer, model as jev_model},
    beat_script_tests::{input, prepared_story},
    pack,
    storyteller::tests::{FakeEndpoint, Reply, teller},
};
use df_model::checkpoint::Checkpoint;
use df_session::inbox::{AdmissionSequence, InboxHandle, Reducer, bounded_inbox};
use serde_json::json;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};

struct TestOwner {
    checkpoint: Checkpoint,
    updates: tokio::sync::watch::Sender<Checkpoint>,
    table: Arc<crate::gameplay::table_log::TableLog>,
    notebook: Arc<crate::gameplay::notebook::NotebookLedger>,
    published: std::collections::BTreeSet<String>,
    calls: Arc<AtomicUsize>,
    entered: Option<oneshot::Sender<()>>,
    release: Option<std::sync::mpsc::Receiver<()>>,
    refuse: bool,
}

impl Reducer<actor::Call> for TestOwner {
    fn reduce(&mut self, _: AdmissionSequence, call: actor::Call) {
        let actor::Call::ObservationFinding {
            expected_basis,
            record,
            detail,
            reply,
            ..
        } = call
        else {
            panic!("only observation completions enter this fake owner");
        };
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(entered) = self.entered.take() {
            let _ = entered.send(());
        }
        if let Some(release) = self.release.take() {
            release.recv_timeout(Duration::from_secs(3)).unwrap();
        }
        let outcome = if self.refuse {
            self.refuse = false;
            Err(Status::unavailable("controlled owner refusal"))
        } else {
            journey::living_scene::commit_observation_with_detail(
                &self.checkpoint,
                expected_basis,
                record.clone(),
                detail,
            )
            .map_err(actor::unavailable)
            .and_then(|checkpoint| {
                let actor = checkpoint
                    .state()
                    .characters
                    .iter()
                    .find(|actor| journey::hex(actor.entity.as_bytes()) == record.actor)
                    .unwrap();
                actor::materialize_observation(
                    &self.table,
                    &self.notebook,
                    &mut self.published,
                    actor.owner,
                    &record,
                )
                .map_err(|_| Status::unavailable("controlled notebook failure"))?;
                self.checkpoint = checkpoint;
                self.updates.send_replace(self.checkpoint.clone());
                Ok(record)
            })
        };
        let _ = reply.send(outcome);
    }
}

struct OwnerThread {
    sender: InboxHandle<actor::Call>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for OwnerThread {
    fn drop(&mut self) {
        self.sender.stop().unwrap();
        self.thread.take().unwrap().join().unwrap();
    }
}

fn service(
    loaded: pack::LoadedPack,
    operation: u8,
    voice: Arc<NpcVoice>,
    entered: Option<oneshot::Sender<()>>,
    release: Option<std::sync::mpsc::Receiver<()>>,
    refuse: bool,
) -> (
    Arc<Service>,
    OwnerThread,
    rpc::SubmitActionRequest,
    rpc::SubmitActionResponse,
    Arc<AtomicUsize>,
    Arc<crate::gameplay::notebook::NotebookLedger>,
) {
    let member = MemberId::from_bytes(&[0x61; 16]).unwrap();
    let prepared = prepared_story();
    let current = journey::stage_with_supplier(
        &prepared,
        &input(&prepared, member, 5, "begin-story", vec![]),
        &mut |_| panic!("opening does not draw"),
    )
    .unwrap();
    let choices = [
        ("attempt-skill", "insight"),
        ("attempt-thing", "person-courier"),
    ]
    .into_iter()
    .map(|(group, option)| (model::label(group).unwrap(), model::label(option).unwrap()))
    .collect();
    let checked = journey::stage_with_supplier(
        &current,
        &input(&current, member, operation, "script-choice", choices),
        &mut |_| Ok(18),
    )
    .unwrap();
    let decision = checked.state().decisions.last().unwrap();
    let accepted = journey::accepted(decision).unwrap();
    let request = rpc::SubmitActionRequest {
        action_kind: rpc::GameplayActionKind::ExamineHarborSeal as i32,
        operation_id: Some(rpc::OperationId {
            value: Some(decision.operation.as_bytes().to_vec()),
        }),
        ..Default::default()
    };
    let response = wire::committed(wire::receipt(
        checked.basis(),
        decision.operation,
        rpc::decision_receipt::Outcome::Accepted(Box::new(accepted)),
        false,
    ));
    let (updates, receiver) = tokio::sync::watch::channel(checked.clone());
    let (sender, inbox) = bounded_inbox();
    let table = crate::gameplay::table_log::TableLog::new();
    let notebook = crate::gameplay::notebook::NotebookLedger::none();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut owner = TestOwner {
        checkpoint: checked,
        updates,
        table: table.clone(),
        notebook: notebook.clone(),
        published: Default::default(),
        calls: calls.clone(),
        entered,
        release,
        refuse,
    };
    let thread = std::thread::spawn(move || {
        pack::with_test_pack(loaded, || {
            inbox.run(&mut owner).unwrap();
        });
    });
    let owner_thread = OwnerThread {
        sender: sender.clone(),
        thread: Some(thread),
    };
    let service = Arc::new(Service {
        look_ahead: Default::default(),
        portraits: crate::gameplay::creator_portraits::Portraits::disabled(),
        display_credential: [0x82; 32],
        layout: crate::gameplay::dev_control::layout::LayoutState::new(),
        actor: sender,
        updates: receiver,
        streams: Arc::new(tokio::sync::Semaphore::new(6)),
        voice: crate::gameplay::voice::Voice::disabled(
            crate::gameplay::startup_configuration::VoiceOff::Qualification,
        ),
        npc_voice: voice,
        table_log: table,
        social: crate::gameplay::social::SocialLedger::none(),
        living: crate::gameplay::living_scenes::LivingScenes::disabled(),
    });
    (service, owner_thread, request, response, calls, notebook)
}

fn loaded() -> pack::LoadedPack {
    pack::load_from_bytes(
        include_bytes!("../../../../content/harbor-journey/pack.json"),
        include_bytes!("fixtures/harbor-journey-tuning-baseline.json"),
    )
    .unwrap()
}

#[test]
fn service_observation_duplicate_and_lost_waiter_share_one_judgement_and_owner_record() {
    let loaded = loaded();
    pack::with_test_pack(loaded.clone(), || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let jev = FakeEndpoint::start(vec![answer("o2")]);
            let line = "Brynn sees the courier's frightened backward glances.";
            let expression = FakeEndpoint::start(vec![Reply::Output(json!({"finding": line}))]);
            let cache = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "../../artifacts/tmp/lanes/HS23/flight-success-{}",
                std::process::id()
            ));
            let voice = Arc::new(
                NpcVoice::new(Arc::new(teller(
                    expression.address,
                    20_000,
                    Duration::from_secs(5),
                    &cache,
                )))
                .with_approach_model(jev_model(&jev)),
            );
            let (entered, ready) = oneshot::channel();
            let (release, gate) = std::sync::mpsc::channel();
            let (service, owner, request, response, calls, notebook) =
                service(loaded, 0x91, voice, Some(entered), Some(gate), false);
            let first = {
                let service = service.clone();
                let request = request.clone();
                let response = response.clone();
                tokio::spawn(async move {
                    service
                        .complete_observation([0x91; 32], &request, &response)
                        .await
                })
            };
            ready.await.unwrap();
            let second = service.complete_observation([0x91; 32], &request, &response);
            tokio::pin!(second);
            assert!(
                std::future::poll_fn(|context| std::task::Poll::Ready(
                    second.as_mut().poll(context).is_pending()
                ))
                .await
            );
            first.abort();
            assert!(first.await.unwrap_err().is_cancelled());
            release.send(()).unwrap();
            let stored = second.await.unwrap().unwrap();
            assert_eq!(stored.text, line);
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert_eq!(jev.connections(), 1);
            assert_eq!(expression.connections(), 1);
            let replay = service
                .complete_observation([0x91; 32], &request, &response)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(replay, stored);
            assert_eq!(jev.connections(), 1);
            assert_eq!(
                service
                    .table_log
                    .lines()
                    .iter()
                    .filter(|entry| entry.text == line)
                    .count(),
                1
            );
            assert_eq!(
                notebook.visible_to(MemberId::from_bytes(&[0x61; 16]).unwrap())[0].text,
                line
            );
            drop(owner);
        });
    });
}

#[test]
fn service_observation_failed_owner_delivery_retries_prepared_record_without_provider_retry() {
    let loaded = loaded();
    pack::with_test_pack(loaded.clone(), || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let jev = FakeEndpoint::start(vec![answer("o2")]);
            let expression = FakeEndpoint::start(vec![Reply::Output(
                json!({"finding": "Brynn notices the courier's fear."}),
            )]);
            let cache = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "../../artifacts/tmp/lanes/HS23/flight-refusal-{}",
                std::process::id()
            ));
            let voice = Arc::new(
                NpcVoice::new(Arc::new(teller(
                    expression.address,
                    20_000,
                    Duration::from_secs(5),
                    &cache,
                )))
                .with_approach_model(jev_model(&jev)),
            );
            let (service, owner, request, response, calls, _) =
                service(loaded, 0x92, voice, None, None, true);
            assert_eq!(
                service
                    .complete_observation([0x92; 32], &request, &response)
                    .await
                    .unwrap_err()
                    .code(),
                tonic::Code::Unavailable
            );
            let stored = service
                .complete_observation([0x92; 32], &request, &response)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(stored.text, "Brynn notices the courier's fear.");
            assert_eq!(jev.connections(), 1);
            assert_eq!(expression.connections(), 1);
            assert_eq!(calls.load(Ordering::SeqCst), 2);
            assert_eq!(service.table_log.lines().last().unwrap().text, stored.text);
            drop(owner);
        });
    });
}
#[test]
fn service_question_observation_carries_the_actual_question_through_owner_commit_and_replay() {
    let loaded = loaded();
    pack::with_test_pack(loaded.clone(), || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let jev = FakeEndpoint::start(vec![answer("o2"), answer("o1")]);
            let text = "Brynn sees the courier glance fearfully back toward the enforcer.";
            let expression = FakeEndpoint::start(vec![Reply::Output(json!({"answer":json!({
                "answer":text, "established_detail":null}).to_string()}))]);
            let cache = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../artifacts/tmp/lanes/HS40/question-owner")
                .join(std::process::id().to_string());
            let voice = Arc::new(
                NpcVoice::new(Arc::new(teller(
                    expression.address,
                    0,
                    Duration::from_secs(5),
                    &cache,
                )))
                .with_approach_model(jev_model(&jev)),
            );
            let (service, owner, request, response, calls, _) =
                service(loaded, 0x96, voice, None, None, false);
            let question = "Who is the courier looking back at?";
            let result = service
                .complete_observation_for_question([0x96; 32], &request, &response, question)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(result.text, text);
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert!(
                expression.bodies()[0]["input"]
                    .as_str()
                    .unwrap()
                    .contains(question)
            );
            let again = service
                .complete_observation_for_question([0x96; 32], &request, &response, question)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(again, result);
            assert_eq!(expression.connections(), 1);
            assert_eq!(jev.connections(), 2);
            assert_eq!(
                service
                    .table_log
                    .lines()
                    .iter()
                    .filter(|line| line.text == text)
                    .count(),
                1
            );
            drop(owner);
        });
    });
}
