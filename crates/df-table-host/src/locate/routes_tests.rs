use df_scene_layout::{PictureBinding, SCHEMA_VERSION, SceneLayout, SceneRegion};

use super::request::{LocateError, LocateReply, StoredLayout};
use super::routes::path;
use crate::dev_client;
use crate::dev_http::{BodyCause, DevReply};
use crate::test_support::{STARTER_FILES, start};

fn one_region_layout(picture: &str, bytes: &[u8]) -> SceneLayout {
    SceneLayout {
        schema: SCHEMA_VERSION,
        picture: PictureBinding {
            name: picture.to_owned(),
            sha256: df_imagery::oneshot_segment::picture_sha256(bytes),
            width_px: 100,
            height_px: 50,
        },
        model: "sam-3.1".to_owned(),
        prompt: "person".to_owned(),
        regions: vec![SceneRegion {
            id: df_scene_layout::region_id(b"elf"),
            label: Some("elf".to_owned()),
            left_permille: 300,
            top_permille: 200,
            right_permille: 600,
            bottom_permille: 1000,
            head_x_permille: 450,
            head_y_permille: 200,
            score_permille: 900,
            mask: None,
        }],
    }
}

fn rejected<T: std::fmt::Debug>(reply: LocateReply<T>) -> LocateError {
    match reply {
        DevReply::Rejected { error, message } => {
            assert_eq!(message, error.to_string());
            error
        }
        DevReply::Done { value } => panic!("not refused: {value:?}"),
    }
}

#[tokio::test]
async fn show_is_a_get_whose_answer_is_the_envelope_with_a_typed_error() {
    let running = start(&STARTER_FILES).await;
    running.write_asset("media/plate.png", "plate");
    let layout = one_region_layout("plate.png", b"plate");
    let layouts = running.playground_path("assets/layouts");
    crate::locate::write_layout(&layouts, "plate.png", &layout).unwrap();
    let port = running.port;
    let show = |query: &'static str| async move {
        let address = format!("{}{query}", path::LOCATE_SHOW);
        dev_client::get_json::<LocateReply<StoredLayout>>(port, &address)
            .await
            .unwrap()
    };

    let DevReply::Done { value } = show("?picture=plate.png").await else {
        panic!("the stored layout is shown");
    };
    assert_eq!(value.layout, layout);
    assert_eq!(value.layout_file, "plate.png.layout.json");

    assert_eq!(
        rejected(show("?picture=other.png").await),
        LocateError::NoLayout {
            picture: "other.png".to_owned()
        }
    );
    assert_eq!(
        rejected(show("?picture=a%20b.png").await),
        LocateError::InvalidPictureName {
            name: "a b.png".to_owned()
        }
    );
    assert!(matches!(
        rejected(show("?picture=plate.png&extra=1").await),
        LocateError::InvalidQuery { .. }
    ));
    assert!(matches!(
        rejected(show("").await),
        LocateError::InvalidQuery { .. }
    ));
    let response = dev_client::get(port, &format!("{}?picture=x.png", path::LOCATE_SHOW))
        .await
        .unwrap();
    assert_eq!(response.status, 404);
}

#[tokio::test]
async fn a_label_with_an_unknown_field_is_an_invalid_body_and_one_audit_line() {
    let running = start(&STARTER_FILES).await;
    let port = running.port;
    let reply: LocateReply<StoredLayout> = dev_client::post_json(
        port,
        path::LOCATE_LABEL,
        &serde_json::json!({"picture": "plate.png", "region": "0", "colour": "red"}),
    )
    .await
    .unwrap();
    let LocateError::InvalidBody(body) = rejected(reply) else {
        panic!("an unknown field is a body error");
    };
    assert_eq!(body.cause, BodyCause::Data);
    let refusals: Vec<String> = running
        .state
        .log()
        .after_sequence(0, usize::MAX)
        .into_iter()
        .map(|line| line.text)
        .filter(|text| text.starts_with("LOCATE refused"))
        .collect();
    assert_eq!(
        refusals,
        vec!["LOCATE refused verb=label picture=- error=invalid_body".to_owned()]
    );
}

#[tokio::test]
async fn the_cli_shows_a_layout_and_reports_a_refusal() {
    let running = start(&STARTER_FILES).await;
    running.write_asset("media/plate.png", "plate");
    let layouts = running.playground_path("assets/layouts");
    crate::locate::write_layout(
        &layouts,
        "plate.png",
        &one_region_layout("plate.png", b"plate"),
    )
    .unwrap();
    let words =
        |text: &str| -> Vec<String> { text.split_whitespace().map(str::to_owned).collect() };
    let shown = crate::dev::run_dev(running.port, &words("media locate show plate.png"))
        .await
        .unwrap();
    assert!(shown.contains("0: elf"), "{shown}");
    let points = crate::dev::run_dev(running.port, &words("media locate points plate.png"))
        .await
        .unwrap();
    assert!(points.starts_with("elf:"), "{points}");
    let refused = crate::dev::run_dev(running.port, &words("media locate show missing.png"))
        .await
        .unwrap_err();
    let crate::dev::DevError::Rejected(text) = &refused else {
        panic!("{refused:?}");
    };
    assert!(text.starts_with("refused (no_layout)"), "{text}");
}
