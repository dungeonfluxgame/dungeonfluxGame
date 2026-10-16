//! Development-only live layout. The page controller owns its stylesheet, inventory clock
//! and pending metadata request; gameplay owns every original style and animation.
use super::{Client, request, rpc};
use crate::layout_contract::{
    self as contract, Bounds, EffectiveTransform, ElementReadback, ElementStatus, LayoutDocument,
    LayoutReadback, Viewport, VisualOwner,
};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    rc::{Rc, Weak},
    time::Duration,
};
use wasm_bindgen::{JsCast, JsValue};
use web_sys::Element;
const CLOCK_MS: u32 = 500;
const REPORT_MS: f64 = 1000.0;
const MAX_SCAN: usize = 16384;
thread_local! {
    static ACTIVE:RefCell<Option<Controller>>=const{RefCell::new(None)};
    static GENERATION:Cell<u64>=const{Cell::new(0)};
    static SAMPLE:Cell<u64>=const{Cell::new(0)};
}
struct Controller {
    client: Weak<RefCell<Client>>,
    root: Element,
    stylesheet: Element,
    document: LayoutDocument,
    registry: BTreeMap<String, Element>,
    next_id: u64,
    sample: u64,
    generation: u64,
    dirty: bool,
    inflight: bool,
    last_sent: f64,
    overflow: usize,
    clock: Option<gloo_timers::callback::Interval>,
}
impl Drop for Controller {
    fn drop(&mut self) {
        self.clock.take();
        self.stylesheet.remove();
    }
}
const ALIASES: &[(&str, &str)] = &[
    (".tv-stagebox", "dm-overlay"),
    (".tv-narration", "narration"),
    (".tv-stage", "stage"),
    (".tv-title", "title"),
    (".tv-scene-context", "scene-context"),
    (".tv-art:not(.tv-art-previous)", "background"),
    (".tv-plate", "plate"),
    (".tv-plate-loading", "plate-loading"),
    (".tv-lower", "subtitle"),
    (".tv-hud", "hud"),
    (".tv-choices", "choices"),
    (".tv-partybar", "partybar"),
    (".tv-rules", "rules"),
    (".tv-sound-gate", "sound-gate"),
    (".tv-safe", "safe-content"),
    (".live-scene-item", "item-card"),
    (".tv-bigcard", "scene-card"),
];
fn eligible(element: &Element) -> bool {
    !matches!(
        element.tag_name().to_ascii_lowercase().as_str(),
        "script"
            | "style"
            | "template"
            | "meta"
            | "link"
            | "base"
            | "defs"
            | "clippath"
            | "lineargradient"
            | "radialgradient"
            | "stop"
    ) && !(element.tag_name().eq_ignore_ascii_case("input")
        && element.get_attribute("type").as_deref() == Some("hidden"))
}
fn priority(element: &Element) -> bool {
    let classes = element.get_attribute("class").unwrap_or_default();
    [
        "bubble",
        "loader",
        "intent",
        "thinking",
        "hud-card",
        "check",
        "narration",
        "title",
        "choice",
        "scene-actor",
        "tv-fig",
        "button",
        "control",
        "item",
    ]
    .iter()
    .any(|key| classes.contains(key))
        || matches!(
            element.tag_name().to_ascii_lowercase().as_str(),
            "button" | "input" | "textarea" | "select"
        )
}
fn visual_owner(element: &Element) -> Result<Option<VisualOwner>, JsValue> {
    let owner = element
        .closest("[data-slot],[data-actor-id]")?
        .or(element.closest("[data-layout-owner-id]")?);
    let Some(owner) = owner else {
        return Ok(None);
    };
    let id = owner
        .get_attribute("data-slot")
        .or_else(|| owner.get_attribute("data-actor-id"))
        .or_else(|| owner.get_attribute("data-layout-owner-id"));
    let Some(id) = id.filter(|id| !id.is_empty() && id.len() <= 128) else {
        return Ok(None);
    };
    let name = owner
        .get_attribute("data-name")
        .or_else(|| {
            owner
                .query_selector(".tv-hud-name,.tv-bubble-name")
                .ok()
                .flatten()
                .and_then(|name| name.text_content())
        })
        .or_else(|| owner.get_attribute("alt"))
        .or_else(|| {
            owner
                .query_selector("img[alt]")
                .ok()
                .flatten()
                .and_then(|image| image.get_attribute("alt"))
        })
        .or_else(|| owner.get_attribute("data-layout-owner-name"))
        .filter(|name| {
            !name.is_empty() && name.len() <= 128 && !name.chars().any(char::is_control)
        });
    Ok(Some(VisualOwner { id, name }))
}
fn annotate_owner(element: &Element) -> Result<(), JsValue> {
    if let Some(owner) = visual_owner(element)? {
        element.set_attribute("data-layout-owner-id", &owner.id)?;
        if let Some(name) = &owner.name {
            element.set_attribute("data-layout-owner-name", name)?;
        }
        let classes = element.class_list();
        let semantic = if classes.contains("tv-fig") {
            Some(format!("actor-{}", owner.id))
        } else if classes.contains("tv-hud-card") {
            Some(format!("hud-{}", owner.id))
        } else if classes.contains("tv-scene-actor") {
            Some(format!("scene-actor-{}", owner.id))
        } else if classes.contains("tv-fig-bubbles") {
            Some(format!("bubbles-{}", owner.id))
        } else {
            None
        };
        if let Some(id) = semantic.filter(|id| contract::validate_id(id).is_ok()) {
            element.set_attribute("data-layout-id", &id)?;
        }
    }
    Ok(())
}
impl Controller {
    fn refresh(&mut self) -> Result<(), JsValue> {
        let before: BTreeSet<_> = self.registry.keys().cloned().collect();
        self.root.set_attribute("data-layout-id", "game-root")?;
        let mut aliases = Vec::new();
        for (selector, id) in ALIASES {
            if let Some(element) = self.root.query_selector(selector)? {
                element.set_attribute("data-layout-id", id)?;
                aliases.push(element);
            }
        }
        let nodes = self.root.query_selector_all("*")?;
        let mut candidates = vec![self.root.clone()];
        candidates.extend(aliases);
        let mut rest = Vec::new();
        let mut overflow = 0;
        for index in 0..nodes.length() {
            if index as usize >= MAX_SCAN {
                overflow += 1;
                continue;
            }
            let Some(element) = nodes.item(index).and_then(|n| n.dyn_into::<Element>().ok()) else {
                continue;
            };
            if !eligible(&element) {
                continue;
            }
            annotate_owner(&element)?;
            if priority(&element) {
                candidates.push(element);
            } else {
                rest.push(element);
            }
        }
        candidates.extend(rest);
        self.registry.clear();
        for element in candidates {
            if element
                .get_attribute("data-layout-id")
                .and_then(|id| self.registry.get(&id))
                .is_some_and(|registered| registered.is_same_node(Some(&element)))
            {
                continue;
            }
            if self.registry.len() >= contract::MAX_ELEMENTS {
                element.remove_attribute("data-layout-id")?;
                overflow += 1;
                continue;
            }
            let known = element
                .get_attribute("data-layout-id")
                .filter(|id| contract::validate_id(id).is_ok() && !self.registry.contains_key(id));
            let id = match known {
                Some(id) => id,
                None => {
                    self.next_id = self
                        .next_id
                        .checked_add(1)
                        .ok_or_else(|| JsValue::from_str("layout ID capacity exhausted"))?;
                    let kind = if element.tag_name().eq_ignore_ascii_case("img") {
                        "image"
                    } else {
                        "visual"
                    };
                    let id = format!(
                        "{kind}-{}-{}-{}",
                        super::live_motion::display_instance(),
                        self.generation,
                        self.next_id
                    );
                    element.set_attribute("data-layout-id", &id)?;
                    id
                }
            };
            self.registry.insert(id, element);
        }
        self.overflow = overflow;
        self.dirty |= before != self.registry.keys().cloned().collect();
        self.root
            .set_attribute("data-layout-overflow", &overflow.to_string())?;
        Ok(())
    }
    fn sample(&mut self) -> Result<LayoutReadback, JsValue> {
        let window =
            web_sys::window().ok_or_else(|| JsValue::from_str("layout window unavailable"))?;
        let width = window
            .inner_width()?
            .as_f64()
            .ok_or_else(|| JsValue::from_str("layout viewport width unavailable"))?;
        let height = window
            .inner_height()?
            .as_f64()
            .ok_or_else(|| JsValue::from_str("layout viewport height unavailable"))?;
        self.sample = SAMPLE.with(|sample| {
            let next = sample
                .get()
                .checked_add(1)
                .ok_or_else(|| JsValue::from_str("layout sample capacity exhausted"))?;
            sample.set(next);
            Ok::<_, JsValue>(next)
        })?;
        let mut elements = Vec::new();
        for (id, element) in &self.registry {
            let rect = element.get_bounding_client_rect();
            let style = window
                .get_computed_style(element)?
                .ok_or_else(|| JsValue::from_str("layout computed style unavailable"))?;
            elements.push(ElementReadback {
                id: id.clone(),
                tag: element.tag_name().to_ascii_lowercase(),
                classes: element
                    .get_attribute("class")
                    .unwrap_or_default()
                    .split_whitespace()
                    .filter(|s| {
                        s.bytes()
                            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
                    })
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(" ")
                    .chars()
                    .take(192)
                    .collect(),
                owner: visual_owner(element)?,
                configured: self.document.overrides.get(id).cloned(),
                effective: Some(EffectiveTransform {
                    translate: style.get_property_value("translate")?,
                    scale: style.get_property_value("scale")?,
                    rotate: style.get_property_value("rotate")?,
                    matrix: style.get_property_value("transform")?,
                }),
                rect: Some(Bounds {
                    left: rect.left(),
                    top: rect.top(),
                    width: rect.width(),
                    height: rect.height(),
                }),
                status: ElementStatus::Applied,
            });
        }
        for (id, configured) in &self.document.overrides {
            if !self.registry.contains_key(id) {
                elements.push(ElementReadback {
                    id: id.clone(),
                    tag: "absent".into(),
                    classes: String::new(),
                    owner: None,
                    configured: Some(configured.clone()),
                    effective: None,
                    rect: None,
                    status: ElementStatus::Absent,
                });
            }
        }
        let report = LayoutReadback {
            sample: self.sample,
            revision: self.document.revision,
            viewport: Viewport { width, height },
            elements,
            overflow: self.overflow,
        };
        report
            .validate()
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        Ok(report)
    }
}

/// Called only for an enabled server projection. A reconnect updates the retained controller;
/// a replaced root cancels its old clock and removes only its dedicated stylesheet.
pub(super) fn apply(
    client: &Rc<RefCell<Client>>,
    text: &str,
    revision: u64,
) -> Result<(), JsValue> {
    if text.is_empty() {
        stop();
        return Ok(());
    }
    let document = LayoutDocument::parse(text).map_err(|e| JsValue::from_str(&e.to_string()))?;
    if document.revision != revision {
        return Err(JsValue::from_str("layout envelope revision mismatch"));
    }
    let root = client.borrow().root.clone();
    ACTIVE.with(|active| -> Result<(), JsValue> {
        let mut active = active.borrow_mut();
        if active
            .as_ref()
            .is_some_and(|c| !c.root.is_same_node(Some(&root)))
        {
            active.take();
        }
        if active.is_none() {
            let dom = root
                .owner_document()
                .ok_or_else(|| JsValue::from_str("layout document unavailable"))?;
            let style = dom.create_element("style")?;
            style.set_attribute("data-layout-owner", "runtime")?;
            dom.query_selector("head")?
                .ok_or_else(|| JsValue::from_str("layout stylesheet host unavailable"))?
                .append_child(&style)?;
            let generation = GENERATION.with(|g| {
                let next = g
                    .get()
                    .checked_add(1)
                    .ok_or_else(|| JsValue::from_str("layout owner capacity exhausted"))?;
                g.set(next);
                Ok::<_, JsValue>(next)
            })?;
            *active = Some(Controller {
                client: Rc::downgrade(client),
                root: root.clone(),
                stylesheet: style,
                document: LayoutDocument::default(),
                registry: BTreeMap::new(),
                next_id: 0,
                sample: 0,
                generation,
                dirty: true,
                inflight: false,
                last_sent: 0.0,
                overflow: 0,
                clock: Some(gloo_timers::callback::Interval::new(CLOCK_MS, tick)),
            });
        }
        let controller = active
            .as_mut()
            .ok_or_else(|| JsValue::from_str("layout owner unavailable"))?;
        if document.revision < controller.document.revision {
            return Ok(());
        }
        let css = document
            .stylesheet()
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        if controller.stylesheet.text_content().as_deref() != Some(css.as_str()) {
            controller.stylesheet.set_text_content(Some(&css));
            controller.dirty = true;
        }
        controller.dirty |= document.revision != controller.document.revision;
        controller.document = document;
        controller.refresh()?;
        root.set_attribute("data-layout-revision", &revision.to_string())?;
        Ok(())
    })?;
    tick();
    Ok(())
}
fn stop() {
    ACTIVE.with(|active| {
        active.borrow_mut().take();
    });
}
fn tick() {
    let work = ACTIVE.with(|active| -> Result<_, JsValue> {
        let mut active = active.borrow_mut();
        let Some(controller) = active.as_mut() else {
            return Ok(None);
        };
        if !controller.root.is_connected() || controller.client.upgrade().is_none() {
            active.take();
            return Ok(None);
        }
        controller.refresh()?;
        let now = js_sys::Date::now();
        if controller.inflight || (!controller.dirty && now - controller.last_sent < REPORT_MS) {
            return Ok(None);
        }
        let Some(client) = controller.client.upgrade() else {
            return Ok(None);
        };
        let (sessions, credential) = {
            let c = client.borrow();
            let Some(sessions) = c.sessions.clone() else {
                return Ok(None);
            };
            (sessions, c.credential.clone())
        };
        let report = controller.sample()?;
        let pages = report
            .pages()
            .map_err(|e| JsValue::from_str(&e.to_string()))?
            .into_iter()
            .map(|page| {
                serde_json::to_string(&page)
                    .map_err(|_| JsValue::from_str("layout page encoding failed"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        controller.inflight = true;
        controller.dirty = false;
        controller.last_sent = now;
        controller
            .root
            .set_attribute("data-layout-report", "pending")?;
        Ok(Some((
            sessions,
            credential,
            pages,
            report.revision,
            controller.generation,
        )))
    });
    let work = match work {
        Ok(work) => work,
        Err(_error) => {
            ACTIVE.with(|active| {
                if let Some(c) = active.borrow().as_ref() {
                    let _recorded = c
                        .root
                        .set_attribute("data-layout-report", "sampling_failed");
                }
            });
            return;
        }
    };
    let Some((mut sessions, credential, pages, revision, generation)) = work else {
        return;
    };
    wasm_bindgen_futures::spawn_local(async move {
        let outcome = async {
            let started = js_sys::Date::now();
            for text in pages {
                if !ACTIVE.with(|active| {
                    active
                        .borrow()
                        .as_ref()
                        .is_some_and(|c| c.generation == generation)
                }) {
                    return Err(JsValue::from_str("layout owner retired"));
                }
                if js_sys::Date::now() - started > 5000.0 {
                    return Err(JsValue::from_str("layout upload deadline expired"));
                }
                let message = rpc::PresentedRequest {
                    sequence: 0,
                    display_instance: super::live_motion::display_instance(),
                    layout_readback_json: text,
                };
                if prost::Message::encoded_len(&message) > df_rpc_bridge::RPC_MESSAGE_BYTES - 1024 {
                    return Err(JsValue::from_str("layout page exceeds RPC budget"));
                }
                let mut call = request(message, &credential)?;
                call.set_timeout(Duration::from_millis(2000));
                sessions
                    .report_presented(call)
                    .await
                    .map_err(|_| JsValue::from_str("layout report rejected"))?;
            }
            Ok::<_, JsValue>(())
        }
        .await;
        ACTIVE.with(|active| {
            let mut active = active.borrow_mut();
            if let Some(c) = active.as_mut().filter(|c| c.generation == generation) {
                c.inflight = false;
                let status = if outcome.is_err() {
                    "failed"
                } else if c.document.revision == revision {
                    "applied"
                } else {
                    "stale"
                };
                let _recorded = c.root.set_attribute("data-layout-report", status);
                if status == "applied" {
                    let _recorded = c
                        .root
                        .set_attribute("data-layout-reported-revision", &revision.to_string());
                }
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout_contract::{Distance, Transform, Unit};
    fn controller(html: &str) -> Controller {
        let document = web_sys::window().unwrap().document().unwrap();
        let root = document.create_element("section").unwrap();
        root.set_inner_html(html);
        document.body().unwrap().append_child(&root).unwrap();
        let stylesheet = document.create_element("style").unwrap();
        document
            .query_selector("head")
            .unwrap()
            .unwrap()
            .append_child(&stylesheet)
            .unwrap();
        Controller {
            client: Weak::new(),
            root,
            stylesheet,
            document: LayoutDocument::default(),
            registry: BTreeMap::new(),
            next_id: 0,
            sample: 0,
            generation: 1,
            dirty: true,
            inflight: false,
            last_sent: 0.0,
            overflow: 0,
            clock: None,
        }
    }
    #[test]
    fn every_nested_visual_gets_a_unique_stable_id_and_dynamic_nodes_join_inventory() {
        let mut c = controller(
            "<div class='tv-stagebox'><p class='tv-narration'><span>Words</span></p></div><figure class='tv-fig' data-slot='hero-1'><img alt='Brakka'><div class='tv-fig-bubbles'><span class='tv-bubble-name'>Brakka</span></div></figure><button>Continue</button><script type='application/json'>{}</script>",
        );
        c.refresh().unwrap();
        assert!(c.registry.contains_key("dm-overlay"));
        assert!(c.registry.contains_key("actor-hero-1"));
        assert!(c.registry.contains_key("bubbles-hero-1"));
        let span = c
            .root
            .query_selector(".tv-narration span")
            .unwrap()
            .unwrap();
        let id = span.get_attribute("data-layout-id").unwrap();
        c.refresh().unwrap();
        assert_eq!(span.get_attribute("data-layout-id").unwrap(), id);
        assert!(
            c.root
                .query_selector("script")
                .unwrap()
                .unwrap()
                .get_attribute("data-layout-id")
                .is_none()
        );
        let loader = c
            .root
            .owner_document()
            .unwrap()
            .create_element("div")
            .unwrap();
        loader.set_attribute("class", "thinking-loader").unwrap();
        c.root.append_child(&loader).unwrap();
        c.refresh().unwrap();
        assert!(loader.has_attribute("data-layout-id"));
        let report = c.sample().unwrap();
        let actor = report
            .elements
            .iter()
            .find(|e| e.id == "actor-hero-1")
            .unwrap();
        assert_eq!(
            actor.owner.as_ref().unwrap().name.as_deref(),
            Some("Brakka")
        );
        c.root.remove();
    }
    #[test]
    fn stylesheet_updates_and_reset_preserve_current_application_inline_styles() {
        let mut c = controller(
            "<div class='tv-stagebox' style='left:20px;transform:translateX(12px);animation: original-motion 10s infinite'>Words</div>",
        );
        c.refresh().unwrap();
        let element = c.registry["dm-overlay"].clone();
        let before = element.get_attribute("style");
        c.document.overrides.insert(
            "dm-overlay".into(),
            Transform {
                translate_y: Some(Distance {
                    value: 100.0,
                    unit: Unit::Px,
                }),
                rotate: Some(10.0),
                ..Default::default()
            },
        );
        c.stylesheet
            .set_text_content(Some(&c.document.stylesheet().unwrap()));
        assert_eq!(element.get_attribute("style"), before);
        let style = web_sys::window()
            .unwrap()
            .get_computed_style(&element)
            .unwrap()
            .unwrap();
        assert!(
            style
                .get_property_value("translate")
                .unwrap()
                .contains("100px")
        );
        assert_eq!(
            style.get_property_value("animation-name").unwrap(),
            "original-motion"
        );
        element
            .set_attribute(
                "style",
                "left:40px;transform:rotate(3deg);animation: new-motion 5s infinite",
            )
            .unwrap();
        c.document.overrides.clear();
        c.stylesheet
            .set_text_content(Some(&c.document.stylesheet().unwrap()));
        assert_eq!(
            element.get_attribute("style").as_deref(),
            Some("left:40px;transform:rotate(3deg);animation: new-motion 5s infinite")
        );
        let style = web_sys::window()
            .unwrap()
            .get_computed_style(&element)
            .unwrap()
            .unwrap();
        assert_eq!(style.get_property_value("translate").unwrap(), "none");
        assert_eq!(
            style.get_property_value("animation-name").unwrap(),
            "new-motion"
        );
        c.root.remove();
    }
    #[test]
    fn overflow_is_explicit_and_removed_targets_report_absent() {
        let mut c = controller("<div class='tv-stagebox'>DM</div>");
        for _ in 0..550 {
            let node = c
                .root
                .owner_document()
                .unwrap()
                .create_element("span")
                .unwrap();
            c.root.append_child(&node).unwrap();
        }
        c.refresh().unwrap();
        assert_eq!(c.registry.len(), contract::MAX_ELEMENTS);
        assert!(c.overflow > 0);
        assert!(c.registry.contains_key("dm-overlay"));
        c.document.overrides.insert(
            "dm-overlay".into(),
            Transform {
                rotate: Some(10.0),
                ..Default::default()
            },
        );
        c.registry["dm-overlay"].remove();
        c.refresh().unwrap();
        let report = c.sample().unwrap();
        assert_eq!(
            report
                .elements
                .iter()
                .find(|e| e.id == "dm-overlay")
                .unwrap()
                .status,
            ElementStatus::Absent
        );
        c.root.remove();
    }
}
