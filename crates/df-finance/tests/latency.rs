#![cfg(not(target_arch = "wasm32"))]

//! Latency of the hot path against the 10 ms budget (planning/finance-ledger-research.md 19.4), on a
//! database that already holds 100,000 account keys, 300,000 usage events and 100,000 reservations
//! (the scale at which a missing index or a whole-table sum shows). The figures go to stdout
//! (`--nocapture`). `DUNGEONFLUX_FINANCE_STRICT_LATENCY` holds the three flows to the budget (median)
//! and twice it (95th percentile) on a quiet machine; without it the test holds them to a
//! regression ceiling, because the test database shares a machine with other work.

mod support;

use std::time::{Duration, Instant};

use df_commerce::finance_ids::{AttemptId, EffectId, OperationKey};
use df_commerce::lifecycle::Deadlines;
use df_commerce::settlement::UsageSource;
use support::world::{World, operation, payer, purchase};

const BUDGET: Duration = Duration::from_millis(10);
/// The ceiling the test holds all three flows to without the strict switch.
const CEILING: Duration = Duration::from_millis(50);

fn summary(name: &str, mut samples: Vec<Duration>) -> (Duration, Duration) {
    samples.sort_unstable();
    let at = |fraction: f64| samples[(((samples.len() - 1) as f64) * fraction) as usize];
    let (p50, p95, max) = (at(0.5), at(0.95), samples[samples.len() - 1]);
    println!(
        "LATENCY {name}: n={} p50={:.2} ms p95={:.2} ms max={:.2} ms (budget {} ms)",
        samples.len(),
        p50.as_secs_f64() * 1000.0,
        p95.as_secs_f64() * 1000.0,
        max.as_secs_f64() * 1000.0,
        BUDGET.as_millis()
    );
    (p50, p95)
}

/// Rows of other sessions and old history, written with the triggers off the way a long-running
/// system would hold them: keys, usage events and reservations in bulk.
async fn seed_history(world: &World) {
    let price_version = format!("\\x{}", world.version.id.hex());
    world
        .seed(&format!(
            "INSERT INTO finance.account_key (code, currency, payer_account_id, lot_id, dimension_key) \
             SELECT 2110, 'USD', p, l, finance.dimension_key(2110, 'USD', p, l, NULL, NULL, NULL, NULL, NULL, NULL) \
               FROM (SELECT decode(lpad(to_hex(g), 32, '0'), 'hex') AS p, decode(lpad(to_hex(g + 10000000), 32, '0'), 'hex') AS l \
                       FROM generate_series(1, 100000) g) q; \
             INSERT INTO finance.usage_event (usage_event_id, meter_session_id, meter_segment_seq, session_id, category, \
                    quantity_text, retail_micro_dollars, supplier_micro_dollars, price_version_id, settled_at_ms) \
             SELECT decode(lpad(to_hex(g + 20000000), 32, '0'), 'hex'), decode(lpad(to_hex(g % 50), 32, '0'), 'hex'), g, \
                    CASE WHEN g % 3 = 0 THEN decode('{session}', 'hex') ELSE decode(lpad(to_hex(g % 997), 32, '0'), 'hex') END, \
                    'ai_usage', 'usage', 1000, 600, '{price_version}'::bytea, 1000000000000 + g \
               FROM generate_series(1, 300000) g; \
             INSERT INTO finance.reservation (reservation_id, session_id, payer_kind, kind, operation_id, price_version_id, \
                    retail_maximum_micro_dollars, admitted_at_ms, state, charged_micro_dollars) \
             SELECT decode(lpad(to_hex(g + 30000000), 32, '0'), 'hex'), decode('{session}', 'hex'), 'table', 'provider', \
                    decode(lpad(to_hex(g + 40000000), 32, '0'), 'hex'), '{price_version}'::bytea, 2000, 1000000000000 + g, 'charged', 2000 \
               FROM generate_series(1, 100000) g; \
             ANALYZE finance.account_key; ANALYZE finance.usage_event; ANALYZE finance.reservation",
            session = world.session.hex(),
        ))
        .await;
}

#[test]
fn lock_flow_and_admission_and_settlement_stay_within_the_budget() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let profile = std::env::var("LATENCY_PROFILE").is_ok();
        if profile {
            world
                .admin
                .batch_execute("DO $$ BEGIN EXECUTE format('ALTER DATABASE %I SET track_functions = %L', current_database(), 'all'); END $$")
                .await
                .expect("track functions");
        }
        let mut service = world.service().await;
        for tag in 1..=3 {
            for index in 0..3 {
                service
                    .apply_purchase(purchase(
                        payer(tag),
                        &format!("pay_{tag}_{index}"),
                        10_000_000,
                    ))
                    .await
                    .expect("purchase");
            }
            service
                .grant_trial(payer(tag), [tag; 32], support::world::CURRENCY)
                .await
                .expect("trial");
            service
                .allocate(
                    payer(tag),
                    world.session,
                    operation(tag),
                    25_000_000,
                    world.consent(),
                )
                .await
                .expect("allocate");
        }
        seed_history(&world).await;

        let base = world.admit_command(1, 40_000, 10_000);
        let command = |n: u32| {
            let mut command = base.clone();
            let id = |salt: u8| {
                let mut bytes = [salt; 16];
                bytes[..4].copy_from_slice(&n.to_be_bytes());
                bytes
            };
            command.operation = OperationKey::from_array(id(0x41));
            command.attempt = AttemptId::from_array(id(0x61));
            command.effect = EffectId::from_array(id(0x81));
            command.request_digest = id(0x91).repeat(2).try_into().expect("32 bytes");
            command
        };

        // lock_flow alone, as the admission flow calls it, and rolled back.
        let args = serde_json::json!({
            "session": world.session.hex(),
            "principal": world.campaign.hex(),
            "operation": command(0).operation.hex(),
            "now_ms": support::attack::now_ms(),
        })
        .to_string();
        let mut lock_samples = Vec::new();
        let client = service.client();
        for _ in 0..200 {
            let started = Instant::now();
            client.batch_execute("BEGIN").await.expect("begin");
            client
                .query_one(
                    "SELECT finance.lock_flow('admission', $1::text::jsonb)::text",
                    &[&args],
                )
                .await
                .expect("lock_flow");
            client.batch_execute("ROLLBACK").await.expect("rollback");
            lock_samples.push(started.elapsed());
        }
        let (lock_p50, lock_p95) =
            summary("lock_flow(admission) with its round trip", lock_samples);

        let mut admit_samples = Vec::new();
        for n in 1..=200 {
            let started = Instant::now();
            service.admit(&command(n)).await.expect("admitted");
            admit_samples.push(started.elapsed());
        }
        let (admit_p50, admit_p95) = summary("admission end to end", admit_samples);

        if profile {
            let rows = world
                .admin
                .query(
                    "SELECT funcname, calls, round(total_time::numeric, 1)::float8, round(self_time::numeric, 1)::float8                      FROM pg_stat_user_functions ORDER BY self_time DESC LIMIT 22",
                    &[],
                )
                .await
                .expect("function statistics");
            for row in rows {
                println!(
                    "PROFILE {:<36} calls={:<7} total={:>9.1} ms self={:>9.1} ms",
                    row.get::<_, String>(0),
                    row.get::<_, i64>(1),
                    row.get::<_, f64>(2),
                    row.get::<_, f64>(3)
                );
            }
        }
        let mut settle_samples = Vec::new();
        for n in 1..=100 {
            let attempt = command(n).attempt;
            service
                .prepare_dispatch(attempt, Deadlines::default())
                .await
                .expect("prepared");
            service
                .mark_attempt_sent(attempt, [9; 16], Deadlines::default())
                .await
                .expect("sent");
            let started = Instant::now();
            service
                .settle(
                    attempt,
                    vec![("text_input".into(), 20_000), ("text_output".into(), 5_000)],
                    UsageSource::Reported,
                )
                .await
                .expect("settled");
            settle_samples.push(started.elapsed());
        }
        let (settle_p50, settle_p95) = summary("settlement end to end", settle_samples);

        let strict = std::env::var("DUNGEONFLUX_FINANCE_STRICT_LATENCY").is_ok();
        for (name, p50, p95) in [
            ("lock_flow", lock_p50, lock_p95),
            ("admission", admit_p50, admit_p95),
            ("settlement", settle_p50, settle_p95),
        ] {
            if p50 > BUDGET {
                println!("LATENCY MISS {name}: median {p50:?} is over the {BUDGET:?} budget");
            }
            let (median_limit, tail_limit) = if strict {
                (BUDGET, BUDGET * 2)
            } else {
                (CEILING, CEILING * 3)
            };
            assert!(
                p50 <= median_limit,
                "{name}: median {p50:?} is over {median_limit:?}"
            );
            assert!(
                p95 <= tail_limit,
                "{name}: 95th percentile {p95:?} is over {tail_limit:?}"
            );
        }
        drop(service);
        world.finish().await;
    });
}
