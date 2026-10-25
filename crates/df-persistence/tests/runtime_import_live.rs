//! An import never overlaps a live owner (decision 19), and unclosed import jobs are purged.
//! Each test reports SKIPPED without a scratch server.

mod runtime_support;

use runtime_support::*;
use tokio_postgres::Client;

const JOB_TOKEN: &str = "5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a";

const INSERT_STORY: &str = "INSERT INTO df_story.story_memory (run_id, natural_key, source, kind, \
     visibility, body, body_digest, tenant_id, campaign_id) \
     VALUES ('run', 'k', 'test', 'narration', 'public', 'a body', $3::bytea, $1::bytea, $2::bytea)";

async fn open(client: &Client, table: &Table) -> Result<(), tokio_postgres::Error> {
    client
        .execute(
            "SELECT df_runtime.open_import($1, $2, $3, 600)",
            &[&table.tenant, &table.campaign, &digest_of_token(JOB_TOKEN)],
        )
        .await
        .map(|_| ())
}

async fn close(client: &Client) {
    client
        .execute(
            "SELECT df_runtime.close_import($1)",
            &[&digest_of_token(JOB_TOKEN)],
        )
        .await
        .expect("close");
}

#[test]
fn a_job_cannot_open_on_a_campaign_with_a_live_owner() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        // Serving within its lease.
        let (serving, _, _) = serving_table(&client, 1, 1).await;
        assert_eq!(
            sqlstate(&open(&client, &serving).await.expect_err("live")).as_deref(),
            Some("DF007")
        );
        // Draining within its lease.
        client
            .execute(
                "UPDATE df_runtime.campaign_owners SET state = 'draining'",
                &[],
            )
            .await
            .expect("drain");
        assert_eq!(
            sqlstate(&open(&client, &serving).await.expect_err("draining")).as_deref(),
            Some("DF007")
        );
        // Placing within its placing lease.
        let placing = table_for(2);
        let placement = placement_for(2);
        register_worker(&client, &placement.worker).await;
        request_placement(&client, &placing).await;
        place(&client, &placing, &placement).await.expect("placed");
        assert_eq!(
            sqlstate(&open(&client, &placing).await.expect_err("placing")).as_deref(),
            Some("DF007")
        );
        // Once the leases are over, the same campaigns accept a job.
        expire_leases(&client, &serving).await;
        expire_leases(&client, &placing).await;
        open(&client, &serving).await.expect("serving but expired");
        close(&client).await;
        open(&client, &placing).await.expect("placing but expired");
        close(&client).await;
        // And a campaign nobody ever placed.
        open(&client, &table_for(3)).await.expect("never placed");
        scratch.finish().await;
    });
}

#[test]
fn an_importer_sees_nothing_once_its_campaign_has_a_live_owner() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let table = table_for(1);
        open(&client, &table)
            .await
            .expect("job on a released campaign");
        let session = vec![0x71_u8; 16];
        let created = as_importer(
            &client,
            JOB_TOKEN,
            "SELECT df_runtime.import_game_session($1, $2, 1, 0)",
            &[&session, &vec![0x01_u8; 16]],
        )
        .await
        .expect("import");
        assert!(created[0].get::<_, bool>(0), "no owner yet");
        // An owner comes alive anyway (a restored directory row, for instance).
        client
            .execute(
                "UPDATE df_runtime.campaign_owners SET state = 'serving', placement_id = $1, \
                 lease_until = clock_timestamp() + interval '30 seconds'",
                &[&vec![0x09_u8; 16]],
            )
            .await
            .expect("live owner");
        let scoped = as_importer(
            &client,
            JOB_TOKEN,
            "SELECT count(*) FROM df_runtime.importing_campaign()",
            &[],
        )
        .await
        .expect("importing_campaign");
        assert_eq!(
            scoped[0].get::<_, i64>(0),
            0,
            "no campaign while an owner is live"
        );
        let story = as_importer(
            &client,
            JOB_TOKEN,
            INSERT_STORY,
            &[&table.tenant, &table.campaign, &vec![0x33_u8; 32]],
        )
        .await;
        assert_eq!(
            sqlstate(&story.expect_err("refused")).as_deref(),
            Some("42501")
        );
        let another = as_importer(
            &client,
            JOB_TOKEN,
            "SELECT df_runtime.import_game_session($1, $2, 1, 0)",
            &[&vec![0x72_u8; 16], &vec![0x01_u8; 16]],
        )
        .await
        .expect("import");
        assert!(
            !another[0].get::<_, bool>(0),
            "no session while an owner is live"
        );
        let visible = as_importer(
            &client,
            JOB_TOKEN,
            "SELECT session_id FROM df_game.sessions",
            &[],
        )
        .await
        .expect("read");
        assert!(visible.is_empty());
        scratch.finish().await;
    });
}

#[test]
fn a_placement_is_refused_while_an_unexpired_import_job_exists() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let table = table_for(1);
        let placement = placement_for(1);
        register_worker(&client, &placement.worker).await;
        request_placement(&client, &table).await;
        open(&client, &table).await.expect("job");
        assert!(
            place(&client, &table, &placement).await.is_none(),
            "an import is open"
        );
        close(&client).await;
        assert!(
            place(&client, &table, &placement).await.is_some(),
            "after the job closed"
        );
        // An expired job no longer blocks either.
        expire_leases(&client, &table).await;
        client
            .execute(
                "SELECT df_runtime.release_after_exit($1, $2, 1, 0, $3)",
                &[&table.tenant, &table.campaign, &placement.id],
            )
            .await
            .expect("release");
        client
            .execute(
                "INSERT INTO df_runtime.import_jobs VALUES ($1, $2, $3, clock_timestamp() - interval '2 hours', \
                 clock_timestamp() - interval '1 hour')",
                &[&digest_of_token(JOB_TOKEN), &table.tenant, &table.campaign],
            )
            .await
            .expect("an expired job");
        assert!(place(&client, &table, &placement_for(2)).await.is_some());
        scratch.finish().await;
    });
}

#[test]
fn opening_a_job_waits_for_a_placement_in_flight_and_then_refuses_the_live_campaign() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let table = table_for(1);
        let placement = placement_for(1);
        register_worker(&client, &placement.worker).await;
        request_placement(&client, &table).await;
        let (holder, operator, observer) = (
            scratch.connect().await,
            scratch.connect().await,
            scratch.connect().await,
        );
        let operator_pid = backend_pid(&operator).await;
        begin_as(&holder, "df_runner", None).await;
        holder
            .query(
                PLACE_SQL,
                &[
                    &table.tenant,
                    &table.campaign,
                    &placement.id,
                    &placement.worker,
                    &placement.token_digest(),
                ],
            )
            .await
            .expect("placement inside its open transaction");
        let (job_table, digest) = (table.clone(), digest_of_token(JOB_TOKEN));
        let opening = tokio::spawn(async move {
            operator
                .execute(
                    "SELECT df_runtime.open_import($1, $2, $3, 600)",
                    &[&job_table.tenant, &job_table.campaign, &digest],
                )
                .await
        });
        wait_until_blocked(&observer, operator_pid).await;
        commit(&holder).await;
        let refused = opening.await.expect("operator task").expect_err("live now");
        assert_eq!(sqlstate(&refused).as_deref(), Some("DF007"));
        let jobs: i64 = client
            .query_one("SELECT count(*) FROM df_runtime.import_jobs", &[])
            .await
            .expect("count")
            .get(0);
        assert_eq!(jobs, 0);
        scratch.finish().await;
    });
}

#[test]
fn unclosed_import_jobs_are_purged_after_their_retention_and_only_by_the_control_plane() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let table = table_for(1);
        for (tag, expires) in [
            (1_u8, "clock_timestamp() - interval '10 days'"),
            (2, "clock_timestamp() - interval '1 day'"),
            (3, "clock_timestamp() + interval '1 hour'"),
        ] {
            client
                .execute(
                    &format!(
                        "INSERT INTO df_runtime.import_jobs VALUES ($1, $2, $3, \
                         clock_timestamp() - interval '11 days', {expires})"
                    ),
                    &[&vec![tag; 32], &table.tenant, &table.campaign],
                )
                .await
                .expect("job");
        }
        let refused = as_importer(
            &client,
            JOB_TOKEN,
            "SELECT df_runtime.purge_import_jobs(604800)",
            &[],
        )
        .await;
        assert_eq!(
            sqlstate(&refused.expect_err("refused")).as_deref(),
            Some("42501")
        );
        let purged: i64 = as_role(
            &client,
            "df_control",
            "SELECT df_runtime.purge_import_jobs(604800)",
            &[],
        )
        .await
        .expect("purge")[0]
            .get(0);
        assert_eq!(purged, 1, "only the job expired more than 7 days ago");
        let remaining: Vec<Vec<u8>> = client
            .query(
                "SELECT import_token_digest FROM df_runtime.import_jobs ORDER BY 1",
                &[],
            )
            .await
            .expect("remaining")
            .iter()
            .map(|row| row.get(0))
            .collect();
        assert_eq!(remaining, [vec![2_u8; 32], vec![3_u8; 32]]);
        scratch.finish().await;
    });
}

#[test]
fn purge_import_jobs_deletes_at_most_one_thousand_expired_rows_per_call() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        client
            .execute(
                "INSERT INTO df_runtime.import_jobs \
                 SELECT decode(lpad(to_hex(i), 64, '0'), 'hex'), \
                        decode(repeat('01', 16), 'hex'), decode(repeat('02', 16), 'hex'), \
                        clock_timestamp() - interval '10 days', \
                        clock_timestamp() - interval '9 days' \
                   FROM generate_series(1, 2500) AS i",
                &[],
            )
            .await
            .expect("expired import jobs");
        client
            .execute(
                "INSERT INTO df_runtime.import_jobs VALUES \
                 (decode(repeat('ff', 32), 'hex'), decode(repeat('01', 16), 'hex'), \
                  decode(repeat('02', 16), 'hex'), clock_timestamp(), \
                  clock_timestamp() + interval '1 day')",
                &[],
            )
            .await
            .expect("live import job");

        for (expected_deleted, expected_remaining) in [(1000_i64, 1500_i64), (1000, 500), (500, 0)]
        {
            let deleted: i64 = as_role(
                &client,
                "df_control",
                "SELECT df_runtime.purge_import_jobs(0, 2500)",
                &[],
            )
            .await
            .expect("purge batch")[0]
                .get(0);
            assert_eq!(
                deleted, expected_deleted,
                "the requested limit is capped at 1000"
            );
            let remaining: i64 = client
                .query_one(
                    "SELECT count(*) FROM df_runtime.import_jobs WHERE expires_at < clock_timestamp()",
                    &[],
                )
                .await
                .expect("remaining expired rows")
                .get(0);
            assert_eq!(remaining, expected_remaining);
        }
        let live: i64 = client
            .query_one(
                "SELECT count(*) FROM df_runtime.import_jobs WHERE expires_at > clock_timestamp()",
                &[],
            )
            .await
            .expect("live row survives")
            .get(0);
        assert_eq!(live, 1);
        scratch.finish().await;
    });
}
