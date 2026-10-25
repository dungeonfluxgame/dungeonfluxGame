#![cfg(not(target_arch = "wasm32"))]

//! Money edges (planning/finance-ledger-research.md 4, 10, 12): a billed result whose retail rounds
//! to zero still settles and frees its hold, and the hourly ceiling counts time on the service clock
//! that stamped the charges.

mod support;

use std::sync::Arc;

use df_commerce::admission::AdmissionRefusal;
use df_commerce::finance_ids::SessionKey;
use df_commerce::funding_flows::SessionPolicy;
use df_commerce::lifecycle::Deadlines;
use df_commerce::settlement::{SettleRefusal, UsageSource};
use df_finance::fakes::{FakeClock, FakeFence, FakeMedia, FakeMembers, SeededIds};
use df_finance::ports::{FenceVerdict, MediaVerdict};
use df_finance::service::{FinanceConfig, FinanceError, FinanceService};
use support::attack::{now_ms, service_client};
use support::invariants;
use support::world::{World, credit_command, operation, payer, purchase};

const EXTRA: SessionKey = SessionKey::from_array([0x5b; 16]);

#[test]
fn a_billed_result_that_rounds_to_zero_retail_settles_with_no_revenue_and_frees_the_hold() {
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
        let command = world.admit_command(1, 1_000_000, 1_000_000);
        service.admit(&command).await.expect("admitted");
        service
            .prepare_dispatch(command.attempt, Deadlines::default())
            .await
            .expect("prepared");
        service
            .mark_attempt_sent(command.attempt, [9; 16], Deadlines::default())
            .await
            .expect("sent");
        // One output token: supplier 0.5 rounds half up to 1, retail 0.75 rounds down to 0.
        let settled = service
            .settle(
                command.attempt,
                vec![("text_input".into(), 0), ("text_output".into(), 1)],
                UsageSource::Reported,
            )
            .await
            .expect("settled")
            .expect("a settlement");
        assert_eq!(settled.entries.len(), 1);
        assert_eq!(
            world.natural(2130, None, None, None).await,
            0,
            "nothing stays held"
        );
        assert_eq!(
            world.natural(2120, None, None, None).await,
            5_000_000,
            "the whole hold went back"
        );
        assert_eq!(world.line_sum(4110).await, 0, "no revenue");
        assert_eq!(world.line_sum(5100).await, 1, "the supplier cost is booked");
        assert_eq!(world.line_sum(2300).await, -1);
        let (state, charged): (String, i64) = {
            let row = world
                .admin
                .query_one(
                    "SELECT state, charged_micro_dollars FROM finance.reservation",
                    &[],
                )
                .await
                .expect("reservation");
            (row.get(0), row.get(1))
        };
        assert_eq!((state.as_str(), charged), ("charged", 0));
        let customer: String = world
            .admin
            .query_one("SELECT customer_state FROM finance.provider_attempt", &[])
            .await
            .expect("attempt")
            .get(0);
        assert_eq!(customer, "charged");
        let usage = world
            .admin
            .query_one("SELECT usage_event_id FROM finance.usage_event", &[])
            .await
            .expect("usage event")
            .get::<_, Vec<u8>>(0);
        let usage = df_commerce::finance_ids::UsageEventId::from_slice(&usage).expect("id");
        let credit = service
            .credit(credit_command(usage, operation(2), 1, "case-1"))
            .await;
        assert!(
            matches!(
                credit,
                Err(FinanceError::Settle(SettleRefusal::OverReversal {
                    remaining: 0,
                    ..
                }))
            ),
            "a zero charge has nothing to credit: {credit:?}"
        );
        invariants::all(&world.admin).await.expect("invariants");
        world.finish().await;
    });
}

#[test]
fn the_hourly_ceiling_counts_the_last_hour_on_the_clock_that_stamped_the_charges() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut funding = world.service().await;
        world
            .open_session(
                &mut funding,
                EXTRA,
                SessionPolicy {
                    hourly_ceiling_micro_dollars: 20_000_000,
                    session_ceiling_micro_dollars: None,
                    per_request_threshold_micro_dollars: 100_000_000,
                },
            )
            .await;
        funding
            .apply_purchase(purchase(payer(1), "pay_big", 60_000_000))
            .await
            .expect("purchase");
        funding
            .allocate(payer(1), EXTRA, operation(1), 50_000_000, world.consent())
            .await
            .expect("allocate");
        let clock = Arc::new(FakeClock::new(now_ms()));
        let mut service = FinanceService::new(
            service_client(&world).await,
            FakeFence::new(FenceVerdict::Current),
            FakeMedia::new(MediaVerdict::Admitted),
            FakeMembers::new(world.payers.iter().copied()),
            Box::new(Arc::clone(&clock)),
            Box::new(SeededIds::new(5)),
            FinanceConfig::default(),
        );
        // 120,000,000 input tokens hold 18,000,000; 113,000,000 settle to 16,950,000.
        let mut big = world.admit_command(1, 120_000_000, 0);
        big.session = EXTRA;
        service.admit(&big).await.expect("admitted");
        service
            .prepare_dispatch(big.attempt, Deadlines::default())
            .await
            .expect("prepared");
        service
            .mark_attempt_sent(big.attempt, [9; 16], Deadlines::default())
            .await
            .expect("sent");
        service
            .settle(
                big.attempt,
                vec![
                    ("text_input".into(), 113_000_000),
                    ("text_output".into(), 0),
                ],
                UsageSource::Reported,
            )
            .await
            .expect("settled")
            .expect("a settlement");
        let mut next = world.admit_command(2, 33_400_000, 0);
        next.session = EXTRA;
        let blocked = service.admit(&next).await;
        assert!(
            matches!(
                blocked,
                Err(FinanceError::Admission(AdmissionRefusal::CeilingReached(_)))
            ),
            "16,950,000 charged and 5,010,000 asked is over 20,000,000 an hour: {blocked:?}"
        );
        clock.advance(3_700_000);
        service
            .admit(&next)
            .await
            .expect("an hour later the charge no longer counts");
        world.finish().await;
    });
}
