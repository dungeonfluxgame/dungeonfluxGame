//! The `transaction_timeout` contract: the role default is the guarantee (checked at startup),
//! `SET LOCAL` is only a second guard, a termination discards the connection and a termination
//! during COMMIT is `Unknown`.

use df_campaign::{
    config::Tunables,
    errors::{Failure, StoreError},
};

use crate::{
    campaign_store::{OpenRefusal, PgCampaignStore, TimeoutPolicy},
    fixture::{TestDb, World},
};

async fn open_as(
    db: &mut TestDb,
    timeout: Option<&str>,
    policy: TimeoutPolicy,
    size: usize,
) -> Result<PgCampaignStore, OpenRefusal> {
    let role = db.login_role(timeout).await;
    PgCampaignStore::open(
        db.connector(&role),
        db.connector(&role),
        size,
        Tunables::default(),
        policy,
    )
    .await
}

#[tokio::test]
async fn startup_refuses_a_role_default_that_is_unset_zero_or_longer_than_the_bound() {
    let mut db = TestDb::create("startup").await;
    // The role default of 60 seconds is refused even though a SET LOCAL of 10 seconds would pass.
    assert_eq!(
        open_as(&mut db, Some("60s"), TimeoutPolicy::Enforced, 1)
            .await
            .err(),
        Some(OpenRefusal::TimeoutTooLong)
    );
    assert_eq!(
        open_as(&mut db, Some("5min"), TimeoutPolicy::Enforced, 1)
            .await
            .err(),
        Some(OpenRefusal::TimeoutTooLong)
    );
    assert_eq!(
        open_as(&mut db, Some("11s"), TimeoutPolicy::Enforced, 1)
            .await
            .err(),
        Some(OpenRefusal::TimeoutTooLong)
    );
    assert_eq!(
        open_as(&mut db, Some("0"), TimeoutPolicy::Enforced, 1)
            .await
            .err(),
        Some(OpenRefusal::TimeoutNotSet)
    );
    assert_eq!(
        open_as(&mut db, None, TimeoutPolicy::Enforced, 1)
            .await
            .err(),
        Some(OpenRefusal::TimeoutNotSet)
    );
    // Units are normalized: 10 seconds in any spelling passes, and so does a shorter default.
    assert!(
        open_as(&mut db, Some("10s"), TimeoutPolicy::Enforced, 1)
            .await
            .is_ok()
    );
    assert!(
        open_as(&mut db, Some("10000ms"), TimeoutPolicy::Enforced, 1)
            .await
            .is_ok()
    );
    assert!(
        open_as(&mut db, Some("2s"), TimeoutPolicy::Enforced, 1)
            .await
            .is_ok()
    );
    // A scratch server whose roles are not provisioned can opt out.
    assert!(
        open_as(&mut db, None, TimeoutPolicy::NotChecked, 1)
            .await
            .is_ok()
    );
    db.cleanup().await;
}

#[tokio::test]
async fn the_configuration_is_refused_when_the_overlap_does_not_exceed_the_transaction_timeout() {
    let mut db = TestDb::create("overlap").await;
    let role = db.login_role(Some("10s")).await;
    let tunables = Tunables {
        feed_overlap_seconds: 10,
        ..Tunables::default()
    };
    let opened = PgCampaignStore::open(
        db.connector(&role),
        db.connector(&role),
        1,
        tunables,
        TimeoutPolicy::Enforced,
    )
    .await;
    assert!(matches!(opened, Err(OpenRefusal::Configuration(_))));
    db.cleanup().await;
}

#[tokio::test]
async fn every_event_bumping_method_sets_the_transaction_timeout() {
    // Under the provisioned role the store's own transactions run with the bound...
    let world = World::new("setlocal").await;
    assert_eq!(
        world
            .store
            .debug_text("SELECT current_setting('transaction_timeout')")
            .await
            .unwrap(),
        "10s"
    );
    world.cleanup().await;
    // ...and on a role with no default, the second guard sets it. A shorter default is kept.
    let mut db = TestDb::create("setlocal2").await;
    let none = open_as(&mut db, None, TimeoutPolicy::NotChecked, 1)
        .await
        .unwrap();
    assert_eq!(
        none.debug_text("SELECT current_setting('transaction_timeout')")
            .await
            .unwrap(),
        "10s"
    );
    let short = open_as(&mut db, Some("2s"), TimeoutPolicy::Enforced, 1)
        .await
        .unwrap();
    assert_eq!(
        short
            .debug_text("SELECT current_setting('transaction_timeout')")
            .await
            .unwrap(),
        "2s"
    );
    let long = open_as(&mut db, Some("60s"), TimeoutPolicy::NotChecked, 1)
        .await
        .unwrap();
    assert_eq!(
        long.debug_text("SELECT current_setting('transaction_timeout')")
            .await
            .unwrap(),
        "10s"
    );
    db.cleanup().await;
}

#[tokio::test]
async fn set_local_does_not_shorten_a_timer_already_running_on_the_connection() {
    // A measurement of the target server, not a requirement: the design does not rely on the
    // outcome. It exists so a changed server behaviour is noticed.
    let mut db = TestDb::create("measure").await;
    let role = db.login_role(Some("5s")).await;
    let session = db.raw_as(&role).await;
    session
        .client
        .batch_execute("BEGIN; SELECT 1; SET LOCAL transaction_timeout = '1s'")
        .await
        .unwrap();
    let started = std::time::Instant::now();
    let outcome = session.client.batch_execute("SELECT pg_sleep(2.5)").await;
    eprintln!(
        "SET LOCAL after the timer started: terminated={} after {:?}",
        outcome.is_err(),
        started.elapsed()
    );
    db.cleanup().await;
}

#[tokio::test]
async fn a_role_default_of_one_second_ends_a_longer_transaction_and_the_pool_discards_the_connection()
 {
    let mut db = TestDb::create("terminate").await;
    let store = open_as(&mut db, Some("1s"), TimeoutPolicy::Enforced, 1)
        .await
        .unwrap();
    let failure = store
        .debug_statements(&["SELECT pg_sleep(2)"])
        .await
        .expect_err("ended by the server");
    assert_eq!(failure, StoreError::Unavailable);
    // The pool has one slot, so this only works if the dead connection was replaced.
    assert_eq!(store.debug_text("SELECT 'alive'").await.unwrap(), "alive");
    db.cleanup().await;
}

#[tokio::test]
async fn a_termination_during_commit_is_reported_as_unknown() {
    let mut db = TestDb::create("commitunknown").await;
    db.admin
        .client
        .batch_execute(
            "CREATE TABLE public.slow_commit (x integer); \
             CREATE FUNCTION public.slow_commit_fn() RETURNS trigger LANGUAGE plpgsql AS \
             $$ BEGIN PERFORM pg_sleep(2); RETURN NULL; END $$; \
             CREATE CONSTRAINT TRIGGER slow AFTER INSERT ON public.slow_commit \
             DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.slow_commit_fn(); \
             GRANT ALL ON public.slow_commit TO df_campaigns",
        )
        .await
        .unwrap();
    let store = open_as(&mut db, Some("1s"), TimeoutPolicy::Enforced, 1)
        .await
        .unwrap();
    // The statements finish inside the bound; the deferred trigger runs during COMMIT and the
    // server ends the session while the commit is in progress.
    let result = store
        .debug_statements(&["INSERT INTO public.slow_commit VALUES (1)"])
        .await;
    assert_eq!(
        result,
        Err(StoreError::Unknown),
        "never reported as failure or success"
    );
    assert_eq!(store.debug_text("SELECT 'alive'").await.unwrap(), "alive");
    db.cleanup().await;
}

#[tokio::test]
async fn a_failed_commit_by_the_event_age_check_is_retryable_not_unknown() {
    let world = World::new("agecheck").await;
    let host = world.account(1).await;
    let campaign = world.campaign(host, 4).await;
    world
        .db
        .admin
        .client
        .execute(
            "UPDATE campaigns.feed_bound SET max_transaction_ms = 100",
            &[],
        )
        .await
        .unwrap();
    let id = campaign.id.as_bytes().to_vec();
    let bump = format!(
        "UPDATE campaigns.campaign SET event_seq = event_seq + 1 WHERE campaign_id = '\\x{}'",
        id.iter().map(|b| format!("{b:02x}")).collect::<String>()
    );
    let result = world
        .store
        .debug_statements(&[&bump, "SELECT pg_sleep(0.5)"])
        .await;
    assert_eq!(result, Err(StoreError::Retryable));
    let _ = Failure::Store(StoreError::Retryable);
    world.cleanup().await;
}
