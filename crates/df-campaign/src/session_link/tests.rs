use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use axum::response::IntoResponse;

use super::*;
use crate::fakes::FakeSessions;

#[derive(Default)]
struct FakeRuntime {
    down: AtomicBool,
    replaced: AtomicBool,
    allocations: AtomicUsize,
}

impl RuntimeLauncher for Arc<FakeRuntime> {
    async fn allocate(&self, _: &Placement) -> Result<Room, RuntimeError> {
        self.allocations.fetch_add(1, Ordering::SeqCst);
        if self.down.load(Ordering::SeqCst) {
            Err(RuntimeError::Unavailable)
        } else {
            let mut room = fake_room();
            if self.replaced.load(Ordering::SeqCst) {
                room.run_identity = "demo-run-2".to_owned();
            }
            Ok(room)
        }
    }

    async fn serve(
        &self,
        _: &Placement,
        _: &Room,
        _: Audience,
        _: Surface,
        _: Request,
    ) -> Result<Response, RuntimeError> {
        Ok("game".into_response())
    }
}

fn fake_room() -> Room {
    Room {
        code: "LANTERN".to_owned(),
        run_identity: "demo-run-1".to_owned(),
    }
}

#[tokio::test]
async fn a_runtime_generation_change_does_not_silently_replace_the_assigned_room() {
    let campaign = CampaignId::from_bytes(&[1; 16]).unwrap();
    let session = SessionId::from_bytes(&[4; 16]).unwrap();
    let directory = FakeSessions::new();
    directory.set(session, campaign, SessionPhase::Lobby);
    let runtime = Arc::new(FakeRuntime::default());
    let launcher = SessionLauncher::new(directory, Arc::clone(&runtime), Duration::from_secs(2));
    launcher
        .start_session(session, campaign, &roster())
        .await
        .unwrap();
    assert_eq!(launcher.room().await, Some(fake_room()));
    runtime.replaced.store(true, Ordering::SeqCst);
    assert_eq!(
        launcher
            .allocate(&launcher.placement().await.unwrap())
            .await,
        Err(RuntimeError::Occupied)
    );
    assert_eq!(launcher.room().await, Some(fake_room()));
}

fn roster() -> Vec<SeatedMember> {
    vec![SeatedMember {
        member: MemberId::from_bytes(&[2; 16]).unwrap(),
        account: df_types::AccountId::from_bytes(&[3; 16]).unwrap(),
    }]
}

#[tokio::test]
async fn start_calls_the_launcher_and_retains_the_exact_campaign_roster() {
    let campaign = CampaignId::from_bytes(&[1; 16]).unwrap();
    let session = SessionId::from_bytes(&[4; 16]).unwrap();
    let directory = FakeSessions::new();
    directory.set(session, campaign, SessionPhase::Lobby);
    let runtime = Arc::new(FakeRuntime::default());
    let launcher = SessionLauncher::new(
        directory.clone(),
        Arc::clone(&runtime),
        Duration::from_secs(2),
    );
    launcher
        .start_session(session, campaign, &roster())
        .await
        .unwrap();
    let placement = launcher.placement().await.unwrap();
    assert_eq!(placement.session, session);
    assert_eq!(placement.campaign, campaign);
    assert_eq!(placement.roster[0].member, roster()[0].member);
    assert_eq!(directory.started().len(), 1);
    assert_eq!(runtime.allocations.load(Ordering::SeqCst), 1);
    launcher
        .start_session(session, campaign, &roster())
        .await
        .unwrap();
    assert_eq!(directory.started().len(), 1);
}

#[tokio::test]
async fn an_unavailable_runtime_keeps_start_true_and_can_be_retried() {
    let campaign = CampaignId::from_bytes(&[1; 16]).unwrap();
    let session = SessionId::from_bytes(&[4; 16]).unwrap();
    let directory = FakeSessions::new();
    directory.set(session, campaign, SessionPhase::Lobby);
    let runtime = Arc::new(FakeRuntime::default());
    runtime.down.store(true, Ordering::SeqCst);
    let launcher = SessionLauncher::new(
        directory.clone(),
        Arc::clone(&runtime),
        Duration::from_secs(2),
    );
    launcher
        .start_session(session, campaign, &roster())
        .await
        .unwrap();
    assert_eq!(
        directory.session(session).await.unwrap().unwrap().phase,
        SessionPhase::Playing
    );
    let placement = launcher.placement().await.unwrap();
    assert_eq!(
        launcher.allocate(&placement).await,
        Err(RuntimeError::Unavailable)
    );
    runtime.down.store(false, Ordering::SeqCst);
    assert_eq!(launcher.allocate(&placement).await, Ok(fake_room()));
    assert_eq!(directory.started().len(), 1);
}

#[tokio::test]
async fn a_second_session_never_reuses_another_tables_room() {
    let campaign = CampaignId::from_bytes(&[1; 16]).unwrap();
    let session = SessionId::from_bytes(&[4; 16]).unwrap();
    let other = SessionId::from_bytes(&[5; 16]).unwrap();
    let directory = FakeSessions::new();
    directory.set(session, campaign, SessionPhase::Lobby);
    directory.set(other, campaign, SessionPhase::Lobby);
    let launcher = SessionLauncher::new(
        directory.clone(),
        Arc::new(FakeRuntime::default()),
        Duration::from_secs(2),
    );
    launcher
        .start_session(session, campaign, &roster())
        .await
        .unwrap();
    assert_eq!(
        launcher.start_session(other, campaign, &roster()).await,
        Err(StoreError::Unavailable)
    );
    assert_eq!(
        directory.session(other).await.unwrap().unwrap().phase,
        SessionPhase::Lobby
    );
    assert_eq!(launcher.placement().await.unwrap().session, session);
}

#[tokio::test]
async fn an_oversized_or_duplicate_roster_is_refused_before_start() {
    let campaign = CampaignId::from_bytes(&[1; 16]).unwrap();
    let session = SessionId::from_bytes(&[4; 16]).unwrap();
    let directory = FakeSessions::new();
    directory.set(session, campaign, SessionPhase::Lobby);
    let launcher = SessionLauncher::new(
        directory.clone(),
        Arc::new(FakeRuntime::default()),
        Duration::from_secs(2),
    );
    let duplicate = vec![roster()[0]; 2];
    let oversized = vec![roster()[0]; 3];
    for invalid in [&[][..], duplicate.as_slice(), oversized.as_slice()] {
        assert_eq!(
            launcher.start_session(session, campaign, invalid).await,
            Err(StoreError::InvalidRecord)
        );
    }
    assert!(directory.started().is_empty());
    assert!(launcher.placement().await.is_none());
}
