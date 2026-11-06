//! The views and dev controls of the owner: `dump`, `viable`, `projection show`, `queue show`,
//! `describe`, `health`, the broker's views, and the `clock`, `dice` and `chaos` controls.
//!
//! Every read is of the committed world, never of a batch in progress (a batch runs inside one
//! owner step, so no read can see one), which is the S1 rule that nothing is shown before its
//! commit. Each call is one audited line. A view of a seat is the only place a seat's choices are
//! computed, and it is computed from the machine's own `accepts`. A control that the table's
//! adapters do not offer (it is not the dev table) answers `NotAvailable` and changes nothing.

use df_table::identity::SeatId;
use df_table::inspect;
use df_table::owner_queue::Lane;
use df_table::pipeline::PipelineTable;
use df_table::ports::{Broker, Clock, PurposeName, Readiness};

use crate::broker::report::{LedgerReport, QueuesReport};
use crate::engine::audit::EngineAudit;
use crate::engine::cli::verb_usages;
use crate::engine::commit::{CommitId, ScriptedCommit};
use crate::engine::labels::sender_label;
use crate::engine::limits::{MAXIMUM_LEDGER_ENTRIES_PER_ANSWER, MAXIMUM_LINES_PER_ANSWER};
use crate::engine::projection_adapter::project_view;
use crate::engine::report::{
    BrokerRequested, ChaosReport, ClockReport, CommitReport, Description, DevControl, DiceReport,
    EngineDump, EngineError, HandOffDump, HealthReport, InProgressReport, LaneDepth,
    MachineDescription, QueueReport, SeatPending, TableDump, ViableReport, ViewCause,
};
use crate::engine::request::{AudienceSpec, ChaosRequest, ClockControl, CommitFault, DiceControl};
use crate::engine::setup::WIRED_PORTS;
use crate::engine::turn::{TurnContext, machine_state, viable_actions};
use crate::engine::verb_set::{not_built, verb_lines};

use super::Owner;

fn not_available(control: DevControl) -> EngineError {
    EngineError::NotAvailable { control }
}

fn table_dump(table: &PipelineTable) -> TableDump {
    let facts = table.facts();
    let snapshot = facts.snapshot();
    TableDump {
        seat_capacity: facts.seat_capacity(),
        seats: facts.seats().iter().map(|seat| seat.get()).collect(),
        active_scene: facts.active_scene().map(|scene| scene.get()),
        active_beat: facts.active_beat().map(ToString::to_string),
        last_hand_off: facts.last_hand_off().map(|record| HandOffDump {
            from: record.from.to_string(),
            to: record.to.to_string(),
            carried: record.carried,
        }),
        scenes: snapshot.scenes().map(|(scene, _)| scene.get()).collect(),
        cast: snapshot.cast_len(),
        world_time_s: snapshot.world_time_s(),
        status: snapshot.status().key().to_owned(),
        digest: format!("{:016x}", table.state_hash()),
    }
}

fn readiness_text(readiness: Readiness) -> String {
    match readiness {
        Readiness::Pending => "pending".to_owned(),
        Readiness::Ready => "ready".to_owned(),
        Readiness::Failed(cause) => format!("failed:{}", cause.key()),
    }
}

impl Owner {
    /// The dump of `table` as commit `commit`: what `dump` prints and a checkpoint holds.
    pub(super) fn dump_of(
        &self,
        table: &PipelineTable,
        revision: u64,
        commit: CommitId,
    ) -> Result<EngineDump, EngineError> {
        let snapshot = table.facts().snapshot();
        let active = snapshot
            .focus_machine()
            .ok_or_else(|| EngineError::UnknownMachine {
                machine: "none".to_owned(),
            })?;
        let (machine, state) = machine_state(&self.turn_context(), table);
        Ok(EngineDump {
            revision,
            commit: commit.get(),
            machine,
            state,
            schema_version: active.slot().schema_version(),
            slot: active.slot().payload().clone(),
            table: table_dump(table),
        })
    }

    pub(super) fn turn_context(&self) -> TurnContext<'_> {
        TurnContext {
            registry: &self.registry,
            catalog: &self.catalog,
            settings: &self.config.settings,
            limits: &self.config.limits,
            adapters: &self.adapters,
            sink: &*self.sink,
            machine_surface: self.config.story,
        }
    }

    pub(super) fn seat_is_at_the_table(&self, seat: u64) -> Result<SeatId, EngineError> {
        let seat_id = SeatId::new(seat);
        match self.queue.seat_pending(seat_id) {
            Some(_) => Ok(seat_id),
            None => Err(EngineError::UnknownSeat { seat }),
        }
    }

    pub(super) fn dump(&self) -> Result<EngineDump, EngineError> {
        let dump = self.dump_of(&self.committed, self.revision(), self.commit_id)?;
        self.audit(EngineAudit::Dump {
            revision: dump.revision,
            commit: dump.commit,
        });
        Ok(dump)
    }

    pub(super) fn viable(&self, seat: u64) -> Result<ViableReport, EngineError> {
        let seat_id = self.seat_is_at_the_table(seat)?;
        let actions: Vec<String> = viable_actions(&self.turn_context(), &self.committed, seat_id)
            .iter()
            .map(|action| action.as_str().to_owned())
            .collect();
        let (machine, state) = machine_state(&self.turn_context(), &self.committed);
        self.audit(EngineAudit::Viable {
            seat,
            count: actions.len(),
        });
        Ok(ViableReport {
            seat,
            machine,
            state,
            actions,
        })
    }

    pub(super) fn projection(&self, spec: AudienceSpec) -> Result<serde_json::Value, EngineError> {
        let audience = spec.audience();
        if let AudienceSpec::Seat { seat } = spec {
            self.seat_is_at_the_table(seat)?;
        }
        let view = project_view(&self.committed, audience).map_err(|error| {
            EngineError::ViewUnavailable {
                cause: ViewCause::Projection,
                detail: error.to_string(),
            }
        })?;
        self.audit(EngineAudit::Projection {
            audience,
            revision: self.revision(),
        });
        Ok(view)
    }

    pub(super) fn queue_show(&self) -> QueueReport {
        let now_ms = self.now_ms();
        let newest = |count: usize| count.saturating_sub(MAXIMUM_LINES_PER_ANSWER);
        let skip_submissions = newest(self.recent_submissions.len());
        let skip_turns = newest(self.recent_turns.len());
        let report = QueueReport {
            serving: if self.held { "held" } else { "running" }.to_owned(),
            now_ms,
            lanes: Lane::ALL
                .iter()
                .map(|lane| LaneDepth {
                    lane: *lane,
                    depth: self.queue.depth(*lane),
                })
                .collect(),
            in_progress: self
                .queue
                .in_progress(now_ms)
                .into_iter()
                .map(|operation| InProgressReport {
                    sender: sender_label(operation.sender),
                    operation: operation.operation_id.to_string(),
                    turn_seq: operation.turn_seq.get(),
                    age_ms: operation.age_ms,
                })
                .collect(),
            seats: (1..=self.config.seats)
                .filter_map(|seat| {
                    self.queue
                        .seat_pending(SeatId::new(seat))
                        .map(|pending| SeatPending { seat, pending })
                })
                .collect(),
            remembered_operations: self.queue.remembered_operations(),
            last_turn_seq: self.queue.last_turn_seq().get(),
            commit: CommitReport {
                policy: self.config.policy.key().to_owned(),
                deadline_ms: self.config.commit_deadline_ms,
                commits: self.stats.commits,
                failures: self.stats.failures,
                last_commit: (self.commit_id.get() > 0).then(|| self.commit_id.get()),
                last_checkpoint_bytes: self.stats.last_bytes,
                backing_off_for_ms: self.backoff_remaining_ms(now_ms),
            },
            submissions: self
                .recent_submissions
                .iter()
                .skip(skip_submissions)
                .cloned()
                .collect(),
            turns: self.recent_turns.iter().skip(skip_turns).cloned().collect(),
        };
        self.audit(EngineAudit::Queue {
            verb: "show",
            served: 0,
        });
        report
    }

    pub(super) fn describe(&self) -> Description {
        let verb_set = verb_lines(&inspect::describe(&self.registry).verbs);
        let machines = self
            .registry
            .report()
            .machines
            .into_iter()
            .map(|status| {
                let spec = self
                    .registry
                    .machine(&status.machine)
                    .map(|machine| machine.spec().clone());
                let names = |ids: Option<Vec<String>>| ids.unwrap_or_default();
                MachineDescription {
                    scope: status.scope.key().to_owned(),
                    enabled: spec.is_some(),
                    active: self.committed.running_machine().as_ref() == Some(&status.machine),
                    states: names(
                        spec.as_ref()
                            .map(|spec| spec.states().iter().map(ToString::to_string).collect()),
                    ),
                    hand_off_targets: names(spec.as_ref().map(|spec| {
                        spec.hand_off_targets()
                            .iter()
                            .map(ToString::to_string)
                            .collect()
                    })),
                    rules_actions: names(spec.as_ref().map(|spec| {
                        spec.rules_actions()
                            .iter()
                            .map(|action| action.as_str().to_owned())
                            .collect()
                    })),
                    actions: self
                        .catalog
                        .actions(&status.machine)
                        .iter()
                        .map(|action| action.as_str().to_owned())
                        .collect(),
                    machine: status.machine.to_string(),
                }
            })
            .collect();
        self.audit(EngineAudit::Describe);
        Description {
            machines,
            lanes: Lane::ALL.to_vec(),
            ports: WIRED_PORTS
                .iter()
                .map(|port| port.key().to_owned())
                .collect(),
            commit_policy: self.config.policy.key().to_owned(),
            verbs: verb_usages(),
            not_built: not_built(&verb_set),
            verb_set,
        }
    }

    pub(super) fn health(&self) -> HealthReport {
        let now_ms = self.now_ms();
        let queues = self.adapters.broker.hub.queues_report();
        self.audit(EngineAudit::Health);
        HealthReport {
            owner: "running".to_owned(),
            serving: if self.held { "held" } else { "running" }.to_owned(),
            waiting_inputs: self.waiting_inputs(),
            oldest_in_progress_ms: self
                .queue
                .in_progress(now_ms)
                .first()
                .map(|operation| operation.age_ms),
            commits: self.stats.commits,
            failures: self.stats.failures,
            backing_off_for_ms: self.backoff_remaining_ms(now_ms),
            broker_epoch: queues.epoch,
            broker_closed: queues.closed,
            now_ms,
            events: self.observer.health(),
        }
    }

    pub(super) fn clock(&self, control: ClockControl) -> Result<ClockReport, EngineError> {
        let clock = self
            .adapters
            .controls
            .clock
            .as_ref()
            .ok_or_else(|| not_available(DevControl::Clock))?;
        let verb = match control {
            ClockControl::Now => "now",
            ClockControl::Advance { ms } => {
                clock.advance_ms(ms);
                "advance"
            }
            ClockControl::Freeze => {
                clock.freeze();
                "freeze"
            }
            ClockControl::Resume => {
                clock.resume();
                "resume"
            }
        };
        let now_ms = clock.now_ms();
        self.audit(EngineAudit::Clock { verb, now_ms });
        Ok(ClockReport {
            now_ms,
            frozen: clock.is_frozen(),
        })
    }

    pub(super) fn dice(&self, control: DiceControl) -> Result<DiceReport, EngineError> {
        let dice = self
            .adapters
            .controls
            .dice
            .as_ref()
            .ok_or_else(|| not_available(DevControl::Dice))?;
        let (verb, values) = match control {
            DiceControl::Seed { seed } => {
                dice.reseed(seed);
                ("seed", 0)
            }
            DiceControl::Force { values } => {
                let count = values.len();
                dice.force(values);
                ("force", count)
            }
        };
        self.audit(EngineAudit::Dice { verb, values });
        Ok(DiceReport {
            forced_waiting: dice.forced_waiting(),
            seed: Some(dice.seed()),
        })
    }

    pub(super) fn chaos(&self, request: ChaosRequest) -> Result<ChaosReport, EngineError> {
        let controls = &self.adapters.controls;
        match request {
            ChaosRequest::Commit { fault, count } => {
                let store = controls
                    .commit
                    .as_ref()
                    .ok_or_else(|| not_available(DevControl::CommitStore))?;
                let scripted = match fault {
                    CommitFault::Unavailable => {
                        ScriptedCommit::Fails(crate::engine::commit::CommitError::Unavailable)
                    }
                    CommitFault::Refused => {
                        ScriptedCommit::Fails(crate::engine::commit::CommitError::Refused)
                    }
                    CommitFault::Deadline => ScriptedCommit::Succeeds {
                        latency_ms: u64::MAX,
                    },
                };
                store.push(scripted, u64::from(count));
                let fault_key = fault.key();
                self.audit(EngineAudit::Chaos {
                    dependency: "commit",
                    fault: fault_key,
                    count,
                });
                Ok(ChaosReport {
                    fault: format!("commit {fault_key}"),
                    in_force: format!("{} scripted writes wait", store.scripted_remaining()),
                })
            }
            ChaosRequest::Interpreter { down } => {
                let interpreter = controls
                    .interpreter
                    .as_ref()
                    .ok_or_else(|| not_available(DevControl::InterpreterSwitch))?;
                interpreter.set_down(down);
                self.audit(EngineAudit::Chaos {
                    dependency: "interpreter",
                    fault: if down { "down" } else { "up" },
                    count: 0,
                });
                Ok(ChaosReport {
                    fault: "interpreter".to_owned(),
                    in_force: if down { "down" } else { "up" }.to_owned(),
                })
            }
        }
    }

    pub(super) fn broker_queues(&self) -> QueuesReport {
        self.audit(EngineAudit::Broker {
            verb: "queues",
            purpose: None,
        });
        self.adapters.broker.hub.queues_report()
    }

    pub(super) fn broker_ledger(&self) -> LedgerReport {
        self.audit(EngineAudit::Broker {
            verb: "ledger",
            purpose: None,
        });
        self.adapters
            .broker
            .hub
            .ledger_report(MAXIMUM_LEDGER_ENTRIES_PER_ANSWER)
    }

    /// A dev request for a resource, through the same port the machines use.
    pub(super) fn broker_request(&mut self, purpose: &str) -> Result<BrokerRequested, EngineError> {
        let name = PurposeName::new(purpose).map_err(|_| EngineError::InvalidName {
            what: purpose.to_owned(),
        })?;
        let hub = std::sync::Arc::clone(&self.adapters.broker.hub);
        if !hub.knows_purpose(&name) {
            return Err(EngineError::UnknownPurpose {
                purpose: purpose.to_owned(),
            });
        }
        let operation = self.mint_engine_operation();
        let correlation = self.correlation_for(operation);
        let handle = hub.request(correlation, &name)?;
        let readiness = hub.readiness(correlation, handle)?;
        self.audit(EngineAudit::Broker {
            verb: "request",
            purpose: Some(name),
        });
        Ok(BrokerRequested {
            handle: handle.get(),
            purpose: purpose.to_owned(),
            readiness: readiness_text(readiness),
        })
    }
}
