//! The fenced write `session_started`: `enter_service()`, then the six-argument fence, then the
//! campaign lock.

use df_campaign::{
    errors::{Failure, Refusal, StoreError},
    store::{CampaignStore, OwnerFence},
};
use df_types::CampaignId;

use crate::{
    fixture::{World, tenant_id},
    membership::refusal_of,
};

const PLACEMENT: [u8; 16] = [7; 16];

async fn own(world: &World, campaign: CampaignId) {
    world
        .db
        .admin
        .client
        .execute(
            "INSERT INTO df_runtime.campaign_owners (tenant_id, campaign_id, recovery_epoch, \
             claim_seq, state, placement_id, lease_until) \
             VALUES ($1, $2, 1, 5, 'serving', $3, now() + interval '1 hour')",
            &[
                &tenant_id().as_bytes().as_slice(),
                &campaign.as_bytes().as_slice(),
                &PLACEMENT.as_slice(),
            ],
        )
        .await
        .unwrap();
}

fn fence(recovery_epoch: i64, claim_seq: i64, placement: [u8; 16]) -> OwnerFence {
    OwnerFence {
        recovery_epoch,
        claim_seq,
        placement_id: placement,
    }
}

#[tokio::test]
async fn a_write_with_the_current_fence_succeeds_and_a_stale_one_is_refused() {
    let world = World::new("fence").await;
    let host = world.account(1).await;
    let campaign = world.campaign(host, 4).await;
    own(&world, campaign.id).await;
    world
        .store
        .session_started(campaign.id, fence(1, 5, PLACEMENT), world.now())
        .await
        .unwrap();
    // A stale claim sequence.
    assert_eq!(
        world
            .store
            .session_started(campaign.id, fence(1, 4, PLACEMENT), world.now())
            .await,
        Err(Failure::Store(StoreError::FenceStale))
    );
    // The current claim with another placement (a claim lost on failover).
    assert_eq!(
        world
            .store
            .session_started(campaign.id, fence(1, 5, [8; 16]), world.now())
            .await,
        Err(Failure::Store(StoreError::FenceStale))
    );
    // A raised recovery epoch (a restore).
    assert_eq!(
        world
            .store
            .session_started(campaign.id, fence(2, 5, PLACEMENT), world.now())
            .await,
        Err(Failure::Store(StoreError::FenceStale))
    );
    // An unknown campaign.
    assert_eq!(
        refusal_of(
            world
                .store
                .session_started(
                    CampaignId::from_bytes(&[0xEE; 16]).unwrap(),
                    fence(1, 5, PLACEMENT),
                    world.now()
                )
                .await
        ),
        Refusal::NotFound
    );
    assert_eq!(
        world.events(campaign.id).await.len(),
        1,
        "a refused fenced write writes nothing"
    );
    world.cleanup().await;
}

#[tokio::test]
async fn session_started_refuses_a_campaign_that_is_not_active() {
    let world = World::new("fencearchived").await;
    let host = world.account(1).await;
    let campaign = world.campaign(host, 4).await;
    own(&world, campaign.id).await;
    world
        .store
        .archive_campaign(
            host,
            campaign.id,
            "The Sunken Bell".into(),
            false,
            world.now(),
        )
        .await
        .unwrap();
    assert_eq!(
        refusal_of(
            world
                .store
                .session_started(campaign.id, fence(1, 5, PLACEMENT), world.now())
                .await
        ),
        Refusal::CampaignUnavailable
    );
    world.cleanup().await;
}

#[tokio::test]
async fn a_fenced_write_during_a_halt_gets_the_retryable_refusal_and_succeeds_after_it() {
    let world = World::new("fencehalt").await;
    let host = world.account(1).await;
    let campaign = world.campaign(host, 4).await;
    own(&world, campaign.id).await;
    // The halt holds the service lock exclusively, as O1's 6.3 does.
    let halt = world.db.raw_as("postgres").await;
    halt.client
        .batch_execute("BEGIN; SELECT pg_advisory_xact_lock(7283410100)")
        .await
        .unwrap();
    let during = world
        .store
        .session_started(campaign.id, fence(1, 5, PLACEMENT), world.now())
        .await;
    assert_eq!(during, Err(Failure::Store(StoreError::ServiceHalting)));
    halt.client.batch_execute("COMMIT").await.unwrap();
    world
        .store
        .session_started(campaign.id, fence(1, 5, PLACEMENT), world.now())
        .await
        .unwrap();
    world.cleanup().await;
}

#[tokio::test]
async fn the_fence_itself_raises_df002_when_the_service_lock_was_not_taken_first() {
    // The fence's own `require_service` never waits: during a halt it raises DF002, which the
    // store maps to `ServiceHalting` too.
    let world = World::new("fencedf002").await;
    let halt = world.db.raw_as("postgres").await;
    halt.client
        .batch_execute("BEGIN; SELECT pg_advisory_xact_lock(7283410100)")
        .await
        .unwrap();
    let result = world
        .store
        .debug_statements(&["SELECT df_runtime.require_service()"])
        .await;
    assert_eq!(result, Err(StoreError::ServiceHalting));
    halt.client.batch_execute("COMMIT").await.unwrap();
    world.cleanup().await;
}
