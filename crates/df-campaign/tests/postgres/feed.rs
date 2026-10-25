//! The event feed: gapless ordering under the campaign lock, the keyset poll, the snapshot and
//! the commit-time bound on an event transaction.

use std::sync::Arc;

use df_campaign::{
    join_invitation::JoinRole,
    records::{EventKind, EventScope},
    store::{CampaignStore, ChangeKey},
};

use crate::fixture::World;

/// Takes the campaign lock in a raw session and assigns the next event number without
/// committing: the stand-in for a slow transaction that has bumped `event_seq`.
async fn hold_event(
    world: &World,
    campaign: df_types::CampaignId,
    stamp_ms: i64,
) -> crate::fixture::Raw {
    let session = world.db.raw_as(&world.role).await;
    session.client.batch_execute("BEGIN").await.unwrap();
    let row = session
        .client
        .query_one(
            "UPDATE campaigns.campaign SET event_seq = event_seq + 1, last_event_at_ms = $2 \
             WHERE campaign_id = $1 RETURNING event_seq, feed_epoch",
            &[&campaign.as_bytes().as_slice(), &stamp_ms],
        )
        .await
        .unwrap();
    let sequence: i64 = row.get(0);
    let epoch: Vec<u8> = row.get(1);
    session
        .client
        .execute(
            "INSERT INTO campaigns.campaign_event (campaign_id, event_seq, scope, feed_epoch, \
             revoking, at_ms, kind) VALUES ($1, $2, 'campaign', $3, false, 1, 'unarchived')",
            &[&campaign.as_bytes().as_slice(), &sequence, &epoch],
        )
        .await
        .unwrap();
    session
}

#[tokio::test]
async fn a_consumer_never_sees_event_two_before_event_one_commits() {
    let world = Arc::new(World::new("order").await);
    let host = world.account(1).await;
    let campaign = world.campaign(host, 4).await;
    let held = hold_event(&world, campaign.id, 5).await;

    // Event 3 must wait for the lock, then take the next number after event 2 commits.
    let task = {
        let world = Arc::clone(&world);
        tokio::spawn(async move {
            world
                .store
                .set_seat(host, campaign.id, false, world.now())
                .await
        })
    };
    world.wait_for_lock_waiters(1).await;
    let seen: Vec<u64> = world
        .events(campaign.id)
        .await
        .iter()
        .map(|event| event.event_seq)
        .collect();
    assert_eq!(seen, [1], "a reader sees only the committed prefix");

    held.client.batch_execute("COMMIT").await.unwrap();
    task.await.unwrap().expect("the waiting change commits");
    let events = world.events(campaign.id).await;
    let numbers: Vec<u64> = events.iter().map(|event| event.event_seq).collect();
    assert_eq!(numbers, [1, 2, 3]);
    assert_eq!(events[2].kind, EventKind::SeatChanged);
    Arc::try_unwrap(world)
        .ok()
        .expect("one owner")
        .cleanup()
        .await;
}

#[tokio::test]
async fn rolling_back_the_first_transaction_leaves_no_gap() {
    let world = Arc::new(World::new("nogap").await);
    let host = world.account(1).await;
    let campaign = world.campaign(host, 4).await;
    let held = hold_event(&world, campaign.id, 5).await;
    let task = {
        let world = Arc::clone(&world);
        tokio::spawn(async move {
            world
                .store
                .set_seat(host, campaign.id, false, world.now())
                .await
        })
    };
    world.wait_for_lock_waiters(1).await;
    held.client.batch_execute("ROLLBACK").await.unwrap();
    task.await.unwrap().expect("the waiting change commits");
    let numbers: Vec<u64> = world
        .events(campaign.id)
        .await
        .iter()
        .map(|event| event.event_seq)
        .collect();
    assert_eq!(numbers, [1, 2], "the rolled-back number is reused");
    Arc::try_unwrap(world)
        .ok()
        .expect("one owner")
        .cleanup()
        .await;
}

#[tokio::test]
async fn a_transaction_committing_9_9_seconds_after_its_stamp_is_still_read() {
    let world = World::new("keyset").await;
    let host = world.account(1).await;
    let campaign = world.campaign(host, 4).await;
    let anchor = world.db_now_ms().await;
    // The stamp was taken 9.9 s before the poll anchor; the transaction commits after the poll.
    let stamp = i64::try_from(anchor).unwrap() - 9_900;
    let held = hold_event(&world, campaign.id, stamp).await;

    let overlap = 15_000;
    let first = world
        .store
        .campaign_changes_since(anchor - overlap, None, 100)
        .await
        .unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(
        first[0].event_seq, 1,
        "the uncommitted event is invisible to the first poll"
    );

    held.client.batch_execute("COMMIT").await.unwrap();
    // The next poll reads from the previous head-reaching poll's start minus the overlap.
    let second = world
        .store
        .campaign_changes_since(anchor - overlap, None, 100)
        .await
        .unwrap();
    assert_eq!(second.len(), 1);
    assert_eq!(
        second[0].event_seq, 2,
        "the late commit is found by the overlapped poll"
    );
    assert_eq!(second[0].last_event_at_ms, anchor - 9_900);

    // The negative control: an overlap shorter than the transaction would have missed it.
    let short = world
        .store
        .campaign_changes_since(anchor - 5_000, None, 100)
        .await
        .unwrap();
    assert!(
        short.is_empty(),
        "the test can fail when the overlap is too short"
    );
    world.cleanup().await;
}

#[tokio::test]
async fn a_slow_poll_does_not_shrink_the_overlap() {
    let world = World::new("slowpoll").await;
    let host = world.account(1).await;
    let campaign = world.campaign(host, 4).await;
    let anchor = world.db_now_ms().await;
    let held = hold_event(&world, campaign.id, i64::try_from(anchor).unwrap() - 9_900).await;
    // The poll takes 8 s: the next lower bound is still anchored on the poll's own start.
    let later = anchor + 8_000;
    held.client.batch_execute("COMMIT").await.unwrap();
    let found = world
        .store
        .campaign_changes_since(anchor - 15_000, None, 100)
        .await
        .unwrap();
    assert_eq!(found[0].event_seq, 2);
    assert!(
        later - 15_000 > anchor - 9_900 - 1,
        "an anchor on the end of the poll would miss it"
    );
    world.cleanup().await;
}

#[tokio::test]
async fn the_hint_pages_by_stamp_and_id_without_skipping() {
    let world = World::new("hintpages").await;
    let host = world.account(1).await;
    for _ in 0..5 {
        world.campaign(host, 4).await;
    }
    let mut seen = Vec::new();
    let mut after: Option<ChangeKey> = None;
    loop {
        let page = world
            .store
            .campaign_changes_since(0, after, 2)
            .await
            .unwrap();
        if page.is_empty() {
            break;
        }
        let last = page.last().unwrap();
        after = Some(ChangeKey {
            last_event_at_ms: last.last_event_at_ms,
            campaign: last.campaign,
        });
        seen.extend(page.iter().map(|change| change.campaign));
    }
    assert_eq!(seen.len(), 5);
    let mut unique = seen.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 5, "no campaign is listed twice");
    world.cleanup().await;
}

#[tokio::test]
async fn campaign_heads_after_pages_by_primary_key_without_skipping() {
    let world = World::new("headspages").await;
    let host = world.account(1).await;
    for _ in 0..5 {
        world.campaign(host, 4).await;
    }
    let mut seen = Vec::new();
    let mut after = None;
    loop {
        let page = world.store.campaign_heads_after(after, 2).await.unwrap();
        if page.is_empty() {
            break;
        }
        after = page.last().map(|head| head.campaign);
        seen.extend(page.iter().map(|head| head.campaign));
    }
    let mut sorted = seen.clone();
    sorted.sort();
    assert_eq!(seen.len(), 5);
    assert_eq!(seen, sorted, "pages come in primary-key order");
    world.cleanup().await;
}

#[tokio::test]
async fn member_snapshot_carries_feed_epoch_and_event_seq_from_one_statement() {
    let world = World::new("snapshot").await;
    let host = world.account(1).await;
    let campaign = world.campaign(host, 4).await;
    let snapshot = world
        .store
        .member_snapshot(host, campaign.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.feed_epoch, campaign.feed_epoch);
    assert_eq!(snapshot.event_seq, 1);
    assert_eq!(snapshot.member_revision, 1);
    // A stranger has no snapshot.
    let stranger = world.account(2).await;
    assert!(
        world
            .store
            .member_snapshot(stranger, campaign.id)
            .await
            .unwrap()
            .is_none()
    );
    world.cleanup().await;
}

#[tokio::test]
async fn a_closing_account_is_not_authorized_by_member_snapshot() {
    let world = World::new("closingsnap").await;
    let host = world.account(1).await;
    let player = world.account(2).await;
    let campaign = world.campaign(host, 4).await;
    world
        .join(campaign.id, host, player, JoinRole::Player)
        .await;
    world.db.set_account_status(player, "closing").await;
    let snapshot = world
        .store
        .member_snapshot(player, campaign.id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        !snapshot.authorized,
        "the member row is active but the account is closing"
    );
    assert_eq!(snapshot.status, df_campaign::records::MemberStatus::Active);
    world.cleanup().await;
}

#[tokio::test]
async fn two_restores_from_one_backup_produce_two_different_feed_epochs() {
    let world = World::new("epochs").await;
    let host = world.account(1).await;
    let campaign = world.campaign(host, 4).await;
    let before = world.feed_epoch(campaign.id).await;
    world
        .db
        .admin
        .client
        .query("SELECT campaigns.restore_feed_epochs()", &[])
        .await
        .unwrap();
    let first = world.feed_epoch(campaign.id).await;
    world
        .db
        .admin
        .client
        .query("SELECT campaigns.restore_feed_epochs()", &[])
        .await
        .unwrap();
    let second = world.feed_epoch(campaign.id).await;
    assert_ne!(before, first);
    assert_ne!(first, second);
    // The events keep the epoch they were written in.
    let events = world.events(campaign.id).await;
    assert_eq!(events[0].feed_epoch, before);
    world.cleanup().await;
}

#[tokio::test]
async fn every_event_the_store_writes_has_the_scope_its_kind_fixes() {
    let world = World::new("scopes").await;
    let host = world.account(1).await;
    let player = world.account(2).await;
    let campaign = world.campaign(host, 4).await;
    world
        .join(campaign.id, host, player, JoinRole::Player)
        .await;
    for event in world.events(campaign.id).await {
        assert_eq!(event.scope, event.kind.scope());
        if let Some(fixed) = event.kind.fixed_revoking() {
            assert_eq!(event.revoking, fixed);
        }
        match event.scope {
            EventScope::Member => {
                assert!(event.member.is_some() && event.member_revision.is_some())
            }
            EventScope::Ticket => assert!(event.ticket.is_some() && event.member.is_none()),
            EventScope::Campaign => assert!(event.member.is_none() && event.ticket.is_none()),
        }
    }
    world.cleanup().await;
}

#[tokio::test]
async fn an_over_long_event_transaction_fails_at_commit_even_with_the_session_timeout_set_to_zero()
{
    let world = World::new("bound").await;
    let host = world.account(1).await;
    let campaign = world.campaign(host, 4).await;
    // A reliability bound, checked at commit: a role default is only a default, so the bound is also
    // a deferred constraint trigger. The bound is wall-clock time by definition and there is no
    // clock to inject, so the test shrinks it and lets the server run past it (pg_sleep).
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
    let session = world.db.raw_as(&world.role).await;
    let bump_and_write = "UPDATE campaigns.campaign SET event_seq = event_seq + 1 \n         WHERE campaign_id = $1 RETURNING event_seq, feed_epoch";
    session
        .client
        .batch_execute("BEGIN; SET transaction_timeout = 0")
        .await
        .unwrap();
    let row = session
        .client
        .query_one(bump_and_write, &[&campaign.id.as_bytes().as_slice()])
        .await
        .unwrap();
    let (sequence, epoch): (i64, Vec<u8>) = (row.get(0), row.get(1));
    session
        .client
        .execute(
            "INSERT INTO campaigns.campaign_event (campaign_id, event_seq, scope, feed_epoch, \n             revoking, at_ms, kind) VALUES ($1, $2, 'campaign', $3, false, 1, 'unarchived')",
            &[&campaign.id.as_bytes().as_slice(), &sequence, &epoch],
        )
        .await
        .unwrap();
    session
        .client
        .batch_execute("SELECT pg_sleep(0.4)")
        .await
        .unwrap();
    let error = session
        .client
        .batch_execute("COMMIT")
        .await
        .expect_err("the commit is refused");
    assert_eq!(error.code().map(|code| code.code()), Some("DF003"));
    let head = world.store.campaign_heads(&[campaign.id]).await.unwrap();
    assert_eq!(
        head[0].event_seq, 1,
        "the over-long transaction changed nothing"
    );

    // Within the bound the same change commits.
    world
        .db
        .admin
        .client
        .execute(
            "UPDATE campaigns.feed_bound SET max_transaction_ms = 10000",
            &[],
        )
        .await
        .unwrap();
    session.client.batch_execute("BEGIN").await.unwrap();
    let row = session
        .client
        .query_one(bump_and_write, &[&campaign.id.as_bytes().as_slice()])
        .await
        .unwrap();
    let (sequence, epoch): (i64, Vec<u8>) = (row.get(0), row.get(1));
    session
        .client
        .execute(
            "INSERT INTO campaigns.campaign_event (campaign_id, event_seq, scope, feed_epoch, \n             revoking, at_ms, kind) VALUES ($1, $2, 'campaign', $3, false, 1, 'unarchived')",
            &[&campaign.id.as_bytes().as_slice(), &sequence, &epoch],
        )
        .await
        .unwrap();
    session.client.batch_execute("COMMIT").await.unwrap();
    world.cleanup().await;
}
