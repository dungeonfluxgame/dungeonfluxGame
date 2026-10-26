//! Roles, grants and row security of the hosted database (hosted runtime design 4.3, 4.4): the
//! catalog walk that keeps an unprotected granted table from ever reaching a runtime role, the
//! per-campaign scope of the game and story tables, the dispatcher identity and the re-fetch
//! queue. Each test reports SKIPPED without a scratch server.

mod runtime_support;

use runtime_support::*;
use tokio_postgres::Client;

#[test]
fn the_runtime_reaches_the_directory_only_through_its_functions() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (table, placement, _) = serving_table(&client, 1, 1).await;
        for sql in [
            "SELECT * FROM df_runtime.campaign_owners",
            "SELECT * FROM df_runtime.placement_secrets",
            "SELECT * FROM df_runtime.campaign_game_sessions",
            "SELECT * FROM df_meta.service_state",
            "UPDATE df_runtime.campaign_owners SET lease_until = 'infinity'",
            "DELETE FROM df_runtime.placements",
        ] {
            let refused = as_runtime(&client, &placement.token, sql, &[]).await;
            assert_eq!(
                sqlstate(&refused.expect_err(sql)).as_deref(),
                Some("42501"),
                "{sql}"
            );
        }
        for (role, sql) in [
            ("df_runner", "SELECT * FROM df_runtime.campaign_owners"),
            ("df_egress", "SELECT * FROM df_runtime.campaign_owners"),
            ("df_control", "SELECT * FROM df_runtime.placement_secrets"),
            (
                "df_control",
                "UPDATE df_runtime.campaign_owners SET lease_until = 'infinity'",
            ),
        ] {
            let refused = as_role(&client, role, sql, &[]).await;
            assert_eq!(
                sqlstate(&refused.expect_err(sql)).as_deref(),
                Some("42501"),
                "{role}: {sql}"
            );
        }
        let seen = as_role(
            &client,
            "df_control",
            "SELECT count(*) FROM df_runtime.campaign_owners WHERE tenant_id = $1",
            &[&table.tenant],
        )
        .await
        .expect("control reads the directory");
        assert_eq!(seen[0].get::<_, i64>(0), 1);
        scratch.finish().await;
    });
}

async fn two_serving_campaigns(client: &Client) -> ((Table, Placement), (Table, Placement)) {
    let first_table = table_for(1);
    let second_table = Table {
        tenant: first_table.tenant.clone(),
        campaign: vec![0xd1; 16],
    };
    let (_, first, _) = serving_table_of(client, &first_table, 1).await;
    let (_, second, _) = serving_table_of(client, &second_table, 2).await;
    add_game_session(client, &first_table, &[0x61; 16]).await;
    add_game_session(client, &second_table, &[0x62; 16]).await;
    ((first_table, first), (second_table, second))
}

async fn visible_game_sessions(client: &Client, token: &str) -> Vec<Vec<u8>> {
    as_runtime(
        client,
        token,
        "SELECT session_id FROM df_game.sessions ORDER BY session_id",
        &[],
    )
    .await
    .expect("sessions")
    .iter()
    .map(|row| row.get::<_, Vec<u8>>(0))
    .collect()
}

#[test]
fn a_runtime_sees_only_the_game_sessions_of_the_campaign_its_token_leases() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let ((_, first), (_, second)) = two_serving_campaigns(&client).await;
        assert_eq!(
            visible_game_sessions(&client, &first.token).await,
            [vec![0x61; 16]]
        );
        assert_eq!(
            visible_game_sessions(&client, &second.token).await,
            [vec![0x62; 16]]
        );
        assert!(
            visible_game_sessions(&client, "").await.is_empty(),
            "no token, no rows"
        );
        assert!(
            visible_game_sessions(&client, &"00".repeat(32))
                .await
                .is_empty(),
            "an unknown token, no rows"
        );
        scratch.finish().await;
    });
}

#[test]
fn a_runtime_cannot_write_a_game_row_for_another_campaigns_session() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let ((first_table, first), _) = two_serving_campaigns(&client).await;
        let foreign = as_runtime(
            &client,
            &first.token,
            "INSERT INTO df_game.checkpoints (tenant_id, session_id, recovery_epoch, in_epoch_sequence, run_id, \
             schema_version, codec_version, complete_envelope) \
             VALUES ($1::bytea, $2::bytea, 1, 1, $3::bytea, 1, 1, '\\x00')",
            &[&first_table.tenant, &vec![0x62_u8; 16], &vec![0x01_u8; 16]],
        )
        .await;
        assert_eq!(
            sqlstate(&foreign.expect_err("refused")).as_deref(),
            Some("42501")
        );
        let own = as_runtime(
            &client,
            &first.token,
            "INSERT INTO df_game.checkpoints (tenant_id, session_id, recovery_epoch, in_epoch_sequence, run_id, \
             schema_version, codec_version, complete_envelope) \
             VALUES ($1::bytea, $2::bytea, 1, 1, $3::bytea, 1, 1, '\\x00')",
            &[&first_table.tenant, &vec![0x61_u8; 16], &vec![0x01_u8; 16]],
        )
        .await;
        assert!(own.is_ok(), "its own session accepts the row");
        scratch.finish().await;
    });
}

#[test]
fn a_runtime_cannot_read_or_write_another_campaigns_checkpoint_operation_fact_or_intent() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let ((_, first), (second_table, _)) = two_serving_campaigns(&client).await;
        let session = vec![0x62_u8; 16];
        let principal = vec![0x71_u8; 16];
        let namespace = vec![0x72_u8; 16];
        let operation = vec![0x73_u8; 16];
        let run_id = vec![0x01_u8; 16];
        let operation_foreign = vec![0x76_u8; 16];
        let fingerprint_foreign = vec![0x77_u8; 32];
        let effect_foreign = vec![0x78_u8; 16];
        add_game_rows(&client, &second_table, &session).await;

        for table in ["checkpoints", "operations", "facts", "intents"] {
            let sql = format!("SELECT count(*) FROM df_game.{table} WHERE session_id = $1");
            let rows = as_runtime(&client, &first.token, &sql, &[&session])
                .await
                .expect("foreign row is hidden");
            assert_eq!(rows[0].get::<_, i64>(0), 0, "{table}");
        }

        let refused = as_runtime(
            &client,
            &first.token,
            "INSERT INTO df_game.checkpoints (tenant_id, session_id, recovery_epoch, \
             in_epoch_sequence, run_id, schema_version, codec_version, complete_envelope) \
             VALUES ($1::bytea, $2::bytea, 1, 2, $3::bytea, 1, 1, '\\x00')",
            &[&second_table.tenant, &session, &run_id],
        )
        .await;
        assert_eq!(
            sqlstate(&refused.expect_err("foreign checkpoint")).as_deref(),
            Some("42501")
        );
        let refused = as_runtime(
            &client,
            &first.token,
            "INSERT INTO df_game.operations (tenant_id, session_id, principal_id, command_namespace, \
             recovery_epoch, operation_id, fingerprint_version, canonical_fingerprint, committed_epoch, \
             committed_sequence, receipt_version, receipt) VALUES ($1::bytea, $2::bytea, $3::bytea, \
             $4::bytea, 1, $5::bytea, 1, $6, 1, 1, 1, NULL)",
            &[
                &second_table.tenant,
                &session,
                &principal,
                &namespace,
                &operation_foreign,
                &fingerprint_foreign,
            ],
        )
        .await;
        assert_eq!(
            sqlstate(&refused.expect_err("foreign operation")).as_deref(),
            Some("42501")
        );
        let refused = as_runtime(
            &client,
            &first.token,
            "INSERT INTO df_game.facts (tenant_id, session_id, committed_epoch, committed_sequence, \
             ordinal, fact_version, fact) VALUES ($1::bytea, $2::bytea, 1, 1, 1, 1, '\\x02')",
            &[&second_table.tenant, &session],
        )
        .await;
        assert_eq!(
            sqlstate(&refused.expect_err("foreign fact")).as_deref(),
            Some("42501")
        );
        let refused = as_runtime(
            &client,
            &first.token,
            "INSERT INTO df_game.intents (tenant_id, session_id, principal_id, command_namespace, \
             recovery_epoch, operation_id, slot, effect_id, run_id, committed_epoch, committed_sequence, \
             process_generation, execution_mode, intent_kind, intent_version, intent) \
             VALUES ($1::bytea, $2::bytea, $3::bytea, $4::bytea, 1, $5::bytea, 1, $6::bytea, \
             $7::bytea, 1, 1, 1, 1, 1, 1, '\\x02')",
            &[
                &second_table.tenant,
                &session,
                &principal,
                &namespace,
                &operation,
                &effect_foreign,
                &run_id,
            ],
        )
        .await;
        assert_eq!(
            sqlstate(&refused.expect_err("foreign intent")).as_deref(),
            Some("42501")
        );
        scratch.finish().await;
    });
}

#[test]
fn a_superseded_or_expired_token_sees_no_game_rows() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let ((first_table, first), _) = two_serving_campaigns(&client).await;
        assert_eq!(visible_game_sessions(&client, &first.token).await.len(), 1);
        expire_leases(&client, &first_table).await;
        assert!(
            visible_game_sessions(&client, &first.token)
                .await
                .is_empty(),
            "expired lease"
        );
        let newer = placement_for(9);
        place(&client, &first_table, &newer)
            .await
            .expect("superseded");
        assert!(
            visible_game_sessions(&client, &first.token)
                .await
                .is_empty(),
            "superseded token"
        );
        assert_eq!(
            visible_game_sessions(&client, &newer.token).await.len(),
            1,
            "the placing token can claim"
        );
        scratch.finish().await;
    });
}

#[test]
fn a_runtime_cannot_rewrite_its_own_fence_or_lease_but_can_advance_the_sequence() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let ((_, first), _) = two_serving_campaigns(&client).await;
        for sql in [
            "UPDATE df_game.sessions SET lease_until = 'infinity'",
            "UPDATE df_game.sessions SET owner_fence = '\\x0000000000000000000000000000ffff'",
        ] {
            let refused = as_runtime(&client, &first.token, sql, &[]).await;
            assert_eq!(
                sqlstate(&refused.expect_err(sql)).as_deref(),
                Some("42501"),
                "{sql}"
            );
        }
        let updated = as_runtime(
            &client,
            &first.token,
            "WITH moved AS (UPDATE df_game.sessions SET in_epoch_sequence = in_epoch_sequence + 1 RETURNING 1) \
             SELECT count(*) FROM moved",
            &[],
        )
        .await
        .expect("advance");
        assert_eq!(updated[0].get::<_, i64>(0), 1, "only its own session moves");
        scratch.finish().await;
    });
}

#[test]
fn a_runtime_reads_only_the_table_sessions_and_segments_of_its_own_campaign() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let ((first_table, first), (second_table, second)) = two_serving_campaigns(&client).await;
        for (index, table) in [&first_table, &second_table].into_iter().enumerate() {
            let session = vec![0x70 + index as u8; 16];
            client
                .execute(
                    "INSERT INTO df_runtime.table_sessions (tenant_id, campaign_id, session_id, phase, created_at) \
                     VALUES ($1, $2, $3, 'active', clock_timestamp())",
                    &[&table.tenant, &table.campaign, &session],
                )
                .await
                .expect("table session");
            client
                .execute(
                    "INSERT INTO df_runtime.meter_segments (tenant_id, session_id, opened_at) \
                     VALUES ($1, $2, clock_timestamp())",
                    &[&table.tenant, &session],
                )
                .await
                .expect("segment");
        }
        for (token, expected) in [(&first.token, 0x70_u8), (&second.token, 0x71_u8)] {
            let sessions = as_runtime(
                &client,
                token,
                "SELECT session_id FROM df_runtime.table_sessions",
                &[],
            )
            .await
            .expect("sessions");
            assert_eq!(
                sessions
                    .iter()
                    .map(|row| row.get::<_, Vec<u8>>(0))
                    .collect::<Vec<_>>(),
                [vec![expected; 16]]
            );
            let segments = as_runtime(
                &client,
                token,
                "SELECT session_id FROM df_runtime.meter_segments",
                &[],
            )
            .await
            .expect("segments");
            assert_eq!(
                segments
                    .iter()
                    .map(|row| row.get::<_, Vec<u8>>(0))
                    .collect::<Vec<_>>(),
                [vec![expected; 16]]
            );
        }
        let written = as_runtime(
            &client,
            &first.token,
            "DELETE FROM df_runtime.meter_segments",
            &[],
        )
        .await;
        assert_eq!(
            sqlstate(&written.expect_err("read only")).as_deref(),
            Some("42501")
        );
        scratch.finish().await;
    });
}

async fn add_story_event(
    client: &Client,
    token: &str,
    table: &Table,
    key: &str,
) -> Result<(), tokio_postgres::Error> {
    as_runtime(
        client,
        token,
        "INSERT INTO df_story.story_memory (run_id, natural_key, source, kind, visibility, body, body_digest, \
         tenant_id, campaign_id) VALUES ('run', $3, 'test', 'narration', 'public', 'a body', $4::bytea, $1::bytea, $2::bytea)",
        &[&table.tenant, &table.campaign, &key, &vec![0x33_u8; 32]],
    )
    .await
    .map(|_| ())
}

#[test]
fn story_rows_are_scoped_to_the_leased_campaign() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let ((first_table, first), (second_table, second)) = two_serving_campaigns(&client).await;
        add_story_event(&client, &first.token, &first_table, "one")
            .await
            .expect("own campaign");
        add_story_event(&client, &second.token, &second_table, "two")
            .await
            .expect("own campaign");
        let cross = add_story_event(&client, &first.token, &second_table, "three").await;
        assert_eq!(
            sqlstate(&cross.expect_err("refused")).as_deref(),
            Some("42501")
        );
        let visible = as_runtime(
            &client,
            &first.token,
            "SELECT natural_key FROM df_story.story_memory",
            &[],
        )
        .await
        .expect("read");
        assert_eq!(
            visible
                .iter()
                .map(|row| row.get::<_, String>(0))
                .collect::<Vec<_>>(),
            ["one"]
        );
        scratch.finish().await;
    });
}

#[test]
fn the_append_only_escape_needs_both_the_setting_and_the_maintenance_role() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let ((first_table, first), _) = two_serving_campaigns(&client).await;
        add_story_event(&client, &first.token, &first_table, "one")
            .await
            .expect("event");
        client
            .batch_execute(&format!(
                "SET ROLE df_campaign_runtime; \
                 SELECT set_config('df.placement_token', '{}', false); \
                 SELECT set_config('df_story.maintenance', 'on', false)",
                first.token
            ))
            .await
            .expect("runtime session with the variable set");
        let runtime_edit = client
            .execute("UPDATE df_story.story_memory SET body = 'rewritten'", &[])
            .await;
        client
            .batch_execute("RESET df_story.maintenance; RESET df.placement_token; RESET ROLE")
            .await
            .expect("reset");
        let runtime_edit =
            runtime_edit.expect_err("a runtime that sets the variable cannot rewrite history");
        assert!(
            runtime_edit
                .as_db_error()
                .is_some_and(|db| db.message().contains("immutable")),
            "{runtime_edit:?}"
        );
        let refused = client
            .batch_execute("SET ROLE df_story_maintenance; DELETE FROM df_story.story_memory")
            .await;
        client
            .batch_execute("RESET ROLE")
            .await
            .expect("reset role");
        let refused = refused.expect_err("append-only");
        assert!(
            refused
                .as_db_error()
                .is_some_and(|db| db.message().contains("append-only")),
            "{refused:?}"
        );
        let allowed = client
            .batch_execute(
                "SET ROLE df_story_maintenance; SET df_story.maintenance = 'on'; \
                 DELETE FROM df_story.story_memory; RESET df_story.maintenance; RESET ROLE",
            )
            .await;
        assert!(
            allowed.is_ok(),
            "the maintenance role may erase inside an explicit maintenance transaction"
        );
        let remaining: i64 = client
            .query_one("SELECT count(*) FROM df_story.story_memory", &[])
            .await
            .expect("count")
            .get(0);
        assert_eq!(remaining, 0);
        scratch.finish().await;
    });
}

const DISPATCHER_A: &str = "da";
const DISPATCHER_B: &str = "db";

fn dispatcher_token(tag: &str) -> String {
    tag.repeat(32)
}

async fn register_dispatcher(client: &Client, worker: &[u8], instance: &[u8], token: &str) {
    as_role(
        client,
        "df_egress",
        "SELECT df_runtime.register_dispatcher($1, $2, $3)",
        &[&worker, &instance, &digest_of_token(token)],
    )
    .await
    .expect("register dispatcher");
}

async fn current_dispatcher(client: &Client, token: &str) -> Option<Vec<u8>> {
    as_dispatcher(client, token, "SELECT df_runtime.current_dispatcher()", &[])
        .await
        .expect("current dispatcher")[0]
        .get(0)
}

#[test]
fn a_dispatcher_instance_is_current_only_with_its_token_and_a_fresh_heartbeat() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let worker = vec![0x77; 16];
        register_worker(&client, &worker).await;
        let token = dispatcher_token(DISPATCHER_A);
        register_dispatcher(&client, &worker, &[0xd1; 16], &token).await;
        assert_eq!(
            current_dispatcher(&client, &token).await,
            Some(vec![0xd1; 16])
        );
        assert_eq!(
            current_dispatcher(&client, &dispatcher_token(DISPATCHER_B)).await,
            None
        );
        assert_eq!(current_dispatcher(&client, "").await, None);
        client
            .execute("UPDATE df_runtime.dispatcher_instances SET last_seen_at = clock_timestamp() - interval '31 seconds'", &[])
            .await
            .expect("age");
        assert_eq!(
            current_dispatcher(&client, &token).await,
            None,
            "silent for 31 seconds"
        );
        let revived = as_dispatcher(
            &client,
            &token,
            "SELECT df_runtime.dispatcher_heartbeat()",
            &[],
        )
        .await
        .expect("heartbeat");
        assert!(
            !revived[0].get::<_, bool>(0),
            "a stale instance is not revived by a heartbeat"
        );
        scratch.finish().await;
    });
}

#[test]
fn a_heartbeat_keeps_an_instance_current_and_ending_it_removes_it() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let worker = vec![0x77; 16];
        register_worker(&client, &worker).await;
        let token = dispatcher_token(DISPATCHER_A);
        register_dispatcher(&client, &worker, &[0xd1; 16], &token).await;
        client
            .execute("UPDATE df_runtime.dispatcher_instances SET last_seen_at = clock_timestamp() - interval '20 seconds'", &[])
            .await
            .expect("age");
        let beat = as_dispatcher(
            &client,
            &token,
            "SELECT df_runtime.dispatcher_heartbeat()",
            &[],
        )
        .await
        .expect("heartbeat");
        assert!(beat[0].get::<_, bool>(0));
        client
            .execute("UPDATE df_runtime.dispatcher_instances SET last_seen_at = last_seen_at - interval '20 seconds'", &[])
            .await
            .expect("age again");
        assert!(
            current_dispatcher(&client, &token).await.is_some(),
            "the heartbeat reset the clock"
        );
        let ended = as_dispatcher(&client, &token, "SELECT df_runtime.end_dispatcher()", &[])
            .await
            .expect("end");
        assert!(ended[0].get::<_, bool>(0));
        assert_eq!(current_dispatcher(&client, &token).await, None);
        scratch.finish().await;
    });
}

const REQUEST_REFETCH: &str = "INSERT INTO df_runtime.refetch_requests AS r \
    (attempt_id, requested_at, request_count, attempts, claimed_by_instance, claimed_until, outcome, finished_at) \
    VALUES ($1, clock_timestamp(), 1, 0, NULL, NULL, NULL, NULL) \
    ON CONFLICT (attempt_id) DO UPDATE SET requested_at = clock_timestamp(), \
    request_count = r.request_count + 1, attempts = 0, claimed_by_instance = NULL, claimed_until = NULL, \
    outcome = NULL, finished_at = NULL WHERE r.outcome IS NOT NULL";

const CLAIM_REFETCH: &str = "UPDATE df_runtime.refetch_requests SET \
    claimed_by_instance = df_runtime.current_dispatcher(), claimed_until = clock_timestamp() + interval '60 seconds', \
    attempts = attempts + 1 WHERE attempt_id = $1 AND outcome IS NULL AND attempts < 5 \
    AND (claimed_until IS NULL OR claimed_until < clock_timestamp()) RETURNING attempt_id";

const FINISH_REFETCH: &str = "UPDATE df_runtime.refetch_requests SET outcome = 'staged', finished_at = clock_timestamp() \
    WHERE attempt_id = $1 RETURNING attempt_id";

async fn two_dispatchers(client: &Client) -> (String, String) {
    let worker = vec![0x77; 16];
    register_worker(client, &worker).await;
    let (a, b) = (
        dispatcher_token(DISPATCHER_A),
        dispatcher_token(DISPATCHER_B),
    );
    register_dispatcher(client, &worker, &[0xd1; 16], &a).await;
    register_dispatcher(client, &worker, &[0xd2; 16], &b).await;
    (a, b)
}

#[test]
fn a_refetch_request_is_claimed_by_one_instance_and_only_its_holder_may_finish_it() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (a, b) = two_dispatchers(&client).await;
        let attempt = vec![0xaa_u8; 16];
        as_role(&client, "df_control", REQUEST_REFETCH, &[&attempt])
            .await
            .expect("request");
        let first = as_dispatcher(&client, &a, CLAIM_REFETCH, &[&attempt])
            .await
            .expect("claim a");
        assert_eq!(first.len(), 1);
        let second = as_dispatcher(&client, &b, CLAIM_REFETCH, &[&attempt])
            .await
            .expect("claim b");
        assert!(second.is_empty(), "a held request is not claimable");
        let stolen = as_dispatcher(&client, &b, FINISH_REFETCH, &[&attempt])
            .await
            .expect("finish by b");
        assert!(stolen.is_empty(), "only the holder finishes");
        let finished = as_dispatcher(&client, &a, FINISH_REFETCH, &[&attempt])
            .await
            .expect("finish by a");
        assert_eq!(finished.len(), 1);
        scratch.finish().await;
    });
}

#[test]
fn an_expired_claim_can_be_taken_by_another_instance() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (a, b) = two_dispatchers(&client).await;
        let attempt = vec![0xaa_u8; 16];
        as_role(&client, "df_control", REQUEST_REFETCH, &[&attempt])
            .await
            .expect("request");
        as_dispatcher(&client, &a, CLAIM_REFETCH, &[&attempt])
            .await
            .expect("claim a");
        client
            .execute("UPDATE df_runtime.refetch_requests SET claimed_until = clock_timestamp() - interval '1 second'", &[])
            .await
            .expect("expire");
        let taken = as_dispatcher(&client, &b, CLAIM_REFETCH, &[&attempt])
            .await
            .expect("claim b");
        assert_eq!(taken.len(), 1);
        scratch.finish().await;
    });
}

#[test]
fn a_finished_refetch_request_is_never_claimed_again_and_a_new_request_resets_it() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (a, _) = two_dispatchers(&client).await;
        let attempt = vec![0xaa_u8; 16];
        as_role(&client, "df_control", REQUEST_REFETCH, &[&attempt])
            .await
            .expect("request");
        as_dispatcher(&client, &a, CLAIM_REFETCH, &[&attempt])
            .await
            .expect("claim");
        as_dispatcher(&client, &a, FINISH_REFETCH, &[&attempt])
            .await
            .expect("finish");
        let again = as_dispatcher(&client, &a, CLAIM_REFETCH, &[&attempt])
            .await
            .expect("claim again");
        assert!(again.is_empty(), "a finished request is not claimable");
        as_role(&client, "df_control", REQUEST_REFETCH, &[&attempt])
            .await
            .expect("request again");
        let reset = client
            .query_one("SELECT request_count, attempts, outcome IS NULL, claimed_by_instance IS NULL FROM df_runtime.refetch_requests", &[])
            .await
            .expect("row");
        assert_eq!(
            (
                reset.get::<_, i32>(0),
                reset.get::<_, i32>(1),
                reset.get::<_, bool>(2),
                reset.get::<_, bool>(3)
            ),
            (2, 0, true, true)
        );
        let claimable = as_dispatcher(&client, &a, CLAIM_REFETCH, &[&attempt])
            .await
            .expect("claim after reset");
        assert_eq!(claimable.len(), 1);
        scratch.finish().await;
    });
}

#[test]
fn a_request_for_an_open_refetch_is_left_alone() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (a, _) = two_dispatchers(&client).await;
        let attempt = vec![0xaa_u8; 16];
        as_role(&client, "df_control", REQUEST_REFETCH, &[&attempt])
            .await
            .expect("request");
        as_dispatcher(&client, &a, CLAIM_REFETCH, &[&attempt])
            .await
            .expect("claim");
        as_role(&client, "df_control", REQUEST_REFETCH, &[&attempt])
            .await
            .expect("repeat request");
        let row = client
            .query_one(
                "SELECT request_count, attempts FROM df_runtime.refetch_requests",
                &[],
            )
            .await
            .expect("row");
        assert_eq!((row.get::<_, i32>(0), row.get::<_, i32>(1)), (1, 1));
        scratch.finish().await;
    });
}

#[test]
fn a_dispatcher_without_a_live_registration_claims_nothing() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        two_dispatchers(&client).await;
        let attempt = vec![0xaa_u8; 16];
        as_role(&client, "df_control", REQUEST_REFETCH, &[&attempt])
            .await
            .expect("request");
        let claimed = as_dispatcher(&client, &"ee".repeat(32), CLAIM_REFETCH, &[&attempt]).await;
        let refused = claimed.expect_err("an unknown token cannot take a claim");
        assert_eq!(sqlstate(&refused).as_deref(), Some("42501"));
        let held: i64 = client
            .query_one("SELECT count(*) FROM df_runtime.refetch_requests WHERE claimed_by_instance IS NOT NULL", &[])
            .await
            .expect("count")
            .get(0);
        assert_eq!(held, 0);
        scratch.finish().await;
    });
}
