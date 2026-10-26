#![cfg(not(target_arch = "wasm32"))]

mod support;

use df_commerce::dispute::DisputeOutcome;
use df_commerce::finance_ids::{EntryId, EventId, LotId, RefundLotId};
use df_commerce::funding::LotState;
use df_commerce::ledger::CurrencyTag;
use df_commerce::refund::{RefundPaidRequest, plan_refund_observed, plan_refund_paid};
use df_finance::fakes::{FakeRefundMode, FakeRefundProcessor};
use df_finance::ports::{RefundCall, RefundProcessor};
use df_finance::refunds::RefundPayOutcome;
use df_finance::wire::parse_refund_lot_snapshot;
use support::world::{World, operation, payer, purchase};
use tokio::sync::Notify;

async fn requested(world: &World) -> (support::world::Service, RefundLotId, LotId) {
    let mut service = world.service().await;
    service
        .apply_purchase(purchase(payer(1), "refund-state", 5_000_000))
        .await
        .expect("purchase");
    service
        .request_refund(payer(1), operation(96), 2_000_000, CurrencyTag::USD)
        .await
        .expect("refund");
    let row = world
        .admin
        .query_one("SELECT refund_lot_id, lot_id FROM finance.refund_lot", &[])
        .await
        .expect("lot");
    (
        service,
        RefundLotId::from_slice(&row.get::<_, Vec<u8>>(0)).expect("refund id"),
        LotId::from_slice(&row.get::<_, Vec<u8>>(1)).expect("lot id"),
    )
}

async fn state(world: &World, refund_lot: RefundLotId) -> String {
    world
        .admin
        .query_one(
            "SELECT state FROM finance.refund_lot WHERE refund_lot_id = $1",
            &[&refund_lot.as_bytes().to_vec()],
        )
        .await
        .expect("state")
        .get(0)
}

struct PausedProcessor {
    entered: Notify,
    released: Notify,
    result: RefundCall,
}

impl PausedProcessor {
    fn new(result: RefundCall) -> Self {
        Self {
            entered: Notify::new(),
            released: Notify::new(),
            result,
        }
    }
}

impl RefundProcessor for PausedProcessor {
    async fn refund(&self, _key: &str, _amount_micro_dollars: u64) -> RefundCall {
        self.entered.notify_one();
        self.released.notified().await;
        self.result.clone()
    }
}

#[test]
fn terminal_and_manual_refunds_never_call_the_processor() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let (mut service, refund_lot, _) = requested(&world).await;
        let processor = FakeRefundProcessor::new(FakeRefundMode::Succeed);
        for state in [
            "failed_review",
            "returned_to_credit",
            "cancelled_dispute_lost",
            "manual_pending",
        ] {
            world
                .seed(&format!("UPDATE finance.refund_lot SET state = '{state}'"))
                .await;
            assert!(
                service
                    .pay_refund_lot(refund_lot, &processor)
                    .await
                    .is_err(),
                "{state}"
            );
            assert_eq!(processor.attempts(), 0, "{state}");
        }
        world
            .seed("UPDATE finance.refund_lot SET state = 'pending', payout_method = 'manual'")
            .await;
        assert!(
            service
                .pay_refund_lot(refund_lot, &processor)
                .await
                .is_err()
        );
        assert_eq!(processor.attempts(), 0);
        world.finish().await;
    });
}

#[test]
fn returned_credit_cannot_also_be_paid_by_a_stale_outbox_item() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let (mut service, refund_lot, _) = requested(&world).await;
        let failed = FakeRefundProcessor::new(FakeRefundMode::KnownFailure);
        assert_eq!(
            service
                .pay_refund_lot(refund_lot, &failed)
                .await
                .expect("failure"),
            RefundPayOutcome::KnownFailure
        );
        support::attack::run_event(
            &world,
            Some((
                "refund_lot",
                serde_json::json!({"refund_lot": refund_lot.hex()}),
            )),
            |document| {
                let snapshot = parse_refund_lot_snapshot(document)
                    .expect("snapshot")
                    .expect("lot");
                let mut plan = df_commerce::refund::plan_refund_returned(
                    &snapshot,
                    EventId::from_array([90; 16]),
                    EntryId::from_array([91; 16]),
                    df_finance::ports::Clock::now_ms(&df_finance::fakes::SystemClock),
                )
                .expect("return plan")
                .expect("new return");
                plan.entries[0].actor = df_commerce::ledger::Actor::Operator {
                    actor_id: [1; 16],
                    case_ref: "synthetic-refund-return".into(),
                    approver_id: None,
                };
                df_finance::wire::event_json(&plan)
            },
        )
        .await
        .expect("operator-authorized return");
        let processor = FakeRefundProcessor::new(FakeRefundMode::Succeed);
        assert!(
            service
                .pay_refund_lot(refund_lot, &processor)
                .await
                .is_err()
        );
        assert_eq!(processor.attempts(), 0);
        assert_eq!(
            world.natural(2110, Some(payer(1)), None, None).await,
            5_000_000
        );
        world.finish().await;
    });
}

#[test]
fn frozen_and_charged_back_lots_never_start_processor_refunds() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let (mut service, refund_lot, _) = requested(&world).await;
        let processor = FakeRefundProcessor::new(FakeRefundMode::Succeed);
        for state in ["frozen", "charged_back"] {
            world
                .seed(&format!("UPDATE finance.funding_lot SET state = '{state}'"))
                .await;
            assert_eq!(
                service
                    .pay_refund_lot(refund_lot, &processor)
                    .await
                    .expect("blocked"),
                RefundPayOutcome::BlockedByDispute
            );
            assert_eq!(processor.attempts(), 0);
        }
        world.finish().await;
    });
}

async fn snapshot(
    world: &World,
    refund_lot: RefundLotId,
) -> df_commerce::refund::RefundLotSnapshot {
    let client = support::attack::service_client(world).await;
    let row = client
        .query_one(
            "SELECT finance.lock_flow('refund_lot', $1::text::jsonb)::text",
            &[&serde_json::json!({"refund_lot": refund_lot.hex()}).to_string()],
        )
        .await
        .expect("snapshot");
    parse_refund_lot_snapshot(
        &serde_json::from_str::<serde_json::Value>(&row.get::<_, String>(0)).expect("json"),
    )
    .expect("parse")
    .expect("present")
}

#[test]
fn observation_planner_refuses_terminal_and_dispute_blocked_states() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let (_, refund_lot, _) = requested(&world).await;
        let mut snapshot = snapshot(&world, refund_lot).await;
        for state in [
            "paid",
            "returned_to_credit",
            "cancelled_dispute_lost",
            "blocked_by_dispute",
            "manual_pending",
            "failed_review",
        ] {
            snapshot.state = state.into();
            for unknown in [true, false] {
                assert!(
                    plan_refund_observed(&snapshot, EventId::from_array([80; 16]), unknown, 1)
                        .is_err(),
                    "{state} {unknown}"
                );
            }
        }
        world.finish().await;
    });
}

#[test]
fn paid_planner_refuses_frozen_and_charged_back_funding_lots() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let (_, refund_lot, _) = requested(&world).await;
        let mut snapshot = snapshot(&world, refund_lot).await;
        let request = RefundPaidRequest {
            event_id: EventId::from_array([80; 16]),
            entry_id: EntryId::from_array([81; 16]),
            processor_refund_ref: "fake-paid".into(),
            now_ms: 1,
        };
        for lot_state in [LotState::Frozen, LotState::ChargedBack] {
            snapshot.lot_state = lot_state;
            assert!(plan_refund_paid(&snapshot, &request).is_err());
        }
        world.finish().await;
    });
}

#[test]
fn unknown_refund_is_blocked_and_cancelled_by_a_full_dispute_loss() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let (mut service, refund_lot, lot) = requested(&world).await;
        service
            .pay_refund_lot(
                refund_lot,
                &FakeRefundProcessor::new(FakeRefundMode::TimeoutOnce),
            )
            .await
            .expect("unknown");
        assert_eq!(state(&world, refund_lot).await, "unknown");
        service
            .open_dispute(lot, "fake", "unknown-dispute", 5_000_000, 0)
            .await
            .expect("open");
        assert_eq!(state(&world, refund_lot).await, "blocked_by_dispute");
        service
            .close_dispute(
                lot,
                "fake",
                "unknown-dispute",
                DisputeOutcome::Lost {
                    disputed_micro_dollars: 5_000_000,
                },
            )
            .await
            .expect("close");
        assert_eq!(state(&world, refund_lot).await, "cancelled_dispute_lost");
        world.finish().await;
    });
}

#[test]
fn partial_dispute_loss_releases_unconsumed_refunds_to_payment() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let (mut service, refund_lot, lot) = requested(&world).await;
        service
            .open_dispute(lot, "fake", "partial-dispute", 1_000_000, 0)
            .await
            .expect("open");
        service
            .close_dispute(
                lot,
                "fake",
                "partial-dispute",
                DisputeOutcome::Lost {
                    disputed_micro_dollars: 1_000_000,
                },
            )
            .await
            .expect("close");
        assert_eq!(state(&world, refund_lot).await, "pending");
        service
            .pay_refund_lot(
                refund_lot,
                &FakeRefundProcessor::new(FakeRefundMode::Succeed),
            )
            .await
            .expect("pay");
        assert_eq!(state(&world, refund_lot).await, "paid");
        world.finish().await;
    });
}

#[test]
fn dispute_identity_cannot_close_another_payers_lot() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let (mut service, _, lot) = requested(&world).await;
        service
            .apply_purchase(purchase(payer(2), "other-payment", 5_000_000))
            .await
            .expect("other purchase");
        let other = world.lots_of(payer(2)).await[0];
        service
            .open_dispute(lot, "fake", "identity-dispute", 5_000_000, 0)
            .await
            .expect("open");
        assert!(
            service
                .close_dispute(
                    other,
                    "fake",
                    "identity-dispute",
                    DisputeOutcome::Lost {
                        disputed_micro_dollars: 5_000_000
                    }
                )
                .await
                .is_err()
        );
        let dispute_state: String = world
            .admin
            .query_one("SELECT status FROM finance.dispute", &[])
            .await
            .expect("dispute")
            .get(0);
        assert_eq!(dispute_state, "open");
        assert_eq!(
            world.natural(2110, Some(payer(2)), Some(other), None).await,
            5_000_000
        );
        world.finish().await;
    });
}

#[test]
fn dispute_processor_must_match_the_funding_lot() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let (mut service, _, lot) = requested(&world).await;
        assert!(
            service
                .open_dispute(lot, "other", "wrong-processor", 5_000_000, 0)
                .await
                .is_err()
        );
        service
            .open_dispute(lot, "fake", "right-processor", 5_000_000, 0)
            .await
            .expect("open");
        assert!(
            service
                .close_dispute(lot, "other", "right-processor", DisputeOutcome::Won)
                .await
                .is_err()
        );
        world.finish().await;
    });
}

async fn raced_paid_result(is_loss: bool) {
    let Some(world) = World::open().await else {
        return;
    };
    let (mut service, refund_lot, lot) = requested(&world).await;
    let mut disputes = world.service().await;
    let processor = PausedProcessor::new(RefundCall::Paid {
        processor_refund_ref: "fake-raced-paid".into(),
    });
    let (paid, ()) = tokio::join!(service.pay_refund_lot(refund_lot, &processor), async {
        processor.entered.notified().await;
        disputes
            .open_dispute(lot, "fake", "raced-dispute", 5_000_000, 0)
            .await
            .expect("open");
        if is_loss {
            disputes
                .close_dispute(
                    lot,
                    "fake",
                    "raced-dispute",
                    DisputeOutcome::Lost {
                        disputed_micro_dollars: 5_000_000,
                    },
                )
                .await
                .expect("close");
        }
        processor.released.notify_one();
    });
    assert_eq!(
        paid.expect("durable observation"),
        RefundPayOutcome::ReconciliationRequired {
            processor_refund_ref: "fake-raced-paid".into()
        }
    );
    let row = world.admin.query_one("SELECT outcome::text FROM finance.operation_receipt WHERE namespace = 'refund_processor_paid' AND principal = $1",
        &[&refund_lot.as_bytes().to_vec()]).await.expect("receipt");
    let outcome: serde_json::Value =
        serde_json::from_str(&row.get::<_, String>(0)).expect("outcome");
    assert_eq!(outcome["processor_refund_ref"], "fake-raced-paid");
    assert_eq!(outcome["reconciliation_required"], true);
    assert_eq!(
        state(&world, refund_lot).await,
        if is_loss {
            "cancelled_dispute_lost"
        } else {
            "blocked_by_dispute"
        }
    );
    world.finish().await;
}

#[test]
fn paid_result_after_dispute_open_is_retained_for_reconciliation() {
    support::run(raced_paid_result(false));
}

#[test]
fn paid_result_after_dispute_loss_is_retained_for_reconciliation() {
    support::run(raced_paid_result(true));
}

#[test]
fn a_concurrent_worker_cannot_dispatch_a_leased_refund() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let (mut service, refund_lot, _) = requested(&world).await;
        let mut second = world.service().await;
        let processor = PausedProcessor::new(RefundCall::Paid {
            processor_refund_ref: "fake-leased".into(),
        });
        let other_processor = FakeRefundProcessor::new(FakeRefundMode::Succeed);
        let (first, ()) = tokio::join!(service.pay_refund_lot(refund_lot, &processor), async {
            processor.entered.notified().await;
            assert!(
                second
                    .pay_refund_lot(refund_lot, &other_processor)
                    .await
                    .is_err()
            );
            assert_eq!(other_processor.attempts(), 0);
            processor.released.notify_one();
        });
        assert!(matches!(
            first.expect("first result"),
            RefundPayOutcome::Paid { .. }
        ));
        world.finish().await;
    });
}

#[test]
fn a_return_to_credit_during_processor_dispatch_is_refused() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let (mut service, refund_lot, _) = requested(&world).await;
        let mut second = world.service().await;
        let processor = PausedProcessor::new(RefundCall::Paid {
            processor_refund_ref: "fake-return-race".into(),
        });
        let (paid, ()) = tokio::join!(service.pay_refund_lot(refund_lot, &processor), async {
            processor.entered.notified().await;
            assert!(second.return_refund_to_credit(refund_lot).await.is_err());
            assert_eq!(
                world.natural(2110, Some(payer(1)), None, None).await,
                3_000_000
            );
            assert_eq!(state(&world, refund_lot).await, "unknown");
            processor.released.notify_one();
        });
        assert!(matches!(paid.expect("paid"), RefundPayOutcome::Paid { .. }));
        world.finish().await;
    });
}

#[test]
fn nonpaid_completions_never_resurrect_dispute_blocked_or_cancelled_refunds() {
    support::run(async {
        for call in [RefundCall::Unknown, RefundCall::KnownFailure] {
            for is_loss in [false, true] {
                let Some(world) = World::open().await else {
                    return;
                };
                let (mut service, refund_lot, lot) = requested(&world).await;
                let mut disputes = world.service().await;
                let processor = PausedProcessor::new(call.clone());
                let (result, ()) =
                    tokio::join!(service.pay_refund_lot(refund_lot, &processor), async {
                        processor.entered.notified().await;
                        disputes
                            .open_dispute(lot, "fake", "nonpaid-race", 5_000_000, 0)
                            .await
                            .expect("open");
                        if is_loss {
                            disputes
                                .close_dispute(
                                    lot,
                                    "fake",
                                    "nonpaid-race",
                                    DisputeOutcome::Lost {
                                        disputed_micro_dollars: 5_000_000,
                                    },
                                )
                                .await
                                .expect("close");
                        }
                        processor.released.notify_one();
                    });
                result.expect("safe completion");
                assert_eq!(
                    state(&world, refund_lot).await,
                    if is_loss {
                        "cancelled_dispute_lost"
                    } else {
                        "blocked_by_dispute"
                    }
                );
                world.finish().await;
            }
        }
    });
}

#[test]
fn cancelling_dispatch_keeps_an_unknown_claim_and_prevents_credit_return() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let (mut service, refund_lot, _) = requested(&world).await;
        let processor = PausedProcessor::new(RefundCall::Unknown);
        {
            let payment = service.pay_refund_lot(refund_lot, &processor);
            tokio::pin!(payment);
            tokio::select! {
                () = processor.entered.notified() => {},
                result = &mut payment => panic!("unexpected completion: {result:?}"),
            }
        }
        assert_eq!(state(&world, refund_lot).await, "unknown");
        assert!(service.return_refund_to_credit(refund_lot).await.is_err());
        let second = FakeRefundProcessor::new(FakeRefundMode::Succeed);
        assert!(service.pay_refund_lot(refund_lot, &second).await.is_err());
        assert_eq!(second.attempts(), 0);
        world.finish().await;
    });
}

#[test]
fn the_close_planner_refuses_a_processor_that_does_not_own_the_lot() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let (mut service, _, lot) = requested(&world).await;
        service
            .open_dispute(lot, "fake", "planner-dispute", 5_000_000, 0)
            .await
            .expect("open");
        let client = support::attack::service_client(&world).await;
        let row = client.query_one("SELECT finance.lock_flow('dispute_close', $1::text::jsonb)::text",
            &[&serde_json::json!({"lot": lot.hex(), "processor": "fake", "dispute_ref": "planner-dispute"}).to_string()]).await.expect("snapshot");
        let document =
            serde_json::from_str::<serde_json::Value>(&row.get::<_, String>(0)).expect("json");
        let snapshot = df_finance::wire::parse_dispute_snapshot(&document)
            .expect("parse")
            .expect("present");
        let request = df_commerce::dispute::DisputeCloseRequest {
            event_id: EventId::from_array([83; 16]),
            entry_id: EntryId::from_array([84; 16]),
            processor: "other".into(),
            dispute_ref: "planner-dispute".into(),
            outcome: DisputeOutcome::Won,
            now_ms: 1,
        };
        assert!(df_commerce::dispute::plan_dispute_close(&snapshot, &request).is_err());
        world.finish().await;
    });
}

#[test]
fn manual_refund_survivors_keep_their_payout_method_after_dispute_resolution() {
    support::run(async {
        for outcome in [
            DisputeOutcome::Won,
            DisputeOutcome::Lost {
                disputed_micro_dollars: 1_000_000,
            },
        ] {
            let Some(world) = World::open().await else {
                return;
            };
            let (mut service, refund_lot, lot) = requested(&world).await;
            world.seed("UPDATE finance.refund_lot SET state = 'manual_pending', payout_method = 'manual'").await;
            service
                .open_dispute(lot, "fake", "manual-dispute", 1_000_000, 0)
                .await
                .expect("open");
            service
                .close_dispute(lot, "fake", "manual-dispute", outcome)
                .await
                .expect("close");
            assert_eq!(state(&world, refund_lot).await, "manual_pending");
            let processor = FakeRefundProcessor::new(FakeRefundMode::Succeed);
            assert!(
                service
                    .pay_refund_lot(refund_lot, &processor)
                    .await
                    .is_err()
            );
            assert_eq!(processor.attempts(), 0);
            world.finish().await;
        }
    });
}

#[test]
fn a_stale_failure_cannot_replace_a_new_workers_unknown_claim() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let (mut service, refund_lot, _) = requested(&world).await;
        let mut second = world.service().await;
        let old = PausedProcessor::new(RefundCall::KnownFailure);
        let new = PausedProcessor::new(RefundCall::Paid {
            processor_refund_ref: "fake-new-worker".into(),
        });
        let old_payment = service.pay_refund_lot(refund_lot, &old);
        tokio::pin!(old_payment);
        tokio::select! {
            () = old.entered.notified() => {},
            result = &mut old_payment => panic!("unexpected: {result:?}"),
        }
        world
            .seed("UPDATE finance.refund_outbox SET lease_until_ms = 0")
            .await;
        let (paid, ()) = tokio::join!(second.pay_refund_lot(refund_lot, &new), async {
            new.entered.notified().await;
            old.released.notify_one();
            old_payment.await.expect("old failure");
            assert_eq!(state(&world, refund_lot).await, "unknown");
            new.released.notify_one();
        });
        assert!(matches!(
            paid.expect("new paid"),
            RefundPayOutcome::Paid { .. }
        ));
        world.finish().await;
    });
}

#[test]
fn a_late_paid_receipt_prevents_returning_a_newer_failed_attempt_to_credit() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let (mut service, refund_lot, _) = requested(&world).await;
        let mut second = world.service().await;
        let old = PausedProcessor::new(RefundCall::Paid {
            processor_refund_ref: "fake-late-paid".into(),
        });
        let (result, ()) = tokio::join!(service.pay_refund_lot(refund_lot, &old), async {
            old.entered.notified().await;
            world
                .seed("UPDATE finance.refund_outbox SET lease_until_ms = 0")
                .await;
            second
                .pay_refund_lot(
                    refund_lot,
                    &FakeRefundProcessor::new(FakeRefundMode::KnownFailure),
                )
                .await
                .expect("new failure");
            assert_eq!(state(&world, refund_lot).await, "failed_review");
            old.released.notify_one();
        });
        assert!(matches!(
            result.expect("late paid"),
            RefundPayOutcome::ReconciliationRequired { .. }
        ));
        assert!(matches!(
            second.return_refund_to_credit(refund_lot).await,
            Err(df_finance::service::FinanceError::Refund(
                df_commerce::refund::RefundRefusal::RefundLotNotPayable
            ))
        ));
        assert_eq!(
            world.natural(2110, Some(payer(1)), None, None).await,
            3_000_000
        );
        world.finish().await;
    });
}

struct MutableClock(std::sync::Arc<std::sync::atomic::AtomicI64>);

impl df_finance::ports::Clock for MutableClock {
    fn now_ms(&self) -> i64 {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[test]
fn a_dispatch_lease_starts_after_waiting_for_the_refund_lock() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let (_, refund_lot, _) = requested(&world).await;
        let holder = support::attack::service_client(&world).await;
        holder.batch_execute("BEGIN").await.expect("begin holder");
        holder
            .query_one(
                "SELECT finance.lock_flow('refund_lot', $1::text::jsonb)::text",
                &[&serde_json::json!({"refund_lot": refund_lot.hex()}).to_string()],
            )
            .await
            .expect("lock holder");
        let initial_now_ms = df_finance::ports::Clock::now_ms(&df_finance::fakes::SystemClock);
        let clock = std::sync::Arc::new(std::sync::atomic::AtomicI64::new(initial_now_ms));
        let mut service = service_with_clock(&world, clock.clone()).await;
        let backend_pid: i32 = service
            .client()
            .query_one("SELECT pg_backend_pid()", &[])
            .await
            .expect("pid")
            .get(0);
        let processor = PausedProcessor::new(RefundCall::KnownFailure);
        let (result, ()) = tokio::join!(service.pay_refund_lot(refund_lot, &processor), async {
            let mut waiting = false;
            for _ in 0..10_000 {
                waiting = world
                    .admin
                    .query_one(
                        "SELECT EXISTS (SELECT 1 FROM pg_locks WHERE pid = $1 AND NOT granted)",
                        &[&backend_pid],
                    )
                    .await
                    .expect("wait status")
                    .get(0);
                if waiting {
                    break;
                }
                tokio::task::yield_now().await;
            }
            assert!(waiting, "payment must reach the held row lock");
            let advanced_now_ms = initial_now_ms + 60_000;
            clock.store(advanced_now_ms, std::sync::atomic::Ordering::SeqCst);
            holder
                .batch_execute("ROLLBACK")
                .await
                .expect("release lock");
            processor.entered.notified().await;
            let lease_until_ms: Option<i64> = world
                .admin
                .query_one(
                    "SELECT lease_until_ms FROM finance.refund_outbox WHERE refund_lot_id = $1",
                    &[&refund_lot.as_bytes().to_vec()],
                )
                .await
                .expect("lease")
                .get(0);
            assert!(lease_until_ms.is_some_and(|until| until > advanced_now_ms + 1_500));
            processor.released.notify_one();
        });
        assert_eq!(
            result.expect("failure observed"),
            RefundPayOutcome::KnownFailure
        );
        world.finish().await;
    });
}

async fn service_with_clock(
    world: &World,
    clock: std::sync::Arc<std::sync::atomic::AtomicI64>,
) -> support::world::Service {
    let client = support::attack::service_client(world).await;
    df_finance::service::FinanceService::new(
        client,
        df_finance::fakes::FakeFence::new(df_finance::ports::FenceVerdict::Current),
        df_finance::fakes::FakeMedia::new(df_finance::ports::MediaVerdict::Admitted),
        df_finance::fakes::FakeMembers::new(vec![payer(1)]),
        Box::new(MutableClock(clock)),
        Box::new(df_finance::fakes::SeededIds::new(3000)),
        df_finance::service::FinanceConfig::default(),
    )
}

#[test]
fn an_expired_claim_after_commit_never_dispatches_a_processor_call() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let (_, refund_lot, _) = requested(&world).await;
        let holder = world.scratch.connect().await;
        holder
            .batch_execute("BEGIN; LOCK TABLE finance.pending_event IN ACCESS EXCLUSIVE MODE")
            .await
            .expect("hold journal");
        let initial_now_ms = df_finance::ports::Clock::now_ms(&df_finance::fakes::SystemClock);
        let clock = std::sync::Arc::new(std::sync::atomic::AtomicI64::new(initial_now_ms));
        let mut service = service_with_clock(&world, clock.clone()).await;
        let backend_pid: i32 = service
            .client()
            .query_one("SELECT pg_backend_pid()", &[])
            .await
            .expect("pid")
            .get(0);
        let processor = FakeRefundProcessor::new(FakeRefundMode::Succeed);
        let (result, ()) = tokio::join!(service.pay_refund_lot(refund_lot, &processor), async {
            let mut waiting = false;
            for _ in 0..10_000 {
                waiting = world.admin.query_one("SELECT EXISTS (SELECT 1 FROM pg_locks WHERE pid = $1 AND relation = 'finance.pending_event'::regclass AND NOT granted)", &[&backend_pid]).await.expect("wait status").get(0);
                if waiting {
                    break;
                }
                tokio::task::yield_now().await;
            }
            assert!(waiting, "claim must reach journal commit");
            clock.store(initial_now_ms + 60_000, std::sync::atomic::Ordering::SeqCst);
            holder
                .batch_execute("ROLLBACK")
                .await
                .expect("release journal");
        });
        assert_eq!(result.expect("expired claim"), RefundPayOutcome::Unknown);
        assert_eq!(processor.attempts(), 0);
        assert_eq!(state(&world, refund_lot).await, "unknown");
        world.finish().await;
    });
}

#[test]
fn paid_observation_survives_a_contested_refund_state_lock_timeout() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let (mut service, refund_lot, _) = requested(&world).await;
        let backend_pid: i32 = service
            .client()
            .query_one("SELECT pg_backend_pid()", &[])
            .await
            .expect("pid")
            .get(0);
        let processor = PausedProcessor::new(RefundCall::Paid {
            processor_refund_ref: "fake-contested-paid".into(),
        });
        let payment = service.pay_refund_lot(refund_lot, &processor);
        tokio::pin!(payment);
        tokio::select! {
            () = processor.entered.notified() => {},
            result = &mut payment => panic!("unexpected: {result:?}"),
        }
        let holder = support::attack::service_client(&world).await;
        holder.batch_execute("BEGIN").await.expect("begin holder");
        holder
            .query_one(
                "SELECT finance.lock_flow('refund_lot', $1::text::jsonb)::text",
                &[&serde_json::json!({"refund_lot": refund_lot.hex()}).to_string()],
            )
            .await
            .expect("hold refund rows");
        processor.released.notify_one();
        let (result, ()) = tokio::join!(payment, async {
            let mut waiting = false;
            for _ in 0..10_000 {
                waiting = world
                    .admin
                    .query_one(
                        "SELECT EXISTS (SELECT 1 FROM pg_locks WHERE pid = $1 AND NOT granted)",
                        &[&backend_pid],
                    )
                    .await
                    .expect("wait status")
                    .get(0);
                if waiting {
                    break;
                }
                tokio::task::yield_now().await;
            }
            assert!(waiting, "posting must reach the contested lock");
            let reference: String = world.admin.query_one("SELECT outcome ->> 'processor_refund_ref' FROM finance.operation_receipt WHERE namespace = 'refund_processor_paid' AND principal = $1",
                &[&refund_lot.as_bytes().to_vec()]).await.expect("receipt precedes contested posting").get(0);
            assert_eq!(reference, "fake-contested-paid");
        });
        let error = result.expect_err("state posting must time out while holder retains its lock");
        assert!(error.is_lock_timeout(), "{error:?}");
        holder
            .batch_execute("ROLLBACK")
            .await
            .expect("release holder");
        let reference: String = world.admin.query_one("SELECT outcome ->> 'processor_refund_ref' FROM finance.operation_receipt WHERE namespace = 'refund_processor_paid' AND principal = $1",
            &[&refund_lot.as_bytes().to_vec()]).await.expect("receipt survives timeout").get(0);
        assert_eq!(reference, "fake-contested-paid");
        assert_eq!(state(&world, refund_lot).await, "unknown");
        let processor = FakeRefundProcessor::new(FakeRefundMode::Succeed);
        let mut second = world.service().await;
        assert!(second.pay_refund_lot(refund_lot, &processor).await.is_err());
        assert_eq!(processor.attempts(), 0);
        world.finish().await;
    });
}

async fn overlapping_paid_observations(contradictory: bool) {
    let Some(world) = World::open().await else {
        return;
    };
    let (mut service, refund_lot, _) = requested(&world).await;
    let mut second = world.service().await;
    let canonical_ref = format!("fake-refund-{}", refund_lot.hex());
    let old_ref = if contradictory {
        "fake-contradictory-ref".into()
    } else {
        canonical_ref.clone()
    };
    let old = PausedProcessor::new(RefundCall::Paid {
        processor_refund_ref: old_ref.clone(),
    });
    let (result, ()) = tokio::join!(service.pay_refund_lot(refund_lot, &old), async {
        old.entered.notified().await;
        world
            .seed("UPDATE finance.refund_outbox SET lease_until_ms = 0")
            .await;
        second
            .pay_refund_lot(
                refund_lot,
                &FakeRefundProcessor::new(FakeRefundMode::Succeed),
            )
            .await
            .expect("new payment");
        old.released.notify_one();
    });
    if contradictory {
        assert_eq!(
            result.expect("retained contradictory observation"),
            RefundPayOutcome::ReconciliationRequired {
                processor_refund_ref: old_ref.clone()
            }
        );
    } else {
        assert!(matches!(
            result.expect("duplicate observation"),
            RefundPayOutcome::Paid { .. } | RefundPayOutcome::AlreadyPaid
        ));
    }
    let refs: Vec<String> = world.admin.query("SELECT outcome ->> 'processor_refund_ref' FROM finance.operation_receipt WHERE namespace = 'refund_processor_paid' AND principal = $1 ORDER BY 1",
        &[&refund_lot.as_bytes().to_vec()]).await.expect("observations").iter().map(|row| row.get(0)).collect();
    if contradictory {
        let mut expected = vec![canonical_ref.clone(), old_ref];
        expected.sort();
        assert_eq!(refs, expected);
    } else {
        assert_eq!(refs, vec![canonical_ref.clone()]);
    }
    let paid_entries: i64 = world
        .admin
        .query_one(
            "SELECT count(*) FROM finance.journal_entry WHERE kind = 'refund_paid'",
            &[],
        )
        .await
        .expect("paid entries")
        .get(0);
    assert_eq!(
        paid_entries, 1,
        "observations never authorize a second payout posting"
    );
    let stored: String = world
        .admin
        .query_one(
            "SELECT processor_refund_ref FROM finance.refund_lot WHERE refund_lot_id = $1",
            &[&refund_lot.as_bytes().to_vec()],
        )
        .await
        .expect("stored reference")
        .get(0);
    assert_eq!(stored, canonical_ref);
    world.finish().await;
}

#[test]
fn distinct_paid_references_are_both_retained_without_a_second_payout_entry() {
    support::run(overlapping_paid_observations(true));
}

#[test]
fn exact_repeated_paid_observations_are_deduplicated() {
    support::run(overlapping_paid_observations(false));
}
