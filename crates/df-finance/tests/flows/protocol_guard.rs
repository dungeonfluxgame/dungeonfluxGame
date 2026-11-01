//! Deterministic crossing-session plans retain the ordinary multi-attempt API.
use super::*;
use df_commerce::finance_ids::{EventId, SessionKey};
use df_commerce::funding_flows::SessionPolicy;
use df_commerce::plan::{EventKind, FinancePlan};
use std::time::Duration;

async fn prepare_plan(
    service: &support::world::Service,
    attempts: &[AttemptId],
    tag: u8,
) -> FinancePlan {
    let event = EventId::from_array([tag; 16]);
    let mut plan = FinancePlan::new(event, EventKind::AttemptPrepared);
    for attempt in attempts {
        let text: String = service
            .client()
            .query_one(
                "SELECT finance.lock_flow('attempt',jsonb_build_object('attempt',$1::text))::text",
                &[&attempt.hex()],
            )
            .await
            .unwrap()
            .get(0);
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        let row = df_finance::wire::parse_attempt_row(&value["attempt"]).unwrap();
        plan.changes.extend(
            df_commerce::lifecycle::plan_prepare_dispatch(
                event,
                &row,
                Deadlines::default(),
                service.now_ms(),
            )
            .unwrap()
            .changes,
        );
    }
    plan
}
async fn wait_for_database_lock(admin: &tokio_postgres::Client, pid: i32) {
    for _ in 0..100 {
        if admin
            .query_one("SELECT cardinality(pg_blocking_pids($1))>0", &[&pid])
            .await
            .unwrap()
            .get::<_, bool>(0)
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("database lock barrier was not reached");
}
async fn crossing_plans(shared_lots: usize) {
    let Some(world) = World::open().await else {
        return;
    };
    let mut setup = world.service().await;
    let second = SessionKey::from_array([0x61; 16]);
    world
        .open_session(
            &mut setup,
            second,
            SessionPolicy {
                hourly_ceiling_micro_dollars: 20_000_000,
                session_ceiling_micro_dollars: None,
                per_request_threshold_micro_dollars: 500_000,
            },
        )
        .await;
    for tag in 1..=shared_lots {
        let tag = u8::try_from(tag).unwrap();
        setup
            .apply_purchase(purchase(
                payer(tag),
                if tag == 1 {
                    "guard-purchase-1"
                } else {
                    "guard-purchase-2"
                },
                1_000 / u64::try_from(shared_lots).unwrap(),
            ))
            .await
            .unwrap();
        for (index, session) in [world.session, second].iter().enumerate() {
            setup
                .allocate(
                    payer(tag),
                    *session,
                    operation(tag * 2 + u8::try_from(index).unwrap() + 20),
                    400 / u64::try_from(shared_lots).unwrap(),
                    world.consent(),
                )
                .await
                .unwrap();
        }
    }
    let mut attempts = Vec::new();
    for tag in 1..=4 {
        let mut command = world.admit_command(tag, 100, 100);
        if tag == 2 || tag == 4 {
            command.session = second;
        }
        setup.admit(&command).await.unwrap();
        attempts.push(command.attempt);
    }
    let mut left = world.service().await;
    let mut right = world.service().await;
    let left_plan = prepare_plan(&left, &[attempts[0], attempts[3]], 0xe2).await;
    let right_plan = prepare_plan(&right, &[attempts[1], attempts[2]], 0xe3).await;
    let left_pid: i32 = left
        .client()
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0);
    let right_pid: i32 = right
        .client()
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0);
    world.admin.batch_execute("CREATE TABLE public.protocol_guard_gate(pid integer PRIMARY KEY,gate bigint NOT NULL);
      CREATE FUNCTION public.protocol_guard_barrier() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $gate$
      DECLARE wanted bigint;
      BEGIN
       IF NEW.table_name='session_funding' THEN
        SELECT gate INTO wanted FROM public.protocol_guard_gate WHERE pid=pg_backend_pid();
        IF wanted IS NOT NULL THEN PERFORM pg_advisory_xact_lock(wanted); END IF;
       END IF;
       RETURN NEW;
      END $gate$;
      CREATE TRIGGER protocol_guard_barrier AFTER INSERT ON finance.flow_lock FOR EACH ROW EXECUTE FUNCTION public.protocol_guard_barrier();").await.unwrap();
    let left_gate = 8_050_001_i64;
    let right_gate = 8_050_002_i64;
    world
        .admin
        .execute(
            "INSERT INTO public.protocol_guard_gate VALUES($1,$2),($3,$4)",
            &[&left_pid, &left_gate, &right_pid, &right_gate],
        )
        .await
        .unwrap();
    world
        .admin
        .query_one(
            "SELECT pg_advisory_lock($1),pg_advisory_lock($2)",
            &[&left_gate, &right_gate],
        )
        .await
        .unwrap();
    let left_task = tokio::spawn(async move { left.record_event(&left_plan).await });
    wait_for_database_lock(&world.admin, left_pid).await;
    let right_task = tokio::spawn(async move { right.record_event(&right_plan).await });
    wait_for_database_lock(&world.admin, right_pid).await;
    let unlocked = world
        .admin
        .query_one(
            "SELECT pg_advisory_unlock($1),pg_advisory_unlock($2)",
            &[&left_gate, &right_gate],
        )
        .await
        .unwrap();
    assert!(unlocked.get::<_, bool>(0) && unlocked.get::<_, bool>(1));
    tokio::time::timeout(Duration::from_secs(5), left_task)
        .await
        .unwrap()
        .unwrap()
        .expect("left crossing plan must commit without retry/deadlock");
    tokio::time::timeout(Duration::from_secs(5), right_task)
        .await
        .unwrap()
        .unwrap()
        .expect("right crossing plan must commit without retry/deadlock");
    assert_eq!(world.admin.query_one("SELECT count(*) FROM finance.provider_attempt WHERE dispatch_state='dispatch_prepared'",&[]).await.unwrap().get::<_,i64>(0),4);
    assert_eq!(world.natural(2130, None, None, None).await, 360);
    assert_eq!(world.line_sum(9100).await, 240);
    world.finish().await;
}
#[test]
fn crossing_sessions_and_shared_lots_use_one_global_lock_order() {
    support::run(async {
        crossing_plans(1).await;
        crossing_plans(2).await;
    });
}
