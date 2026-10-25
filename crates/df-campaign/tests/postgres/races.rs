//! Forced interleavings. A second connection holds a lock, the operations under test are started
//! and proven blocked through `pg_stat_activity`, then the lock is released. Nothing sleeps: the
//! order is decided by the locks, not by timing.

use std::sync::Arc;

use df_campaign::{
    errors::{Failure, Refusal},
    join_invitation::{JoinFailure, JoinRole},
    records::{CampaignStatus, MemberStatus, TransferKind},
    roles::CampaignRole,
    store::{CampaignStore, NewInvitation, NewProposal},
};
use df_types::{AccountId, CampaignId};

use crate::{
    account_closure::ClosureParticipant,
    campaign_store::{PgCampaignStore, TimeoutPolicy},
    closure::{Bench, archive, mark_closing},
    fixture::{Raw, World, digest_of, operation_id, session_id, token_digest},
    membership::refusal_of,
};

/// A session as the production role that holds a campaign's row lock until `release`.
async fn hold_campaign(world: &World, campaigns: &[CampaignId]) -> Raw {
    let holder = world.db.raw_as(&world.role).await;
    holder.client.batch_execute("BEGIN").await.unwrap();
    let ids: Vec<Vec<u8>> = campaigns.iter().map(|id| id.as_bytes().to_vec()).collect();
    holder
        .client
        .query(
            "SELECT campaign_id FROM campaigns.campaign WHERE campaign_id = ANY($1) \
             ORDER BY campaign_id FOR UPDATE",
            &[&ids],
        )
        .await
        .unwrap();
    holder
}

async fn release(holder: &Raw) {
    holder.client.batch_execute("COMMIT").await.unwrap();
}

fn chunk_task(
    world: &Arc<World>,
    b_role: &str,
    account: AccountId,
    reversed: bool,
) -> tokio::task::JoinHandle<Result<usize, df_customer::StoreError>> {
    let world = Arc::clone(world);
    let b_role = b_role.to_owned();
    tokio::spawn(async move {
        let mut participant = world
            .store
            .closure_participant(Arc::new(world.clock.clone()));
        if reversed {
            participant = participant.with_reversed_lock_order();
        }
        let mut session = world.db.raw_as(&b_role).await;
        let tx = session.client.transaction().await.unwrap();
        mark_closing(&tx, account).await;
        match participant.leave_all_chunk(&tx, account, None).await {
            Ok(report) => {
                tx.commit().await.unwrap();
                Ok(report.left.len())
            }
            Err(error) => {
                tx.rollback().await.unwrap();
                Err(error)
            }
        }
    })
}

async fn two_accounts_in_two_campaigns(world: &World) -> (AccountId, AccountId, [CampaignId; 2]) {
    let first = world.account(1).await;
    let second = world.account(2).await;
    let host_a = world.account(3).await;
    let host_b = world.account(4).await;
    let a = world.campaign(host_a, 6).await;
    let b = world.campaign(host_b, 6).await;
    for account in [first, second] {
        world.join(a.id, host_a, account, JoinRole::Player).await;
        world.join(b.id, host_b, account, JoinRole::Player).await;
    }
    (first, second, [a.id, b.id])
}

#[tokio::test]
async fn two_closures_sharing_campaigns_lock_in_the_same_order_and_both_finish() {
    let bench = Bench::new("sharedorder").await;
    let Bench { world, b_role, .. } = bench;
    let world = Arc::new(world);
    let (first, second, campaigns) = two_accounts_in_two_campaigns(&world).await;
    // The holder has both campaigns, so both closures park on the first lock they ask for.
    let holder = hold_campaign(&world, &campaigns).await;
    let one = chunk_task(&world, &b_role, first, false);
    let two = chunk_task(&world, &b_role, second, false);
    world.wait_for_lock_waiters(2).await;
    release(&holder).await;
    assert_eq!(one.await.unwrap(), Ok(2));
    assert_eq!(two.await.unwrap(), Ok(2));
    Arc::try_unwrap(world)
        .ok()
        .expect("one owner")
        .cleanup()
        .await;
}

#[tokio::test]
async fn a_closure_that_locks_in_the_reverse_order_deadlocks_so_the_test_can_fail() {
    let bench = Bench::new("reverseorder").await;
    let Bench { world, b_role, .. } = bench;
    let world = Arc::new(world);
    let (first, second, mut campaigns) = two_accounts_in_two_campaigns(&world).await;
    campaigns.sort_by_key(|id| *id.as_bytes());
    let [low, high] = campaigns;
    // One holder per campaign. The ordered closure parks on the low campaign (its first lock),
    // the reversed one on the high campaign (its first lock).
    let holder_low = hold_campaign(&world, &[low]).await;
    let holder_high = hold_campaign(&world, &[high]).await;
    let ordered = chunk_task(&world, &b_role, first, false);
    world.wait_for_lock_waiters(1).await;
    let reversed = chunk_task(&world, &b_role, second, true);
    world.wait_for_lock_waiters(2).await;
    // The high holder goes: the reversed closure takes the high campaign and asks for the low one,
    // queueing behind the ordered closure, which asked for it first (row lock queues are FIFO).
    release(&holder_high).await;
    world.wait_for_tuple_queue(1).await;
    // The low holder goes: the ordered closure gets the low campaign and asks for the high one,
    // which the reversed closure holds while it waits for the low one: a deadlock.
    release(&holder_low).await;
    let (ordered, reversed) = (ordered.await.unwrap(), reversed.await.unwrap());
    assert!(
        ordered.is_err() ^ reversed.is_err(),
        "exactly one is chosen as the victim: {ordered:?} {reversed:?}"
    );
    Arc::try_unwrap(world)
        .ok()
        .expect("one owner")
        .cleanup()
        .await;
}

#[tokio::test]
async fn the_old_acceptance_order_of_campaign_then_account_deadlocks_against_a_completion() {
    // The order the design replaced: the acceptance takes the campaign and then the account while
    // B's completion takes the account and then the campaign. The harness shows it would be caught.
    let bench = Bench::new("oldorder").await;
    let owner = bench.world.account(1).await;
    let guest = bench.world.account(2).await;
    let campaign = bench.world.campaign(owner, 4).await;
    bench
        .world
        .join(campaign.id, owner, guest, JoinRole::Player)
        .await;
    let acceptance = bench.world.db.raw_as(&bench.world.role).await;
    let completion = bench.b().await;
    acceptance.client.batch_execute("BEGIN").await.unwrap();
    acceptance
        .client
        .query(
            "SELECT 1 FROM campaigns.campaign WHERE campaign_id = $1 FOR UPDATE",
            &[&campaign.id.as_bytes().as_slice()],
        )
        .await
        .unwrap();
    completion.client.batch_execute("BEGIN").await.unwrap();
    completion
        .client
        .execute(
            "UPDATE accounts.account SET status = 'closing' WHERE account_id = $1",
            &[&guest.as_bytes().as_slice()],
        )
        .await
        .unwrap();
    let guest_id: &[u8] = guest.as_bytes();
    let campaign_id: &[u8] = campaign.id.as_bytes();
    let guest_params: [&(dyn tokio_postgres::types::ToSql + Sync); 1] = [&guest_id];
    let campaign_params: [&(dyn tokio_postgres::types::ToSql + Sync); 1] = [&campaign_id];
    let (account_wait, campaign_wait) = tokio::join!(
        acceptance
            .client
            .query("SELECT accounts.lock_account_status($1)", &guest_params),
        completion.client.query(
            "SELECT 1 FROM campaigns.campaign WHERE campaign_id = $1 FOR UPDATE",
            &campaign_params
        ),
    );
    let codes: Vec<_> = [account_wait.err(), campaign_wait.err()]
        .into_iter()
        .flatten()
        .filter_map(|error| error.code().map(|code| code.code().to_owned()))
        .collect();
    assert_eq!(
        codes,
        ["40P01"],
        "exactly one side is chosen as the deadlock victim"
    );
    bench.world.cleanup().await;
}

#[tokio::test]
async fn a_reopening_that_holds_the_campaign_lock_makes_the_due_flip_find_the_schedule_cleared() {
    let bench = Bench::new("cancelfirst").await;
    let Bench {
        world,
        b_role,
        participant,
    } = bench;
    let world = Arc::new(world);
    let owner = world.account(1).await;
    let campaign = world.campaign(owner, 4).await;
    archive(&world, owner, campaign.id).await;
    let notice = world.store.tunables().closure_archived_delete_notice_ms();
    let mut b = world.db.raw_as(&b_role).await;
    let tx = b.client.transaction().await.unwrap();
    mark_closing(&tx, owner).await;
    participant
        .schedule_archived_deletions(&tx, owner, world.now() + notice)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    world.clock.advance_ms(notice);
    // The person logs in: the reopening cancels the schedule and holds the campaign lock.
    world.db.set_account_status(owner, "active").await;
    let tx = b.client.transaction().await.unwrap();
    let report = participant
        .cancel_scheduled_deletions_report(&tx, owner)
        .await
        .unwrap();
    assert_eq!(report.cancelled, [campaign.id]);
    let flip = {
        let world = Arc::clone(&world);
        tokio::spawn(async move { world.store.execute_scheduled_deletions(world.now()).await })
    };
    world.wait_for_lock_waiters(1).await;
    tx.commit().await.unwrap();
    assert_eq!(flip.await.unwrap().unwrap().deletions_started, 0);
    let snapshot = world
        .store
        .member_snapshot(owner, campaign.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.campaign_status, CampaignStatus::Archived);
    Arc::try_unwrap(world)
        .ok()
        .expect("one owner")
        .cleanup()
        .await;
}

#[tokio::test]
async fn a_flip_queued_before_the_cancel_wins_and_the_cancel_is_refused() {
    let bench = Bench::new("flipfirst").await;
    let Bench {
        world,
        b_role,
        participant,
    } = bench;
    let world = Arc::new(world);
    let owner = world.account(1).await;
    let campaign = world.campaign(owner, 4).await;
    archive(&world, owner, campaign.id).await;
    let notice = world.store.tunables().closure_archived_delete_notice_ms();
    let mut b = world.db.raw_as(&b_role).await;
    let tx = b.client.transaction().await.unwrap();
    mark_closing(&tx, owner).await;
    participant
        .schedule_archived_deletions(&tx, owner, world.now() + notice)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    world.clock.advance_ms(notice);
    // The flip asks for the lock first, the cancel second; the queue decides.
    let holder = hold_campaign(&world, &[campaign.id]).await;
    let flip = {
        let world = Arc::clone(&world);
        tokio::spawn(async move { world.store.execute_scheduled_deletions(world.now()).await })
    };
    world.wait_for_lock_waiters(1).await;
    let cancel = {
        let world = Arc::clone(&world);
        let b_role = b_role.clone();
        tokio::spawn(async move {
            let participant = world
                .store
                .closure_participant(Arc::new(world.clock.clone()));
            let mut session = world.db.raw_as(&b_role).await;
            let tx = session.client.transaction().await.unwrap();
            let report = participant
                .cancel_scheduled_deletions_report(&tx, owner)
                .await
                .unwrap();
            tx.commit().await.unwrap();
            report
        })
    };
    world.wait_for_lock_waiters(2).await;
    release(&holder).await;
    assert_eq!(flip.await.unwrap().unwrap().deletions_started, 1);
    let report = cancel.await.unwrap();
    assert_eq!(report.already_deleting, [campaign.id]);
    assert!(report.cancelled.is_empty());
    Arc::try_unwrap(world)
        .ok()
        .expect("one owner")
        .cleanup()
        .await;
}

#[tokio::test]
async fn an_unarchive_parked_on_the_campaign_lock_holds_the_account_lock_so_the_close_waits_and_then_sees_a_blocker()
 {
    let bench = Bench::new("unarchiveholds").await;
    let Bench { world, b_role, .. } = bench;
    let world = Arc::new(world);
    let owner = world.account(1).await;
    let campaign = world.campaign(owner, 4).await;
    archive(&world, owner, campaign.id).await;
    let holder = hold_campaign(&world, &[campaign.id]).await;
    let unarchive = {
        let world = Arc::clone(&world);
        tokio::spawn(async move {
            world
                .store
                .unarchive_campaign(owner, campaign.id, world.now())
                .await
        })
    };
    // It has taken the account share lock and now waits for the campaign.
    world.wait_for_lock_waiters(1).await;
    let close = {
        let world = Arc::clone(&world);
        let b_role = b_role.clone();
        tokio::spawn(async move {
            let participant = world
                .store
                .closure_participant(Arc::new(world.clock.clone()));
            let mut session = world.db.raw_as(&b_role).await;
            let tx = session.client.transaction().await.unwrap();
            mark_closing(&tx, owner).await;
            let check = participant.closure_campaigns(&tx, owner).await.unwrap();
            tx.rollback().await.unwrap();
            check.blocking.len()
        })
    };
    // B's update of the account row waits for the share lock the unarchive still holds.
    world.wait_for_lock_waiters(2).await;
    release(&holder).await;
    unarchive
        .await
        .unwrap()
        .expect("the unarchive commits first");
    assert_eq!(
        close.await.unwrap(),
        1,
        "the unarchived campaign blocks the close"
    );
    Arc::try_unwrap(world)
        .ok()
        .expect("one owner")
        .cleanup()
        .await;
}

#[tokio::test]
async fn an_unarchive_by_an_owner_who_is_not_the_host_waits_for_the_owners_close_and_is_refused() {
    let bench = Bench::new("unarchiveowner").await;
    let Bench { world, b_role, .. } = bench;
    let world = Arc::new(world);
    let owner = world.account(1).await;
    let host = world.account(2).await;
    let campaign = world.campaign(owner, 4).await;
    let host_member = world.join(campaign.id, owner, host, JoinRole::Player).await;
    let proposal = world
        .store
        .propose_transfer(
            owner,
            campaign.id,
            NewProposal {
                kind: TransferKind::Host,
                to_member: host_member,
                old_host_becomes: None,
            },
            world.now(),
        )
        .await
        .unwrap();
    world
        .store
        .accept_transfer(host, campaign.id, proposal, world.now())
        .await
        .unwrap();
    archive(&world, owner, campaign.id).await;
    // B marks the owner closing and has not committed: the owner's account row is locked.
    let mut b = world.db.raw_as(&b_role).await;
    let tx = b.client.transaction().await.unwrap();
    mark_closing(&tx, owner).await;
    let unarchive = {
        let world = Arc::clone(&world);
        tokio::spawn(async move {
            world
                .store
                .unarchive_campaign(owner, campaign.id, world.now())
                .await
        })
    };
    world.wait_for_lock_waiters(1).await;
    tx.commit().await.unwrap();
    assert_eq!(
        refusal_of(unarchive.await.unwrap()),
        Refusal::OwnerAccountNotActive
    );
    Arc::try_unwrap(world)
        .ok()
        .expect("one owner")
        .cleanup()
        .await;
}

#[tokio::test]
async fn a_seat_taken_by_a_join_and_a_seat_retaken_by_the_host_cannot_both_take_the_last_one() {
    let world = Arc::new(World::new("lastseat").await);
    let host = world.account(1).await;
    let guest = world.account(2).await;
    let newcomer = world.account(3).await;
    // Two seats; the host steps out of theirs, so one is free for the host's return or a join.
    let campaign = world.campaign(host, 2).await;
    world.join(campaign.id, host, guest, JoinRole::Player).await;
    world
        .store
        .set_seat(host, campaign.id, false, world.now())
        .await
        .unwrap();
    let token = token_digest();
    world
        .store
        .create_invitation(
            host,
            campaign.id,
            NewInvitation {
                joins_as: JoinRole::Player,
                lifetime_ms: 3_600_000,
                max_uses: 3,
                token_digest: token,
            },
            world.now(),
        )
        .await
        .unwrap();
    let holder = hold_campaign(&world, &[campaign.id]).await;
    let retake = {
        let world = Arc::clone(&world);
        tokio::spawn(async move {
            world
                .store
                .set_seat(host, campaign.id, true, world.now())
                .await
        })
    };
    let join = {
        let world = Arc::clone(&world);
        tokio::spawn(async move {
            world
                .store
                .accept_invitation(newcomer, token, operation_id(), world.now())
                .await
        })
    };
    world.wait_for_lock_waiters(2).await;
    release(&holder).await;
    let (retake, join) = (retake.await.unwrap(), join.await.unwrap());
    let retaken = retake.is_ok();
    let joined = join.is_ok();
    assert!(
        retaken ^ joined,
        "exactly one takes the last seat: {retake:?} {join:?}"
    );
    if !retaken {
        assert_eq!(refusal_of(retake), Refusal::SeatsFull);
    } else {
        assert_eq!(refusal_of(join), Refusal::Join(JoinFailure::Full));
    }
    let seats: i64 = world
        .db
        .admin
        .client
        .query_one(
            "SELECT count(*) FROM campaigns.campaign_member \
             WHERE campaign_id = $1 AND status = 'active' AND holds_seat",
            &[&campaign.id.as_bytes().as_slice()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(seats, 2);
    Arc::try_unwrap(world)
        .ok()
        .expect("one owner")
        .cleanup()
        .await;
}

#[tokio::test]
async fn two_campaigns_approving_one_pairing_at_once_let_exactly_one_win() {
    let world = Arc::new(World::new("doubleapprove").await);
    let host_x = world.account(1).await;
    let host_y = world.account(2).await;
    let x = world.campaign(host_x, 4).await;
    let y = world.campaign(host_y, 4).await;
    let pairing = world
        .store
        .begin_pairing(
            df_campaign::store::NewPairing {
                device_digest: digest_of("device"),
                device_label: df_campaign::records::DeviceLabel::Desktop,
                ip_key: digest_of("ip"),
                ip_key_loose: digest_of("loose"),
                user_code: "CODE0001".into(),
            },
            600_000,
            world.now(),
        )
        .await
        .unwrap();
    // A holder owns the pairing row; each approval takes its own campaign lock and then waits.
    let holder = world.db.raw_as(&world.role).await;
    holder.client.batch_execute("BEGIN").await.unwrap();
    holder
        .client
        .query(
            "SELECT 1 FROM campaigns.display_pairing WHERE pairing_id = $1 FOR UPDATE",
            &[&pairing.id.as_bytes().as_slice()],
        )
        .await
        .unwrap();
    let approve = |campaign: CampaignId, host: AccountId, session: u8| {
        let world = Arc::clone(&world);
        tokio::spawn(async move {
            world
                .store
                .approve_pairing(host, campaign, session_id(session), pairing.id, world.now())
                .await
        })
    };
    let first = approve(x.id, host_x, 1);
    let second = approve(y.id, host_y, 2);
    world.wait_for_lock_waiters(2).await;
    release(&holder).await;
    let results = [first.await.unwrap(), second.await.unwrap()];
    let winners = results.iter().filter(|result| result.is_ok()).count();
    assert_eq!(winners, 1, "{results:?}");
    let loser = results.iter().find(|result| result.is_err()).unwrap();
    assert_eq!(
        *loser,
        Err(Failure::Refused(Refusal::Pairing(
            df_campaign::display_pairing::PairingRefusal::NotPending
        )))
    );
    // The row names one campaign and one session together.
    let row = world
        .db
        .admin
        .client
        .query_one(
            "SELECT campaign_id, session_id FROM campaigns.display_pairing WHERE pairing_id = $1",
            &[&pairing.id.as_bytes().as_slice()],
        )
        .await
        .unwrap();
    let campaign: Vec<u8> = row.get(0);
    let session: Vec<u8> = row.get(1);
    let expected = if results[0].is_ok() {
        (x.id.as_bytes().to_vec(), session_id(1).as_bytes().to_vec())
    } else {
        (y.id.as_bytes().to_vec(), session_id(2).as_bytes().to_vec())
    };
    assert_eq!((campaign, session), expected);
    Arc::try_unwrap(world)
        .ok()
        .expect("one owner")
        .cleanup()
        .await;
}

#[tokio::test]
async fn two_reissues_at_once_replace_one_ticket_each_and_stay_inside_the_cap() {
    let world = Arc::new(World::new("tworeissues").await);
    let host = world.account(1).await;
    let campaign = world.campaign(host, 4).await;
    let session = session_id(1);
    for number in 1..=2_u32 {
        let pairing = world
            .store
            .begin_pairing(
                df_campaign::store::NewPairing {
                    device_digest: digest_of(&format!("device{number}")),
                    device_label: df_campaign::records::DeviceLabel::Desktop,
                    ip_key: digest_of(&format!("ip{number}")),
                    ip_key_loose: digest_of(&format!("loose{number}")),
                    user_code: format!("CODE{number:04}"),
                },
                600_000,
                world.now(),
            )
            .await
            .unwrap();
        world
            .store
            .approve_pairing(host, campaign.id, session, pairing.id, world.now())
            .await
            .unwrap();
        world
            .store
            .redeem_pairing(
                digest_of(&format!("device{number}")),
                None,
                digest_of(&format!("first{number}")),
                true,
                world.now(),
            )
            .await
            .unwrap();
    }
    let holder = hold_campaign(&world, &[campaign.id]).await;
    let reissue = |number: u32| {
        let world = Arc::clone(&world);
        tokio::spawn(async move {
            world
                .store
                .redeem_pairing(
                    digest_of(&format!("device{number}")),
                    None,
                    digest_of(&format!("second{number}")),
                    true,
                    world.now() + 1_000,
                )
                .await
        })
    };
    let one = reissue(1);
    let two = reissue(2);
    world.wait_for_lock_waiters(2).await;
    release(&holder).await;
    assert!(one.await.unwrap().unwrap().reissued);
    assert!(two.await.unwrap().unwrap().reissued);
    let live: i64 = world
        .db
        .admin
        .client
        .query_one(
            "SELECT count(*) FROM campaigns.display_ticket \
             WHERE campaign_id = $1 AND revoked_at_ms IS NULL",
            &[&campaign.id.as_bytes().as_slice()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(live, 2, "each reissue replaced its own ticket");
    Arc::try_unwrap(world)
        .ok()
        .expect("one owner")
        .cleanup()
        .await;
}

#[tokio::test]
async fn the_membership_read_uses_its_own_pool_and_is_not_queued_behind_a_blocked_write() {
    let mut world = World::new("readpool").await;
    let host = world.account(1).await;
    let campaign = world.campaign(host, 4).await;
    let role = world.db.login_role(Some("10s")).await;
    // One connection in each pool: a write that waits on a lock occupies the only write slot.
    let store = Arc::new(
        PgCampaignStore::open(
            world.db.connector(&role),
            world.db.connector(&role),
            1,
            df_campaign::config::Tunables::default(),
            TimeoutPolicy::Enforced,
        )
        .await
        .unwrap(),
    );
    let holder = hold_campaign(&world, &[campaign.id]).await;
    let blocked = {
        let store = Arc::clone(&store);
        let now = world.now();
        tokio::spawn(async move { store.set_seat(host, campaign.id, false, now).await })
    };
    world.wait_for_lock_waiters(1).await;
    let snapshot = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        store.member_snapshot(host, campaign.id),
    )
    .await
    .expect("the read is not queued behind the blocked write")
    .unwrap()
    .unwrap();
    assert_eq!(snapshot.status, MemberStatus::Active);
    assert_eq!(snapshot.role, CampaignRole::Host);
    release(&holder).await;
    blocked.await.unwrap().unwrap();
    world.cleanup().await;
}

#[tokio::test]
async fn the_membership_snapshot_is_one_statement_over_member_campaign_and_account_status() {
    let sql = crate::campaign_store::snapshot_sql();
    assert!(
        !sql.trim_end_matches(';').contains(';'),
        "two statements would let an event commit between them"
    );
    for part in [
        "campaigns.campaign_member",
        "JOIN campaigns.campaign",
        "accounts.account_status",
        "feed_epoch",
        "event_seq",
    ] {
        assert!(sql.contains(part), "{part}");
    }
}

#[tokio::test]
async fn two_creations_at_the_cap_let_exactly_one_through() {
    let tunables = df_campaign::config::Tunables {
        campaigns_per_account: 2,
        ..df_campaign::config::Tunables::default()
    };
    let world = Arc::new(World::with_tunables("createcap", tunables).await);
    let owner = world.account(1).await;
    world.campaign(owner, 3).await;
    // A session holds the account's creation lock, standing in for a creation that has counted
    // and not yet inserted. Both creations must queue on it instead of counting one campaign each.
    let holder = world.db.raw_as(&world.role).await;
    holder.client.batch_execute("BEGIN").await.unwrap();
    holder
        .client
        .execute(
            "SELECT pg_advisory_xact_lock(hashtextextended('df_campaigns.create:' || \
             encode($1::bytea, 'hex'), 0))",
            &[&owner.as_bytes().as_slice()],
        )
        .await
        .unwrap();
    let create = |name: &'static str| {
        let world = Arc::clone(&world);
        tokio::spawn(async move {
            world
                .store
                .create_campaign(
                    df_campaign::records::NewCampaign {
                        tenant: crate::fixture::tenant_id(),
                        owner,
                        name: name.into(),
                        seat_count: 3,
                        content_rating: df_campaign::records::ContentRating::Standard,
                        room_codes: df_campaign::records::RoomCodes::Off,
                        default_co_host_delegations: df_campaign::roles::Delegations::NONE,
                    },
                    operation_id(),
                    digest_of(name),
                    world.now(),
                )
                .await
        })
    };
    let (first, second) = (create("First"), create("Second"));
    world.wait_for_lock_waiters(2).await;
    release(&holder).await;
    let (first, second) = (first.await.unwrap(), second.await.unwrap());
    assert!(
        first.is_ok() ^ second.is_ok(),
        "exactly one fits under the cap: {first:?} {second:?}"
    );
    let refused = if first.is_err() { first } else { second };
    assert_eq!(refusal_of(refused), Refusal::TooManyCampaigns);
    let owned: i64 = world
        .db
        .admin
        .client
        .query_one(
            "SELECT count(*) FROM campaigns.campaign WHERE owner_account_id = $1",
            &[&owner.as_bytes().as_slice()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(owned, 2);
    Arc::try_unwrap(world)
        .ok()
        .expect("one owner")
        .cleanup()
        .await;
}
