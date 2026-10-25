//! The offline importer's role (decision 19): scoped to ONE campaign by an import job token,
//! unable to write a fence or a lease, blind to the directory. Each test reports SKIPPED
//! without a scratch server.

mod runtime_support;

use runtime_support::*;

const JOB_TOKEN: &str = "5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a";
const OTHER_TOKEN: &str = "6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b6b";

/// Two campaigns of one tenant.
fn two_campaigns() -> (Table, Table) {
    let first = table_for(1);
    let second = Table {
        tenant: first.tenant.clone(),
        campaign: vec![0xd1; 16],
    };
    (first, second)
}

const IMPORT_SESSION: &str = "SELECT df_runtime.import_game_session($1, $2, 1, 0)";

const INSERT_CHECKPOINT: &str = "INSERT INTO df_game.checkpoints (tenant_id, session_id, \
     recovery_epoch, in_epoch_sequence, run_id, schema_version, codec_version, complete_envelope) \
     VALUES ($1::bytea, $2::bytea, 1, 1, $3::bytea, 1, 1, '\\x00')";

const INSERT_STORY: &str = "INSERT INTO df_story.story_memory (run_id, natural_key, source, kind, \
     visibility, body, body_digest, tenant_id, campaign_id) \
     VALUES ('run', 'k', 'test', 'narration', 'public', 'a body', $3::bytea, $1::bytea, $2::bytea)";

#[test]
fn an_importer_with_a_job_creates_a_session_and_writes_the_rows_of_that_campaign() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (first, _) = two_campaigns();
        open_import(&client, &first, JOB_TOKEN, 600).await;
        let session = vec![0x71_u8; 16];
        let run_id = vec![0x01_u8; 16];
        let created = as_importer(&client, JOB_TOKEN, IMPORT_SESSION, &[&session, &run_id])
            .await
            .expect("import_game_session");
        assert!(created[0].get::<_, bool>(0));
        as_importer(
            &client,
            JOB_TOKEN,
            INSERT_CHECKPOINT,
            &[&first.tenant, &session, &run_id],
        )
        .await
        .expect("a checkpoint of the imported session");
        as_importer(
            &client,
            JOB_TOKEN,
            INSERT_STORY,
            &[&first.tenant, &first.campaign, &vec![0x33_u8; 32]],
        )
        .await
        .expect("a story row of the imported campaign");
        let read = as_importer(
            &client,
            JOB_TOKEN,
            "SELECT session_id FROM df_game.checkpoints",
            &[],
        )
        .await
        .expect("read back");
        assert_eq!(read.len(), 1);
        let unclaimed: bool = client
            .query_one(
                "SELECT lease_until = '-infinity' AND owner_fence <> '\\x00000000000000000000000000000000' \
                 FROM df_game.sessions",
                &[],
            )
            .await
            .expect("session")
            .get(0);
        assert!(
            unclaimed,
            "an import writes an expired lease, never a live owner fence"
        );
        let mapped: i64 = client
            .query_one(
                "SELECT count(*) FROM df_runtime.campaign_game_sessions WHERE campaign_id = $1",
                &[&first.campaign],
            )
            .await
            .expect("mapping")
            .get(0);
        assert_eq!(mapped, 1);
        scratch.finish().await;
    });
}

#[test]
fn an_importer_is_refused_on_another_campaign() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (first, second) = two_campaigns();
        let other_session = vec![0x72_u8; 16];
        add_game_session(&client, &second, &other_session).await;
        add_game_rows(&client, &second, &other_session).await;
        let operation_foreign = vec![0x76_u8; 16];
        let fingerprint_foreign = vec![0x77_u8; 32];
        let effect_foreign = vec![0x78_u8; 16];
        open_import(&client, &first, JOB_TOKEN, 600).await;
        let principal = vec![0x71_u8; 16];
        let namespace = vec![0x72_u8; 16];
        let operation = vec![0x73_u8; 16];
        let run_id = vec![0x01_u8; 16];
        let foreign_checkpoint = as_importer(
            &client,
            JOB_TOKEN,
            INSERT_CHECKPOINT,
            &[&first.tenant, &other_session, &run_id],
        )
        .await;
        assert_eq!(
            sqlstate(&foreign_checkpoint.expect_err("refused")).as_deref(),
            Some("42501")
        );
        for table in ["checkpoints", "operations", "facts", "intents"] {
            let sql = format!("SELECT count(*) FROM df_game.{table} WHERE session_id = $1");
            let rows = as_importer(&client, JOB_TOKEN, &sql, &[&other_session])
                .await
                .expect("foreign row is hidden");
            assert_eq!(rows[0].get::<_, i64>(0), 0, "{table}");
        }
        let foreign_operation = as_importer(
            &client,
            JOB_TOKEN,
            "INSERT INTO df_game.operations (tenant_id, session_id, principal_id, command_namespace, \
             recovery_epoch, operation_id, fingerprint_version, canonical_fingerprint, committed_epoch, \
             committed_sequence, receipt_version, receipt) VALUES ($1::bytea, $2::bytea, $3::bytea, \
             $4::bytea, 1, $5::bytea, 1, $6, 1, 1, 1, NULL)",
            &[
                &second.tenant,
                &other_session,
                &principal,
                &namespace,
                &operation_foreign,
                &fingerprint_foreign,
            ],
        )
        .await;
        assert_eq!(
            sqlstate(&foreign_operation.expect_err("refused")).as_deref(),
            Some("42501")
        );
        let foreign_fact = as_importer(
            &client,
            JOB_TOKEN,
            "INSERT INTO df_game.facts (tenant_id, session_id, committed_epoch, committed_sequence, \
             ordinal, fact_version, fact) VALUES ($1::bytea, $2::bytea, 1, 1, 1, 1, '\\x02')",
            &[&second.tenant, &other_session],
        )
        .await;
        assert_eq!(
            sqlstate(&foreign_fact.expect_err("refused")).as_deref(),
            Some("42501")
        );
        let foreign_intent = as_importer(
            &client,
            JOB_TOKEN,
            "INSERT INTO df_game.intents (tenant_id, session_id, principal_id, command_namespace, \
             recovery_epoch, operation_id, slot, effect_id, run_id, committed_epoch, committed_sequence, \
             process_generation, execution_mode, intent_kind, intent_version, intent) \
             VALUES ($1::bytea, $2::bytea, $3::bytea, $4::bytea, 1, $5::bytea, 1, $6::bytea, \
             $7::bytea, 1, 1, 1, 1, 1, 1, '\\x02')",
            &[
                &second.tenant,
                &other_session,
                &principal,
                &namespace,
                &operation,
                &effect_foreign,
                &run_id,
            ],
        )
        .await;
        assert_eq!(
            sqlstate(&foreign_intent.expect_err("refused")).as_deref(),
            Some("42501")
        );
        let foreign_story = as_importer(
            &client,
            JOB_TOKEN,
            INSERT_STORY,
            &[&second.tenant, &second.campaign, &vec![0x33_u8; 32]],
        )
        .await;
        assert_eq!(
            sqlstate(&foreign_story.expect_err("refused")).as_deref(),
            Some("42501")
        );
        let taken = as_importer(
            &client,
            JOB_TOKEN,
            IMPORT_SESSION,
            &[&other_session, &run_id],
        )
        .await
        .expect("import_game_session");
        assert!(
            !taken[0].get::<_, bool>(0),
            "a session id already mapped to another campaign is not importable"
        );
        let visible = as_importer(
            &client,
            JOB_TOKEN,
            "SELECT session_id FROM df_game.sessions",
            &[],
        )
        .await
        .expect("read");
        assert!(
            visible.is_empty(),
            "the other campaign's session is invisible"
        );
        scratch.finish().await;
    });
}

#[test]
fn an_importer_without_a_job_token_or_with_a_wrong_closed_or_expired_one_is_refused() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (first, _) = two_campaigns();
        open_import(&client, &first, JOB_TOKEN, 600).await;
        let run_id = vec![0x01_u8; 16];
        let session = vec![0x71_u8; 16];
        for (case, token) in [("none", ""), ("wrong", OTHER_TOKEN)] {
            let story = as_importer(
                &client,
                token,
                INSERT_STORY,
                &[&first.tenant, &first.campaign, &vec![0x33_u8; 32]],
            )
            .await;
            assert_eq!(
                sqlstate(&story.expect_err(case)).as_deref(),
                Some("42501"),
                "{case}"
            );
            let created = as_importer(&client, token, IMPORT_SESSION, &[&session, &run_id])
                .await
                .expect("call");
            assert!(!created[0].get::<_, bool>(0), "{case}");
        }
        client
            .execute(
                "UPDATE df_runtime.import_jobs SET created_at = created_at - interval '2 hours', \
                 expires_at = clock_timestamp() - interval '1 hour'",
                &[],
            )
            .await
            .expect("expire the job");
        let expired = as_importer(&client, JOB_TOKEN, IMPORT_SESSION, &[&session, &run_id])
            .await
            .expect("call");
        assert!(!expired[0].get::<_, bool>(0), "expired");
        open_import(&client, &first, OTHER_TOKEN, 600).await;
        let closed: bool = client
            .query_one(
                "SELECT df_runtime.close_import($1)",
                &[&digest_of_token(OTHER_TOKEN)],
            )
            .await
            .expect("close")
            .get(0);
        assert!(closed);
        let after_close = as_importer(&client, OTHER_TOKEN, IMPORT_SESSION, &[&session, &run_id])
            .await
            .expect("call");
        assert!(!after_close[0].get::<_, bool>(0), "closed");
        scratch.finish().await;
    });
}

#[test]
fn an_importer_cannot_write_a_fence_or_a_lease_or_touch_the_directory() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (first, _) = two_campaigns();
        let session = vec![0x71_u8; 16];
        add_game_session(&client, &first, &session).await;
        open_import(&client, &first, JOB_TOKEN, 600).await;
        for sql in [
            "INSERT INTO df_game.sessions (tenant_id, session_id, run_id, recovery_epoch, \
             in_epoch_sequence, owner_fence, lease_until) VALUES ('\\x00', '\\x00', '\\x00', 1, 0, '\\x00', now())",
            "UPDATE df_game.sessions SET lease_until = 'infinity'",
            "UPDATE df_game.sessions SET owner_fence = '\\x0000000000000000000000000000ffff'",
            "DELETE FROM df_game.checkpoints",
            "UPDATE df_story.story_memory SET body = 'x'",
            "SELECT * FROM df_runtime.campaign_owners",
            "SELECT * FROM df_runtime.import_jobs",
            "SELECT * FROM df_runtime.campaign_game_sessions",
            "SELECT * FROM df_runtime.placement_secrets",
            "SELECT * FROM df_meta.service_state",
        ] {
            let refused = as_importer(&client, JOB_TOKEN, sql, &[]).await;
            assert_eq!(
                sqlstate(&refused.expect_err(sql)).as_deref(),
                Some("42501"),
                "{sql}"
            );
        }
        let operator_only = as_importer(
            &client,
            JOB_TOKEN,
            "SELECT df_runtime.open_import($1, $2, $3, 60)",
            &[
                &first.tenant,
                &first.campaign,
                &digest_of_token(OTHER_TOKEN),
            ],
        )
        .await;
        assert_eq!(
            sqlstate(&operator_only.expect_err("operator only")).as_deref(),
            Some("42501")
        );
        scratch.finish().await;
    });
}
