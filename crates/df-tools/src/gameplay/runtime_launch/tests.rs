use super::*;
use df_campaign::{
    lobby_state::SeatedMember,
    session_link::{
        Audience, HttpRuntime, Placement, Room, RuntimeError, RuntimeLauncher, SessionLauncher,
        Surface,
    },
    store::{SessionDirectory, SessionPhase},
};
use df_types::{AccountId, CampaignId, MemberId, SessionId};
use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

#[test]
fn non_development_profiles_require_a_strong_explicit_launch_secret() {
    assert!(secret_for_profile(None, Some("production"), true).is_err());
    assert!(secret_for_profile(None, None, false).is_err());
    assert!(secret_for_profile(Some("short"), Some("development"), true).is_err());
    assert!(
        secret_for_profile(None, Some("development"), false)
            .unwrap()
            .is_none()
    );
    assert!(
        secret_for_profile(Some(&"x".repeat(32)), Some("production"), false)
            .unwrap()
            .is_some()
    );
}

#[test]
fn placement_refuses_empty_duplicate_and_oversize_rosters() {
    let mut allocation = Allocation {
        session: [1; 16],
        campaign: [2; 16],
        roster: vec![Seat {
            account: [3; 16],
            member: [4; 16],
        }],
        display_tickets: vec![],
    };
    assert!(valid_allocation(&allocation));
    allocation.roster.push(allocation.roster[0].clone());
    assert!(!valid_allocation(&allocation));
    allocation.roster.clear();
    assert!(!valid_allocation(&allocation));
    allocation.roster = (1..=3)
        .map(|id| Seat {
            account: [id; 16],
            member: [id; 16],
        })
        .collect();
    assert!(!valid_allocation(&allocation));
}

fn placement() -> Placement {
    Placement {
        session: SessionId::from_bytes(&[1; 16]).unwrap(),
        campaign: CampaignId::from_bytes(&[2; 16]).unwrap(),
        roster: vec![
            SeatedMember {
                account: AccountId::from_bytes(&[3; 16]).unwrap(),
                member: MemberId::from_bytes(&[4; 16]).unwrap(),
            },
            SeatedMember {
                account: AccountId::from_bytes(&[5; 16]).unwrap(),
                member: MemberId::from_bytes(&[6; 16]).unwrap(),
            },
        ],
    }
}

fn offered_headers(state: &LaunchState) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        SECRET_HEADER,
        axum::http::HeaderValue::from_bytes(state.secret.header_value()).unwrap(),
    );
    headers
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn start_allocates_seats_and_entry_delivers_their_views_over_the_session_tunnel() {
    let mut harness = super::harness::Harness::new().await;
    let state = harness.state.clone();
    let peer = ConnectInfo("127.0.0.1:1234".parse().unwrap());
    let allocation = Allocation::from(&placement());
    let refused = allocate(
        State(state.clone()),
        ConnectInfo("192.0.2.1:1234".parse().unwrap()),
        offered_headers(&state),
        Json(allocation.clone()),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::NOT_FOUND);
    assert!(state.slot.lock().await.is_none());
    assert_eq!(
        allocate(
            State(state.clone()),
            peer,
            HeaderMap::new(),
            Json(allocation)
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );

    harness.start_rpc();
    let runtime_address = harness.listen(routes(state.clone())).await;
    let runtime = HttpRuntime::new(
        runtime_address,
        state.secret.clone(),
        std::time::Duration::from_secs(5),
    )
    .unwrap();
    let placement = placement();
    let directory = df_campaign::fakes::FakeSessions::new();
    directory.set(placement.session, placement.campaign, SessionPhase::Lobby);
    let launcher = SessionLauncher::new(
        directory.clone(),
        runtime.clone(),
        std::time::Duration::from_secs(5),
    );
    launcher
        .start_session(placement.session, placement.campaign, &placement.roster)
        .await
        .unwrap();
    assert_eq!(directory.started().len(), 1);
    let room = launcher
        .room()
        .await
        .expect("Start allocated the game room");
    assert_eq!(runtime.allocate(&placement).await.unwrap(), room);
    let revision = harness.service.updates.borrow().basis().revision;
    assert_eq!(state.slot.lock().await.as_ref().unwrap().seats.len(), 2);
    let mut changed = placement.clone();
    changed.session = SessionId::from_bytes(&[7; 16]).unwrap();
    assert_eq!(
        runtime.allocate(&changed).await,
        Err(RuntimeError::Occupied)
    );
    changed = placement.clone();
    changed.roster[0].account = AccountId::from_bytes(&[8; 16]).unwrap();
    assert_eq!(
        runtime.allocate(&changed).await,
        Err(RuntimeError::Occupied)
    );

    for seat in &placement.roster {
        let audience = Audience::Player(seat.member);
        let page = player_page(&runtime, &placement, &room, audience).await;
        assert!(page.contains(&format!(
            "data-tunnel-path=\"/sessions/{}/tunnel\"",
            super::super::hex(placement.session.as_bytes())
        )));
        assert!(page.contains("data-asset-base=\"/\""));
        assert!(!page.contains(ENTRY_HEADER));
        let scope = scope_from_page(&page);
        assert_eq!(
            scope_from_page(&player_page(&runtime, &placement, &room, audience).await),
            scope
        );
        let app = tunnel_app(runtime.clone(), placement.clone(), room.clone(), audience);
        let portal = harness.listen(app).await;
        let view = watch_over_tunnel(&mut harness, portal, &placement, &scope).await;
        assert!(matches!(
            view.audience,
            Some(rpc::view_message::Audience::Player(_))
        ));
        assert_eq!(
            harness.service.updates.borrow().basis().revision,
            revision,
            "entry and replay must never add seats"
        );
    }
    let first = scope_from_page(
        &player_page(
            &runtime,
            &placement,
            &room,
            Audience::Player(placement.roster[0].member),
        )
        .await,
    );
    let second = scope_from_page(
        &player_page(
            &runtime,
            &placement,
            &room,
            Audience::Player(placement.roster[1].member),
        )
        .await,
    );
    assert_ne!(first.credential, second.credential);
    assert_ne!(first.binding, second.binding);
    let display = runtime
        .serve(
            &placement,
            &room,
            Audience::Display,
            Surface::Display,
            axum::extract::Request::new(axum::body::Body::empty()),
        )
        .await
        .unwrap();
    assert_eq!(display.status(), StatusCode::OK);
    let body = String::from_utf8(
        axum::body::to_bytes(display.into_body(), 256 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(body.contains("id=\"display\""));
    assert!(body.contains(&format!(
        "data-page-path=\"/sessions/{}/display\"",
        super::super::hex(placement.session.as_bytes())
    )));
    let display_credential = body
        .split("data-local-binding=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .to_owned();
    let basis = harness.service.updates.borrow().basis();
    let scope = Scope {
        credential: display_credential,
        session: basis.session.as_bytes().to_vec(),
        run: basis.run.as_bytes().to_vec(),
        binding: vec![0x72; 16],
    };
    let portal = harness
        .listen(tunnel_app(
            runtime.clone(),
            placement.clone(),
            room.clone(),
            Audience::Display,
        ))
        .await;
    let display_view = watch_over_tunnel(&mut harness, portal, &placement, &scope).await;
    assert!(matches!(
        display_view.audience,
        Some(rpc::view_message::Audience::Display(_))
    ));
    entry_refusals(&state, &placement, &room).await;
    harness.shutdown().await;
}

#[derive(Debug, Eq, PartialEq)]
struct Scope {
    credential: String,
    session: Vec<u8>,
    run: Vec<u8>,
    binding: Vec<u8>,
}

fn scope_from_page(page: &str) -> Scope {
    let text = page
        .split("data-campaign-scope=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let fields: Vec<_> = text.split('.').collect();
    Scope {
        credential: fields[1].into(),
        session: df_campaign::secrets::unhex(fields[2]).unwrap(),
        run: df_campaign::secrets::unhex(fields[3]).unwrap(),
        binding: df_campaign::secrets::unhex(fields[4]).unwrap(),
    }
}

async fn player_page(
    runtime: &HttpRuntime,
    placement: &Placement,
    room: &Room,
    audience: Audience,
) -> String {
    let response = runtime
        .serve(
            placement,
            room,
            audience,
            Surface::Play,
            axum::extract::Request::new(axum::body::Body::empty()),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    String::from_utf8(
        axum::body::to_bytes(response.into_body(), 256 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap()
}

fn tunnel_app(
    runtime: HttpRuntime,
    placement: Placement,
    room: Room,
    audience: Audience,
) -> Router {
    Router::new().route(
        &format!(
            "/sessions/{}/tunnel",
            super::super::hex(placement.session.as_bytes())
        ),
        get(move |request: axum::extract::Request| {
            let runtime = runtime.clone();
            let placement = placement.clone();
            let room = room.clone();
            async move {
                runtime
                    .serve(&placement, &room, audience, Surface::Tunnel, request)
                    .await
                    .unwrap()
            }
        }),
    )
}

async fn watch_over_tunnel(
    harness: &mut super::harness::Harness,
    address: SocketAddr,
    placement: &Placement,
    scope: &Scope,
) -> rpc::ViewMessage {
    let mut upgrade = format!(
        "ws://{address}/sessions/{}/tunnel",
        super::super::hex(placement.session.as_bytes())
    )
    .into_client_request()
    .unwrap();
    upgrade.headers_mut().insert(
        header::ORIGIN,
        axum::http::HeaderValue::from_static("http://127.0.0.1"),
    );
    let (socket, _) = tokio_tungstenite::connect_async(upgrade).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    harness.tasks.spawn(async move {
        let (pipe, _) = listener.accept().await.unwrap();
        let (mut read, mut write) = pipe.into_split();
        let (mut sink, mut stream) = socket.split();
        let upload = async {
            let mut buffer = vec![0; 16 * 1024];
            loop {
                let count = read.read(&mut buffer).await.unwrap();
                if count == 0 {
                    break;
                }
                sink.send(Message::Binary(buffer[..count].to_vec().into()))
                    .await
                    .unwrap();
            }
        };
        let download = async {
            while let Some(message) = stream.next().await {
                match message.unwrap() {
                    Message::Binary(bytes) => write.write_all(&bytes).await.unwrap(),
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        };
        tokio::select! { _ = upload => {}, _ = download => {} }
    });
    let channel = tonic::transport::Endpoint::from_shared(format!("http://{address}"))
        .unwrap()
        .connect_timeout(std::time::Duration::from_secs(3))
        .connect()
        .await
        .unwrap();
    let mut client = rpc::session_service_client::SessionServiceClient::new(channel);
    let mut request = tonic::Request::new(rpc::WatchViewRequest {
        session_id: Some(rpc::SessionId {
            value: Some(scope.session.clone()),
        }),
        run_id: Some(rpc::RunId {
            value: Some(scope.run.clone()),
        }),
        client_binding_id: Some(rpc::ClientBindingId {
            value: Some(scope.binding.clone()),
        }),
        after_revision: None,
    });
    request
        .metadata_mut()
        .insert("x-df-local-binding", scope.credential.parse().unwrap());
    request.set_timeout(std::time::Duration::from_secs(5));
    let mut views = client.watch(request).await.unwrap().into_inner();
    tokio::time::timeout(std::time::Duration::from_secs(5), views.message())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
}

async fn entry_refusals(state: &LaunchState, placement: &Placement, room: &Room) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let valid = Entry {
        session: *placement.session.as_bytes(),
        campaign: *placement.campaign.as_bytes(),
        audience: EntryAudience::Player(*placement.roster[0].member.as_bytes()),
        generation: room.run_identity.clone(),
        expires_at_unix_seconds: now + ENTRY_LIFETIME_SECONDS,
    };
    for mutation in 0..8 {
        let mut claim = valid.clone();
        match mutation {
            0 => claim.session = [9; 16],
            1 => claim.campaign = [9; 16],
            2 => claim.generation = "stale-generation".into(),
            3 => claim.expires_at_unix_seconds = now,
            4 => claim.audience = EntryAudience::Player([9; 16]),
            6 => claim.audience = EntryAudience::Display([9; 16]),
            _ => {}
        }
        let mut token = state.secret.sign(&claim).unwrap();
        if mutation == 5 {
            token.push('0');
        }
        let mut headers = offered_headers(state);
        headers.insert(ENTRY_HEADER, token.parse().unwrap());
        let mut request = axum::extract::Request::new(axum::body::Body::empty());
        *request.headers_mut() = headers;
        request
            .extensions_mut()
            .insert(ConnectInfo("127.0.0.1:1234".parse::<SocketAddr>().unwrap()));
        let response = entry(
            State(state.clone()),
            Path(if mutation >= 6 { "display" } else { "play" }.into()),
            request,
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "claim mutation {mutation}"
        );
    }
}
