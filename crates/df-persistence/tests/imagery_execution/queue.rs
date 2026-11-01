//! Page traversal must reach eligible work beyond arbitrarily long denied prefixes.
use super::*;
use df_persistence::imagery_execution::{record_urgency_in, scan_queue_after_in, scan_queue_in};
#[test]
fn queue_pages_reach_eligible_work_beyond_128_immediately_due_source_denials() {
    runtime().block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        let tx = client.transaction().await.unwrap();
        set_capacity_bounds_in(
            &tx,
            "live/generation",
            &CapacityBounds::new(1, 200, 1 << 20).unwrap(),
            at(0),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        for index in 1..=130u8 {
            accept_and_enqueue(&mut client, index, index, index, index, at(0)).await;
        }
        let mut cursor = None;
        let mut seen = std::collections::HashSet::new();
        for index in 1..=130u8 {
            let now = at(i64::from(index));
            let tx = client.transaction().await.unwrap();
            let page = scan_queue_after_in(&tx, "live/generation", cursor, 1, now)
                .await
                .unwrap();
            assert_eq!(page.stages.len(), 1);
            let row = &page.stages[0];
            assert_eq!(row.job, job_id(index));
            assert!(seen.insert(row.stage));
            cursor = page.next;
            if index < 130 {
                defer_queued_in(&tx, &row.owner, &row.job, &row.stage, now + 1, now)
                    .await
                    .unwrap();
            }
            tx.commit().await.unwrap();
        }
        assert_eq!(seen.len(), 130);
        let tx = client.transaction().await.unwrap();
        assert!(
            scan_queue_after_in(&tx, "live/generation", cursor, 1, at(200))
                .await
                .unwrap()
                .stages
                .is_empty()
        );
        assert_eq!(
            scan_queue_in(&tx, "live/generation", 1, at(200))
                .await
                .unwrap()[0]
                .job,
            job_id(1)
        );
        claim_stage_in(
            &tx,
            &claim_command(130, 130, GATE_REVISION, 0xE0, 0, at(200)),
        )
        .await
        .expect("eligible tail is claimed without hidden total-prefix cap");
        tx.commit().await.unwrap();
        assert_eq!(
            capacity_counts(&client, "live/generation").await,
            (1, 129, 129 * 128)
        );
        scratch.finish().await;
    });
}
#[test]
fn queue_cursor_keeps_priority_epoch_fixed_while_actual_eligibility_and_next_wrap_advance() {
    runtime().block_on(async {
        use df_imagery::Urgency;
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        for index in 1..=3u8 {
            accept_and_enqueue(&mut client, index, index, index, index, at(0)).await;
        }
        let tx = client.transaction().await.unwrap();
        for (index, urgency) in [
            (1, Urgency::Speculative),
            (2, Urgency::Visible),
            (3, Urgency::Present),
        ] {
            record_urgency_in(
                &tx,
                &owner(),
                &job_id(index),
                &ImageInterestId::from_bytes(&id(index)).unwrap(),
                urgency,
                at(0),
            )
            .await
            .unwrap();
        }
        tx.commit().await.unwrap();
        let tx = client.transaction().await.unwrap();
        let first = scan_queue_after_in(&tx, "live/generation", None, 1, at(1))
            .await
            .unwrap();
        assert_eq!(first.stages[0].job, job_id(2));
        let cursor = first.next.unwrap();
        assert_eq!(cursor.ranking_at_ms, at(1));
        let second = scan_queue_after_in(&tx, "live/generation", Some(cursor), 1, at(20_000))
            .await
            .unwrap();
        assert_eq!(second.stages[0].job, job_id(3));
        assert_eq!(second.next.unwrap().ranking_at_ms, at(1));
        let third = scan_queue_after_in(&tx, "live/generation", second.next, 1, at(20_001))
            .await
            .unwrap();
        assert_eq!(third.stages[0].job, job_id(1));
        assert_eq!(third.next.unwrap().effective_urgency, 2);
        assert!(
            scan_queue_after_in(&tx, "live/generation", third.next, 1, at(20_002))
                .await
                .unwrap()
                .stages
                .is_empty()
        );
        assert_eq!(
            scan_queue_after_in(&tx, "live/generation", None, 1, at(20_003))
                .await
                .unwrap()
                .stages[0]
                .job,
            job_id(1)
        );
        assert_eq!(
            scan_queue_after_in(
                &tx,
                "live/generation",
                Some(df_imagery::execution::QueueCursor {
                    ranking_at_ms: at(30_000),
                    ..cursor
                }),
                1,
                at(20_003)
            )
            .await,
            Err(ExecutionError::InvalidRecord)
        );
        tx.rollback().await.unwrap();
        scratch.finish().await;
    });
}
