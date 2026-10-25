//! The schema: no cross-owner foreign keys, the scope CHECK, append-only events and the
//! production role's privileges.

use df_campaign::records::{EventKind, EventScope};

use tokio_postgres::error::SqlState;

use crate::fixture::World;

#[tokio::test]
async fn no_campaigns_table_references_another_owners_table() {
    let world = World::new("nofk").await;
    let rows = world
        .db
        .admin
        .client
        .query(
            "SELECT c.conrelid::regclass::text, c.confrelid::regclass::text \
             FROM pg_constraint c JOIN pg_namespace n ON n.oid = c.connamespace \
             WHERE c.contype = 'f' AND n.nspname = 'campaigns'",
            &[],
        )
        .await
        .unwrap();
    assert!(
        !rows.is_empty(),
        "the campaigns schema has its own foreign keys"
    );
    for row in rows {
        let from: String = row.get(0);
        let to: String = row.get(1);
        assert!(from.starts_with("campaigns."), "{from}");
        assert!(to.starts_with("campaigns."), "{from} references {to}");
    }
    world.cleanup().await;
}

#[tokio::test]
async fn the_production_role_cannot_update_any_accounts_table_but_can_lock_through_the_definer() {
    let world = World::new("privileges").await;
    let rows = world
        .db
        .admin
        .client
        .query(
            "SELECT tablename FROM pg_tables WHERE schemaname = 'accounts'",
            &[],
        )
        .await
        .unwrap();
    assert!(!rows.is_empty());
    for row in rows {
        let table: String = row.get(0);
        let update = world
            .db
            .admin
            .client
            .query_one(
                "SELECT has_table_privilege($1, $2, 'UPDATE')",
                &[&world.role, &format!("accounts.{table}")],
            )
            .await
            .unwrap();
        let allowed: bool = update.get(0);
        assert!(
            !allowed,
            "the campaign role must not update accounts.{table}"
        );
    }
    let session = world.db.raw_as(&world.role).await;
    let account = world.account(1).await;
    session
        .client
        .query(
            "SELECT accounts.lock_account_status($1)",
            &[&account.as_bytes().as_slice()],
        )
        .await
        .expect("the definer function takes the share lock");
    let direct = session
        .client
        .query(
            "SELECT status FROM accounts.account WHERE account_id = $1 FOR SHARE",
            &[&account.as_bytes().as_slice()],
        )
        .await;
    assert!(direct.is_err(), "a direct FOR SHARE needs UPDATE privilege");
    world.cleanup().await;
}

#[tokio::test]
async fn events_are_append_only() {
    let world = World::new("appendonly").await;
    let host = world.account(1).await;
    let campaign = world.campaign(host, 4).await;
    let session = world.db.raw_as("postgres").await;
    let update = session
        .client
        .execute("UPDATE campaigns.campaign_event SET revoking = true", &[])
        .await;
    assert!(update.is_err());
    let delete = session
        .client
        .execute("DELETE FROM campaigns.campaign_event", &[])
        .await;
    assert!(delete.is_err());
    assert_eq!(world.events(campaign.id).await.len(), 1);
    world.cleanup().await;
}

/// Moves the campaign's counter by one, as every event-writing change does, so the row inserted next
/// carries the counter itself.
const BUMP: &str = "UPDATE campaigns.campaign SET event_seq = event_seq + 1; ";

fn row_sql(kind: EventKind, revoking: bool) -> String {
    let columns = "campaign_id, event_seq, scope, feed_epoch, member_id, member_revision, \
                   revoking, at_ms, kind, ticket_id";
    let (member, revision, ticket) = match kind.scope() {
        EventScope::Member => ("'\\x11111111111111111111111111111111'::bytea", "1", "NULL"),
        EventScope::Ticket => (
            "NULL",
            "NULL",
            "'\\x22222222222222222222222222222222'::bytea",
        ),
        EventScope::Campaign => ("NULL", "NULL", "NULL"),
    };
    format!(
        "INSERT INTO campaigns.campaign_event ({columns}) SELECT campaign_id, event_seq, '{}', \
         feed_epoch, {member}, {revision}, {revoking}, 1, '{}', {ticket} \
         FROM campaigns.campaign LIMIT 1",
        kind.scope().text(),
        kind.text()
    )
}

#[tokio::test]
async fn every_event_kind_satisfies_the_scope_check_constraint() {
    let world = World::new("kindcheck").await;
    let host = world.account(1).await;
    world.campaign(host, 4).await;
    for kind in EventKind::ALL {
        let flags: Vec<bool> = match kind.fixed_revoking() {
            Some(fixed) => vec![fixed],
            None => vec![false, true],
        };
        for revoking in flags {
            world
                .db
                .admin
                .client
                .batch_execute(&format!("{BUMP}{}", row_sql(kind, revoking)))
                .await
                .unwrap_or_else(|error| panic!("{kind:?} revoking={revoking}: {error}"));
        }
        // The opposite of a fixed flag is refused by the CHECK, not by some other rule.
        if let Some(fixed) = kind.fixed_revoking() {
            let refused = world
                .db
                .admin
                .client
                .batch_execute(&format!("{BUMP}{}", row_sql(kind, !fixed)))
                .await
                .expect_err("the opposite flag must be refused");
            assert_eq!(refused.code(), Some(&SqlState::CHECK_VIOLATION), "{kind:?}");
        }
    }
    world.cleanup().await;
}

#[tokio::test]
async fn the_scope_check_constraint_rejects_a_malformed_row() {
    let world = World::new("scopecheck").await;
    let host = world.account(1).await;
    world.campaign(host, 4).await;
    let statements = [
        // A member row with no revision.
        "INSERT INTO campaigns.campaign_event SELECT campaign_id, event_seq, 'member', feed_epoch, \
         '\\x11111111111111111111111111111111'::bytea, NULL, true, 1, 'left', NULL, NULL, NULL \
         FROM campaigns.campaign",
        // A ticket row with a member.
        "INSERT INTO campaigns.campaign_event SELECT campaign_id, event_seq, 'ticket', feed_epoch, \
         '\\x11111111111111111111111111111111'::bytea, 1, true, 1, 'display_ticket_revoked', NULL, \
         '\\x22222222222222222222222222222222'::bytea, NULL FROM campaigns.campaign",
        // A campaign row with a member.
        "INSERT INTO campaigns.campaign_event SELECT campaign_id, event_seq, 'campaign', feed_epoch, \
         '\\x11111111111111111111111111111111'::bytea, 1, true, 1, 'archived', NULL, NULL, NULL \
         FROM campaigns.campaign",
        // A kind under the wrong scope.
        "INSERT INTO campaigns.campaign_event SELECT campaign_id, event_seq, 'campaign', feed_epoch, \
         NULL, NULL, false, 1, 'joined', NULL, NULL, NULL FROM campaigns.campaign",
    ];
    for statement in statements {
        let refused = world
            .db
            .admin
            .client
            .batch_execute(&format!("{BUMP}{statement}"))
            .await
            .expect_err("must be refused");
        assert_eq!(
            refused.code(),
            Some(&SqlState::CHECK_VIOLATION),
            "refused by the scope CHECK: {statement}"
        );
    }
    world.cleanup().await;
}

#[tokio::test]
async fn the_active_host_index_allows_one_host_per_campaign() {
    let world = World::new("onehost").await;
    let host = world.account(1).await;
    let other = world.account(2).await;
    let campaign = world.campaign(host, 4).await;
    world
        .join(
            campaign.id,
            host,
            other,
            df_campaign::join_invitation::JoinRole::Player,
        )
        .await;
    let result = world
        .db
        .admin
        .client
        .execute(
            "UPDATE campaigns.campaign_member SET role = 'host' WHERE account_id = $1",
            &[&other.as_bytes().as_slice()],
        )
        .await;
    assert!(
        result.is_err(),
        "a second active host is refused by the partial unique index"
    );
    world.cleanup().await;
}
