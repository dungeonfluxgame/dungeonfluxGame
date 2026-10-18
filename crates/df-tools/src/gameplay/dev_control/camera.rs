//! Display-only home and named-shot controls, published through the existing view watch.
use super::{Outcome, Shared, authorized, bad_request};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use df_protocol::common as rpc;
use df_render::battle::{CameraPose, ShotName, Vec3};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Default)]
pub(super) struct CameraState {
    pub(super) command: rpc::BattleCameraOverride,
    report: Option<(Value, Instant)>,
}

impl CameraState {
    // ReportPresented already authenticates the display credential. The existing bounded
    // development readback channel carries this tagged camera report alongside layout pages.
    pub(super) fn readback(
        &mut self,
        instance: u64,
        text: &str,
    ) -> Result<(), crate::layout_contract::LayoutError> {
        use crate::layout_contract::LayoutError;
        if instance == 0 || text.len() > 4096 {
            return Err(LayoutError::Limit);
        }
        let value: Value = serde_json::from_str(text).map_err(|_| LayoutError::Shape)?;
        let report = &value["battle_camera"];
        if report["revision"].as_u64() != Some(self.command.revision) {
            return Err(LayoutError::Revision);
        }
        for key in ["live", "home", "authored_home"] {
            validate_report_pose(&report[key]).ok_or(LayoutError::Shape)?;
        }
        if report["scene"].as_str().is_none() || report["at_home"].as_bool().is_none() {
            return Err(LayoutError::Shape);
        }
        if self
            .report
            .as_ref()
            .map(|(previous, _)| &previous["at_home"])
            != Some(&report["at_home"])
        {
            let fields = serde_json::Map::from_iter([
                ("display_instance".to_owned(), json!(instance)),
                ("camera".to_owned(), report.clone()),
            ]);
            crate::gameplay::actor::record_audit_diagnostic(
                "gameplay.camera.observed",
                format!(
                    "CAMERA observed scene={} at_home={}",
                    report["scene"], report["at_home"]
                ),
                fields,
                &[("camera.at_home", report["at_home"].to_string())],
            );
        }
        self.report = Some((report.clone(), Instant::now()));
        Ok(())
    }

    fn document(&self, save: bool) -> Value {
        let report = self.report.as_ref();
        let fresh = report.is_some_and(|(_, seen)| seen.elapsed() <= Duration::from_secs(15));
        let mut value = json!({"status": "ok", "revision": self.command.revision,
            "display_status": if fresh {"reported"} else {"unavailable"},
            "report_age_ms": report.map(|(_, seen)| seen.elapsed().as_millis()),
            "display": report.map(|(value, _)| value),
            "override": self.command.home.as_ref().map(|home| json!({"pitch_deg":home.pitch_deg,
                "yaw_deg":home.yaw_deg,"distance":home.distance,"fov_deg":home.fov_deg})),
            "shot": self.command.shot});
        if save {
            if let Some((report, _)) = report.filter(|_| fresh) {
                value["save"] = json!({"scene":report["scene"], "home":report["home"]});
            } else {
                value["status"] = "unavailable".into();
                value["error"] =
                    "no fresh display camera report; open the battle TV and retry".into();
            }
        }
        value
    }

    fn edit(&mut self, body: EditRequest) -> Result<(), &'static str> {
        let (home, shot) = match body.command.as_str() {
            "home" => {
                let home = rpc::BattleCameraHome {
                    pitch_deg: body.pitch.ok_or("home needs pitch")?,
                    yaw_deg: body.yaw.ok_or("home needs yaw")?,
                    distance: body.distance.ok_or("home needs distance")?,
                    fov_deg: body.fov.ok_or("home needs fov")?,
                };
                CameraPose {
                    target: Vec3::ZERO,
                    pitch_deg: home.pitch_deg,
                    yaw_deg: home.yaw_deg,
                    distance: home.distance,
                    fov_deg: home.fov_deg,
                }
                .validate()
                .map_err(|_| "invalid home camera pose")?;
                (Some(home), String::new())
            }
            "shot" => {
                let shot = body.shot.ok_or("shot needs a name")?;
                ShotName::from_key(&shot).ok_or("unknown camera shot")?;
                (self.command.home.clone(), shot)
            }
            "reset" => (None, String::new()),
            _ => return Err("camera command must be home, shot, reset or save"),
        };
        let revision = self
            .command
            .revision
            .checked_add(1)
            .ok_or("camera revision exhausted")?;
        self.command = rpc::BattleCameraOverride {
            revision,
            home,
            shot,
        };
        self.report = None;
        Ok(())
    }
}

fn validate_report_pose(value: &Value) -> Option<()> {
    let number = |key| value[key].as_f64().map(|value| value as f32);
    let target = value["target"].as_array()?;
    let [x, y, z] = target.as_slice() else {
        return None;
    };
    CameraPose {
        target: Vec3::new(x.as_f64()? as f32, y.as_f64()? as f32, z.as_f64()? as f32),
        distance: number("distance")?,
        yaw_deg: number("yaw_deg")?,
        pitch_deg: number("pitch_deg")?,
        fov_deg: number("fov_deg")?,
    }
    .validate()
    .ok()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EditRequest {
    command: String,
    pitch: Option<f32>,
    yaw: Option<f32>,
    distance: Option<f32>,
    fov: Option<f32>,
    shot: Option<String>,
}

pub(super) async fn read(State(state): State<Arc<Shared>>, headers: HeaderMap) -> Outcome {
    authorized(&headers, &state.token)?;
    Ok((
        StatusCode::OK,
        Json(state.layout.camera.lock().await.document(false)),
    ))
}

pub(super) async fn edit(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<EditRequest>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let mut camera = state.layout.camera.lock().await;
    if body.command == "save" {
        return Ok((StatusCode::OK, Json(camera.document(true))));
    }
    let command = body.command.clone();
    camera.edit(body).map_err(bad_request)?;
    let document = camera.document(false);
    let fields = serde_json::Map::from_iter([
        ("camera_revision".to_owned(), json!(camera.command.revision)),
        ("command".to_owned(), json!(command)),
        ("camera".to_owned(), document.clone()),
    ]);
    crate::gameplay::actor::record_audit_diagnostic(
        "gameplay.camera.changed",
        format!(
            "CAMERA command={command} revision={}",
            camera.command.revision
        ),
        fields,
        &[
            ("camera.command", command),
            ("camera.revision", camera.command.revision.to_string()),
        ],
    );
    drop(camera);
    state.layout.camera_changed();
    Ok((StatusCode::OK, Json(document)))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(value: Value) -> EditRequest {
        serde_json::from_value(value).unwrap()
    }
    #[tokio::test]
    async fn camera_override_reaches_only_display_and_reset_is_projected() {
        let layout = super::super::layout::LayoutState::new();
        layout.enable();
        let mut wake = layout.subscribe();
        layout
            .camera
            .lock()
            .await
            .edit(request(
                json!({"command":"home","pitch":15,"yaw":12,"distance":8,"fov":38}),
            ))
            .unwrap();
        layout.camera_changed();
        assert!(wake.has_changed().unwrap());
        wake.borrow_and_update();
        let mut display = rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Display(Default::default())),
            ..Default::default()
        };
        layout.attach(&mut display).await.unwrap();
        let Some(rpc::view_message::Audience::Display(view)) = display.audience else {
            panic!("display");
        };
        assert_eq!(view.camera_override.unwrap().home.unwrap().yaw_deg, 12.0);
        let mut player = rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Player(Default::default())),
            ..Default::default()
        };
        let original = player.clone();
        layout.attach(&mut player).await.unwrap();
        assert_eq!(player, original);
        layout
            .camera
            .lock()
            .await
            .edit(request(json!({"command":"reset"})))
            .unwrap();
        let mut display = rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Display(Default::default())),
            ..Default::default()
        };
        layout.attach(&mut display).await.unwrap();
        let Some(rpc::view_message::Audience::Display(view)) = display.audience else {
            panic!("display");
        };
        let command = view.camera_override.unwrap();
        assert_eq!(command.revision, 2);
        assert!(command.home.is_none());
        assert!(command.shot.is_empty());
    }

    #[test]
    fn camera_readback_reports_actual_pose_and_save_requires_fresh_revision() {
        let mut state = CameraState::default();
        assert_eq!(state.document(true)["status"], "unavailable");
        let pose = json!({"target":[0,1,0],"distance":8,"pitch_deg":15,"yaw_deg":0,"fov_deg":38});
        let report = json!({"battle_camera":{"revision":0,"scene":"scan-riverbed",
            "live":pose,"home":pose,"authored_home":pose,"at_home":true}})
        .to_string();
        state.readback(1, &report).unwrap();
        assert_eq!(state.document(false)["display"]["at_home"], true);
        assert_eq!(state.document(true)["save"]["home"], pose);
        state.edit(request(json!({"command":"reset"}))).unwrap();
        assert!(state.readback(1, &report).is_err());
        assert_eq!(state.document(true)["status"], "unavailable");
    }

    #[test]
    fn camera_home_shot_reset_validate_and_clear_display_state() {
        let mut state = CameraState::default();
        state
            .edit(request(
                json!({"command":"home","pitch":15,"yaw":359,"distance":8,"fov":38}),
            ))
            .unwrap();
        assert_eq!(state.command.home.as_ref().unwrap().distance, 8.0);
        state
            .edit(request(json!({"command":"shot","shot":"impact"})))
            .unwrap();
        assert_eq!(state.command.shot, "impact");
        assert!(state.command.home.is_some());
        state.edit(request(json!({"command":"reset"}))).unwrap();
        assert_eq!(state.command.revision, 3);
        assert!(state.command.home.is_none());
        assert!(state.command.shot.is_empty());
        assert!(
            state
                .edit(request(
                    json!({"command":"home","pitch":0,"yaw":0,"distance":8,"fov":38})
                ))
                .is_err()
        );
        assert!(
            state
                .edit(request(json!({"command":"shot","shot":"unknown"})))
                .is_err()
        );
        assert_eq!(state.command.revision, 3);
    }
}
