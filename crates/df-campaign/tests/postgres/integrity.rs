//! What the database itself enforces about the event feed and about who may write what, so a bug
//! in a service role's code (or a hostile statement) cannot break the feed's invariants.

use df_campaign::{config::Tunables, store::CampaignStore};
use tokio_postgres::error::SqlState;

use crate::{
    campaign_store::{OpenRefusal, PgCampaignStore, TimeoutPolicy},
    fixture::World,
};

const FEED_INVARIANT: &str = "DF004";

fn code(error: &tokio_postgres::Error) -> Option<&str> {
    error.code().map(SqlState::code)
}

async fn world_with_campaign(label: &str) -> (World, df_types::CampaignId) {
    let world = World::new(label).await;
    let host = world.account(1).await;
    let campaign = world.campaign(host, 4).await;
    (world, campaign.id)
}

#[tokio::test]
async fn an_event_row_that_is_not_the_campaigns_counter_is_refused_at_insert() {
    let (world, campaign) = world_with_campaign("phantom").await;
    let session = world.db.raw_as(&world.role).await;
    session.client.batch_execute("BEGIN").await.unwrap();
    session
        .client
        .execute(
            "UPDATE campaigns.campaign SET event_seq = event_seq + 1 WHERE campaign_id = $1",
            &[&campaign.as_bytes().as_slice()],
        )
        .await
        .unwrap();
    // The counter is 2; a row numbered 5 would be a phantom, and a row numbered 1 a duplicate.
    for number in [5_i64, 1] {
        let error = session
            .client
            .execute(
                "INSERT INTO campaigns.campaign_event (campaign_id, event_seq, scope, feed_epoch, \
                 revoking, at_ms, kind) SELECT campaign_id, $2, 'campaign', feed_epoch, false, 1, \
                 'unarchived' FROM campaigns.campaign WHERE campaign_id = $1",
                &[&campaign.as_bytes().as_slice(), &number],
            )
            .await
            .expect_err("a row that is not the head is refused");
        assert_eq!(code(&error), Some(FEED_INVARIANT));
        session
            .client
            .batch_execute("ROLLBACK; BEGIN")
            .await
            .unwrap();
        session
            .client
            .execute(
                "UPDATE campaigns.campaign SET event_seq = event_seq + 1 WHERE campaign_id = $1",
                &[&campaign.as_bytes().as_slice()],
            )
            .await
            .unwrap();
    }
    world.cleanup().await;
}

#[tokio::test]
async fn an_event_row_in_another_epoch_is_refused() {
    let (world, campaign) = world_with_campaign("wrongepoch").await;
    let session = world.db.raw_as(&world.role).await;
    session.client.batch_execute("BEGIN").await.unwrap();
    session
        .client
        .execute(
            "UPDATE campaigns.campaign SET event_seq = event_seq + 1 WHERE campaign_id = $1",
            &[&campaign.as_bytes().as_slice()],
        )
        .await
        .unwrap();
    let error = session
        .client
        .execute(
            "INSERT INTO campaigns.campaign_event (campaign_id, event_seq, scope, feed_epoch, \
             revoking, at_ms, kind) SELECT campaign_id, event_seq, 'campaign', \
             '\\x00000000000000000000000000000001'::bytea, false, 1, 'unarchived' \
             FROM campaigns.campaign WHERE campaign_id = $1",
            &[&campaign.as_bytes().as_slice()],
        )
        .await
        .expect_err("another epoch is refused");
    assert_eq!(code(&error), Some(FEED_INVARIANT));
    world.cleanup().await;
}

#[tokio::test]
async fn a_counter_bump_with_no_event_row_fails_at_commit() {
    let (world, campaign) = world_with_campaign("bumponly").await;
    let session = world.db.raw_as(&world.role).await;
    session.client.batch_execute("BEGIN").await.unwrap();
    session
        .client
        .execute(
            "UPDATE campaigns.campaign SET event_seq = event_seq + 1 WHERE campaign_id = $1",
            &[&campaign.as_bytes().as_slice()],
        )
        .await
        .unwrap();
    let error = session
        .client
        .batch_execute("COMMIT")
        .await
        .expect_err("a phantom number is refused at commit");
    assert_eq!(code(&error), Some(FEED_INVARIANT));
    let head = world.store.campaign_heads(&[campaign]).await.unwrap();
    assert_eq!(head[0].event_seq, 1);
    world.cleanup().await;
}

#[tokio::test]
async fn a_hole_below_the_head_fails_at_commit_even_when_the_head_has_its_event() {
    let (world, campaign) = world_with_campaign("hole").await;
    let session = world.db.raw_as(&world.role).await;
    let id = campaign.as_bytes().to_vec();
    session.client.batch_execute("BEGIN").await.unwrap();
    // Two bumps and one event row: the counter equals the highest number written, so the head
    // check alone passes; the number between them has no row.
    for _ in 0..2 {
        session
            .client
            .execute(
                "UPDATE campaigns.campaign SET event_seq = event_seq + 1 WHERE campaign_id = $1",
                &[&id],
            )
            .await
            .unwrap();
    }
    session
        .client
        .execute(
            "INSERT INTO campaigns.campaign_event (campaign_id, event_seq, scope, feed_epoch, \
             revoking, at_ms, kind) SELECT campaign_id, event_seq, 'campaign', feed_epoch, false, \
             1, 'unarchived' FROM campaigns.campaign WHERE campaign_id = $1",
            &[&id],
        )
        .await
        .unwrap();
    let error = session
        .client
        .batch_execute("COMMIT")
        .await
        .expect_err("a gap is refused at commit");
    assert_eq!(code(&error), Some(FEED_INVARIANT));
    let head = world.store.campaign_heads(&[campaign]).await.unwrap();
    assert_eq!(head[0].event_seq, 1, "nothing of the transaction survived");
    world.cleanup().await;
}

#[tokio::test]
async fn the_counter_moves_only_forward_by_one() {
    let (world, campaign) = world_with_campaign("steps").await;
    let session = world.db.raw_as(&world.role).await;
    for delta in ["event_seq + 2", "event_seq - 1", "0"] {
        let error = session
            .client
            .execute(
                &format!(
                    "UPDATE campaigns.campaign SET event_seq = {delta} WHERE campaign_id = $1"
                ),
                &[&campaign.as_bytes().as_slice()],
            )
            .await
            .expect_err("the counter cannot jump or go back");
        assert_eq!(code(&error), Some(FEED_INVARIANT), "{delta}");
    }
    world.cleanup().await;
}

#[tokio::test]
async fn a_service_role_cannot_rewrite_the_tenant_the_epoch_or_the_identity_of_a_campaign() {
    let (world, campaign) = world_with_campaign("immutable").await;
    let session = world.db.raw_as(&world.role).await;
    for column in [
        "tenant_id = '\\x01'::bytea",
        "feed_epoch = '\\x01'::bytea",
        "campaign_id = '\\x01'::bytea",
        "created_at_ms = 0",
    ] {
        let error = session
            .client
            .execute(
                &format!("UPDATE campaigns.campaign SET {column} WHERE campaign_id = $1"),
                &[&campaign.as_bytes().as_slice()],
            )
            .await
            .expect_err("no column privilege");
        assert_eq!(
            error.code(),
            Some(&SqlState::INSUFFICIENT_PRIVILEGE),
            "{column}"
        );
    }
    // The restore step, which is not a service role, still rotates the epoch.
    world
        .db
        .admin
        .client
        .query("SELECT campaigns.restore_feed_epochs()", &[])
        .await
        .unwrap();
    world.cleanup().await;
}

#[tokio::test]
async fn the_guard_trigger_refuses_an_epoch_change_by_a_member_of_a_service_role_even_with_the_column_privilege()
 {
    // Defence in depth behind the column grants: give the privilege to a login role in
    // df_campaigns and the trigger still refuses the change, while the control plane may.
    let mut world = World::new("guardtrigger").await;
    let host = world.account(1).await;
    let campaign = world.campaign(host, 4).await.id;
    let role = world.db.login_role(Some("10s")).await;
    world
        .db
        .admin
        .client
        .batch_execute(&format!(
            "GRANT UPDATE (feed_epoch, tenant_id) ON campaigns.campaign TO {role}"
        ))
        .await
        .unwrap();
    let session = world.db.raw_as(&role).await;
    for column in ["feed_epoch = '\\x01'::bytea", "tenant_id = '\\x01'::bytea"] {
        let error = session
            .client
            .execute(
                &format!("UPDATE campaigns.campaign SET {column} WHERE campaign_id = $1"),
                &[&campaign.as_bytes().as_slice()],
            )
            .await
            .expect_err("the trigger refuses it");
        // The constraint on the column length may fire first for tenant_id; the epoch is the point.
        assert!(
            code(&error) == Some(FEED_INVARIANT)
                || error.code() == Some(&SqlState::CHECK_VIOLATION),
            "{column}: {error}"
        );
    }
    let valid_epoch = "'\\x0102030405060708090a0b0c0d0e0f10'::bytea";
    let error = session
        .client
        .execute(
            &format!(
                "UPDATE campaigns.campaign SET feed_epoch = {valid_epoch} WHERE campaign_id = $1"
            ),
            &[&campaign.as_bytes().as_slice()],
        )
        .await
        .expect_err("a well-formed epoch from a service role is refused too");
    assert_eq!(code(&error), Some(FEED_INVARIANT));
    world.cleanup().await;
}

#[tokio::test]
async fn the_grants_are_column_level_and_the_accounts_role_has_only_what_closure_writes() {
    let (world, _) = world_with_campaign("grants").await;
    let allowed = |role: &str, table: &str, column: &str| {
        let role = role.to_owned();
        let table = table.to_owned();
        let column = column.to_owned();
        let admin = &world.db.admin;
        async move {
            let row = admin
                .client
                .query_one(
                    "SELECT has_column_privilege($1, $2, $3, 'UPDATE')",
                    &[&role, &format!("campaigns.{table}"), &column],
                )
                .await
                .unwrap();
            row.get::<_, bool>(0)
        }
    };
    // The campaign service role.
    for column in ["tenant_id", "feed_epoch", "campaign_id", "created_at_ms"] {
        assert!(
            !allowed("df_campaigns", "campaign", column).await,
            "df_campaigns {column}"
        );
    }
    for column in ["status", "owner_account_id", "event_seq", "revision"] {
        assert!(
            allowed("df_campaigns", "campaign", column).await,
            "df_campaigns {column}"
        );
    }
    assert!(!allowed("df_campaigns", "campaign_member", "account_id").await);
    assert!(!allowed("df_campaigns", "campaign_invitation", "token_digest").await);
    assert!(!allowed("df_campaigns", "display_ticket", "ticket_digest").await);
    // B's role writes only the closure columns.
    for column in [
        "owner_account_id",
        "status",
        "tenant_id",
        "seat_count",
        "name",
        "feed_epoch",
    ] {
        assert!(
            !allowed("df_accounts", "campaign", column).await,
            "df_accounts {column}"
        );
    }
    for column in [
        "scheduled_delete_at_ms",
        "revision",
        "event_seq",
        "last_event_at_ms",
    ] {
        assert!(
            allowed("df_accounts", "campaign", column).await,
            "df_accounts {column}"
        );
    }
    assert!(!allowed("df_accounts", "campaign_member", "account_id").await);
    // Nothing in the closure path reads a join request, so B's role cannot.
    let select = world
        .db
        .admin
        .client
        .query_one(
            "SELECT has_table_privilege('df_accounts', 'campaigns.join_request', 'SELECT')",
            &[],
        )
        .await
        .unwrap()
        .get::<_, bool>(0);
    assert!(!select);
    // The bound the commit check reads is not writable by either service role.
    for role in ["df_campaigns", "df_accounts"] {
        let write = world
            .db
            .admin
            .client
            .query_one(
                "SELECT has_table_privilege($1, 'campaigns.feed_bound', 'UPDATE')",
                &[&role],
            )
            .await
            .unwrap()
            .get::<_, bool>(0);
        assert!(!write, "{role}");
    }
    world.cleanup().await;
}

#[test]
fn the_migration_fails_when_the_accounts_role_is_missing_instead_of_granting_nothing() {
    let migration = include_str!("../../../df-persistence/migrations/0008_campaigns.sql");
    assert!(migration.contains("RAISE EXCEPTION 'role df_accounts is missing"));
    assert!(
        !migration.contains("IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'df_accounts')"),
        "no silent skip of the accounts grants"
    );
    let lobby = include_str!("../../../df-persistence/migrations/0009_lobby.sql");
    assert!(!lobby.contains("IF EXISTS (SELECT 1 FROM pg_roles"));
}

#[tokio::test]
async fn the_store_refuses_to_open_when_the_databases_commit_bound_is_longer_than_the_timeout() {
    let mut world = World::new("boundconfig").await;
    world
        .db
        .admin
        .client
        .execute(
            "UPDATE campaigns.feed_bound SET max_transaction_ms = 20000",
            &[],
        )
        .await
        .unwrap();
    let role = world.db.login_role(Some("10s")).await;
    let opened = PgCampaignStore::open(
        world.db.connector(&role),
        world.db.connector(&role),
        1,
        Tunables::default(),
        TimeoutPolicy::Enforced,
    )
    .await;
    assert_eq!(opened.err(), Some(OpenRefusal::EventBoundTooLong));
    // A stricter bound is fine, and so is raising the configured timeout to match.
    world
        .db
        .admin
        .client
        .execute(
            "UPDATE campaigns.feed_bound SET max_transaction_ms = 5000",
            &[],
        )
        .await
        .unwrap();
    let opened = PgCampaignStore::open(
        world.db.connector(&role),
        world.db.connector(&role),
        1,
        Tunables::default(),
        TimeoutPolicy::Enforced,
    )
    .await;
    assert!(opened.is_ok());
    world.cleanup().await;
}
