#[cfg(target_arch = "wasm32")]
#[path = "../support/resource_flat.rs"]
mod geometry;
#[cfg(target_arch = "wasm32")]
#[path = "../support/lifecycle.rs"]
mod support;

#[cfg(target_arch = "wasm32")]
mod browser {
    use crate::{geometry, support};
    use df_render::{BrowserImageDecode, ResourceLifecycle};
    use std::cell::RefCell;
    use std::rc::Rc;
    use support::*;
    use wasm_bindgen::{JsCast, prelude::*};
    use web_sys::{CanvasRenderingContext2d, Element, HtmlCanvasElement};

    struct Host(Element);
    impl Drop for Host {
        fn drop(&mut self) {
            self.0.remove();
        }
    }

    struct Fixture {
        host: Host,
        status: Element,
        lifecycle: Rc<RefCell<ResourceLifecycle>>,
        key: df_client::cache::CacheKey,
        unselected: df_client::cache::CacheKey,
        canvas: HtmlCanvasElement,
        token: Element,
        phase: u8,
        busy: bool,
    }

    thread_local! {
        static FIXTURE: RefCell<Option<Fixture>> = const { RefCell::new(None) };
    }

    fn error(value: impl std::fmt::Debug) -> JsValue {
        JsValue::from_str(&format!("{value:?}"))
    }
    fn require(value: bool, message: &str) -> Result<(), JsValue> {
        if value {
            Ok(())
        } else {
            Err(JsValue::from_str(message))
        }
    }
    fn query(host: &Element, selector: &str) -> Result<Element, JsValue> {
        host.query_selector(selector)?
            .ok_or_else(|| error(selector))
    }
    fn pixel(canvas: &HtmlCanvasElement) -> Result<Vec<u8>, JsValue> {
        let context = canvas
            .get_context("2d")?
            .ok_or_else(|| error("2d context missing"))?
            .dyn_into::<CanvasRenderingContext2d>()?;
        Ok(context.get_image_data(0.0, 0.0, 1.0, 1.0)?.data().0)
    }
    fn describe(fixture: &Fixture) -> Result<(), JsValue> {
        let owner = fixture.lifecycle.borrow();
        fixture
            .host
            .0
            .set_attribute("data-lifecycle-phase", &fixture.phase.to_string())?;
        fixture
            .host
            .0
            .set_attribute("data-decoded-bytes", &owner.decoded_bytes().to_string())?;
        fixture
            .host
            .0
            .set_attribute("data-work-bytes", &owner.work_bytes().to_string())?;
        fixture
            .host
            .0
            .set_attribute("data-surface-bytes", &owner.surface_bytes().to_string())?;
        fixture
            .host
            .0
            .set_attribute("data-resident-bytes", &owner.resident_bytes().to_string())?;
        fixture
            .host
            .0
            .set_attribute("data-leases", &owner.lease_count().to_string())?;
        fixture.status.set_text_content(Some(match fixture.phase {
            0 => "Two actual PNG keys decoded into separate leased residents. Current illustration is selected.",
            1 => "Unselected image invalidation preserved current pixels and the same canvas and flat token.",
            2 => "Byte-only invalidation scrubbed the canvas. Flat scene remains mounted.",
            3 => "Accepted reference revocation cancelled actual codec work. Terminal callback drained it.",
            4 => "Scope replacement scrubbed the old mount. Actual codec callback drained old work.",
            _ => "Disposed.",
        }));
        Ok(())
    }

    #[wasm_bindgen]
    pub async fn resource_lifecycle_fixture_start() -> Result<(), JsValue> {
        FIXTURE.with(|slot| require(slot.borrow().is_none(), "fixture already mounted"))?;
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| error("document unavailable"))?;
        let host = Host(document.create_element("section")?);
        host.0
            .set_attribute("data-lifecycle-fixture", "actual-png-closed-owner")?;
        let status = document.create_element("p")?;
        status.set_text_content(Some("Decoding the actual immutable fixture PNG…"));
        host.0.append_child(&status)?;
        document
            .body()
            .ok_or_else(|| error("body unavailable"))?
            .append_child(&host.0)?;
        let mut lifecycle = ResourceLifecycle::mount(
            &host.0,
            scope(),
            two_asset_cache_limits(),
            resource_limits(),
            label("bounded-png-v1"),
            8,
        )
        .map_err(error)?;
        let key = key("owned-png");
        let unselected = support::key("unselected-png");
        let references = [unselected.clone(), key.clone()];
        lifecycle
            .apply_current(scope(), revision(1, 0), &references)
            .map_err(error)?;
        lifecycle
            .update_scene(
                label("same-scene"),
                geometry::flat(revision(1, 0)),
                &references,
            )
            .map_err(error)?;
        install(&mut lifecycle, &unselected);
        install(&mut lifecycle, &key);
        let lifecycle = Rc::new(RefCell::new(lifecycle));
        let canvas = query(&host.0, "canvas")?.dyn_into::<HtmlCanvasElement>()?;
        canvas.set_attribute("style", "width:96px;height:96px;image-rendering:pixelated")?;
        let token = query(&host.0, "[data-token-key]")?;
        FIXTURE.with(|slot| {
            *slot.borrow_mut() = Some(Fixture {
                host,
                status,
                lifecycle: lifecycle.clone(),
                key: key.clone(),
                unselected: unselected.clone(),
                canvas,
                token,
                phase: 0,
                busy: true,
            });
        });
        let result = async {
            let mut unrelated_decode = BrowserImageDecode::start_owned(
                lifecycle.clone(),
                &unselected,
                decode_limits(),
                10_000,
            )
            .map_err(error)?;
            require(
                unrelated_decode.wait().await.map_err(error)?,
                "unselected actual PNG did not decode",
            )?;
            let mut decode =
                BrowserImageDecode::start_owned(lifecycle.clone(), &key, decode_limits(), 10_000)
                    .map_err(error)?;
            require(
                decode.wait().await.map_err(error)?,
                "actual PNG did not present",
            )?;
            FIXTURE.with(|slot| {
                let mut slot = slot.borrow_mut();
                let fixture = slot
                    .as_mut()
                    .ok_or_else(|| error("fixture removed during decode"))?;
                require(
                    pixel(&fixture.canvas)? == [16, 32, 48, 255],
                    "actual PNG pixels differ",
                )?;
                require(
                    lifecycle.borrow().decoded_bytes() == 8
                        && lifecycle.borrow().work_bytes() == 0
                        && lifecycle.borrow().lease_count() == 2
                        && lifecycle.borrow().resident_bytes() == PNG.len() * 2
                        && lifecycle.borrow().surface_bytes() == 8,
                    "initial codec accounting differs",
                )?;
                fixture.busy = false;
                describe(fixture)
            })
        }
        .await;
        if let Err(value) = result {
            lifecycle.borrow_mut().dispose().map_err(error)?;
            FIXTURE.with(|slot| {
                *slot.borrow_mut() = None;
            });
            return Err(value);
        }
        Ok(())
    }

    #[wasm_bindgen]
    pub async fn resource_lifecycle_fixture_advance() -> Result<u8, JsValue> {
        let (owner, key, unselected, phase) = FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let fixture = slot.as_mut().ok_or_else(|| error("fixture absent"))?;
            require(!fixture.busy, "fixture callback still running")?;
            require(fixture.phase < 4, "fixture terminal")?;
            fixture.busy = true;
            Ok::<_, JsValue>((
                fixture.lifecycle.clone(),
                fixture.key.clone(),
                fixture.unselected.clone(),
                fixture.phase,
            ))
        })?;
        let result = async {
            match phase {
                0 => {
                    FIXTURE.with(|slot| {
                        let slot = slot.borrow();
                        let fixture = slot.as_ref().ok_or_else(|| error("fixture absent"))?;
                        require(fixture.canvas.width() == 1 && fixture.canvas.height() == 1
                            && pixel(&fixture.canvas)? == [16, 32, 48, 255] && fixture.token.is_connected(),
                            "selected actual image baseline differs")
                    })?;
                    let work_bytes = owner.borrow().work_bytes();
                    owner.borrow_mut().release(&unselected).map_err(error)?;
                    // No fixture refresh, present or scene update: canonical invalidation
                    // itself must preserve the distinct current selected illustration.
                    FIXTURE.with(|slot| {
                        let slot = slot.borrow();
                        let fixture = slot.as_ref().ok_or_else(|| error("fixture absent"))?;
                        require(fixture.canvas.is_same_node(Some(&query(&fixture.host.0, "canvas")?.into()))
                            && fixture.canvas.width() == 1 && fixture.canvas.height() == 1
                            && pixel(&fixture.canvas)? == [16, 32, 48, 255]
                            && fixture.token.is_same_node(Some(&query(&fixture.host.0, "[data-token-key]")?.into()))
                            && fixture.token.is_connected(),
                            "unselected canonical invalidation replaced or cleared selected pixels/nodes")?;
                        let lifecycle = owner.borrow();
                        require(
                            work_bytes == 0
                                && lifecycle.work_bytes() == work_bytes
                                && lifecycle.decoded_bytes() == 4
                                && lifecycle.surface_bytes() == 8
                                && lifecycle.lease_count() == 1
                                && lifecycle.resident_bytes() == PNG.len(),
                            "unselected invalidation changed selected work/surface or retained unrelated lease")
                    })?;
                }
                1 => {
                    owner.borrow_mut().release(&key).map_err(error)?;
                    FIXTURE.with(|slot| {
                        let slot = slot.borrow();
                        let fixture = slot.as_ref().ok_or_else(|| error("fixture absent"))?;
                        require(fixture.canvas.width() == 0 && fixture.token.is_connected(),
                            "byte-only invalidation did not scrub without a Scene revision")?;
                        require(owner.borrow().decoded_bytes() == 0 && owner.borrow().surface_bytes() == 0,
                            "invalidated actual pixels retained")
                    })?;
                }
                2 => {
                    install(&mut owner.borrow_mut(), &key);
                    let mut decode = BrowserImageDecode::start_owned(owner.clone(), &key, decode_limits(), 10_000)
                        .map_err(error)?;
                    let work_bytes = owner.borrow().work_bytes();
                    require(work_bytes != 0, "actual running codec not admitted")?;
                    owner.borrow_mut().apply_current(scope(), revision(1, 1), &[]).map_err(error)?;
                    require(
                        owner.borrow().work_bytes() == work_bytes,
                        "abort request freed work before terminal codec",
                    )?;
                    require(decode.wait().await.is_err(), "revoked actual codec published")?;
                    require(owner.borrow().work_bytes() == 0 && owner.borrow().decoded_bytes() == 0,
                        "terminal revoked codec retained work or pixels")?;
                }
                3 => {
                    owner.borrow_mut().apply_current(scope(), revision(1, 2), std::slice::from_ref(&key))
                        .map_err(error)?;
                    install(&mut owner.borrow_mut(), &key);
                    let mut decode = BrowserImageDecode::start_owned(owner.clone(), &key, decode_limits(), 10_000)
                        .map_err(error)?;
                    let work_bytes = owner.borrow().work_bytes();
                    let mut replacement = scope();
                    replacement.binding = df_types::ClientBindingId::from_bytes(&[9; 16]).map_err(error)?;
                    owner.borrow_mut().replace_scope(replacement).map_err(error)?;
                    require(work_bytes != 0 && owner.borrow().work_bytes() == work_bytes,
                        "scope replacement released still-running codec")?;
                    FIXTURE.with(|slot| {
                        let slot = slot.borrow();
                        let fixture = slot.as_ref().ok_or_else(|| error("fixture absent"))?;
                        require(!fixture.canvas.is_connected() && !fixture.token.is_connected(),
                            "scope replacement retained old mount")
                    })?;
                    require(decode.wait().await.is_err(), "old scope callback published")?;
                    require(owner.borrow().work_bytes() == 0 && owner.borrow().resident_bytes() == 0,
                        "old terminal callback retained allocations")?;
                }
                _ => return Err(error("fixture terminal")),
            }
            Ok::<_, JsValue>(())
        }.await;
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let fixture = slot
                .as_mut()
                .ok_or_else(|| error("fixture removed during callback"))?;
            fixture.busy = false;
            result?;
            fixture.phase += 1;
            describe(fixture)?;
            Ok(fixture.phase)
        })
    }

    #[wasm_bindgen]
    pub fn resource_lifecycle_fixture_dispose() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            if let Some(fixture) = slot.as_ref() {
                require(
                    !fixture.busy,
                    "await actual codec callback before fixture teardown",
                )?;
                fixture.lifecycle.borrow_mut().dispose().map_err(error)?;
                require(
                    fixture.lifecycle.borrow().work_bytes() == 0,
                    "fixture has nonterminal codec work",
                )?;
            }
            *slot = None;
            Ok(())
        })
    }
}
