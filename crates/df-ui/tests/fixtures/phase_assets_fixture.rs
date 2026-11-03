#[cfg(target_arch = "wasm32")]
mod browser {
    use df_ui::{ActionView, ConceptScene, ControlledAction, PHASE_ASSETS, PhaseAsset};
    use std::{cell::RefCell, rc::Rc};
    use wasm_bindgen::{JsValue, closure::Closure, prelude::wasm_bindgen};
    use web_sys::{Document, Element, Event};

    const STYLES: &str = r#"
.df-assets{background:#070d14;color:#f5eee0;font-family:Georgia,serif;min-height:100svh}
.df-assets *{box-sizing:border-box}.df-assets .hero{position:relative;isolation:isolate;padding:64px max(5vw,24px) 52px;min-height:340px;overflow:hidden;border-bottom:1px solid #dfb97450}
.df-assets .scene{position:absolute;inset:0;width:100%;height:100%;object-fit:cover;z-index:-2}.df-assets .hero:before{content:'';position:absolute;inset:0;background:linear-gradient(90deg,#070d14ee,#070d1440),linear-gradient(0deg,#070d14,transparent);z-index:-1}
.df-assets .eyebrow{font:11px/1.6 system-ui,sans-serif;letter-spacing:2.2px;text-transform:uppercase;color:#dfb974}.df-assets h1{font-size:clamp(36px,5vw,64px);font-weight:400;margin:16px 0;max-width:750px}.df-assets h2{font-size:25px;font-weight:400;margin:0 0 12px}.df-assets p{font:14px/1.8 system-ui,sans-serif;color:#b6bec6;max-width:680px}.df-assets .content{max-width:1400px;margin:auto;padding:36px max(5vw,24px) 56px}
.df-assets .toolbar{display:flex;gap:12px;flex-wrap:wrap;margin:20px 0}.df-assets button{min-height:44px;font:12px system-ui,sans-serif;color:#f5eee0;background:#10212e;border:1px solid #dfb97480;padding:12px 18px;border-radius:3px;cursor:pointer}.df-assets button:hover{background:#203849}.df-assets button:focus-visible{outline:2px solid #87b8c5;outline-offset:4px}
.df-assets .grid{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:16px}.df-assets .card{background:linear-gradient(145deg,#10202c,#0b1420);border:1px solid #dfb97440;padding:24px;border-radius:3px}.df-assets .card h2{font-size:20px}.df-assets .sizes{display:flex;align-items:center;gap:25px;height:88px}.df-assets .sample{display:grid;justify-items:center;gap:10px;min-width:48px}.df-assets .size-label{font:10px system-ui,sans-serif;color:#87b8c5;letter-spacing:.7px}.df-assets .slot{display:grid;place-items:center}.df-assets .path{overflow-wrap:anywhere;font:11px/1.6 ui-monospace,monospace;color:#90a6b6;margin-top:10px}
.df-assets .ornaments{display:grid;grid-template-columns:1fr 2fr;gap:28px;margin-top:28px}.df-assets .portrait{width:120px;height:144px;position:relative;margin-top:20px}.df-assets .portrait-photo{position:absolute;left:8px;top:8px;width:104px;height:128px;object-fit:cover}.df-assets .portrait-frame{position:absolute;inset:0;width:120px;height:144px}.df-assets .divider-slot{width:min(240px,100%);height:16px;margin:24px 0}.df-assets .divider{display:block;width:100%;height:100%}.df-assets .row{display:flex;align-items:center;gap:14px;border-top:1px solid #dfb97425;padding:15px 0;font-size:17px}.df-assets .row .slot{width:24px;height:24px;flex-shrink:0}.df-assets .status{border-left:2px solid #87b8c5;padding-left:14px;font:12px/1.7 system-ui,sans-serif;color:#b6bec6}.df-assets [hidden]{display:none!important}
@media(max-width:900px){.df-assets .grid{grid-template-columns:repeat(2,minmax(0,1fr))}.df-assets .ornaments{grid-template-columns:1fr 1fr}}@media(max-width:600px){.df-assets .grid,.df-assets .ornaments{grid-template-columns:1fr}.df-assets .hero{padding-top:44px}.df-assets .card{padding:20px}}
"#;

    struct LoadListener {
        image: Element,
        callback: Closure<dyn FnMut(Event)>,
    }

    struct Fixture {
        root: Element,
        listeners: Vec<LoadListener>,
        controls: Vec<Rc<ControlledAction>>,
    }

    thread_local! {
        static FIXTURE: RefCell<Option<Fixture>> = const { RefCell::new(None) };
    }

    fn child(
        document: &Document,
        parent: &Element,
        tag: &str,
        class: &str,
        text: Option<&str>,
    ) -> Result<Element, JsValue> {
        let node = document.create_element(tag)?;
        node.set_class_name(class);
        node.set_text_content(text);
        parent.append_child(&node)?;
        Ok(node)
    }

    fn image(
        document: &Document,
        parent: &Element,
        path: &str,
        alt: &str,
        class: &str,
        status: &Element,
        listeners: &mut Vec<LoadListener>,
    ) -> Result<Element, JsValue> {
        let image = child(document, parent, "img", class, None)?;
        image.set_attribute("alt", alt)?;
        if alt.is_empty() {
            image.set_attribute("aria-hidden", "true")?;
        }
        let failed_image = image.clone();
        let status = status.clone();
        let callback = Closure::wrap(Box::new(move |_event: Event| {
            let result = failed_image
                .set_attribute("hidden", "")
                .and_then(|()| failed_image.set_attribute("data-load-failed", "true"));
            status.set_text_content(Some(if result.is_ok() {
                "Optional artwork unavailable. Labels and controls remain available; no retry was started."
            } else {
                "Optional artwork unavailable. Its fallback could not be applied."
            }));
        }) as Box<dyn FnMut(Event)>);
        image.add_event_listener_with_callback("error", callback.as_ref().unchecked_ref())?;
        listeners.push(LoadListener {
            image: image.clone(),
            callback,
        });
        image.set_attribute("src", path)?;
        Ok(image)
    }

    use wasm_bindgen::JsCast;

    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), JsValue> {
        if FIXTURE.with(|slot| slot.borrow().is_some()) {
            return Err(JsValue::from_str("asset showcase already mounted"));
        }
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| JsValue::from_str("browser document unavailable"))?;
        let body = document
            .body()
            .ok_or_else(|| JsValue::from_str("browser body unavailable"))?;
        body.set_attribute("style", "margin:0;background:#070d14")?;
        document.set_title("DungeonFlux · Reusable phase artwork");
        let root = document.create_element("main")?;
        root.set_class_name("df-assets");
        let style = child(&document, &root, "style", "", Some(STYLES))?;
        style.set_attribute("data-fixture-styles", "phase-assets")?;
        let hero = child(&document, &root, "header", "hero", None)?;
        child(
            &document,
            &hero,
            "div",
            "eyebrow",
            Some("DungeonFlux · Interface collection"),
        )?;
        child(
            &document,
            &hero,
            "h1",
            "",
            Some("Small symbols. A shared world."),
        )?;
        child(
            &document,
            &hero,
            "p",
            "",
            Some(
                "Original vector artwork for the Drowned Lantern. Gold, ivory and storm teal carry the same quiet detail from the cinematic scene into every phase.",
            ),
        )?;
        let content = child(&document, &root, "section", "content", None)?;
        child(&document, &content, "h2", "", Some("The action kit"))?;
        child(
            &document,
            &content,
            "p",
            "",
            Some(
                "Nine icons shown at 24, 32 and 48 pixels. Text supplies meaning; the artwork supports it.",
            ),
        )?;
        let toolbar = child(&document, &content, "div", "toolbar", None)?;
        let status = child(
            &document,
            &content,
            "p",
            "status",
            Some("Ready · Local static artwork. No gameplay session connected."),
        )?;
        status.set_attribute("role", "status")?;
        let mut listeners = Vec::new();
        image(
            &document,
            &hero,
            ConceptScene::Harbor.asset_path(),
            ConceptScene::Harbor.description(),
            "scene",
            &status,
            &mut listeners,
        )?;
        let grid = child(&document, &content, "div", "grid", None)?;
        let mut kit_images = Vec::new();
        for asset in PHASE_ASSETS {
            let Some(label) = asset.accessible_label() else {
                continue;
            };
            let card = child(&document, &grid, "article", "card", None)?;
            card.set_attribute("data-asset", asset.name())?;
            child(&document, &card, "h2", "", Some(label))?;
            let sizes = child(&document, &card, "div", "sizes", None)?;
            for size in [24, 32, 48] {
                let sample = child(&document, &sizes, "div", "sample", None)?;
                let slot = child(&document, &sample, "span", "slot", None)?;
                slot.set_attribute("style", &format!("width:{size}px;height:{size}px"))?;
                let icon = image(
                    &document,
                    &slot,
                    asset.asset_path(),
                    "",
                    "",
                    &status,
                    &mut listeners,
                )?;
                icon.set_attribute("width", &size.to_string())?;
                icon.set_attribute("height", &size.to_string())?;
                kit_images.push(icon);
                child(
                    &document,
                    &sample,
                    "span",
                    "size-label",
                    Some(&format!("{size} px")),
                )?;
            }
            child(&document, &card, "div", "path", Some(asset.asset_path()))?;
        }
        let ornaments = child(&document, &content, "section", "ornaments", None)?;
        let portrait_card = child(&document, &ornaments, "article", "card", None)?;
        child(&document, &portrait_card, "h2", "", Some("Portrait frame"))?;
        child(
            &document,
            &portrait_card,
            "p",
            "",
            Some("The Storyteller · Preserved original avatar, with a separate transparent frame."),
        )?;
        let portrait = child(&document, &portrait_card, "div", "portrait", None)?;
        image(
            &document,
            &portrait,
            "assets/placeholder/avatar-narrator.webp",
            "The Storyteller",
            "portrait-photo",
            &status,
            &mut listeners,
        )?;
        kit_images.push(image(
            &document,
            &portrait,
            PhaseAsset::PortraitFrame.asset_path(),
            "",
            "portrait-frame",
            &status,
            &mut listeners,
        )?);
        let section_card = child(&document, &ornaments, "article", "card", None)?;
        child(
            &document,
            &section_card,
            "h2",
            "",
            Some("The explorer’s satchel"),
        )?;
        child(
            &document,
            &section_card,
            "p",
            "",
            Some(
                "Ornaments use a quieter weight. Missing artwork keeps the same content and spacing.",
            ),
        )?;
        // The slot retains the divider bounds when only its optional artwork is hidden.
        let divider_slot = child(&document, &section_card, "div", "divider-slot", None)?;
        divider_slot.set_attribute("aria-hidden", "true")?;
        kit_images.push(image(
            &document,
            &divider_slot,
            PhaseAsset::SectionOrnament.asset_path(),
            "",
            "divider",
            &status,
            &mut listeners,
        )?);
        for (asset, label) in [
            (PhaseAsset::Compass, "Follow the harbor trail"),
            (PhaseAsset::Lantern, "Lantern · Equipment"),
            (PhaseAsset::Spellbook, "Spellbook · Inventory"),
        ] {
            let row = child(&document, &section_card, "div", "row", None)?;
            let slot = child(&document, &row, "span", "slot", None)?;
            kit_images.push(image(
                &document,
                &slot,
                asset.asset_path(),
                "",
                "",
                &status,
                &mut listeners,
            )?);
            child(&document, &row, "span", "", Some(label))?;
        }
        let mut controls = Vec::new();
        for (label, hidden) in [
            ("Preview label-only fallback", true),
            ("Restore kit", false),
        ] {
            let control = Rc::new(
                ControlledAction::create(
                    &document,
                    ActionView {
                        label,
                        enabled: true,
                        pending: false,
                    },
                )
                .map_err(|error| JsValue::from_str(&error.to_string()))?,
            );
            let images = kit_images.clone();
            let status = status.clone();
            control.on_activate(move || {
                let result = images.iter().try_for_each(|image| {
                    if hidden {
                        image.set_attribute("hidden", "")
                    } else if image.get_attribute("data-load-failed").is_some() {
                        Ok(())
                    } else {
                        image.remove_attribute("hidden")
                    }
                });
                status.set_text_content(Some(if result.is_err() {
                    "FAIL · Fallback preview could not be updated."
                } else if hidden {
                    "Fallback preview · All icon labels and controls remain available. Decorative artwork is omitted."
                } else {
                    "Kit restored where available. Missing artwork keeps its labels."
                }));
            }).map_err(|error| JsValue::from_str(&error.to_string()))?;
            toolbar.append_child(control.element())?;
            controls.push(control);
        }
        body.append_child(&root)?;
        FIXTURE.with(|slot| {
            *slot.borrow_mut() = Some(Fixture {
                root,
                listeners,
                controls,
            })
        });
        Ok(())
    }

    /// Releases all owned error listeners and controls. Repeated calls succeed.
    #[wasm_bindgen]
    pub fn shutdown() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut owned = slot.borrow_mut();
            if let Some(fixture) = owned.as_ref() {
                for listener in &fixture.listeners {
                    listener.image.remove_event_listener_with_callback(
                        "error",
                        listener.callback.as_ref().unchecked_ref(),
                    )?;
                }
                for control in &fixture.controls {
                    control
                        .dispose()
                        .map_err(|error| JsValue::from_str(&error.to_string()))?;
                }
                if let Some(parent) = fixture.root.parent_node() {
                    parent.remove_child(&fixture.root)?;
                }
            }
            *owned = None;
            Ok(())
        })
    }
}
