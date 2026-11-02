//! The kind files shipped in `assets/canvas/kinds` must load and be usable as shipped.

use std::path::PathBuf;

use df_table::canvas::Canvas;
use df_table::testkit::RecordingSink;
use serde_json::json;

use crate::kind_files::load_kind_files;
use crate::view::CanvasView;
use crate::wire::EffectRequest;

fn shipped_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/canvas/kinds")
}

#[test]
fn every_shipped_kind_file_loads_and_each_kind_is_present() {
    let registry = load_kind_files(&shipped_directory()).expect("the shipped kinds load");
    let canvas = Canvas::new(registry);
    let view = CanvasView::from_canvas(&canvas).expect("copies");
    let names: Vec<&str> = view.kinds.iter().map(|kind| kind.name.as_str()).collect();
    for expected in ["character", "item", "marker", "set_piece"] {
        assert!(names.contains(&expected), "missing kind {expected}");
    }
}

#[test]
fn each_shipped_kind_can_be_spawned_with_only_its_required_fields_and_gets_a_label() {
    let registry = load_kind_files(&shipped_directory()).expect("the shipped kinds load");
    let mut canvas = Canvas::new(registry);
    let kinds: Vec<(String, bool)> = CanvasView::from_canvas(&canvas)
        .expect("copies")
        .kinds
        .iter()
        .map(|kind| {
            (
                kind.name.clone(),
                kind.fields.iter().any(|field| field.required),
            )
        })
        .collect();
    for (kind, has_required) in kinds {
        let fields = if has_required {
            json!({"name": "Sample"})
        } else {
            json!({})
        };
        let request: EffectRequest = serde_json::from_value(
            json!({"op": "spawn", "id": format!("sample_{kind}"), "kind": kind, "fields": fields}),
        )
        .expect("parses");
        let effect = request.into_effect(&canvas).expect("converts");
        canvas
            .apply(effect, &RecordingSink::default())
            .unwrap_or_else(|error| panic!("{kind} did not spawn: {error}"));
    }
    let view = CanvasView::from_canvas(&canvas).expect("copies");
    assert!(
        view.elements
            .iter()
            .all(|element| !element.label.is_empty() && element.problem.is_none())
    );
}
