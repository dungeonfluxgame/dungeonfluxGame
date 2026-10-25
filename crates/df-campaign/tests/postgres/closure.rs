//! Account closure: the participant B calls inside its transaction, the chunked `leave_all`,
//! scheduled deletions and the races with `unarchive` and transfer acceptance. B's role and
//! transaction are stood in for by a login role in `df_accounts`.

use df_campaign::store::NewProposal;
use std::sync::Arc;

use df_campaign::{
    errors::Refusal,
    join_invitation::JoinRole,
    records::{CampaignStatus, EventKind, LeaveChunk, MemberStatus, TransferKind},
    roles::CampaignRole,
    store::CampaignStore,
};
use df_customer::BlockReason;
use df_types::{AccountId, CampaignId};

use crate::{
    account_closure::ClosureParticipant,
    campaign_store::PgClosureParticipant,
    fixture::{Raw, World},
    membership::refusal_of,
};

pub(crate) struct Bench {
    pub(crate) world: World,
    pub(crate) b_role: String,
    pub(crate) participant: PgClosureParticipant,
}

impl Bench {
    pub(crate) async fn new(label: &str) -> Self {
        Self::with(label, df_campaign::config::Tunables::default()).await
    }

    pub(crate) async fn with(label: &str, tunables: df_campaign::config::Tunables) -> Self {
        let mut world = World::with_tunables(label, tunables).await;
        let b_role = world.db.accounts_login_role(Some("10s")).await;
        let participant = world
            .store
            .closure_participant(Arc::new(world.clock.clone()));
        Self {
            world,
            b_role,
            participant,
        }
    }

    /// B's session.
    pub(crate) async fn b(&self) -> Raw {
        self.world.db.raw_as(&self.b_role).await
    }
}

/// B's first statement: the account row set to `closing` and held until the transaction ends.
pub(crate) async fn mark_closing(tx: &tokio_postgres::Transaction<'_>, account: AccountId) {
    tx.execute(
        "UPDATE accounts.account SET status = 'closing' WHERE account_id = $1",
        &[&account.as_bytes().as_slice()],
    )
    .await
    .unwrap();
}

async fn status_of(bench: &Bench, account: AccountId) -> String {
    bench
        .world
        .db
        .admin
        .client
        .query_one(
            "SELECT status FROM accounts.account WHERE account_id = $1",
            &[&account.as_bytes().as_slice()],
        )
        .await
        .unwrap()
        .get(0)
}

pub(crate) async fn archive(world: &World, owner: AccountId, campaign: CampaignId) {
    world
        .store
        .archive_campaign(
            owner,
            campaign,
            "The Sunken Bell".into(),
            false,
            world.now(),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn closure_is_blocked_while_the_account_owns_or_hosts_an_active_campaign() {
    let bench = Bench::new("blocked").await;
    let world = &bench.world;
    let owner = world.account(1).await;
    let campaign = world.campaign(owner, 4).await;
    let mut b = bench.b().await;
    let tx = b.client.transaction().await.unwrap();
    mark_closing(&tx, owner).await;
    let check = bench
        .participant
        .closure_campaigns(&tx, owner)
        .await
        .unwrap();
    assert_eq!(check.blocking.len(), 1);
    assert_eq!(check.blocking[0].campaign, campaign.id);
    assert_eq!(check.blocking[0].reason, BlockReason::OwnsAndHosts);
    // B rolls back: the user is not left closing and can follow the transfer links.
    tx.rollback().await.unwrap();
    assert_eq!(status_of(&bench, owner).await, "active");
    bench.world.cleanup().await;
}

#[tokio::test]
async fn closing_with_archived_campaigns_schedules_their_deletion_with_a_notice() {
    let bench = Bench::new("schedule").await;
    let world = &bench.world;
    let owner = world.account(1).await;
    let guest = world.account(2).await;
    let owned = world.campaign(owner, 4).await;
    world.join(owned.id, owner, guest, JoinRole::Player).await;
    archive(world, owner, owned.id).await;
    let mut b = bench.b().await;
    let tx = b.client.transaction().await.unwrap();
    mark_closing(&tx, owner).await;
    let check = bench
        .participant
        .closure_campaigns(&tx, owner)
        .await
        .unwrap();
    assert!(check.blocking.is_empty());
    assert_eq!(check.owned_archived.len(), 1);
    assert_eq!(check.owned_archived[0].campaign, owned.id);
    let delete_after = world.now() + 30 * 24 * 3_600_000;
    let scheduled = bench
        .participant
        .schedule_archived_deletions(&tx, owner, delete_after)
        .await
        .unwrap();
    assert_eq!(scheduled.len(), 1);
    assert_eq!(scheduled[0].campaign, owned.id);
    assert!(scheduled[0].members.contains(&guest) && scheduled[0].members.contains(&owner));
    tx.commit().await.unwrap();
    let events = world.events(owned.id).await;
    let notice = events
        .iter()
        .find(|event| event.kind == EventKind::DeleteScheduled)
        .unwrap();
    assert!(notice.revoking && notice.member.is_none());
    let stored = world
        .db
        .admin
        .client
        .query_one(
            "SELECT scheduled_delete_at_ms FROM campaigns.campaign WHERE campaign_id = $1",
            &[&owned.id.as_bytes().as_slice()],
        )
        .await
        .unwrap();
    assert_eq!(
        stored.get::<_, Option<i64>>(0),
        Some(i64::try_from(delete_after).unwrap())
    );
    // Members still read the campaign meanwhile.
    let snapshot = world
        .store
        .member_snapshot(guest, owned.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.campaign_status, CampaignStatus::Archived);
    bench.world.cleanup().await;
}

#[tokio::test]
async fn a_host_proposal_in_a_campaign_it_does_not_own_is_cancelled_by_closure() {
    let bench = Bench::new("cancelproposals").await;
    let world = &bench.world;
    let owner = world.account(1).await;
    let host = world.account(2).await;
    let target = world.account(3).await;
    let campaign = world.campaign(owner, 4).await;
    let host_member = world.join(campaign.id, owner, host, JoinRole::Player).await;
    let target_member = world
        .join(campaign.id, owner, target, JoinRole::Player)
        .await;
    let to_host = world
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
        .accept_transfer(host, campaign.id, to_host, world.now())
        .await
        .unwrap();
    // `host` hosts a campaign owned by someone else and proposes a hand-over.
    let proposal = world
        .store
        .propose_transfer(
            host,
            campaign.id,
            NewProposal {
                kind: TransferKind::Host,
                to_member: target_member,
                old_host_becomes: None,
            },
            world.now(),
        )
        .await
        .unwrap();
    // Archived campaigns do not block, so archive it (the owner may) to let `host` close.
    archive(world, owner, campaign.id).await;
    let mut b = bench.b().await;
    let tx = b.client.transaction().await.unwrap();
    mark_closing(&tx, host).await;
    assert!(
        bench
            .participant
            .closure_campaigns(&tx, host)
            .await
            .unwrap()
            .blocking
            .is_empty()
    );
    bench
        .participant
        .schedule_archived_deletions(&tx, host, world.now() + 1)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let state: String = world
        .db
        .admin
        .client
        .query_one(
            "SELECT state FROM campaigns.transfer_proposal WHERE proposal_id = $1",
            &[&proposal.as_bytes().as_slice()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(state, "cancelled");
    bench.world.cleanup().await;
}

#[tokio::test]
async fn reopening_cancels_scheduled_deletions_but_not_one_already_deleting() {
    let bench = Bench::new("reopen").await;
    let world = &bench.world;
    let owner = world.account(1).await;
    let first = world.campaign(owner, 4).await;
    let second = world.campaign(owner, 4).await;
    archive(world, owner, first.id).await;
    archive(world, owner, second.id).await;
    let mut b = bench.b().await;
    let tx = b.client.transaction().await.unwrap();
    mark_closing(&tx, owner).await;
    bench
        .participant
        .schedule_archived_deletions(&tx, owner, world.now() + 1_000)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    // The retention task flips the due ones under the campaign lock. A closure never gives less
    // than the configured notice, however short a date it asked for.
    let notice = world.store.tunables().closure_archived_delete_notice_ms();
    world.clock.advance_ms(notice - 1);
    assert_eq!(
        world
            .store
            .execute_scheduled_deletions(world.now())
            .await
            .unwrap()
            .deletions_started,
        0,
        "not due before the notice is over"
    );
    world.clock.advance_ms(1);
    let report = world
        .store
        .execute_scheduled_deletions(world.now())
        .await
        .unwrap();
    assert_eq!(report.deletions_started, 2);
    // The person logs in again (the account is active), archives a third campaign and closes
    // again; reopening then cancels that schedule while the two already deleting stay so.
    world.db.set_account_status(owner, "active").await;
    let third = world.campaign(owner, 4).await;
    archive(world, owner, third.id).await;
    let tx = b.client.transaction().await.unwrap();
    mark_closing(&tx, owner).await;
    bench
        .participant
        .schedule_archived_deletions(&tx, owner, world.now() + 1_000_000)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let tx = b.client.transaction().await.unwrap();
    let report = bench
        .participant
        .cancel_scheduled_deletions_report(&tx, owner)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(report.cancelled, [third.id]);
    assert_eq!(report.already_deleting.len(), 2);
    assert!(
        world
            .events(third.id)
            .await
            .iter()
            .any(|e| e.kind == EventKind::DeleteCancelled)
    );
    bench.world.cleanup().await;
}

#[tokio::test]
async fn leave_all_chunks_resume_after_a_stop_and_skip_what_the_account_owns() {
    let tunables = df_campaign::config::Tunables {
        leave_all_chunk: 2,
        ..Default::default()
    };
    let bench = Bench::with("chunks", tunables).await;
    let world = &bench.world;
    let leaver = world.account(1).await;
    let mut campaigns = Vec::new();
    for number in 2..7 {
        let host = world.account(number).await;
        let campaign = world.campaign(host, 4).await;
        world
            .join(campaign.id, host, leaver, JoinRole::Player)
            .await;
        campaigns.push(campaign.id);
    }
    let owned = world.campaign(leaver, 4).await;
    archive(world, leaver, owned.id).await;
    let mut b = bench.b().await;
    let mut after = None;
    let mut left = Vec::new();
    let mut skipped = Vec::new();
    let mut chunks = 0;
    loop {
        // Each chunk is its own transaction and begins with B's account statement.
        let tx = b.client.transaction().await.unwrap();
        mark_closing(&tx, leaver).await;
        let report = bench
            .participant
            .leave_all_chunk(&tx, leaver, after)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        chunks += 1;
        left.extend(report.left);
        skipped.extend(report.skipped_owned);
        match report.outcome {
            LeaveChunk::Continue(last) => after = Some(last),
            LeaveChunk::Done => break,
            LeaveChunk::Aborted => panic!("the account is closing"),
        }
        assert!(chunks < 10);
    }
    assert!(chunks >= 3, "six memberships in chunks of two");
    left.sort();
    let mut expected = campaigns.clone();
    expected.sort();
    assert_eq!(left, expected);
    assert_eq!(skipped, [owned.id]);
    for campaign in &campaigns {
        let snapshot = world
            .store
            .member_snapshot(leaver, *campaign)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(snapshot.status, MemberStatus::Left);
    }
    let kept = world
        .store
        .member_snapshot(leaver, owned.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        kept.status,
        MemberStatus::Active,
        "an owned campaign keeps its membership"
    );
    bench.world.cleanup().await;
}

#[tokio::test]
async fn a_chunk_aborts_when_the_account_was_reopened_between_chunks() {
    let bench = Bench::new("aborted").await;
    let world = &bench.world;
    let host = world.account(1).await;
    let leaver = world.account(2).await;
    let campaign = world.campaign(host, 4).await;
    world
        .join(campaign.id, host, leaver, JoinRole::Player)
        .await;
    let mut b = bench.b().await;
    // The account is active (a reopening login landed): the chunk changes nothing.
    let tx = b.client.transaction().await.unwrap();
    let report = bench
        .participant
        .leave_all_chunk(&tx, leaver, None)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(report.outcome, LeaveChunk::Aborted);
    assert!(report.left.is_empty());
    let snapshot = world
        .store
        .member_snapshot(leaver, campaign.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.status, MemberStatus::Active);
    bench.world.cleanup().await;
}

/// `owner` owns an archived campaign that `old_host` hosts; `owner` is a seatless contributor.
async fn archived_with_seatless_owner(world: &World) -> (AccountId, AccountId, CampaignId) {
    let owner = world.account(1).await;
    let old_host = world.account(2).await;
    let campaign = world.campaign(owner, 4).await;
    let old_host_member = world
        .join(campaign.id, owner, old_host, JoinRole::Player)
        .await;
    let proposal = world
        .store
        .propose_transfer(
            owner,
            campaign.id,
            NewProposal {
                kind: TransferKind::Host,
                to_member: old_host_member,
                old_host_becomes: None,
            },
            world.now(),
        )
        .await
        .unwrap();
    world
        .store
        .accept_transfer(old_host, campaign.id, proposal, world.now())
        .await
        .unwrap();
    let owner_member = world
        .store
        .member_snapshot(owner, campaign.id)
        .await
        .unwrap()
        .unwrap()
        .member;
    world
        .store
        .set_player_role(
            old_host,
            campaign.id,
            owner_member,
            JoinRole::Contributor,
            world.now(),
        )
        .await
        .unwrap();
    archive(world, owner, campaign.id).await;
    (owner, old_host, campaign.id)
}

#[tokio::test]
async fn leave_all_hands_the_host_role_to_a_contributor_owner_without_a_seat() {
    let bench = Bench::new("handover").await;
    let world = &bench.world;
    let (owner, old_host, campaign) = archived_with_seatless_owner(world).await;
    let mut b = bench.b().await;
    let tx = b.client.transaction().await.unwrap();
    mark_closing(&tx, old_host).await;
    assert!(
        bench
            .participant
            .closure_campaigns(&tx, old_host)
            .await
            .unwrap()
            .blocking
            .is_empty()
    );
    let report = bench
        .participant
        .leave_all_chunk(&tx, old_host, None)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(report.handed_over, [campaign]);
    let new_host = world
        .store
        .member_snapshot(owner, campaign)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(new_host.role, CampaignRole::Host);
    assert!(!new_host.holds_seat, "a seatless host is valid");
    let left = world
        .store
        .member_snapshot(old_host, campaign)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(left.status, MemberStatus::Left);
    let transferred: Vec<_> = world
        .events(campaign)
        .await
        .into_iter()
        .filter(|event| event.kind == EventKind::HostTransferred)
        .collect();
    // The setup's own transfer wrote two rows; the hand-over wrote two more, old revoking.
    assert_eq!(transferred.len(), 4);
    let last_two = &transferred[2..];
    assert!(
        last_two
            .iter()
            .find(|e| e.member == Some(left.member))
            .unwrap()
            .revoking
    );
    assert!(
        !last_two
            .iter()
            .find(|e| e.member == Some(new_host.member))
            .unwrap()
            .revoking
    );
    bench.world.cleanup().await;
}

#[tokio::test]
async fn leave_all_skips_the_handover_when_the_owner_is_not_active() {
    let bench = Bench::new("handoverskipped").await;
    let world = &bench.world;
    let (owner, old_host, campaign) = archived_with_seatless_owner(world).await;
    world.db.set_account_status(owner, "closing").await;
    let mut b = bench.b().await;
    let tx = b.client.transaction().await.unwrap();
    mark_closing(&tx, old_host).await;
    let report = bench
        .participant
        .leave_all_chunk(&tx, old_host, None)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(report.handover_skipped, [campaign]);
    assert!(report.left.is_empty());
    let kept = world
        .store
        .member_snapshot(old_host, campaign)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        kept.role,
        CampaignRole::Host,
        "the closing account keeps the host row"
    );
    assert_eq!(kept.status, MemberStatus::Active);
    assert!(!kept.authorized, "and is unauthorized meanwhile");
    bench.world.cleanup().await;
}

#[tokio::test]
async fn unarchive_is_refused_while_the_hosts_account_is_not_active() {
    let bench = Bench::new("unarchive").await;
    let world = &bench.world;
    let owner = world.account(1).await;
    let campaign = world.campaign(owner, 4).await;
    archive(world, owner, campaign.id).await;
    world.db.set_account_status(owner, "closing").await;
    assert_eq!(
        refusal_of(
            world
                .store
                .unarchive_campaign(owner, campaign.id, world.now())
                .await
        ),
        Refusal::HostAccountNotActive
    );
    world.db.set_account_status(owner, "closed").await;
    assert_eq!(
        refusal_of(
            world
                .store
                .unarchive_campaign(owner, campaign.id, world.now())
                .await
        ),
        Refusal::HostAccountNotActive
    );
    // A missing host account counts as not active.
    world
        .db
        .admin
        .client
        .execute(
            "DELETE FROM accounts.account WHERE account_id = $1",
            &[&owner.as_bytes().as_slice()],
        )
        .await
        .unwrap();
    assert_eq!(
        refusal_of(
            world
                .store
                .unarchive_campaign(owner, campaign.id, world.now())
                .await
        ),
        Refusal::HostAccountNotActive
    );
    bench.world.cleanup().await;
}

#[tokio::test]
async fn accept_transfer_and_the_completion_transaction_never_deadlock() {
    let bench = Bench::new("acceptdeadlock").await;
    let world = Arc::new(bench.world);
    let owner = world.account(1).await;
    let guest = world.account(2).await;
    let campaign = world.campaign(owner, 4).await;
    let guest_member = world
        .join(campaign.id, owner, guest, JoinRole::Player)
        .await;
    let proposal = world
        .store
        .propose_transfer(
            owner,
            campaign.id,
            NewProposal {
                kind: TransferKind::Ownership,
                to_member: guest_member,
                old_host_becomes: None,
            },
            world.now(),
        )
        .await
        .unwrap();
    // B's completion transaction: the account row first, then the campaigns of the account.
    let mut b = world.db.raw_as(&bench.b_role).await;
    let tx = b.client.transaction().await.unwrap();
    mark_closing(&tx, guest).await;
    let accept = {
        let world = Arc::clone(&world);
        tokio::spawn(async move {
            world
                .store
                .accept_transfer(guest, campaign.id, proposal, world.now())
                .await
        })
    };
    // The acceptance waits at the account row, holding no campaign lock.
    world.wait_for_lock_waiters(1).await;
    let report = bench
        .participant
        .leave_all_chunk(&tx, guest, None)
        .await
        .unwrap();
    assert_eq!(report.left, [campaign.id]);
    tx.commit().await.unwrap();
    assert_eq!(refusal_of(accept.await.unwrap()), Refusal::AccountClosing);
    Arc::try_unwrap(world)
        .ok()
        .expect("one owner")
        .cleanup()
        .await;
}

#[tokio::test]
async fn a_closure_method_in_a_callers_transaction_without_a_usable_timeout_is_refused() {
    let mut world = World::new("timeoutrefused").await;
    let participant = world
        .store
        .closure_participant(Arc::new(world.clock.clone()));
    let owner = world.account(1).await;
    for (timeout, accepted) in [
        (None, false),
        (Some("0"), false),
        (Some("60s"), false),
        (Some("1min"), false),
        (Some("10s"), true),
        (Some("10000ms"), true),
    ] {
        let role = world.db.accounts_login_role(timeout).await;
        let mut session = world.db.raw_as(&role).await;
        let tx = session.client.transaction().await.unwrap();
        let result = participant.closure_campaigns(&tx, owner).await;
        assert_eq!(result.is_ok(), accepted, "transaction_timeout {timeout:?}");
        tx.rollback().await.unwrap();
    }
    world.cleanup().await;
}

fn campaign_hex(campaign: CampaignId) -> String {
    crate::routes::id_text(campaign.as_bytes())
}

#[tokio::test]
async fn the_trait_leave_chunk_runs_one_chunk_and_says_where_to_resume() {
    df_observe::install_audit_sink(crate::route_limits::capture);
    let tunables = df_campaign::config::Tunables {
        leave_all_chunk: 2,
        ..Default::default()
    };
    let bench = Bench::with("traitleavechunk", tunables).await;
    let world = &bench.world;
    let few = world.account(1).await;
    let many = world.account(2).await;
    let reopened = world.account(3).await;
    for number in 10..13 {
        let host = world.account(number).await;
        let campaign = world.campaign(host, 4).await;
        world.join(campaign.id, host, many, JoinRole::Player).await;
        if number == 10 {
            world.join(campaign.id, host, few, JoinRole::Player).await;
            world
                .join(campaign.id, host, reopened, JoinRole::Player)
                .await;
        }
    }
    let mut b = bench.b().await;
    // Fewer memberships than the chunk holds: Done.
    let tx = b.client.transaction().await.unwrap();
    mark_closing(&tx, few).await;
    assert_eq!(
        bench.participant.leave_chunk(&tx, few, None).await.unwrap(),
        df_customer::LeaveChunk::Done
    );
    tx.commit().await.unwrap();
    // More than one chunk: the first says where to resume, and the second finishes.
    let tx = b.client.transaction().await.unwrap();
    mark_closing(&tx, many).await;
    let first = bench
        .participant
        .leave_chunk(&tx, many, None)
        .await
        .unwrap();
    let df_customer::LeaveChunk::Continue(last) = first else {
        panic!("two of three memberships is not the end: {first:?}");
    };
    tx.commit().await.unwrap();
    let remaining: i64 = world
        .db
        .admin
        .client
        .query_one(
            "SELECT count(*) FROM campaigns.campaign_member WHERE account_id = $1 AND status = 'active'",
            &[&many.as_bytes().as_slice()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(remaining, 1, "each chunk is its own transaction");
    let tx = b.client.transaction().await.unwrap();
    assert_eq!(
        bench
            .participant
            .leave_chunk(&tx, many, Some(last))
            .await
            .unwrap(),
        df_customer::LeaveChunk::Done
    );
    tx.commit().await.unwrap();
    // A reopened account (not closing): a distinct failure, never progress.
    let tx = b.client.transaction().await.unwrap();
    assert!(
        bench
            .participant
            .leave_chunk(&tx, reopened, None)
            .await
            .is_err()
    );
    tx.rollback().await.unwrap();
    // The cause is kept in the audit stream even though B's error has three variants.
    assert!(!crate::route_limits::lines_for("account_reopened").is_empty());
    bench.world.cleanup().await;
}

#[tokio::test]
async fn closure_alerts_and_notices_are_audited() {
    df_observe::install_audit_sink(crate::route_limits::capture);
    let bench = Bench::new("closureaudit").await;
    let world = &bench.world;
    let (owner, old_host, campaign) = archived_with_seatless_owner(world).await;
    world.db.set_account_status(owner, "closing").await;
    let mut b = bench.b().await;
    let tx = b.client.transaction().await.unwrap();
    mark_closing(&tx, old_host).await;
    bench
        .participant
        .leave_all_chunk(&tx, old_host, None)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let lines = crate::route_limits::lines_for(&campaign_hex(campaign));
    let alert = lines
        .iter()
        .find(|(event, _)| event == "feed.handover_skipped")
        .expect("the skipped hand-over is an alert");
    assert!(
        alert.1.starts_with("FEED handover_skipped campaign="),
        "{}",
        alert.1
    );
    // A scheduled deletion and a block are audited too.
    let lonely = world.account(30).await;
    let owned = world.campaign(lonely, 3).await;
    archive(world, lonely, owned.id).await;
    let tx = b.client.transaction().await.unwrap();
    mark_closing(&tx, lonely).await;
    bench
        .participant
        .schedule_archived_deletions(&tx, lonely, world.now())
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(
        crate::route_limits::lines_for(&campaign_hex(owned.id))
            .iter()
            .any(|(event, _)| event == "campaign.delete_scheduled")
    );
    bench.world.cleanup().await;
}

#[tokio::test]
async fn closing_deletes_the_accounts_unfinished_drafts_and_nothing_else() {
    use df_campaign::store::CampaignStore;
    let bench = Bench::new("closedrafts").await;
    let world = &bench.world;
    let owner = world.account(1).await;
    let other = world.account(2).await;
    let kept = world.campaign(owner, 4).await;
    let draft = world
        .store
        .start_draft(owner, crate::fixture::tenant_id(), world.now())
        .await
        .unwrap();
    let theirs = world
        .store
        .start_draft(other, crate::fixture::tenant_id(), world.now())
        .await
        .unwrap();
    // The campaign is archived first, as it must be before B closes the account.
    archive(world, owner, kept.id).await;
    let mut b = bench.b().await;
    let tx = b.client.transaction().await.unwrap();
    mark_closing(&tx, owner).await;
    bench
        .participant
        .schedule_archived_deletions(&tx, owner, world.now() + 30 * 24 * 3_600_000)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let remaining: Vec<Vec<u8>> = world
        .db
        .admin
        .client
        .query(
            "SELECT campaign_id FROM campaigns.campaign WHERE status = 'draft'",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(remaining, vec![theirs.id.as_bytes().to_vec()]);
    assert!(
        world
            .store
            .get_draft(owner, draft.id, world.now())
            .await
            .is_err()
    );
    bench.world.cleanup().await;
}
