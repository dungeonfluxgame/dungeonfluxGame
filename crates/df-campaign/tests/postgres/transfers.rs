//! Host and ownership transfers.

use df_campaign::store::NewProposal;
use df_campaign::{
    errors::Refusal,
    join_invitation::JoinRole,
    records::{EventKind, OldHostBecomes, TransferKind},
    roles::CampaignRole,
    store::CampaignStore,
};

use crate::{fixture::World, membership::refusal_of};

#[tokio::test]
async fn a_host_transfer_writes_one_revoking_row_for_the_old_host_and_one_plain_row_for_the_new() {
    let world = World::new("hosttransfer").await;
    let owner = world.account(1).await;
    let guest = world.account(2).await;
    let campaign = world.campaign(owner, 4).await;
    let guest_member = world
        .join(campaign.id, owner, guest, JoinRole::Player)
        .await;
    let owner_member = world
        .store
        .member_snapshot(owner, campaign.id)
        .await
        .unwrap()
        .unwrap()
        .member;
    let proposal = world
        .store
        .propose_transfer(
            owner,
            campaign.id,
            NewProposal {
                kind: TransferKind::Host,
                to_member: guest_member,
                old_host_becomes: Some(OldHostBecomes::CoHost),
            },
            world.now(),
        )
        .await
        .unwrap();
    world
        .store
        .accept_transfer(guest, campaign.id, proposal, world.now())
        .await
        .unwrap();
    let events: Vec<_> = world
        .events(campaign.id)
        .await
        .into_iter()
        .filter(|event| event.kind == EventKind::HostTransferred)
        .collect();
    assert_eq!(events.len(), 2);
    let old = events
        .iter()
        .find(|event| event.member == Some(owner_member))
        .unwrap();
    let new = events
        .iter()
        .find(|event| event.member == Some(guest_member))
        .unwrap();
    assert!(old.revoking && !new.revoking);
    // Roles swapped, and the owner is unchanged: a host transfer never touches ownership.
    let old_snapshot = world
        .store
        .member_snapshot(owner, campaign.id)
        .await
        .unwrap()
        .unwrap();
    let new_snapshot = world
        .store
        .member_snapshot(guest, campaign.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(old_snapshot.role, CampaignRole::CoHost);
    assert_eq!(new_snapshot.role, CampaignRole::Host);
    assert_eq!(old.member_revision, Some(old_snapshot.member_revision));
    let row = world
        .db
        .admin
        .client
        .query_one(
            "SELECT owner_account_id FROM campaigns.campaign WHERE campaign_id = $1",
            &[&campaign.id.as_bytes().as_slice()],
        )
        .await
        .unwrap();
    let stored_owner: Vec<u8> = row.get(0);
    assert_eq!(stored_owner, owner.as_bytes().to_vec());
    world.cleanup().await;
}

#[tokio::test]
async fn an_ownership_transfer_revokes_the_old_owner_principal_and_leaves_the_host_alone() {
    let world = World::new("ownertransfer").await;
    let owner = world.account(1).await;
    let guest = world.account(2).await;
    let campaign = world.campaign(owner, 4).await;
    let guest_member = world
        .join(campaign.id, owner, guest, JoinRole::Player)
        .await;
    let owner_member = world
        .store
        .member_snapshot(owner, campaign.id)
        .await
        .unwrap()
        .unwrap()
        .member;
    // Only the owner proposes ownership.
    assert_eq!(
        refusal_of(
            world
                .store
                .propose_transfer(
                    guest,
                    campaign.id,
                    NewProposal {
                        kind: TransferKind::Ownership,
                        to_member: owner_member,
                        old_host_becomes: None
                    },
                    world.now()
                )
                .await
        ),
        Refusal::OwnerOnly
    );
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
    world
        .store
        .accept_transfer(guest, campaign.id, proposal, world.now())
        .await
        .unwrap();
    let rows: Vec<_> = world
        .events(campaign.id)
        .await
        .into_iter()
        .filter(|event| event.kind == EventKind::OwnershipTransferred)
        .collect();
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .find(|e| e.member == Some(owner_member))
            .unwrap()
            .revoking
    );
    assert!(
        !rows
            .iter()
            .find(|e| e.member == Some(guest_member))
            .unwrap()
            .revoking
    );
    let host = world
        .store
        .member_snapshot(owner, campaign.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        host.role,
        CampaignRole::Host,
        "the host role stays with the host"
    );
    // The new owner may archive; the old one may not.
    assert_eq!(
        refusal_of(
            world
                .store
                .archive_campaign(
                    owner,
                    campaign.id,
                    "The Sunken Bell".into(),
                    false,
                    world.now()
                )
                .await
        ),
        Refusal::OwnerOnly
    );
    world
        .store
        .archive_campaign(
            guest,
            campaign.id,
            "The Sunken Bell".into(),
            false,
            world.now(),
        )
        .await
        .unwrap();
    world.cleanup().await;
}

#[tokio::test]
async fn ownership_cannot_go_to_a_contributor_and_acceptance_rechecks_the_same_rule() {
    let world = World::new("targets").await;
    let owner = world.account(1).await;
    let contributor = world.account(2).await;
    let player = world.account(3).await;
    let campaign = world.campaign(owner, 4).await;
    let contributor_member = world
        .join(campaign.id, owner, contributor, JoinRole::Contributor)
        .await;
    let player_member = world
        .join(campaign.id, owner, player, JoinRole::Player)
        .await;
    assert_eq!(
        refusal_of(
            world
                .store
                .propose_transfer(
                    owner,
                    campaign.id,
                    NewProposal {
                        kind: TransferKind::Ownership,
                        to_member: contributor_member,
                        old_host_becomes: None
                    },
                    world.now()
                )
                .await
        ),
        Refusal::TransferTargetNotAllowed
    );
    // Proposed to a seated player, who is then made a contributor before accepting.
    let proposal = world
        .store
        .propose_transfer(
            owner,
            campaign.id,
            NewProposal {
                kind: TransferKind::Ownership,
                to_member: player_member,
                old_host_becomes: None,
            },
            world.now(),
        )
        .await
        .unwrap();
    world
        .store
        .set_player_role(
            owner,
            campaign.id,
            player_member,
            JoinRole::Contributor,
            world.now(),
        )
        .await
        .unwrap();
    assert_eq!(
        refusal_of(
            world
                .store
                .accept_transfer(player, campaign.id, proposal, world.now())
                .await
        ),
        Refusal::TransferTargetNotAllowed
    );
    world.cleanup().await;
}

#[tokio::test]
async fn a_seatless_cohost_can_accept_the_host_role_and_stays_seatless() {
    let world = World::new("seatlesshost").await;
    let owner = world.account(1).await;
    let cohost = world.account(2).await;
    let campaign = world.campaign(owner, 4).await;
    let member = world
        .join(campaign.id, owner, cohost, JoinRole::Player)
        .await;
    world
        .store
        .set_co_host(
            owner,
            campaign.id,
            member,
            df_campaign::roles::Delegations::NONE,
            world.now(),
        )
        .await
        .unwrap();
    world
        .store
        .set_seat(cohost, campaign.id, false, world.now())
        .await
        .unwrap();
    let proposal = world
        .store
        .propose_transfer(
            owner,
            campaign.id,
            NewProposal {
                kind: TransferKind::Host,
                to_member: member,
                old_host_becomes: None,
            },
            world.now(),
        )
        .await
        .unwrap();
    world
        .store
        .accept_transfer(cohost, campaign.id, proposal, world.now())
        .await
        .unwrap();
    let snapshot = world
        .store
        .member_snapshot(cohost, campaign.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.role, CampaignRole::Host);
    assert!(!snapshot.holds_seat);
    world.cleanup().await;
}

#[tokio::test]
async fn acceptance_reverifies_revision_expiry_and_the_account() {
    let world = World::new("reverify").await;
    let owner = world.account(1).await;
    let guest = world.account(2).await;
    let campaign = world.campaign(owner, 4).await;
    let guest_member = world
        .join(campaign.id, owner, guest, JoinRole::Player)
        .await;
    let propose = || {
        world.store.propose_transfer(
            owner,
            campaign.id,
            NewProposal {
                kind: TransferKind::Ownership,
                to_member: guest_member,
                old_host_becomes: None,
            },
            world.now(),
        )
    };
    // Expired.
    let expired = propose().await.unwrap();
    let late = world.now() + world.store.tunables().transfer_expiry_ms();
    assert_eq!(
        refusal_of(
            world
                .store
                .accept_transfer(guest, campaign.id, expired, late)
                .await
        ),
        Refusal::ProposalExpired
    );
    // Stale revision: archiving and unarchiving moves the campaign's revision.
    let stale = propose().await.unwrap();
    world
        .store
        .archive_campaign(
            owner,
            campaign.id,
            "The Sunken Bell".into(),
            false,
            world.now(),
        )
        .await
        .unwrap();
    world
        .store
        .unarchive_campaign(owner, campaign.id, world.now())
        .await
        .unwrap();
    assert_eq!(
        refusal_of(
            world
                .store
                .accept_transfer(guest, campaign.id, stale, world.now())
                .await
        ),
        Refusal::StaleRevision
    );
    // A closing account cannot accept, and a proposal to one is refused.
    let fresh = propose().await.unwrap();
    world.db.set_account_status(guest, "closing").await;
    assert_eq!(
        refusal_of(
            world
                .store
                .accept_transfer(guest, campaign.id, fresh, world.now())
                .await
        ),
        Refusal::AccountClosing
    );
    assert_eq!(refusal_of(propose().await), Refusal::AccountClosing);
    world.cleanup().await;
}

#[tokio::test]
async fn proposals_are_withdrawn_by_the_proposer_or_declined_by_the_target_only() {
    let world = World::new("cancelproposal").await;
    let owner = world.account(1).await;
    let guest = world.account(2).await;
    let stranger = world.account(3).await;
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
                kind: TransferKind::Host,
                to_member: guest_member,
                old_host_becomes: None,
            },
            world.now(),
        )
        .await
        .unwrap();
    assert_eq!(
        refusal_of(
            world
                .store
                .cancel_transfer(stranger, campaign.id, proposal, world.now())
                .await
        ),
        Refusal::NotFound
    );
    world
        .store
        .cancel_transfer(guest, campaign.id, proposal, world.now())
        .await
        .unwrap();
    assert_eq!(
        refusal_of(
            world
                .store
                .accept_transfer(guest, campaign.id, proposal, world.now())
                .await
        ),
        Refusal::StaleRevision
    );
    world.cleanup().await;
}
