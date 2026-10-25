#![cfg(not(target_arch = "wasm32"))]

//! Customer and supplier outcomes at the late-result boundary (planning/master-launch-plan.md 6.3).

mod support;

use df_commerce::lifecycle::Deadlines;
use df_commerce::settlement::{ReleaseReason, UsageSource};
use support::invariants;
use support::world::{World, credit_command, operation, payer, purchase};

#[test]
fn job_04_invalid_and_duplicate_output_is_credited_without_shifting_supplier_cost() {
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
                .expect("first result")
                .is_some()
        );
        let usage_event = df_commerce::finance_ids::UsageEventId::from_slice(
            &world
                .admin
                .query_one(
                    "SELECT usage_event_id FROM finance.usage_event WHERE attempt_id = $1",
                    &[&command.attempt.as_bytes().to_vec()],
                )
                .await
                .expect("usage event")
                .get::<_, Vec<u8>>(0),
        )
        .expect("usage event id");
        service
            .credit(credit_command(
                usage_event,
                operation(73),
                135_000,
                "invalid-output",
            ))
            .await
            .expect("credit invalid output");
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
                .expect("duplicate output")
                .is_none(),
            "the same attempt cannot be charged twice"
        );

        assert_eq!(world.natural(4110, None, None, None).await, 0);
        assert_eq!(world.line_sum(5100).await, 90_000);
        assert_eq!(world.natural(2130, None, None, None).await, 0);
        assert_eq!(
            world.natural(2110, Some(payer(1)), None, None).await,
            5_135_000
        );
        assert_eq!(world.count("usage_event").await, 1);
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
