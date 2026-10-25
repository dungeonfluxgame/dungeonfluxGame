#[cfg(target_arch = "wasm32")]
#[path = "../../../df-render/tests/support/campaign_png.rs"]
mod images;

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::images;
    use df_assets::AssetManifest;
    use df_client::cache::{CacheError, CacheKey, CacheLimits, CacheScope};
    use df_render::{ImageDecodeError, ResourceError};
    use df_types::{
        ClientBindingId, RecoveryEpoch, RevisionLabel, RunId, SessionId, SessionRevision,
    };
    use df_ui::{
        CampaignError, CampaignLimits, CampaignSceneAssets, CampaignSurface, CampaignView,
        ConceptScene, SceneImageError, SceneImageLimits, text_input,
    };
    use std::{cell::RefCell, rc::Rc};
    use wasm_bindgen::{JsCast, JsValue, prelude::wasm_bindgen};
    use web_sys::{CanvasRenderingContext2d, Element, HtmlCanvasElement, HtmlInputElement};

    struct Fixture {
        surface: Rc<CampaignSurface>,
        scope: CacheScope,
        revision: SessionRevision,
        selected: Option<CacheKey>,
        expected: Option<usize>,
        input: HtmlInputElement,
    }
    thread_local! {static FIXTURE:RefCell<Option<Fixture>>=const {RefCell::new(None)};}
    fn error(value: impl std::fmt::Debug) -> JsValue {
        JsValue::from_str(&format!("{value:?}"))
    }
    fn require(value: bool, why: &'static str) -> Result<(), JsValue> {
        if value {
            Ok(())
        } else {
            Err(JsValue::from_str(why))
        }
    }
    fn campaign_view() -> CampaignView<'static> {
        CampaignView {
            scene: ConceptScene::Harbor,
            chapter: "Synthetic resource integration",
            title: "Actual campaign image owner",
            description: "No gameplay session or media provider connected.",
            location: "Fixture",
            scene_label: "Permitted synthetic PNG",
            narration: "Input remains available through image replacement and failure.",
            connection: "Fixture connected",
            notice: "Resource lifecycle boundary",
            members: &[],
            objectives: &[],
        }
    }
    fn limits() -> SceneImageLimits {
        SceneImageLimits {
            cache: CacheLimits {
                max_assets: 2,
                max_pending: 1,
                max_leases: 1,
                max_bytes: 4096,
            },
            max_width: 8,
            max_height: 8,
            max_pixels: 64,
        }
    }
    fn scope(binding: u8) -> Result<CacheScope, JsValue> {
        Ok(CacheScope {
            session: SessionId::from_bytes(&[1; 16]).map_err(error)?,
            run: RunId::from_bytes(&[2; 16]).map_err(error)?,
            binding: ClientBindingId::from_bytes(&[binding; 16]).map_err(error)?,
        })
    }
    fn key(name: &str, bytes: &[u8], digest: [u8; 32]) -> Result<CacheKey, JsValue> {
        Ok(CacheKey {
            version: RevisionLabel::new(Some(name)).map_err(error)?,
            bytes: AssetManifest {
                byte_len: bytes.len() as u64,
                sha256: digest,
            },
        })
    }
    fn next_revision(fixture: &mut Fixture) -> Result<SessionRevision, JsValue> {
        fixture.revision = fixture.revision.next_sequence().map_err(error)?;
        Ok(fixture.revision)
    }
    fn apply(fixture: &mut Fixture, key: &CacheKey, bytes: &[u8]) -> Result<(), JsValue> {
        let revision = next_revision(fixture)?;
        let token = fixture
            .surface
            .update_with_scene_assets(
                &campaign_view(),
                CampaignSceneAssets {
                    scope: fixture.scope,
                    revision,
                    references: std::slice::from_ref(key),
                    selected: Some(key),
                },
            )
            .map_err(error)?;
        fixture.selected = Some(key.clone());
        if let Some(token) = token {
            fixture
                .surface
                .complete_scene_asset(&token, bytes.to_vec())
                .map_err(error)?;
        }
        Ok(())
    }
    fn canvas(root: &Element) -> Result<HtmlCanvasElement, JsValue> {
        root.query_selector("canvas.scene-art")?
            .ok_or_else(|| error("owned illustration surface missing"))?
            .dyn_into()
            .map_err(error)
    }
    fn build(detached: Option<usize>) -> Result<(), JsValue> {
        require(
            FIXTURE.with(|slot| slot.borrow().is_none()),
            "fixture already mounted",
        )?;
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| error("document missing"))?;
        let body = document.body().ok_or_else(|| error("body missing"))?;
        let surface = Rc::new(
            CampaignSurface::create(
                &document,
                &campaign_view(),
                CampaignLimits {
                    max_members: 4,
                    max_objectives: 4,
                    max_text_bytes: 512,
                },
            )
            .map_err(error)?,
        );
        let scope = scope(3)?;
        surface
            .enable_scene_assets(scope, limits())
            .map_err(error)?;
        let (field, input) =
            text_input(&document, "Retained resource fixture draft").map_err(error)?;
        input.set_value("Keep this valid draft");
        surface.navigation().append_child(&field)?;
        let mut fixture = Fixture {
            surface,
            scope,
            revision: SessionRevision::new(RecoveryEpoch::new(1).map_err(error)?, 0),
            selected: None,
            expected: None,
            input,
        };
        if let Some(index) = detached {
            let sample = images::IMAGES
                .get(index)
                .ok_or_else(|| error("sample missing"))?;
            let key = key(sample.name, sample.bytes, sample.digest)?;
            apply(&mut fixture, &key, sample.bytes)?;
            fixture.expected = Some(index);
            require(
                fixture.surface.root().query_selector("canvas")?.is_none(),
                "detached decode allocated a surface",
            )?;
        }
        body.append_child(fixture.surface.root())?;
        fixture.input.focus()?;
        FIXTURE.with(|slot| *slot.borrow_mut() = Some(fixture));
        Ok(())
    }
    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), JsValue> {
        build(None)
    }
    #[wasm_bindgen]
    pub fn mount_detached_format(index: u32) -> Result<(), JsValue> {
        shutdown()?;
        build(Some(index as usize))
    }
    #[wasm_bindgen]
    pub fn select_format(index: u32) -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut owned = slot.borrow_mut();
            let fixture = owned.as_mut().ok_or_else(|| error("fixture missing"))?;
            let sample = images::IMAGES
                .get(index as usize)
                .ok_or_else(|| error("sample missing"))?;
            let key = key(sample.name, sample.bytes, sample.digest)?;
            apply(fixture, &key, sample.bytes)?;
            fixture.expected = Some(index as usize);
            Ok(())
        })
    }
    /// Call only after the independent runner observes real terminal generated status.
    #[wasm_bindgen]
    pub fn check_generated() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let owned = slot.borrow();
            let fixture = owned.as_ref().ok_or_else(|| error("fixture missing"))?;
            let sample = images::IMAGES
                .get(
                    fixture
                        .expected
                        .ok_or_else(|| error("no expected sample"))?,
                )
                .ok_or_else(|| error("sample missing"))?;
            require(
                fixture
                    .surface
                    .root()
                    .get_attribute("data-scene-image")
                    .as_deref()
                    == Some("generated"),
                "real decoder has not presented",
            )?;
            let canvas = canvas(fixture.surface.root())?;
            require(
                (canvas.width(), canvas.height()) == (1, 1) && !canvas.hidden(),
                "wrong real surface dimensions/readiness",
            )?;
            let context = canvas
                .get_context("2d")?
                .ok_or_else(|| error("context missing"))?
                .dyn_into::<CanvasRenderingContext2d>()
                .map_err(error)?;
            let pixels = context.get_image_data(0.0, 0.0, 1.0, 1.0)?.data().0;
            require(
                pixels.as_slice() == sample.pixel.as_slice(),
                "actual decoder pixels mismatch",
            )?;
            require(
                fixture.input.value() == "Keep this valid draft",
                "draft changed",
            )?;
            require(fixture.input.is_connected(), "input removed")?;
            let document = fixture
                .surface
                .root()
                .owner_document()
                .ok_or_else(|| error("document missing"))?;
            require(
                document
                    .active_element()
                    .is_some_and(|active| active.is_same_node(Some(&fixture.input))),
                "focus changed",
            )?;
            require(
                fixture.surface.root().query_selector("svg")?.is_none(),
                "illustration fabricated flat geometry",
            )?;
            // Read shared source facts used by the independent bounded-plan fixture too.
            require(
                sample.source_bytes_per_pixel > 0
                    && (!sample.interlaced || sample.name == "rgba8-adam7"),
                "fixture metadata mismatch",
            )?;
            Ok(())
        })
    }
    #[wasm_bindgen]
    pub fn same_key_updates() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut owned = slot.borrow_mut();
            let fixture = owned.as_mut().ok_or_else(|| error("fixture missing"))?;
            let key = fixture
                .selected
                .clone()
                .ok_or_else(|| error("selection missing"))?;
            let before = canvas(fixture.surface.root())?;
            for _ in 0..64 {
                let revision = next_revision(fixture)?;
                let request = fixture
                    .surface
                    .update_with_scene_assets(
                        &campaign_view(),
                        CampaignSceneAssets {
                            scope: fixture.scope,
                            revision,
                            references: std::slice::from_ref(&key),
                            selected: Some(&key),
                        },
                    )
                    .map_err(error)?;
                require(request.is_none(), "same-key resident refetched")?;
                require(
                    before.is_same_node(Some(&canvas(fixture.surface.root())?.into())),
                    "same-key canvas replaced",
                )?;
            }
            Ok(())
        })
    }
    #[wasm_bindgen]
    pub fn refuse_old_scope_revision_and_version() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let owned = slot.borrow();
            let fixture = owned.as_ref().ok_or_else(|| error("fixture missing"))?;
            let key = fixture
                .selected
                .as_ref()
                .ok_or_else(|| error("selection missing"))?;
            let revision = fixture.revision.next_sequence().map_err(error)?;
            let wrong = fixture.surface.update_with_scene_assets(
                &campaign_view(),
                CampaignSceneAssets {
                    scope: scope(9)?,
                    revision,
                    references: std::slice::from_ref(key),
                    selected: Some(key),
                },
            );
            require(
                matches!(
                    wrong,
                    Err(CampaignError::SceneImage(SceneImageError::Cache(
                        CacheError::WrongScope
                    )))
                ),
                "wrong scope changed component",
            )?;
            let stale = fixture.surface.update_with_scene_assets(
                &campaign_view(),
                CampaignSceneAssets {
                    scope: fixture.scope,
                    revision: fixture.revision,
                    references: std::slice::from_ref(key),
                    selected: Some(key),
                },
            );
            require(
                matches!(
                    stale,
                    Err(CampaignError::SceneImage(SceneImageError::Cache(
                        CacheError::StaleRevision
                    )))
                ),
                "stale revision changed component",
            )?;
            let mut changed = key.clone();
            changed.bytes.sha256 = [0; 32];
            let conflict = fixture.surface.update_with_scene_assets(
                &campaign_view(),
                CampaignSceneAssets {
                    scope: fixture.scope,
                    revision,
                    references: std::slice::from_ref(&changed),
                    selected: Some(&changed),
                },
            );
            require(
                matches!(
                    conflict,
                    Err(CampaignError::SceneImage(SceneImageError::Cache(
                        CacheError::ConflictingReference
                    )))
                ),
                "immutable key changed component",
            )?;
            require(
                canvas(fixture.surface.root())?.width() == 1,
                "refusal scrubbed valid current surface",
            )?;
            Ok(())
        })
    }
    #[wasm_bindgen]
    pub fn omit_selected() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut owned = slot.borrow_mut();
            let fixture = owned.as_mut().ok_or_else(|| error("fixture missing"))?;
            let retained = fixture
                .surface
                .root()
                .query_selector("canvas.scene-art")?
                .map(|node| node.dyn_into::<HtmlCanvasElement>())
                .transpose()
                .map_err(error)?;
            let revision = next_revision(fixture)?;
            fixture
                .surface
                .update_with_scene_assets(
                    &campaign_view(),
                    CampaignSceneAssets {
                        scope: fixture.scope,
                        revision,
                        references: &[],
                        selected: None,
                    },
                )
                .map_err(error)?;
            fixture.selected = None;
            fixture.expected = None;
            if let Some(canvas) = retained {
                require(
                    canvas.width() == 0 && canvas.height() == 0 && canvas.hidden(),
                    "revoked pixels remain",
                )?;
            }
            require(
                fixture
                    .surface
                    .root()
                    .get_attribute("data-scene-image")
                    .as_deref()
                    == Some("fallback"),
                "revocation lost fallback",
            )?;
            Ok(())
        })
    }
    #[wasm_bindgen]
    pub fn race_same_key_epoch(index: u32) -> Result<(), JsValue> {
        select_format(index)?;
        FIXTURE.with(|slot| {
            let mut owned = slot.borrow_mut();
            let fixture = owned.as_mut().ok_or_else(|| error("fixture missing"))?;
            require(
                fixture
                    .surface
                    .root()
                    .get_attribute("data-scene-image")
                    .as_deref()
                    == Some("loading"),
                "race needs actual pending decoder",
            )?;
            let sample = images::IMAGES
                .get(index as usize)
                .ok_or_else(|| error("sample missing"))?;
            let key = fixture
                .selected
                .clone()
                .ok_or_else(|| error("selection missing"))?;
            let epoch = fixture
                .revision
                .epoch()
                .get()
                .checked_add(1)
                .ok_or_else(|| error("epoch exhausted"))?;
            fixture.revision = SessionRevision::new(RecoveryEpoch::new(epoch).map_err(error)?, 0);
            let token = fixture
                .surface
                .update_with_scene_assets(
                    &campaign_view(),
                    CampaignSceneAssets {
                        scope: fixture.scope,
                        revision: fixture.revision,
                        references: std::slice::from_ref(&key),
                        selected: Some(&key),
                    },
                )
                .map_err(error)?
                .ok_or_else(|| error("recovery bytes request missing"))?;
            fixture
                .surface
                .complete_scene_asset(&token, sample.bytes.to_vec())
                .map_err(error)?;
            Ok(())
        })
    }
    #[wasm_bindgen]
    pub fn race_a_b_a() -> Result<(), JsValue> {
        select_format(0)?;
        select_format(3)?;
        select_format(0)
    }
    #[wasm_bindgen]
    pub fn race_owner_replacement() -> Result<(), JsValue> {
        select_format(0)?;
        FIXTURE.with(|slot| {
            let mut owned = slot.borrow_mut();
            let fixture = owned.as_mut().ok_or_else(|| error("fixture missing"))?;
            require(
                fixture
                    .surface
                    .root()
                    .get_attribute("data-scene-image")
                    .as_deref()
                    == Some("loading"),
                "replacement needs actual pending decoder",
            )?;
            for _ in 0..64 {
                let result = fixture.surface.enable_scene_assets(scope(9)?, limits());
                require(
                    matches!(
                        result,
                        Err(CampaignError::SceneImage(SceneImageError::Decode(
                            ImageDecodeError::Resource(ResourceError::PendingCapacity)
                        )))
                    ),
                    "replacement reset cancelled work budget",
                )?;
            }
            require(
                fixture
                    .surface
                    .root()
                    .get_attribute("data-scene-image")
                    .as_deref()
                    == Some("fallback"),
                "replacement did not synchronously scrub",
            )?;
            fixture.selected = None;
            fixture.expected = None;
            Ok(())
        })
    }
    #[wasm_bindgen]
    pub fn replace_after_terminal() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut owned = slot.borrow_mut();
            let fixture = owned.as_mut().ok_or_else(|| error("fixture missing"))?;
            let scope = scope(9)?;
            fixture
                .surface
                .enable_scene_assets(scope, limits())
                .map_err(error)?;
            fixture.scope = scope;
            fixture.revision = SessionRevision::new(RecoveryEpoch::new(1).map_err(error)?, 0);
            Ok(())
        })
    }
    #[wasm_bindgen]
    pub fn refuse_static_metadata(index: u32) -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut owned = slot.borrow_mut();
            let fixture = owned.as_mut().ok_or_else(|| error("fixture missing"))?;
            let (name, bytes, digest) = images::REFUSED
                .get(index as usize)
                .ok_or_else(|| error("refusal missing"))?;
            let key = key(name, bytes, *digest)?;
            let revision = next_revision(fixture)?;
            let token = fixture
                .surface
                .update_with_scene_assets(
                    &campaign_view(),
                    CampaignSceneAssets {
                        scope: fixture.scope,
                        revision,
                        references: std::slice::from_ref(&key),
                        selected: Some(&key),
                    },
                )
                .map_err(error)?
                .ok_or_else(|| error("request missing"))?;
            let result = fixture.surface.complete_scene_asset(&token, bytes.to_vec());
            require(
                matches!(
                    result,
                    Err(CampaignError::SceneImage(SceneImageError::Decode(
                        ImageDecodeError::UnsupportedPng
                    )))
                ),
                "unbounded static format decoded",
            )?;
            require(
                fixture
                    .surface
                    .root()
                    .get_attribute("data-scene-image")
                    .as_deref()
                    == Some("fallback")
                    && fixture.input.is_connected(),
                "format refusal blocked fallback/input",
            )?;
            Ok(())
        })
    }
    #[wasm_bindgen]
    pub fn decode_bad_deflate() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut owned = slot.borrow_mut();
            let fixture = owned.as_mut().ok_or_else(|| error("fixture missing"))?;
            let (name, bytes, digest) = images::BAD_DEFLATE;
            let key = key(name, bytes, digest)?;
            apply(fixture, &key, bytes)?;
            fixture.expected = None;
            Ok(())
        })
    }
    /// The independent runner first observes the real browser promise's fallback.
    #[wasm_bindgen]
    pub fn check_bad_deflate_failure() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut owned = slot.borrow_mut();
            let fixture = owned.as_mut().ok_or_else(|| error("fixture missing"))?;
            require(
                fixture
                    .surface
                    .root()
                    .get_attribute("data-scene-image")
                    .as_deref()
                    == Some("fallback"),
                "actual failure not terminal",
            )?;
            let key = fixture
                .selected
                .clone()
                .ok_or_else(|| error("selection missing"))?;
            let revision = next_revision(fixture)?;
            let result = fixture.surface.update_with_scene_assets(
                &campaign_view(),
                CampaignSceneAssets {
                    scope: fixture.scope,
                    revision,
                    references: std::slice::from_ref(&key),
                    selected: Some(&key),
                },
            );
            require(
                matches!(
                    result,
                    Err(CampaignError::SceneImage(SceneImageError::Decode(
                        ImageDecodeError::BrowserDecode
                    )))
                ),
                "real codec failure was hidden",
            )?;
            require(
                fixture.input.is_connected() && fixture.input.value() == "Keep this valid draft",
                "decode failure blocked input",
            )?;
            Ok(())
        })
    }
    #[wasm_bindgen]
    pub fn refuse_invalid_replacement_limits() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut owned = slot.borrow_mut();
            let fixture = owned.as_mut().ok_or_else(|| error("fixture missing"))?;
            let retained = canvas(fixture.surface.root())?;
            let result = fixture.surface.enable_scene_assets(
                scope(9)?,
                SceneImageLimits {
                    max_width: 0,
                    ..limits()
                },
            );
            require(
                matches!(
                    result,
                    Err(CampaignError::SceneImage(SceneImageError::InvalidLimits))
                ),
                "invalid limits admitted",
            )?;
            require(
                retained.width() == 0 && retained.height() == 0 && retained.hidden(),
                "obsolete scope pixels survived invalid replacement limits",
            )?;
            fixture.selected = None;
            fixture.expected = None;
            Ok(())
        })
    }
    #[wasm_bindgen]
    pub fn shutdown() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut owned = slot.borrow_mut();
            if let Some(fixture) = owned.as_ref() {
                let retained = fixture
                    .surface
                    .root()
                    .query_selector("canvas.scene-art")?
                    .map(|node| node.dyn_into::<HtmlCanvasElement>())
                    .transpose()
                    .map_err(error)?;
                fixture.surface.dispose().map_err(error)?;
                fixture.surface.dispose().map_err(error)?;
                if let Some(canvas) = retained {
                    require(
                        canvas.width() == 0
                            && canvas.height() == 0
                            && canvas.hidden()
                            && !canvas.is_connected(),
                        "disposed pixels retained",
                    )?;
                }
                fixture.input.set_value("");
            }
            *owned = None;
            Ok(())
        })
    }
}
