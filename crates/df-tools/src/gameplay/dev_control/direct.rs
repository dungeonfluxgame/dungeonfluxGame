//! Gated direction admission through the session owner, with bounded CLI polling.

use super::*;
use axum::extract::Path;
use serde::Deserialize;

/// Explicit loopback-only fixture, available solely behind the development gate.
/// It never installs the live head locator or uses a real provider credential.
pub(in crate::gameplay) fn image_fixture(enabled: bool) -> Result<Option<std::net::SocketAddr>, std::io::Error> {
    match std::env::var("DF_GAMEPLAY_DEV_IMAGE_FIXTURE") {
        Err(std::env::VarError::NotPresent) => Ok(None),
        Ok(value) if enabled => {
            let address: std::net::SocketAddr = value.parse()
                .map_err(|_| std::io::Error::other("invalid development image fixture address"))?;
            if !address.ip().is_loopback() || address.port() == 0 {
                return Err(std::io::Error::other("development image fixture must be loopback with a port"));
            }
            Ok(Some(address))
        }
        _ => Err(std::io::Error::other("development image fixture requires the development gate")),
    }
}

pub(in crate::gameplay) fn start_imagery(
    settings: super::super::startup_configuration::ImagerySettings,
    manifest: Option<&std::path::Path>,
    fixture: Option<std::net::SocketAddr>,
) -> Result<super::super::imagery::Imagery, std::io::Error> {
    use super::super::{imagery::Imagery, startup_configuration::ImagerySettings};
    match (settings, fixture) {
        (ImagerySettings::On(configuration), Some(address)) => {
            let config = df_providers::ImageEditConfig::loopback_fixture(address)
                .map_err(|_| std::io::Error::other("development image fixture refused"))?;
            let provider = df_providers::NativeImageEditProvider::new(config, "test-credential", configuration.deadline)
                .map_err(|_| std::io::Error::other("development image fixture refused"))?;
            Imagery::new(configuration, provider, manifest)
        }
        (settings, _) => Imagery::start(settings, manifest),
    }
}

pub(in crate::gameplay) struct Order {
    pub living: super::super::living_scenes::LivingScenes,
    pub text: Option<String>,
    pub keep: bool,
}

pub(in crate::gameplay) enum Admission {
    Cleared,
    Render(oneshot::Receiver<Result<Value, super::super::imagery::ImageFailure>>),
}

#[derive(Deserialize)]
pub(super) struct Request {
    kind: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    keep: bool,
}

pub(super) async fn direct(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<Request>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let text = match body.kind.as_str() {
        "clear" => None,
        "plate" => {
            let text = body.text.trim();
            if text.is_empty() || text.chars().count() > 1000 || text.chars().any(char::is_control) {
                return Err(bad_request("direction needs 1 to 1000 characters without control characters"));
            }
            Some(text.to_owned())
        }
        _ => return Err(bad_request("direct kind must be plate or clear")),
    };
    let seats = state.seats.lock().await;
    let (session, run) = scope_of(&seats)?;
    drop(seats);
    let (reply, receive) = oneshot::channel();
    state.service.actor.try_submit(actor::Call::DevDirect {
        credential: state.display_credential,
        request: rpc::WatchViewRequest {
            session_id: Some(rpc::SessionId { value: Some(session) }),
            run_id: Some(rpc::RunId { value: Some(run) }),
            client_binding_id: Some(rpc::ClientBindingId { value: Some(vec![0x72; 16]) }),
            after_revision: None,
        },
        order: Order { living: state.service.living.clone(), text, keep: body.keep },
        reply,
    }).map_err(|_| failed("gameplay actor queue full"))?;
    let admitted = tokio::time::timeout(timing::active().reply, receive).await
        .map_err(|_| failed("direction admission deadline"))?
        .map_err(|_| failed("gameplay actor stopped"))?
        .map_err(|status| bad_request(status.message()))?;
    let value = match admitted {
        Admission::Cleared => json!({"status": "cleared"}),
        Admission::Render(receive) => tokio::time::timeout(timing::active().reply, receive).await
            .map_err(|_| failed("direction request preparation deadline"))?
            .map_err(|_| failed("plate worker stopped"))?
            .map_err(|reason| bad_request(reason.label()))?,
    };
    Ok((StatusCode::OK, Json(value)))
}

pub(super) async fn status(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let value = state.service.living.direct_status(&key)
        .ok_or_else(|| bad_request("unknown directed image key"))?;
    Ok((StatusCode::OK, Json(value)))
}

/// Runs only on the owner thread, after its ordinary binding/fence/view validation.
pub(in crate::gameplay) fn admit(
    history: &mut history::History,
    view: &rpc::ViewMessage,
    order: Order,
) -> Result<Admission, tonic::Status> {
    let Some(rpc::view_message::Audience::Display(display)) = &view.audience else {
        return Err(tonic::Status::permission_denied("direction needs the display binding"));
    };
    let scene = display.scene.as_ref()
        .ok_or_else(|| tonic::Status::failed_precondition("no current scene"))?;
    history.directions.enter(&format!("{}|{}", scene.title, scene.scene_asset));
    let Some(text) = order.text else {
        history.directions.clear();
        order.living.clear_direction();
        return Ok(Admission::Cleared);
    };
    order.living.observe(view);
    // Queue admission validates imagery, budget, combat and staging before the store changes.
    let receive = order.living.direct_plate(text.clone(), order.keep)
        .map_err(|reason| tonic::Status::failed_precondition(reason))?;
    history.directions.set(directions::Target::Plate, text, order.keep);
    let _used = history.directions.take(directions::Target::Plate);
    Ok(Admission::Render(receive))
}
