use df_campaign::{
    records::{CampaignStatus, EventKind, MemberStatus},
    roles::CampaignRole,
    store::CampaignStore,
};

use crate::fixture::World;

#[tokio::test]
async fn a_new_campaign_has_its_host_member_and_a_joined_event() {
    let world = World::new("smoke").await;
    let owner = world.account(1).await;
    let campaign = world.campaign(owner, 4).await;
    assert_eq!(campaign.status, CampaignStatus::Active);
    assert_eq!(campaign.event_seq, 1);
    let snapshot = world
        .store
        .member_snapshot(owner, campaign.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.role, CampaignRole::Host);
    assert_eq!(snapshot.status, MemberStatus::Active);
    assert!(snapshot.authorized);
    assert!(snapshot.holds_seat);
    let events = world.events(campaign.id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, EventKind::Joined);
    world.cleanup().await;
}
