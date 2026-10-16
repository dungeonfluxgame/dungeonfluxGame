//! Stable, bounded runtime layout controls for the shared display.
use js_sys::{JSON, Object, Reflect};
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Element, HtmlElement};

fn id(element: &Element, value: &str) {
    let _ = element.set_attribute("data-layout-id", value);
}

fn register(stage: &Element) {
    id(stage, "stage");
    for (selector, name) in [
        (".tv-background", "background"),
        (".tv-plate", "plate"),
        (".tv-plate-bubbles", "speech-bubbles"),
        (".tv-scene-context", "scene-context"),
        (".tv-headline", "headline"),
        (".tv-rules", "rules"),
        (".tv-narration", "narration"),
        (".tv-choices", "choices"),
        (".tv-title", "title"),
        (".tv-partybar", "partybar"),
        (".tv-hud", "hud"),
        (".live-scene-item", "item-card"),
        (".tv-loader", "loader"),
        (".tv-check-result", "check-card"),
        (".tv-overlay", "overlay"),
        (".tv-images", "images"),
    ] {
        if let Ok(Some(element)) = stage.query_selector(selector) {
            id(&element, name);
        }
    }
    if let Ok(elements) = stage.query_selector_all(".tv-fig[data-slot]") {
        for index in 0..elements.length() {
            let Some(figure) = elements
                .get(index)
                .and_then(|node| node.dyn_into::<Element>().ok())
            else {
                continue;
            };
            let Some(slot) = figure.get_attribute("data-slot") else {
                continue;
            };
            id(&figure, &slot);
            if let Ok(Some(bubbles)) = figure.query_selector(".tv-fig-bubbles") {
                id(&bubbles, &format!("bubbles-{slot}"));
            }
            if let Ok(bubbles) = figure.query_selector_all(".tv-bubble") {
                for bubble_index in 0..bubbles.length() {
                    let Some(bubble) = bubbles
                        .get(bubble_index)
                        .and_then(|node| node.dyn_into::<Element>().ok())
                    else {
                        continue;
                    };
                    let suffix = bubble
                        .get_attribute("data-line-id")
                        .or_else(|| bubble.get_attribute("data-kind"))
                        .unwrap_or_else(|| bubble_index.to_string());
                    id(&bubble, &format!("bubble-{slot}-{suffix}"));
                }
            }
        }
    }
    if let Ok(cards) = stage.query_selector_all(".tv-hud-card[data-slot]") {
        for index in 0..cards.length() {
            if let Some(card) = cards
                .get(index)
                .and_then(|node| node.dyn_into::<Element>().ok())
            {
                if let Some(slot) = card.get_attribute("data-slot") {
                    id(&card, &format!("hud-{slot}"));
                }
            }
        }
    }
}

fn find(stage: &Element, wanted: &str) -> Option<Element> {
    let elements = stage.query_selector_all("[data-layout-id]").ok()?;
    (0..elements.length()).find_map(|index| {
        let element = elements.get(index)?.dyn_into::<Element>().ok()?;
        (element.get_attribute("data-layout-id").as_deref() == Some(wanted)).then_some(element)
    })
}

fn number(object: &Object, name: &str) -> Option<f64> {
    Reflect::get(object, &JsValue::from_str(name))
        .ok()
        .and_then(|value| value.as_f64())
}

fn text(object: &Object, name: &str) -> Option<String> {
    Reflect::get(object, &JsValue::from_str(name))
        .ok()
        .and_then(|value| value.as_string())
}

fn apply_override(element: &Element, value: &Object) {
    let Some(style) = element.dyn_ref::<HtmlElement>().map(|html| html.style()) else {
        return;
    };
    let kind = text(value, "kind").unwrap_or_default();
    if kind == "move" {
        let unit = text(value, "unit").unwrap_or_else(|| "px".to_owned());
        let suffix = if unit == "percent" { "%" } else { "px" };
        if let Some(value) = number(value, "left") {
            let _ = style.set_property("left", &format!("{value}{suffix}"));
        }
        if let Some(value) = number(value, "top") {
            let _ = style.set_property("top", &format!("{value}{suffix}"));
        }
        if let Some(value) = number(value, "width") {
            let _ = style.set_property("width", &format!("{value}{suffix}"));
        }
        if let Some(value) = number(value, "height") {
            let _ = style.set_property("height", &format!("{value}{suffix}"));
        }
        let _ = style.set_property("position", "relative");
    } else {
        let unit = text(value, "unit").unwrap_or_else(|| "px".to_owned());
        let suffix = if unit == "percent" { "%" } else { "px" };
        let tx = number(value, "translate_x").unwrap_or(0.0);
        let ty = number(value, "translate_y").unwrap_or(0.0);
        let _ = style.set_property("translate", &format!("{tx}{suffix} {ty}{suffix}"));
        if let Some(scale_x) = number(value, "scale_x") {
            let scale_y = number(value, "scale_y").unwrap_or(scale_x);
            let _ = style.set_property("scale", &format!("{scale_x} {scale_y}"));
        }
        if let Some(value) = number(value, "rotate") {
            let _ = style.set_property("rotate", &format!("{value}deg"));
        }
    }
}

pub(super) fn apply(stage: &Element, json: &str, revision: u64) -> Result<(), JsValue> {
    register(stage);
    let root = JSON::parse(json)?.dyn_into::<Object>()?;
    let overrides = Reflect::get(&root, &JsValue::from_str("overrides"))?.dyn_into::<Object>()?;
    // Remove only properties owned by this controller. Application inline styles, including
    // health and actor positioning, remain intact across every incoming view.
    if let Ok(elements) = stage.query_selector_all("[data-layout-id]") {
        for index in 0..elements.length() {
            if let Some(element) = elements
                .get(index)
                .and_then(|node| node.dyn_into::<Element>().ok())
            {
                if let Some(style) = element.dyn_ref::<HtmlElement>().map(|html| html.style()) {
                    for property in [
                        "translate",
                        "scale",
                        "rotate",
                        "left",
                        "top",
                        "width",
                        "height",
                        "position",
                    ] {
                        let _ = style.remove_property(property);
                    }
                }
            }
        }
    }
    let keys = Object::keys(&overrides);
    for index in 0..keys.length() {
        let key = keys.get(index).as_string().unwrap_or_default();
        if let Some(element) = find(stage, &key) {
            if let Ok(value) = Reflect::get(&overrides, &JsValue::from_str(&key)) {
                if let Ok(value) = value.dyn_into::<Object>() {
                    apply_override(&element, &value);
                }
            }
        }
    }
    stage.set_attribute("data-layout-revision", &revision.to_string())?;
    Ok(())
}

pub(super) fn readback(stage: &Element, revision: u64) -> String {
    let Some(window) = web_sys::window() else {
        return String::new();
    };
    let width = window
        .inner_width()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or_default();
    let height = window
        .inner_height()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or_default();
    let elements = stage.query_selector_all("[data-layout-id]").ok();
    let mut entries = Vec::new();
    if let Some(elements) = elements {
        for index in 0..elements.length().min(256) {
            let Some(element) = elements
                .get(index)
                .and_then(|node| node.dyn_into::<Element>().ok())
            else {
                continue;
            };
            let rect = element.get_bounding_client_rect();
            entries.push(serde_json::json!({
                "id": element.get_attribute("data-layout-id").unwrap_or_default(),
                "left": rect.left(), "top": rect.top(), "right": rect.right(), "bottom": rect.bottom(),
                "width": rect.width(), "height": rect.height(),
            }));
        }
    }
    serde_json::json!({"revision": revision, "viewport": {"width": width, "height": height}, "elements": entries}).to_string()
}
