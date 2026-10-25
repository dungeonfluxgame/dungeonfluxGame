#![cfg(not(target_arch = "wasm32"))]

//! The flows end to end on a scratch PostgreSQL: trial, purchase, allocation, admission, send,
//! settlement, release and credit, with the numbers of planning/finance-ledger-research.md 16.
//! Every service runs as `df_finance`, which holds no write privilege.

mod support;

use df_commerce::finance_ids::{AllocationId, AttemptId, LotId, UsageEventId};
use df_commerce::lifecycle::Deadlines;
use df_commerce::settlement::{ReleaseReason, UsageSource};
use df_finance::admit::AdmissionOutcome;
use df_finance::held::QuoteCommand;
use df_finance::ports::{FenceVerdict, MediaVerdict};
use df_finance::service::FinanceError;
use support::world::{CURRENCY, World, credit_command, operation, payer, purchase};

const TRIAL: u64 = 10_000_000;

fn key(tag: u8) -> [u8; 32] {
    [tag; 32]
}

async fn usage_event_of(world: &World, attempt: AttemptId) -> UsageEventId {
    let row = world
        .admin
        .query_one(
            "SELECT usage_event_id FROM finance.usage_event WHERE attempt_id = $1",
            &[&attempt.as_bytes().to_vec()],
        )
        .await
        .expect("usage event");
    UsageEventId::from_slice(&row.get::<_, Vec<u8>>(0)).expect("id")
}

async fn admit_prepare_send(
    world: &World,
    service: &mut support::world::Service,
    tag: u8,
    input: u64,
    output: u64,
) -> AttemptId {
    let command = world.admit_command(tag, input, output);
    let outcome = service.admit(&command).await.expect("admitted");
    assert!(matches!(outcome, AdmissionOutcome::Admitted { .. }));
    service
        .prepare_dispatch(command.attempt, Deadlines::default())
        .await
        .expect("prepared");
    service
        .mark_attempt_sent(command.attempt, [9; 16], Deadlines::default())
        .await
        .expect("sent");
    command.attempt
}

#[test]
fn the_trial_is_granted_once_per_account_and_once_per_email_key() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut service = world.service().await;
        assert!(
            service
                .grant_trial(payer(1), key(1), CURRENCY)
                .await
                .expect("trial")
                .is_some()
        );
        assert_eq!(
            world.natural(2210, Some(payer(1)), None, None).await,
            TRIAL as i64
        );
        assert_eq!(world.line_sum(3910).await, TRIAL as i64);
        assert!(
            service
                .grant_trial(payer(1), key(2), CURRENCY)
                .await
                .expect("again")
                .is_none(),
            "one per account"
        );
        assert!(
            service
                .grant_trial(payer(2), key(1), CURRENCY)
                .await
                .expect("email")
                .is_none(),
            "one per email key"
        );
        assert_eq!(world.count("funding_lot").await, 1);
        assert_eq!(world.count("journal_entry").await, 1);
        world.finish().await;
    });
}

#[test]
fn the_same_payment_applied_twice_posts_one_purchase() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut service = world.service().await;
        assert!(
            service
                .apply_purchase(purchase(payer(1), "pay_1", 20_000_000))
                .await
                .expect("purchase")
                .is_some()
        );
        assert!(
            service
                .apply_purchase(purchase(payer(1), "pay_1", 20_000_000))
                .await
                .expect("again")
                .is_none()
        );
        assert_eq!(
            world.natural(2110, Some(payer(1)), None, None).await,
            20_000_000
        );
        assert_eq!(world.line_sum(1100).await, 20_000_000);
        assert_eq!(world.count("funding_lot").await, 1);
        world.finish().await;
    });
}

#[test]
fn an_allocation_draws_the_trial_credit_before_purchased_credit() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut service = world.service().await;
        service
            .grant_trial(payer(1), key(1), CURRENCY)
            .await
            .expect("trial");
        service
            .apply_purchase(purchase(payer(1), "pay_1", 20_000_000))
            .await
            .expect("purchase");
        service
            .allocate(
                payer(1),
                world.session,
                operation(1),
                15_000_000,
                world.consent(),
            )
            .await
            .expect("allocate");
        assert_eq!(
            world.natural(2220, None, None, None).await,
            10_000_000,
            "all the trial credit first"
        );
        assert_eq!(
            world.natural(2120, None, None, None).await,
            5_000_000,
            "then purchased"
        );
        assert_eq!(world.natural(2210, Some(payer(1)), None, None).await, 0);
        assert_eq!(
            world.natural(2110, Some(payer(1)), None, None).await,
            15_000_000
        );
        // The allocation is not revenue and creates no liability change in total.
        assert_eq!(world.line_sum(4110).await, 0);
        world.finish().await;
    });
}

#[test]
fn an_admission_holds_the_maximum_and_a_repeat_returns_the_stored_outcome() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut service = world.service().await;
        service
            .grant_trial(payer(1), key(1), CURRENCY)
            .await
            .expect("trial");
        service
            .allocate(
                payer(1),
                world.session,
                operation(1),
                TRIAL,
                world.consent(),
            )
            .await
            .expect("allocate");
        let command = world.admit_command(1, 2_000_000, 1_000_000);
        let first = service.admit(&command).await.expect("admitted");
        // 2,000,000 input at 0.1 and 1,000,000 output at 0.5: supplier 700,000, retail 1,050,000.
        let AdmissionOutcome::Admitted {
            retail_maximum_micro_dollars,
            ..
        } = first
        else {
            panic!("expected a fresh admission, got {first:?}");
        };
        assert_eq!(retail_maximum_micro_dollars, 1_050_000);
        let supplier_maximum: i64 = world
            .admin
            .query_one(
                "SELECT supplier_maximum_micro_dollars FROM finance.provider_attempt",
                &[],
            )
            .await
            .expect("attempt")
            .get(0);
        assert_eq!(
            supplier_maximum, 700_000,
            "the supplier side stays in the attempt row"
        );
        assert_eq!(world.natural(2230, None, None, None).await, 1_050_000);
        assert_eq!(
            world.natural(2220, None, None, None).await,
            TRIAL as i64 - 1_050_000
        );
        assert_eq!(world.line_sum(9100).await, 700_000);
        let entries = world.count("journal_entry").await;
        let repeat = service.admit(&command).await.expect("repeat");
        assert!(matches!(
            repeat,
            AdmissionOutcome::AlreadyAdmitted {
                retail_maximum_micro_dollars: 1_050_000,
                ..
            }
        ));
        assert_eq!(
            world.count("journal_entry").await,
            entries,
            "a repeat posts nothing"
        );
        assert_eq!(world.count("reservation").await, 1);
        // The same operation id with another request is a conflict, not a second admission.
        let mut changed = command.clone();
        changed.maximum_units = vec![("text_input".into(), 9), ("text_output".into(), 9)];
        let error = service.admit(&changed).await.expect_err("conflict");
        assert!(
            matches!(
                error,
                FinanceError::Admission(df_commerce::admission::AdmissionRefusal::Conflict)
            ),
            "{error:?}"
        );
        world.finish().await;
    });
}

#[test]
fn a_refused_admission_holds_nothing_and_writes_nothing() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut service = world.service().await;
        service
            .grant_trial(payer(1), key(1), CURRENCY)
            .await
            .expect("trial");
        service
            .allocate(
                payer(1),
                world.session,
                operation(1),
                1_000_000,
                world.consent(),
            )
            .await
            .expect("allocate");
        let baseline = (
            world.count("journal_entry").await,
            world.count("reservation").await,
            world.natural(2220, None, None, None).await,
        );
        use df_commerce::admission::AdmissionRefusal as Refusal;
        // More than the pot can hold: input 10,000,000 tokens is a retail maximum of 1,500,000.
        let too_big = world.admit_command(2, 10_000_000, 0);
        assert!(matches!(
            service.admit(&too_big).await,
            Err(FinanceError::Admission(Refusal::FundingRequired))
        ));
        // A category with no tariff row is refused, not priced at zero.
        let mut unpriced = world.admit_command(3, 1, 1);
        unpriced
            .route
            .declared_categories
            .push("image_input".into());
        unpriced.maximum_units.push(("image_input".into(), 4));
        assert!(
            matches!(service.admit(&unpriced).await, Err(FinanceError::Admission(Refusal::CategoryUnpriced(category))) if category == "image_input")
        );
        // Maximum units that leave out a declared category.
        let mut incomplete = world.admit_command(4, 1, 1);
        incomplete.maximum_units.pop();
        assert!(
            matches!(service.admit(&incomplete).await, Err(FinanceError::Admission(Refusal::MaximumUnitsIncomplete(category))) if category == "text_output")
        );
        // A pending route is refused in hosted paid mode.
        let mut pending = world.admit_command(5, 1, 1);
        pending.route.metering = df_commerce::admission::Metering::Pending;
        assert!(matches!(
            service.admit(&pending).await,
            Err(FinanceError::Admission(Refusal::RouteUnmetered))
        ));
        // The unchanged, small request is admitted: the refusals above are about the requests.
        let small = world.admit_command(6, 100, 100);
        assert!(service.admit(&small).await.is_ok());
        let after = (
            world.count("journal_entry").await,
            world.count("reservation").await,
        );
        assert_eq!(
            after,
            (baseline.0 + 1, baseline.1 + 1),
            "only the admissible request wrote anything"
        );
        world.finish().await;
    });
}

#[test]
fn the_fence_and_the_media_catalog_are_asked_after_the_entry_and_a_refusal_rolls_it_back() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut service = world.service().await;
        service
            .grant_trial(payer(1), key(1), CURRENCY)
            .await
            .expect("trial");
        service
            .allocate(
                payer(1),
                world.session,
                operation(1),
                TRIAL,
                world.consent(),
            )
            .await
            .expect("allocate");
        let before = (
            world.count("journal_entry").await,
            world.natural(2230, None, None, None).await,
        );

        let fence_refused = world.admit_command(1, 1000, 1000);
        for verdict in [
            FenceVerdict::Stale,
            FenceVerdict::PaidWorkFrozen,
            FenceVerdict::ServiceHalting,
        ] {
            let mut refusing = world.service_with(verdict, MediaVerdict::Admitted).await;
            let error = refusing
                .admit(&fence_refused)
                .await
                .expect_err("fence refusal");
            assert!(
                matches!(error, FinanceError::Fence(seen) if seen == verdict),
                "{error:?}"
            );
        }
        let mut media_refusing = world
            .service_with(FenceVerdict::Current, MediaVerdict::QuotaFull)
            .await;
        let mut with_media = world.admit_command(2, 1000, 1000);
        with_media.media = Some(df_finance::ports::MediaRequest {
            attempt: with_media.attempt,
            asset: [7; 16],
        });
        let error = media_refusing
            .admit(&with_media)
            .await
            .expect_err("media refusal");
        assert!(
            matches!(error, FinanceError::Media(MediaVerdict::QuotaFull)),
            "{error:?}"
        );
        assert_eq!(
            (
                world.count("journal_entry").await,
                world.natural(2230, None, None, None).await
            ),
            before
        );
        assert_eq!(world.count("reservation").await, 0);
        // The posted entry took its balance locks before the fence was asked, and rolled back with it.
        assert_eq!(world.count("provider_attempt").await, 0);
        world.finish().await;
    });
}

#[test]
fn settlement_charges_the_actual_units_spends_trial_credit_first_and_releases_the_surplus() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut service = world.service().await;
        service
            .grant_trial(payer(1), key(1), CURRENCY)
            .await
            .expect("trial");
        service
            .allocate(
                payer(1),
                world.session,
                operation(1),
                TRIAL,
                world.consent(),
            )
            .await
            .expect("allocate");
        let attempt = admit_prepare_send(&world, &mut service, 1, 2_000_000, 1_000_000).await;
        // Actual 1,000,000 input and 500,000 output: supplier 350,000, retail 525,000.
        let committed = service
            .settle(
                attempt,
                vec![
                    ("text_input".into(), 1_000_000),
                    ("text_output".into(), 500_000),
                ],
                UsageSource::Reported,
            )
            .await
            .expect("settled")
            .expect("a settlement");
        assert_eq!(committed.entries.len(), 1);
        assert_eq!(
            world.natural(2230, None, None, None).await,
            0,
            "nothing is held any more"
        );
        assert_eq!(
            world.natural(2220, None, None, None).await,
            TRIAL as i64 - 525_000,
            "the surplus went back to the allocation"
        );
        assert_eq!(
            world.line_sum(3910).await,
            TRIAL as i64 - 525_000,
            "promotional consumption is not revenue"
        );
        assert_eq!(world.line_sum(4110).await, 0);
        assert_eq!(
            world.line_sum(5110).await,
            350_000,
            "the supplier cost of promotional usage"
        );
        assert_eq!(world.line_sum(5100).await, 0);
        assert_eq!(world.line_sum(2300).await, -350_000);
        assert_eq!(world.line_sum(9100).await + world.line_sum(9190).await, 0);
        assert_eq!(
            world.line_sum(9100).await,
            0,
            "the exposure closed with the settlement"
        );
        let lot = world.lots_of(payer(1)).await[0];
        let consumed: i64 = world
            .admin
            .query_one(
                "SELECT consumed_micro_dollars FROM finance.funding_lot WHERE lot_id = $1",
                &[&lot.as_bytes().to_vec()],
            )
            .await
            .expect("lot")
            .get(0);
        assert_eq!(consumed, 525_000);
        // A second settlement of the same attempt changes nothing (JOB-02).
        let entries = world.count("journal_entry").await;
        assert!(
            service
                .settle(
                    attempt,
                    vec![("text_input".into(), 1)],
                    UsageSource::Reported
                )
                .await
                .expect("again")
                .is_none()
        );
        assert_eq!(world.count("journal_entry").await, entries);
        world.finish().await;
    });
}

#[test]
fn two_payers_share_a_charge_by_their_held_amounts_and_purchased_credit_is_revenue() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut service = world.service().await;
        service
            .apply_purchase(purchase(payer(1), "pay_1", 20_000_000))
            .await
            .expect("purchase 1");
        service
            .apply_purchase(purchase(payer(2), "pay_2", 10_000_000))
            .await
            .expect("purchase 2");
        service
            .allocate(
                payer(1),
                world.session,
                operation(1),
                20_000_000,
                world.consent(),
            )
            .await
            .expect("allocate 1");
        service
            .allocate(
                payer(2),
                world.session,
                operation(2),
                10_000_000,
                world.consent(),
            )
            .await
            .expect("allocate 2");
        let attempt = admit_prepare_send(&world, &mut service, 1, 3_000_000, 0).await;
        // Retail maximum 450,000 split 2:1.
        let a1 = world.allocation_of(payer(1)).await.expect("allocation 1");
        let a2 = world.allocation_of(payer(2)).await.expect("allocation 2");
        assert_eq!(world.natural(2130, None, None, Some(a1)).await, 300_000);
        assert_eq!(world.natural(2130, None, None, Some(a2)).await, 150_000);
        service
            .settle(
                attempt,
                vec![("text_input".into(), 3_000_000)],
                UsageSource::Reported,
            )
            .await
            .expect("settled");
        assert_eq!(
            world.line_sum(4110).await,
            -450_000,
            "purchased credit is recognized as revenue"
        );
        assert_eq!(
            world.line_sum(5100).await,
            300_000,
            "the supplier cost of purchased usage"
        );
        let lots1 = world.lots_of(payer(1)).await;
        let consumed = |lot: LotId| {
            let admin = &world.admin;
            async move {
                admin
                    .query_one(
                        "SELECT consumed_micro_dollars FROM finance.funding_lot WHERE lot_id = $1",
                        &[&lot.as_bytes().to_vec()],
                    )
                    .await
                    .expect("lot")
                    .get::<_, i64>(0)
            }
        };
        assert_eq!(consumed(lots1[0]).await, 300_000);
        assert_eq!(consumed(world.lots_of(payer(2)).await[0]).await, 150_000);
        world.finish().await;
    });
}

#[test]
fn a_cancel_before_dispatch_releases_the_hold_and_the_exposure_and_a_sent_attempt_cannot_be_cancelled()
 {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut service = world.service().await;
        service
            .grant_trial(payer(1), key(1), CURRENCY)
            .await
            .expect("trial");
        service
            .allocate(
                payer(1),
                world.session,
                operation(1),
                TRIAL,
                world.consent(),
            )
            .await
            .expect("allocate");
        let command = world.admit_command(1, 2_000_000, 1_000_000);
        service.admit(&command).await.expect("admitted");
        assert!(
            service
                .release(command.attempt, ReleaseReason::NotDispatched)
                .await
                .expect("released")
                .is_some()
        );
        assert_eq!(world.natural(2230, None, None, None).await, 0);
        assert_eq!(world.natural(2220, None, None, None).await, TRIAL as i64);
        assert_eq!(
            world.line_sum(9100).await,
            0,
            "the exposure closed with the hold"
        );
        let state: String = world
            .admin
            .query_one("SELECT dispatch_state FROM finance.provider_attempt", &[])
            .await
            .expect("attempt")
            .get(0);
        assert_eq!(state, "cancelled_unsent");
        assert!(
            service
                .release(command.attempt, ReleaseReason::NotDispatched)
                .await
                .expect("again")
                .is_none()
        );

        let sent = admit_prepare_send(&world, &mut service, 2, 1_000_000, 0).await;
        let error = service
            .release(sent, ReleaseReason::NotDispatched)
            .await
            .expect_err("sent");
        assert!(
            matches!(
                error,
                FinanceError::Settle(df_commerce::settlement::SettleRefusal::CannotReleaseSent)
            ),
            "{error:?}"
        );
        // At the customer-hold deadline the hold is released but the supplier exposure stays.
        assert!(
            service
                .release(sent, ReleaseReason::HoldDeadline)
                .await
                .expect("deadline")
                .is_some()
        );
        assert_eq!(world.natural(2230, None, None, None).await, 0);
        assert_eq!(
            world.line_sum(9100).await,
            100_000,
            "the exposure of a sent attempt stays open"
        );
        world.finish().await;
    });
}

#[test]
fn a_credit_returns_to_the_original_lot_never_exceeds_the_charge_and_races_do_not_over_credit() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut service = world.service().await;
        service
            .apply_purchase(purchase(payer(1), "pay_1", 5_000_000))
            .await
            .expect("purchase");
        service
            .allocate(
                payer(1),
                world.session,
                operation(1),
                5_000_000,
                world.consent(),
            )
            .await
            .expect("allocate");
        let attempt = admit_prepare_send(&world, &mut service, 1, 3_000_000, 0).await;
        service
            .settle(
                attempt,
                vec![("text_input".into(), 3_000_000)],
                UsageSource::Reported,
            )
            .await
            .expect("settled");
        let usage = usage_event_of(&world, attempt).await;
        service
            .credit(credit_command(usage, operation(10), 100_000, "case-1"))
            .await
            .expect("credit");
        assert_eq!(
            world.natural(2110, Some(payer(1)), None, None).await,
            100_000,
            "back to the payer's available key"
        );
        assert_eq!(world.line_sum(4110).await, -450_000 + 100_000);
        let over = service
            .credit(credit_command(usage, operation(11), 400_000, "case-2"))
            .await
            .expect_err("over");
        assert!(
            matches!(
                over,
                FinanceError::Settle(df_commerce::settlement::SettleRefusal::OverReversal {
                    remaining: 350_000,
                    ..
                })
            ),
            "{over:?}"
        );
        // Two operators credit the remaining 350,000 at once: exactly one gets it.
        let mut other = world.service().await;
        let (first, second) = tokio::join!(
            service.credit(credit_command(usage, operation(12), 350_000, "case-3")),
            other.credit(credit_command(usage, operation(13), 350_000, "case-4"))
        );
        assert_eq!(
            first.is_ok() as u8 + second.is_ok() as u8,
            1,
            "{first:?} {second:?}"
        );
        let reversed: i64 = world
            .admin
            .query_one(
                "SELECT reversed_micro_dollars FROM finance.usage_event",
                &[],
            )
            .await
            .expect("usage")
            .get(0);
        assert_eq!(reversed, 450_000, "never more than the charge");
        world.finish().await;
    });
}

#[test]
fn a_release_into_a_withdrawn_allocation_lands_in_available_not_allocated() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut service = world.service().await;
        service
            .apply_purchase(purchase(payer(1), "pay_1", 5_000_000))
            .await
            .expect("purchase");
        service
            .allocate(
                payer(1),
                world.session,
                operation(1),
                5_000_000,
                world.consent(),
            )
            .await
            .expect("allocate");
        let attempt = admit_prepare_send(&world, &mut service, 1, 2_000_000, 0).await;
        // The payer withdraws while the job is in flight (the withdrawal flow is round 5; the state
        // is seeded with triggers off).
        world
            .seed("UPDATE finance.allocation SET status = 'withdrawn'")
            .await;
        service
            .settle(
                attempt,
                vec![("text_input".into(), 1_000_000)],
                UsageSource::Reported,
            )
            .await
            .expect("settled");
        // Held 300,000; charged 150,000; the surplus 150,000 returns to available, not allocated.
        assert_eq!(
            world.natural(2110, Some(payer(1)), None, None).await,
            150_000
        );
        assert_eq!(
            world.natural(2120, None, None, None).await,
            5_000_000 - 300_000
        );
        world.finish().await;
    });
}

#[test]
fn a_release_into_a_frozen_lot_posts_to_purchased_frozen() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut service = world.service().await;
        service
            .apply_purchase(purchase(payer(1), "pay_1", 5_000_000))
            .await
            .expect("purchase");
        service
            .allocate(
                payer(1),
                world.session,
                operation(1),
                5_000_000,
                world.consent(),
            )
            .await
            .expect("allocate");
        let command = world.admit_command(1, 2_000_000, 0);
        service.admit(&command).await.expect("admitted");
        let lot = world.lots_of(payer(1)).await[0];
        // A dispute opened (round 5 builds the flow): the lot is frozen and its frozen key exists.
        world
            .seed(&format!(
                "UPDATE finance.funding_lot SET state = 'frozen'; \
                 WITH k AS (INSERT INTO finance.account_key (code, currency, lot_id, dimension_key) \
                            VALUES (2140, 'USD', '\\x{lot}', finance.dimension_key(2140, 'USD', NULL, '\\x{lot}', NULL, NULL, NULL, NULL, NULL, NULL)) \
                            RETURNING key_id) \
                 INSERT INTO finance.account_balance (key_id, natural_micro_dollars) SELECT key_id, 0 FROM k",
                lot = lot.hex()
            ))
            .await;
        service
            .release(command.attempt, ReleaseReason::NotDispatched)
            .await
            .expect("released");
        assert_eq!(
            world.natural(2140, None, Some(lot), None).await,
            300_000,
            "the hold went to the frozen key"
        );
        assert_eq!(
            world.natural(2120, None, None, None).await,
            5_000_000 - 300_000,
            "not back to allocated"
        );
        world.finish().await;
    });
}

#[test]
fn a_quote_funds_one_admission_and_the_second_is_refused() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut service = world.service().await;
        service
            .apply_purchase(purchase(payer(1), "pay_1", 20_000_000))
            .await
            .expect("purchase");
        service
            .allocate(
                payer(1),
                world.session,
                operation(1),
                20_000_000,
                world.consent(),
            )
            .await
            .expect("allocate");
        // 40,000,000 input tokens: supplier 4,000,000, retail 6,000,000, above the 5,000,000 threshold.
        let mut big = world.admit_command(1, 40_000_000, 0);
        use df_commerce::admission::AdmissionRefusal as Refusal;
        assert!(matches!(
            service.admit(&big).await,
            Err(FinanceError::Admission(Refusal::QuoteRequired))
        ));
        let quote = service
            .create_quote(
                &world.version,
                QuoteCommand {
                    session: world.session,
                    route: big.route.clone(),
                    work_class: big.work_class,
                    maximum_units: big.maximum_units.clone(),
                    estimate_units: big.estimate_units.clone(),
                    request_digest: big.request_digest,
                    lifetime_ms: 600_000,
                },
            )
            .await
            .expect("quote");
        big.quote = Some(quote);
        assert!(
            matches!(
                service.admit(&big).await,
                Err(FinanceError::Admission(Refusal::QuoteNotApproved))
            ),
            "an open quote is not approved"
        );
        service
            .decide_quote(world.session, quote, true)
            .await
            .expect("approved");
        assert!(matches!(
            service.admit(&big).await,
            Ok(AdmissionOutcome::Admitted { .. })
        ));
        let mut second = world.admit_command(2, 40_000_000, 0);
        second.quote = Some(quote);
        second.request_digest = big.request_digest;
        assert!(matches!(
            service.admit(&second).await,
            Err(FinanceError::Admission(Refusal::QuoteAlreadyUsed))
        ));
        assert_eq!(world.count("reservation").await, 1);
        world.finish().await;
    });
}

#[test]
fn a_payer_whose_membership_is_revoked_is_excluded_from_the_shares() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut service = world.service().await;
        service
            .apply_purchase(purchase(payer(1), "pay_1", 5_000_000))
            .await
            .expect("purchase 1");
        service
            .apply_purchase(purchase(payer(2), "pay_2", 5_000_000))
            .await
            .expect("purchase 2");
        service
            .allocate(
                payer(1),
                world.session,
                operation(1),
                5_000_000,
                world.consent(),
            )
            .await
            .expect("allocate 1");
        service
            .allocate(
                payer(2),
                world.session,
                operation(2),
                5_000_000,
                world.consent(),
            )
            .await
            .expect("allocate 2");
        let mut only_first = world.service_with_members(vec![payer(1)]).await;
        let command = world.admit_command(1, 2_000_000, 0);
        only_first.admit(&command).await.expect("admitted");
        let a2 = world.allocation_of(payer(2)).await.expect("allocation 2");
        assert_eq!(
            world.natural(2130, None, None, Some(a2)).await,
            0,
            "the removed contributor holds nothing"
        );
        assert_eq!(
            world
                .natural(2130, None, None, world.allocation_of(payer(1)).await)
                .await,
            300_000
        );
        world.finish().await;
    });
}

#[test]
fn allocation_ids_are_per_session_and_a_raised_allocation_keeps_its_id() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut service = world.service().await;
        service
            .apply_purchase(purchase(payer(1), "pay_1", 5_000_000))
            .await
            .expect("purchase");
        service
            .allocate(
                payer(1),
                world.session,
                operation(1),
                1_000_000,
                world.consent(),
            )
            .await
            .expect("first");
        let first: Option<AllocationId> = world.allocation_of(payer(1)).await;
        service
            .allocate(
                payer(1),
                world.session,
                operation(2),
                2_000_000,
                world.consent(),
            )
            .await
            .expect("raised");
        assert_eq!(world.allocation_of(payer(1)).await, first);
        assert_eq!(world.natural(2120, None, None, first).await, 3_000_000);
        assert_eq!(
            world.count("funding_epoch").await,
            3,
            "the opening epoch and one per change"
        );
        world.finish().await;
    });
}
