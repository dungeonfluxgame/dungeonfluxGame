#![cfg(not(target_arch = "wasm32"))]

//! Customer and supplier outcomes at the late-result boundary (planning/master-launch-plan.md 6.3).

mod support;

use df_commerce::lifecycle::Deadlines;
use df_commerce::settlement::{ReleaseReason, UnusableOutcome, UsageSource};
use support::invariants;
use support::world::{World, operation, payer, purchase};

#[test]
fn job_04_invalid_output_from_the_metering_outcome_is_never_charged_and_its_cost_is_company_loss() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut service = world.service().await;
        service
            .apply_purchase(purchase(payer(1), "job_04", 10_000_000))
            .await
            .expect("purchase");
        service
            .allocate(
                payer(1),
                world.session,
                operation(71),
                5_000_000,
                world.consent(),
            )
            .await
            .expect("allocation");
        let command = world.admit_command(72, 400_000, 100_000);
        service.admit(&command).await.expect("admission");
        service
            .prepare_dispatch(command.attempt, Deadlines::default())
            .await
            .expect("prepare");
        service
            .mark_attempt_sent(command.attempt, [9; 16], Deadlines::default())
            .await
            .expect("send");

        let units = || {
            vec![
                ("text_input".into(), 400_000),
                ("text_output".into(), 100_000),
            ]
        };
        assert!(
            service
                .settle_unusable(
                    command.attempt,
                    units(),
                    UsageSource::Reported,
                    UnusableOutcome::InvalidOutput,
                )
                .await
                .expect("invalid-output outcome")
                .is_some(),
            "the metering invalid outcome settles the attempt"
        );
        assert!(
            service
                .settle_unusable(
                    command.attempt,
                    units(),
                    UsageSource::Reported,
                    UnusableOutcome::InvalidOutput,
                )
                .await
                .expect("duplicate invalid outcome")
                .is_none(),
            "a duplicate outcome cannot book the loss twice"
        );
        assert!(
            service
                .settle(command.attempt, units(), UsageSource::Reported)
                .await
                .expect("duplicate usable outcome")
                .is_none(),
            "the same attempt cannot be charged after its invalid outcome"
        );

        assert_eq!(world.natural(4110, None, None, None).await, 0, "no revenue");
        assert_eq!(
            world.count("usage_event").await,
            0,
            "no charge was recorded"
        );
        assert_eq!(world.line_sum(5100).await, 0, "cost is not a service cost");
        assert_eq!(
            world.line_sum(5200).await,
            90_000,
            "the supplier cost is absorbed as invalid_output company loss"
        );
        assert_eq!(world.line_sum(2300).await, -90_000);
        assert_eq!(world.natural(2130, None, None, None).await, 0);
        assert_eq!(
            world.natural(2120, None, None, None).await,
            5_000_000,
            "the whole hold returns to the allocation, uncharged"
        );
        let attempt: (String, String, Option<String>) = {
            let row = world
                .admin
                .query_one(
                    "SELECT customer_state, usability, usability_class \
                     FROM finance.provider_attempt WHERE attempt_id = $1",
                    &[&command.attempt.as_bytes().to_vec()],
                )
                .await
                .expect("attempt outcome");
            (row.get(0), row.get(1), row.get(2))
        };
        assert_eq!(
            attempt,
            (
                "released".to_owned(),
                "no_usable_output".to_owned(),
                Some("invalid_output".to_owned())
            )
        );
        invariants::all(&world.admin).await.expect("invariants");
        world.finish().await;
    });
}

#[test]
fn job_05_late_result_after_hold_release_never_charges_customer() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut service = world.service().await;
        service
            .apply_purchase(purchase(payer(1), "job_05", 10_000_000))
            .await
            .expect("purchase");
        service
            .allocate(
                payer(1),
                world.session,
                operation(74),
                5_000_000,
                world.consent(),
            )
            .await
            .expect("allocation");
        let command = world.admit_command(75, 400_000, 100_000);
        service.admit(&command).await.expect("admission");
        service
            .prepare_dispatch(command.attempt, Deadlines::default())
            .await
            .expect("prepare");
        service
            .mark_attempt_sent(command.attempt, [9; 16], Deadlines::default())
            .await
            .expect("send");
        service
            .release(command.attempt, ReleaseReason::HoldDeadline)
            .await
            .expect("release the customer hold")
            .expect("release event");

        assert!(
            service
                .settle(
                    command.attempt,
                    vec![
                        ("text_input".into(), 400_000),
                        ("text_output".into(), 100_000)
                    ],
                    UsageSource::Reported,
                )
                .await
                .expect("late result")
                .is_none(),
            "released customer holds have no retroactive settlement"
        );
        assert_eq!(world.natural(2120, None, None, None).await, 5_000_000);
        assert_eq!(world.natural(2130, None, None, None).await, 0);
        assert_eq!(world.natural(4110, None, None, None).await, 0);
        assert_eq!(world.line_sum(9100).await, 90_000);
        assert_eq!(world.count("usage_event").await, 0);
        invariants::all(&world.admin).await.expect("invariants");
        world.finish().await;
    });
}
