//! Observe only published durable checkpoints; filesystem work belongs to one bounded worker.
use super::super::{journey, model};
use super::{
    campaign::{Approach, ChapterOutcome, ChapterResult},
    campaign_startup::CampaignRun,
    campaign_store::CampaignStore,
};
use df_model::checkpoint::{AudienceScope, Checkpoint, FactValue};
use df_protocol::common as rpc;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{SyncSender, sync_channel},
};
/// None means the unchanged chapter engine has not reached the campaign terminal policy.
pub(crate) fn terminal_outcome(
    current: &Checkpoint,
) -> Result<Option<ChapterOutcome>, df_session::submission::RepositoryError> {
    if journey::phase(current)? != rpc::JourneyPhase::Complete {
        return Ok(None);
    }
    let state = current.state();
    if let Some(outcome) = super::super::beat_script::with_script(|script| {
        if script.uses_encounter() {
            return None;
        }
        Some((|| {
            let beat = script.current(state)?;
            if !script.terminal.contains(&beat.id) {
                return Ok(None);
            }
            // Preserve the actual member-private disclosure even if another member
            // observed the room before the conversation. It owns the campaign lead.
            let (effect, member) = super::super::beat_script_runtime::private_action(current)?
                .or(super::super::beat_script_runtime::first_action(current)?)
                .ok_or(df_session::submission::RepositoryError::InvalidCandidate)?;
            let accepted = state
                .decisions
                .iter()
                .filter(|decision| decision.source_policy.as_str() == journey::THREAD_POLICY)
                .map(journey::accepted)
                .collect::<Result<Vec<_>, _>>()?;
            let approach = match effect {
                super::super::beat_script::BeatEffect::AskCourier => Approach::Asked,
                super::super::beat_script::BeatEffect::EscortCourier => Approach::Escorted,
                super::super::beat_script::BeatEffect::ThirdApproach => Approach::Third,
                super::super::beat_script::BeatEffect::Observe => Approach::Investigated,
                _ => Approach::Continued,
            };
            Ok(Some(ChapterOutcome {
                result: ChapterResult::Completed,
                approach,
                member: journey::hex(member.as_bytes()),
                rested: false,
                destination_reached: accepted.iter().any(|action| action.scene.is_some()),
                antagonist_spared: false,
                campaign_time: state.logical_time.ticks,
            }))
        })())
    }) {
        return outcome;
    }
    let victory = journey::combat_victory(state)?;
    let rested = state
        .narrative
        .completed_beats
        .contains(&model::content("short-rest-complete")?);
    // The chapter's ending steps follow the inn (after a victory) and the defeat scene.
    let ended = state
        .narrative
        .active_beats
        .iter()
        .any(|beat| matches!(beat.entry.as_str(), "epilogue" | "the-end"));
    let destination =
        ended || state.narrative.active_beats.as_slice() == [model::content("harbor-inn")?];
    if (victory && !destination) || (!victory && !rested && !ended) {
        return Ok(None);
    }
    let private = state
        .facts
        .iter()
        .find_map(|fact| match (&fact.value, &fact.audience) {
            (FactValue::ContentEvent { definition, .. }, AudienceScope::Members(members))
                if definition.entry.as_str() == "private-courier-note" && members.len() == 1 =>
            {
                members.first().copied()
            }
            _ => None,
        });
    let scripted = super::super::beat_script_runtime::first_action(current)?;
    let (approach, member) =
        if let Some((super::super::beat_script::BeatEffect::ThirdApproach, member)) = scripted {
            (Approach::Third, member)
        } else if let Some(member) = private {
            (Approach::Asked, member)
        } else {
            let actor = state
                .facts
                .iter()
                .find_map(|fact| match &fact.value {
                    FactValue::ContentEvent {
                        definition,
                        subjects,
                    } if definition.entry.as_str() == "courier-escort-contact"
                        && subjects.len() == 1 =>
                    {
                        subjects.first().copied()
                    }
                    _ => None,
                })
                .ok_or(df_session::submission::RepositoryError::InvalidCandidate)?;
            let member = state
                .members
                .iter()
                .find(|link| link.character == Some(actor))
                .map(|link| link.member)
                .ok_or(df_session::submission::RepositoryError::InvalidCandidate)?;
            (Approach::Escorted, member)
        };
    let spared=victory && state.facts.iter().any(|fact|matches!(&fact.value,FactValue::ResourceChanged{entity,resource,before:0,after:1,..}if entity.as_bytes()==&[0x65;16] && resource.as_str()=="unconscious"));
    Ok(Some(ChapterOutcome {
        result: if victory {
            ChapterResult::Victory
        } else {
            ChapterResult::Defeat
        },
        approach,
        member: member
            .as_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
        rested,
        destination_reached: destination,
        antagonist_spared: spared,
        campaign_time: state.logical_time.ticks,
    }))
}
struct CompletionMessage {
    outcome: ChapterOutcome,
    context: df_observe::OperationContext,
}
#[derive(Clone)]
pub(crate) struct CompletionObserver {
    sender: SyncSender<CompletionMessage>,
    queued: Arc<AtomicBool>,
    configuration_suffix: String,
}
impl CompletionObserver {
    pub(crate) fn observe(&self, current: &Checkpoint) {
        if self.queued.load(Ordering::Acquire) {
            return;
        }
        if !current
            .pins()
            .build
            .revision(df_types::BuildRevision::Configuration)
            .as_str()
            .ends_with(&self.configuration_suffix)
        {
            if !self.queued.swap(true, Ordering::AcqRel) {
                eprintln!(
                    "CAMPAIGN completion refused: checkpoint campaign pin mismatch; game continues"
                );
            }
            return;
        }
        match terminal_outcome(current) {
            Ok(Some(outcome)) => {
                // Durable checkpoints retain correlation IDs, not the original RPC trace.
                // Start a diagnostic root here and carry its owned SDK context off-thread.
                let context = df_observe::OperationContext {
                    trace_parent: String::new(),
                    build: crate::BUILD_ID.into(),
                };
                let mut span = df_observe::begin(&context, "gameplay.campaign.dispatch");
                let basis = current.basis();
                let hex =
                    |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
                let mut attributes = vec![
                    ("session.id", hex(basis.session.as_bytes())),
                    ("run.id", hex(basis.run.as_bytes())),
                    ("revision.epoch", basis.revision.epoch().get().to_string()),
                    ("revision.sequence", basis.revision.sequence().to_string()),
                ];
                if let Some(decision) = current.state().decisions.last() {
                    attributes.push(("operation.id", hex(decision.operation.as_bytes())));
                }
                span.event("gameplay.campaign.dispatch", &attributes);
                let message = CompletionMessage {
                    outcome,
                    context: span.capture_context(),
                };
                if self.sender.try_send(message).is_ok() {
                    self.queued.store(true, Ordering::Release);
                    span.finish_unmeasured("queued");
                } else {
                    span.finish_unmeasured("deferred");
                }
            }
            Ok(None) => {}
            Err(_) => {
                if !self.queued.swap(true, Ordering::AcqRel) {
                    eprintln!(
                        "CAMPAIGN completion refused: durable terminal facts are inconsistent; game continues"
                    );
                }
            }
        }
    }
}
pub(crate) struct CompletionWorker {
    thread: Option<std::thread::JoinHandle<()>>,
    cancel: Arc<AtomicBool>,
}
impl Drop for CompletionWorker {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            eprintln!("CAMPAIGN completion worker failed; game continues");
        }
    }
}
pub(crate) fn worker(
    run: CampaignRun,
) -> Result<(CompletionObserver, CompletionWorker), std::io::Error> {
    let configuration_suffix = format!(
        ":c-{}",
        run.frame_digest
            .get(..16)
            .ok_or_else(|| std::io::Error::other("campaign frame digest"))?
    );
    let (sender, receiver) = sync_channel::<CompletionMessage>(1);
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let thread = std::thread::Builder::new()
        .name("campaign-completion".into())
        .spawn(move || {
            loop {
                let message = match receiver.recv_timeout(std::time::Duration::from_millis(50)) {
                    Ok(outcome) => outcome,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                        if worker_cancel.load(Ordering::Acquire) =>
                    {
                        break;
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                };
                let mut span =
                    df_observe::begin_dispatched(&message.context, "gameplay.campaign.chapter");
                let result = (|| {
                    let store = CampaignStore::new(&run.root, &run.id)?;
                    let stored =
                        store
                            .load()?
                            .ok_or(super::campaign_store::StoreError::Corrupt(
                                "campaign absent",
                            ))?;
                    let mut record = stored.record;
                    if record.complete(run.number, message.outcome)? {
                        store.save(&record, Some(&stored.digest))?;
                    }
                    span.event(
                        "gameplay.campaign.chapter",
                        &[
                            ("campaign.id", run.id.clone()),
                            ("campaign.chapter", run.number.to_string()),
                            ("campaign.frame_sha256", record.frame_digest.clone()),
                            ("campaign.record_sha256", record.digest()?),
                            ("campaign.clock", record.clock_position.to_string()),
                        ],
                    );
                    Ok::<_, super::campaign_store::StoreError>(())
                })();
                match result {
                    Ok(()) => span.finish_unmeasured("recorded_or_idempotent_replay"),
                    Err(error) => {
                        eprintln!(
                            "CAMPAIGN id={} chapter={} completion: {}; game continues",
                            run.id, run.number, error
                        );
                        span.finish_unmeasured("durable_completion_pending_resume");
                    }
                }
            }
        })?;
    Ok((
        CompletionObserver {
            sender,
            queued: Arc::new(AtomicBool::new(false)),
            configuration_suffix,
        },
        CompletionWorker {
            thread: Some(thread),
            cancel,
        },
    ))
}
