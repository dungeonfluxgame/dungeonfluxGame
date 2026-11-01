//! Actual issuing-host and media-grant delivery regression.
use super::*;
use crate::gameplay::{
    media_grant,
    rpc::{self, view_message::Audience},
    scene_mood::{Look, Mood},
};
use axum::http::Request;
use tonic::codegen::Service;

struct CacheDirectory {
    parent: PathBuf,
    path: PathBuf,
}

impl CacheDirectory {
    fn new() -> Self {
        let parent = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../artifacts/tmp/lanes/PLATESERVE/tmp");
        std::fs::create_dir_all(&parent).unwrap();
        let parent = parent.canonicalize().unwrap();
        let mut nonce = [0; 16];
        getrandom::getrandom(&mut nonce).unwrap();
        let path = parent.join(format!("imagery-{:032x}", u128::from_le_bytes(nonce)));
        std::fs::create_dir(&path).unwrap();
        Self { parent, path }
    }
}

impl Drop for CacheDirectory {
    fn drop(&mut self) {
        if let Ok(path) = self.path.canonicalize()
            && path != self.parent
            && path.starts_with(&self.parent)
        {
            std::fs::remove_dir_all(path).unwrap();
        }
    }
}

async fn get(router: &mut Router, uri: &str) -> Response {
    Service::call(
        router,
        Request::builder()
            .uri(uri)
            .body(axum::body::Body::empty())
            .unwrap(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn a_session_generated_plate_is_served_only_by_the_issuing_game_with_a_grant() {
    let directory = CacheDirectory::new();
    let Some(parent) = tests::DirectionFixture::new(&directory.path).await else {
        return;
    };
    let Some(session) = tests::DirectionFixture::new(&directory.path).await else {
        parent.close().await;
        return;
    };
    let reference = ReferenceStore::new(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ui/generated"),
    )
    .load("scene-salt-quay-arrival-documentary-photo")
    .unwrap();
    let place = PlaceFacts::new("The Heron Shrine", "a shrine beside the marsh").unwrap();
    let key = session
        .imagery
        .generate(
            "plate:session-place",
            ImageRequest {
                kind: ImageKind::ScenePlate,
                references: vec![reference],
                prompt: scene_refresh(ContentRating::Restrained, &place, Look::of(Mood::Menacing)),
                output: OutputSpec {
                    width: 1024,
                    height: 640,
                    transparent: false,
                },
            },
        )
        .await
        .unwrap();
    assert_eq!(session.calls(), 1);
    assert_eq!(parent.calls(), 0);
    let Lookup::Image { bytes: cached, .. } = session.imagery.image(key.as_str()).await else {
        panic!("actual published plate")
    };

    let bare = format!("/gameplay/imagery/{}", key.as_str());
    let mut view = rpc::ViewMessage {
        audience: Some(Audience::Display(rpc::DisplayGameplayView {
            scene: Some(rpc::GameplayScene {
                live_plate: bare.clone(),
                plate_state: rpc::PlateState::Ready.into(),
                ..Default::default()
            }),
            ..Default::default()
        })),
        ..Default::default()
    };
    media_grant::sign_view(&mut view, true, &session.display_credential());
    let Some(Audience::Display(display)) = view.audience else {
        panic!("display view");
    };
    let signed = display.scene.unwrap().live_plate;
    assert!(signed.starts_with(&format!("{bare}?g=")));
    let mut session_router = router(session.imagery.clone());
    let reply = get(&mut session_router, &signed).await;
    assert_eq!(reply.status(), StatusCode::OK);
    assert_eq!(
        axum::body::to_bytes(reply.into_body(), MAXIMUM_IMAGE_BYTES as usize)
            .await
            .unwrap(),
        cached
    );
    assert_eq!(
        get(&mut session_router, &bare).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&mut session_router, &format!("{bare}?g=00"))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let expiry = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 60;
    let wrong_audience = media_grant::sign(
        key.as_str(),
        media_grant::Audience::Voice,
        media_grant::PrincipalAudience::Display,
        &session.display_credential(),
        expiry,
    )
    .unwrap();
    assert_eq!(
        get(&mut session_router, &format!("{bare}?g={wrong_audience}"))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&mut router(parent.imagery.clone()), &signed)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    drop(session_router);
    parent.close().await;
    session.close().await;
}
