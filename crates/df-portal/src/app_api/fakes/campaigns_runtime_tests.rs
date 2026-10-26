use super::*;
use crate::app_api::ports::CampaignDirectory;
use axum::{Json, Router, extract::State, routing::post};
use df_account_app::wire_campaign::{ContentRating, FundingModel, JoinMode, PictureDefault};
use df_campaign::session_link::runtime_protocol::{
    ALLOCATION_PATH, Allocated, Allocation, LaunchSecret, Refusal,
};
use std::{
    sync::{
        Mutex as StdMutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[tokio::test]
async fn play_now_uses_the_shipped_campaign_defaults() {
    let campaigns = FakeCampaigns::new("http://127.0.0.1:8099", Arc::new(df_customer::SystemClock));
    let who = Actor {
        account: df_types::AccountId::from_bytes(&[1; 16]).unwrap(),
        display_name: "Host".into(),
        is_verified: true,
    };

    let campaign = campaigns
        .play_now_campaign(&who, df_types::OperationId::from_bytes(&[2; 16]).unwrap())
        .await
        .unwrap();
    let settings = campaigns.campaign_settings(&who, campaign).await.unwrap();

    assert_eq!(settings.draft.name, "The Broken Seal");
    assert_eq!(
        settings.draft.adventure_id.as_deref(),
        Some("harbor-journey")
    );
    assert_eq!(settings.draft.content_rating, Some(ContentRating::Teen));
    assert_eq!(settings.draft.seat_count, 6);
    assert!(settings.draft.has_voice);
    assert_eq!(settings.draft.pictures, PictureDefault::Automatic);
    assert_eq!(
        settings.draft.pace,
        df_account_app::wire::PacePreset::Immersive
    );
    assert_eq!(settings.draft.join_mode, JoinMode::LinkAndRoomCode);
    assert_eq!(settings.draft.funding, FundingModel::Sponsor);
}

#[derive(Clone)]
struct RuntimeFixture {
    origin: String,
    refusing: Arc<AtomicBool>,
    allocations: Arc<StdMutex<HashMap<[u8; 16], usize>>>,
}
async fn allocate(
    State(state): State<RuntimeFixture>,
    Json(request): Json<Allocation>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if state.refusing.load(Ordering::SeqCst) {
        return (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Json(Refusal::Unavailable),
        )
            .into_response();
    }
    let mut allocations = state.allocations.lock().unwrap();
    let next = allocations.len() + 1;
    let room = *allocations.entry(request.session).or_insert(next);
    Json(Allocated {
        code: format!("ROOM{room}"),
        generation: format!("generation-{room}"),
        links: Some(LocalLinks {
            launcher_url: format!("{}/launcher/{room}", state.origin),
            display_url: format!("{}/display/{room}", state.origin),
            player_url: format!("{}/player/{room}", state.origin),
        }),
    })
    .into_response()
}

#[tokio::test]
async fn start_places_two_sessions_in_distinct_rooms_and_reconciles_repeated_allocation() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let fixture = RuntimeFixture {
        origin: format!("http://{address}"),
        refusing: Arc::new(AtomicBool::new(false)),
        allocations: Arc::new(StdMutex::new(HashMap::new())),
    };
    let app = Router::new()
        .route(ALLOCATION_PATH, post(allocate))
        .with_state(fixture.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let campaigns = FakeCampaigns::new("http://127.0.0.1:8099", Arc::new(df_customer::SystemClock));
    campaigns.use_runtime(
        HttpRuntime::new(
            address,
            LaunchSecret::new(&"r".repeat(32)).unwrap(),
            Duration::from_secs(1),
        )
        .unwrap(),
    );
    let who = Actor {
        account: df_types::AccountId::from_bytes(&[1; 16]).unwrap(),
        display_name: "Host".into(),
        is_verified: true,
    };
    let first_campaign = campaigns.seed_campaign(&who, "First");
    let first = campaigns.seed_lobby(first_campaign).unwrap();
    let second_campaign = campaigns.seed_campaign(&who, "Second");
    let second = campaigns.seed_lobby(second_campaign).unwrap();
    assert!(campaigns.game_links(&who, first).await.unwrap().is_none());
    campaigns.start_session(&who, first).await.unwrap();
    campaigns.start_session(&who, second).await.unwrap();
    let first_links = campaigns.game_links(&who, first).await.unwrap().unwrap();
    let second_links = campaigns.game_links(&who, second).await.unwrap().unwrap();
    assert_eq!(first_links.room_code.as_deref(), Some("ROOM1"));
    assert_eq!(second_links.room_code.as_deref(), Some("ROOM2"));
    assert_ne!(first_links.display_url, second_links.display_url);
    assert_ne!(first_links.player_url, second_links.player_url);
    campaigns.allocate_runtime(&who, first).await.unwrap();
    assert_eq!(
        campaigns.game_links(&who, first).await.unwrap(),
        Some(first_links)
    );
    assert_eq!(fixture.allocations.lock().unwrap().len(), 2);
    let launchers = campaigns
        .run(|state, _, _| {
            Ok((
                state.launcher_urls[&first].clone(),
                state.launcher_urls[&second].clone(),
            ))
        })
        .unwrap();
    assert_ne!(launchers.0, launchers.1);
    assert_eq!(
        campaigns.launcher_url(&who, first).await.unwrap(),
        Some(launchers.0.clone())
    );
    assert_eq!(
        campaigns.launcher_url(&who, second).await.unwrap(),
        Some(launchers.1.clone())
    );
    let later_member = Actor {
        account: df_types::AccountId::from_bytes(&[3; 16]).unwrap(),
        display_name: "Later".into(),
        is_verified: true,
    };
    campaigns.seed_member(
        first_campaign,
        &later_member,
        df_account_app::wire_campaign::CampaignRole::Player,
    );
    assert!(campaigns.game_links(&who, first).await.unwrap().is_some());
    fixture.refusing.store(true, Ordering::SeqCst);
    assert!(campaigns.game_links(&who, first).await.unwrap().is_none());
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}
