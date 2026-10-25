//! Finite synthetic PNG delivery through both actual mounted role adapters.
#[cfg(target_arch = "wasm32")]
mod browser {
    use df_assets::AssetManifest;
    use df_client::cache::{CacheKey, CacheLimits, CacheScope, FetchToken};
    use df_display::DisplayExploration;
    use df_player::PlayerExploration;
    use df_types::{
        ClientBindingId, RecoveryEpoch, RevisionLabel, RunId, SessionId, SessionRevision,
    };
    use df_ui::{
        ActionView, CampaignLimits, CampaignSceneAssets, CampaignView, ConceptScene,
        ControlledAction, ExplorationChoice, ExplorationLimits, ExplorationView, SceneImageLimits,
    };
    use std::{cell::RefCell, rc::Rc};
    use wasm_bindgen::{JsCast, JsValue, prelude::wasm_bindgen};
    use web_sys::{Element, Event, HtmlInputElement};

    const OFFERS: [ExplorationChoice<'static>; 1] = [ExplorationChoice {
        id: "fixture-listen",
        label: "Listen",
        detail: "Send the supplied offer",
        disabled_reason: None,
    }];
    fn error(message: impl std::fmt::Display) -> JsValue {
        JsValue::from_str(&message.to_string())
    }
    fn require(condition: bool, message: &str) -> Result<(), JsValue> {
        if condition {
            Ok(())
        } else {
            Err(JsValue::from_str(message))
        }
    }
    fn scope(binding: u8) -> CacheScope {
        CacheScope {
            session: SessionId::from_bytes(&[1; 16]).unwrap(),
            run: RunId::from_bytes(&[2; 16]).unwrap(),
            binding: ClientBindingId::from_bytes(&[binding; 16]).unwrap(),
        }
    }
    fn limits() -> ExplorationLimits {
        ExplorationLimits {
            campaign: CampaignLimits {
                max_members: 1,
                max_objectives: 1,
                max_text_bytes: 1024,
            },
            max_choices: 2,
            max_text_bytes: 1024,
            max_identifier_bytes: 128,
        }
    }
    fn image_limits() -> SceneImageLimits {
        SceneImageLimits {
            cache: CacheLimits {
                max_assets: 2,
                max_pending: 1,
                max_leases: 1,
                max_bytes: 32768,
            },
            max_width: 512,
            max_height: 256,
            max_pixels: 131072,
        }
    }
    fn view(
        binding: ClientBindingId,
        revision: SessionRevision,
        is_display: bool,
        scene: ConceptScene,
    ) -> ExplorationView<'static> {
        ExplorationView {
            binding,
            revision,
            campaign: CampaignView {
                scene,
                chapter: "Synthetic image lifecycle",
                title: "The lantern river",
                description: "A supplied scene image replaces the permitted harbor fallback.",
                location: "The river crossing",
                scene_label: "Verified scene",
                narration: "Lanterns guide the party toward the bridge.",
                connection: "Controlled fixture",
                notice: if is_display {
                    "Separate public projection"
                } else {
                    "Player projection"
                },
                members: &[],
                objectives: &[],
            },
            npc: None,
            heading: "Current supplied offers",
            choices: &OFFERS,
            draft_label: "Your words",
            submit_label: "Send",
            draft_offer_id: Some("fixture-speak"),
            pending: None,
            rejection: None,
        }
    }
    struct Fixture {
        player: PlayerExploration,
        display: DisplayExploration,
        revision: SessionRevision,
        key: CacheKey,
        scene: ConceptScene,
        stale_concepts: Vec<Element>,
        player_fetch: Option<FetchToken>,
        display_fetch: Option<FetchToken>,
        status: Element,
        controls: Vec<ControlledAction>,
        inputs: Rc<RefCell<usize>>,
    }
    thread_local! { static FIXTURE: RefCell<Option<Fixture>> = const { RefCell::new(None) }; }
    fn status(fixture: &Fixture, result: &str) -> Result<(), JsValue> {
        fixture.status.set_text_content(Some(result));
        fixture.status.set_attribute("data-result", result)
    }
    fn image(root: &Element) -> Result<Element, JsValue> {
        root.query_selector(".scene-art")?
            .ok_or_else(|| JsValue::from_str("mounted image missing"))
    }
    fn update(fixture: &mut Fixture, is_selected: bool) -> Result<(), JsValue> {
        let revision = fixture
            .revision
            .next_sequence()
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))?;
        let keys = if is_selected {
            std::slice::from_ref(&fixture.key)
        } else {
            &[]
        };
        let selected_asset = if is_selected {
            Some(&fixture.key)
        } else {
            None
        };
        fixture.player_fetch = fixture
            .player
            .update_with_scene_assets(
                &view(scope(3).binding, revision, false, fixture.scene),
                CampaignSceneAssets {
                    scope: scope(3),
                    revision,
                    references: keys,
                    selected: selected_asset,
                },
            )
            .map_err(error)?;
        fixture.display_fetch = fixture
            .display
            .update_with_scene_assets(
                &view(scope(4).binding, revision, true, fixture.scene),
                CampaignSceneAssets {
                    scope: scope(4),
                    revision,
                    references: keys,
                    selected: selected_asset,
                },
            )
            .map_err(error)?;
        fixture.revision = revision;
        Ok(())
    }
    fn verify_offer(fixture: &Fixture) -> Result<(), JsValue> {
        let before = *fixture.inputs.borrow();
        for root in [fixture.player.root(), fixture.display.root()] {
            let button = root
                .query_selector(".exploration-choice button")?
                .ok_or_else(|| JsValue::from_str("offer missing"))?
                .dyn_into::<web_sys::HtmlButtonElement>()?;
            button.click();
        }
        require(
            *fixture.inputs.borrow() == before + 2,
            "image lifecycle blocked offered role input",
        )
    }
    #[wasm_bindgen]
    pub fn verify_scene_image_visible() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let slot = slot.borrow();
            let f = slot
                .as_ref()
                .ok_or_else(|| JsValue::from_str("fixture missing"))?;
            for root in [f.player.root(), f.display.root()] {
                let image = image(root)?.dyn_into::<web_sys::HtmlImageElement>()?;
                require(
                    image.src().starts_with("blob:")
                        && image.complete()
                        && image.natural_width() == 320
                        && image.natural_height() == 160,
                    "actual PNG has not decoded and displayed",
                )?;
            }
            verify_offer(f)?;
            status(f, "both-images-visible")
        })
    }
    #[wasm_bindgen]
    pub fn request_scene_image() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let f = slot
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture missing"))?;
            let root = f.player.root().clone();
            let input = root
                .query_selector("input")?
                .ok_or_else(|| JsValue::from_str("draft missing"))?
                .dyn_into::<HtmlInputElement>()?;
            input.set_value("Keep this editable draft");
            input.dispatch_event(&Event::new("input")?)?;
            input.focus()?;
            f.key = png_key();
            update(f, true)?;
            require(
                f.player.root().is_same_node(Some(&root)),
                "image request remounted player",
            )?;
            require(
                input.value() == "Keep this editable draft",
                "image request lost draft",
            )?;
            status(f, "requested")
        })
    }
    #[wasm_bindgen]
    pub fn complete_scene_image() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let f = slot
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture missing"))?;
            let player = f
                .player_fetch
                .take()
                .ok_or_else(|| JsValue::from_str("player fetch absent"))?;
            let display = f
                .display_fetch
                .take()
                .ok_or_else(|| JsValue::from_str("display fetch absent"))?;
            f.player
                .complete_scene_asset(&player, PNG.to_vec())
                .map_err(error)?;
            f.display
                .complete_scene_asset(&display, PNG.to_vec())
                .map_err(error)?;
            status(f, "decode-pending")
        })
    }
    #[wasm_bindgen]
    pub fn retain_scene_image() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let f = slot
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture missing"))?;
            let player = image(f.player.root())?;
            let display = image(f.display.root())?;
            require(
                player
                    .get_attribute("src")
                    .is_some_and(|src| src.starts_with("blob:")),
                "player decode not complete",
            )?;
            update(f, true)?;
            require(
                f.player_fetch.is_none() && f.display_fetch.is_none(),
                "unchanged image refetched",
            )?;
            require(
                image(f.player.root())?.is_same_node(Some(&player))
                    && image(f.display.root())?.is_same_node(Some(&display)),
                "unchanged image recreated",
            )?;
            status(f, "retained")
        })
    }
    #[wasm_bindgen]
    pub fn revoke_scene_image() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let f = slot
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture missing"))?;
            let old_player = image(f.player.root())?;
            let old_display = image(f.display.root())?;
            let late_player = f.player_fetch.take();
            let late_display = f.display_fetch.take();
            update(f, false)?;
            for root in [f.player.root(), f.display.root()] {
                require(
                    image(root)?.get_attribute("src").as_deref() == Some(f.scene.asset_path()),
                    "revoke retained image",
                )?;
            }
            for node in [old_player, old_display] {
                node.dispatch_event(&Event::new("load")?)?;
                node.dispatch_event(&Event::new("error")?)?;
            }
            if let Some(token) = late_player {
                require(
                    f.player.complete_scene_asset(&token, PNG.to_vec()).is_err(),
                    "late player bytes admitted",
                )?;
            }
            if let Some(token) = late_display {
                require(
                    f.display
                        .complete_scene_asset(&token, PNG.to_vec())
                        .is_err(),
                    "late display bytes admitted",
                )?;
            }
            verify_offer(f)?;
            status(f, "revoked")
        })
    }
    #[wasm_bindgen]
    pub fn corrupt_scene_image() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let f = slot
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture missing"))?;
            update(f, false)?;
            f.key = png_key();
            update(f, true)?;
            let player = f
                .player_fetch
                .take()
                .ok_or_else(|| JsValue::from_str("player fetch absent"))?;
            let display = f
                .display_fetch
                .take()
                .ok_or_else(|| JsValue::from_str("display fetch absent"))?;
            let mut corrupt = PNG.to_vec();
            if let Some(byte) = corrupt.last_mut() {
                *byte ^= 1;
            }
            require(
                f.player
                    .complete_scene_asset(&player, corrupt.clone())
                    .is_err(),
                "corrupt player bytes admitted",
            )?;
            require(
                f.display.complete_scene_asset(&display, corrupt).is_err(),
                "corrupt display bytes admitted",
            )?;
            verify_offer(f)?;
            status(f, "corruption-refused")
        })
    }
    #[wasm_bindgen]
    pub fn legacy_scene_image() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let f = slot
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture missing"))?;
            let player = f.player_fetch.take();
            let display = f.display_fetch.take();
            let revision = f
                .revision
                .next_sequence()
                .map_err(|e| JsValue::from_str(&format!("{e:?}")))?;
            f.player
                .update(&view(scope(3).binding, revision, false, f.scene))
                .map_err(error)?;
            f.display
                .update(&view(scope(4).binding, revision, true, f.scene))
                .map_err(error)?;
            f.revision = revision;
            if let Some(token) = player {
                require(
                    f.player.complete_scene_asset(&token, PNG.to_vec()).is_err(),
                    "legacy update admitted late bytes",
                )?;
            }
            if let Some(token) = display {
                require(
                    f.display
                        .complete_scene_asset(&token, PNG.to_vec())
                        .is_err(),
                    "legacy display admitted late bytes",
                )?;
            }
            status(f, "legacy-fallback")
        })
    }
    #[wasm_bindgen]
    pub fn invalid_scene_image_snapshot() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let f = slot
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture missing"))?;
            let next = f
                .revision
                .next_sequence()
                .map_err(|e| JsValue::from_str(&format!("{e:?}")))?;
            let before_player = image(f.player.root())?;
            let player_source = before_player.get_attribute("src");
            let before_display = image(f.display.root())?;
            let display_source = before_display.get_attribute("src");
            let result = f.player.update_with_scene_assets(
                &view(scope(3).binding, next, false, f.scene),
                CampaignSceneAssets {
                    scope: scope(3),
                    revision: next,
                    references: &[],
                    selected: Some(&f.key),
                },
            );
            let public_result = f.display.update_with_scene_assets(
                &view(scope(4).binding, next, true, f.scene),
                CampaignSceneAssets {
                    scope: scope(4),
                    revision: next,
                    references: &[],
                    selected: Some(&f.key),
                },
            );
            require(
                result.is_err()
                    && public_result.is_err()
                    && f.player.revision() == Some(f.revision)
                    && f.display.revision() == Some(f.revision),
                "invalid selection changed watermark",
            )?;
            require(
                image(f.player.root())?.is_same_node(Some(&before_player))
                    && before_player.get_attribute("src") == player_source
                    && image(f.display.root())?.is_same_node(Some(&before_display))
                    && before_display.get_attribute("src") == display_source,
                "invalid selection changed image",
            )?;
            let wrong_revision = f.player.update_with_scene_assets(
                &view(scope(3).binding, next, false, f.scene),
                CampaignSceneAssets {
                    scope: scope(3),
                    revision: f.revision,
                    references: std::slice::from_ref(&f.key),
                    selected: Some(&f.key),
                },
            );
            let wrong_scope = f.display.update_with_scene_assets(
                &view(scope(4).binding, next, true, f.scene),
                CampaignSceneAssets {
                    scope: scope(3),
                    revision: next,
                    references: std::slice::from_ref(&f.key),
                    selected: Some(&f.key),
                },
            );
            require(
                wrong_revision.is_err()
                    && wrong_scope.is_err()
                    && f.player.revision() == Some(f.revision)
                    && f.display.revision() == Some(f.revision),
                "invalid scope or revision changed watermark",
            )?;
            status(f, "invalid-preserved")
        })
    }
    #[wasm_bindgen]
    pub fn recover_scene_image() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let f = slot
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture missing"))?;
            let old_player = f.player_fetch.take();
            let old_display = f.display_fetch.take();
            let epoch = f
                .revision
                .epoch()
                .get()
                .checked_add(1)
                .ok_or_else(|| JsValue::from_str("epoch exhausted"))?;
            let revision = SessionRevision::new(
                RecoveryEpoch::new(epoch).map_err(|e| JsValue::from_str(&format!("{e:?}")))?,
                0,
            );
            f.player_fetch = f
                .player
                .update_with_scene_assets(
                    &view(scope(3).binding, revision, false, f.scene),
                    CampaignSceneAssets {
                        scope: scope(3),
                        revision,
                        references: std::slice::from_ref(&f.key),
                        selected: Some(&f.key),
                    },
                )
                .map_err(error)?;
            f.display_fetch = f
                .display
                .update_with_scene_assets(
                    &view(scope(4).binding, revision, true, f.scene),
                    CampaignSceneAssets {
                        scope: scope(4),
                        revision,
                        references: std::slice::from_ref(&f.key),
                        selected: Some(&f.key),
                    },
                )
                .map_err(error)?;
            f.revision = revision;
            if let Some(token) = old_player {
                require(
                    f.player.complete_scene_asset(&token, PNG.to_vec()).is_err(),
                    "old epoch player completion admitted",
                )?;
            }
            if let Some(token) = old_display {
                require(
                    f.display
                        .complete_scene_asset(&token, PNG.to_vec())
                        .is_err(),
                    "old epoch display completion admitted",
                )?;
            }
            status(f, "recovery-fenced")
        })
    }
    #[wasm_bindgen]
    pub fn dispose_scene_image() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let f = slot
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture missing"))?;
            let player_image = image(f.player.root())?;
            let display_image = image(f.display.root())?;
            f.player.revoke().map_err(error)?;
            f.display.revoke().map_err(error)?;
            for node in [player_image, display_image] {
                require(
                    node.get_attribute("src").is_none(),
                    "disposal retained generated source",
                )?;
                node.dispatch_event(&Event::new("error")?)?;
            }
            status(f, "disposed")
        })
    }
    fn fallback(f: &Fixture) -> Result<(), JsValue> {
        for root in [f.player.root(), f.display.root()] {
            require(
                image(root)?.get_attribute("src").as_deref() == Some(f.scene.asset_path()),
                "permitted fallback missing",
            )?;
        }
        Ok(())
    }
    fn take_fetches(f: &mut Fixture) -> Result<(FetchToken, FetchToken), JsValue> {
        let player = f
            .player_fetch
            .take()
            .ok_or_else(|| JsValue::from_str("player fetch absent"))?;
        let display = f
            .display_fetch
            .take()
            .ok_or_else(|| JsValue::from_str("display fetch absent"))?;
        Ok((player, display))
    }
    fn rejected_payload(
        f: &mut Fixture,
        payload: Vec<u8>,
        key: CacheKey,
        result: &str,
    ) -> Result<(), JsValue> {
        update(f, false)?;
        f.key = key;
        update(f, true)?;
        let (player, display) = take_fetches(f)?;
        require(
            f.player
                .complete_scene_asset(&player, payload.clone())
                .is_err(),
            "invalid player payload admitted",
        )?;
        require(
            f.display.complete_scene_asset(&display, payload).is_err(),
            "invalid display payload admitted",
        )?;
        fallback(f)?;
        verify_offer(f)?;
        status(f, result)
    }
    #[wasm_bindgen]
    pub fn truncated_scene_image() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let f = slot
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture missing"))?;
            rejected_payload(
                f,
                PNG.iter().copied().take(PNG.len() - 1).collect(),
                png_key(),
                "length-refused",
            )
        })
    }
    #[wasm_bindgen]
    pub fn oversized_scene_image() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let f = slot
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture missing"))?;
            rejected_payload(f, vec![0; 32769], png_key(), "encoded-bound-refused")
        })
    }
    #[wasm_bindgen]
    pub fn malformed_scene_image() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let f = slot
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture missing"))?;
            let mut payload = PNG.to_vec();
            *payload
                .first_mut()
                .ok_or_else(|| JsValue::from_str("PNG fixture empty"))? = 0;
            let key = CacheKey {
                version: RevisionLabel::new(Some("synthetic-invalid-header-v1")).unwrap(),
                bytes: AssetManifest {
                    byte_len: payload.len() as u64,
                    sha256: [
                        23, 173, 66, 118, 30, 254, 50, 104, 236, 213, 23, 183, 148, 105, 233, 51,
                        231, 141, 242, 125, 118, 59, 139, 255, 129, 82, 220, 82, 167, 44, 8, 100,
                    ],
                },
            };
            rejected_payload(f, payload, key, "header-refused")
        })
    }
    #[wasm_bindgen]
    pub fn dimension_scene_image() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let f = slot
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture missing"))?;
            let mut payload = PNG.to_vec();
            payload
                .get_mut(16..20)
                .ok_or_else(|| JsValue::from_str("PNG IHDR missing"))?
                .copy_from_slice(&513u32.to_be_bytes());
            let key = CacheKey {
                version: RevisionLabel::new(Some("synthetic-oversized-dimensions-v1")).unwrap(),
                bytes: AssetManifest {
                    byte_len: payload.len() as u64,
                    sha256: [
                        172, 72, 31, 236, 36, 125, 67, 153, 21, 116, 137, 207, 137, 108, 228, 11,
                        21, 204, 21, 213, 54, 100, 225, 140, 16, 33, 235, 61, 188, 15, 145, 240,
                    ],
                },
            };
            rejected_payload(f, payload, key, "dimensions-refused")
        })
    }
    #[wasm_bindgen]
    pub fn undecodable_scene_image() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let f = slot
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture missing"))?;
            update(f, false)?;
            let mut payload = PNG
                .get(..33)
                .ok_or_else(|| JsValue::from_str("PNG IHDR missing"))?
                .to_vec();
            payload.extend_from_slice(
                PNG.get(PNG.len() - 12..)
                    .ok_or_else(|| JsValue::from_str("PNG IEND missing"))?,
            );
            f.key = CacheKey {
                version: RevisionLabel::new(Some("synthetic-no-idat-v1")).unwrap(),
                bytes: AssetManifest {
                    byte_len: payload.len() as u64,
                    sha256: [
                        55, 23, 74, 64, 222, 226, 226, 116, 77, 174, 93, 193, 68, 6, 111, 197, 204,
                        26, 48, 1, 244, 240, 92, 191, 81, 222, 188, 141, 190, 101, 117, 136,
                    ],
                },
            };
            update(f, true)?;
            let (player, display) = take_fetches(f)?;
            f.player
                .complete_scene_asset(&player, payload.clone())
                .map_err(error)?;
            f.display
                .complete_scene_asset(&display, payload)
                .map_err(error)?;
            status(f, "decoder-error-pending")
        })
    }
    #[wasm_bindgen]
    pub fn verify_scene_image_fallback() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let slot = slot.borrow();
            let f = slot
                .as_ref()
                .ok_or_else(|| JsValue::from_str("fixture missing"))?;
            fallback(f)?;
            for root in [f.player.root(), f.display.root()] {
                require(
                    root.get_attribute("data-scene-image").as_deref() == Some("fallback"),
                    "image retirement not finished",
                )?;
            }
            verify_offer(f)?;
            status(f, "fallback-input-usable")
        })
    }
    #[wasm_bindgen]
    pub fn replace_scene_image() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let f = slot
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture missing"))?;
            let old_player = image(f.player.root())?;
            let old_display = image(f.display.root())?;
            let player_source = old_player.get_attribute("src");
            let display_source = old_display.get_attribute("src");
            let old_player_fetch = f.player_fetch.take();
            let old_display_fetch = f.display_fetch.take();
            f.key = png_key();
            let version = format!(
                "synthetic-river-{}-{}",
                f.revision.epoch().get(),
                f.revision.sequence()
            );
            f.key.version = RevisionLabel::new(Some(&version))
                .map_err(|e| JsValue::from_str(&format!("{e:?}")))?;
            update(f, true)?;
            fallback(f)?;
            let (player, display) = take_fetches(f)?;
            f.player
                .complete_scene_asset(&player, PNG.to_vec())
                .map_err(error)?;
            f.display
                .complete_scene_asset(&display, PNG.to_vec())
                .map_err(error)?;
            for (node, source) in [(old_player, player_source), (old_display, display_source)] {
                if source.is_some_and(|source| source.starts_with("blob:")) {
                    require(
                        node.get_attribute("src").is_none(),
                        "old generated source retained",
                    )?;
                }
                node.dispatch_event(&Event::new("load")?)?;
                node.dispatch_event(&Event::new("error")?)?;
            }
            if let Some(token) = old_player_fetch {
                require(
                    f.player.complete_scene_asset(&token, PNG.to_vec()).is_err(),
                    "replaced player completion admitted",
                )?;
            }
            if let Some(token) = old_display_fetch {
                require(
                    f.display
                        .complete_scene_asset(&token, PNG.to_vec())
                        .is_err(),
                    "replaced display completion admitted",
                )?;
            }
            verify_offer(f)?;
            status(f, "replacement-decode-pending")
        })
    }
    #[wasm_bindgen]
    pub fn reset_scene_image_scope() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let f = slot
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture missing"))?;
            let old_player = f.player_fetch.take();
            let old_display = f.display_fetch.take();
            let replacement_run = RunId::from_bytes(&[9; 16]).unwrap();
            f.player
                .enable_scene_assets(
                    CacheScope {
                        run: replacement_run,
                        ..scope(3)
                    },
                    image_limits(),
                )
                .map_err(error)?;
            f.display
                .enable_scene_assets(
                    CacheScope {
                        run: replacement_run,
                        ..scope(4)
                    },
                    image_limits(),
                )
                .map_err(error)?;
            fallback(f)?;
            if let Some(token) = old_player {
                require(
                    f.player.complete_scene_asset(&token, PNG.to_vec()).is_err(),
                    "re-enabled owner admitted late player bytes",
                )?;
            }
            if let Some(token) = old_display {
                require(
                    f.display
                        .complete_scene_asset(&token, PNG.to_vec())
                        .is_err(),
                    "re-enabled owner admitted late display bytes",
                )?;
            }
            f.player
                .enable_scene_assets(scope(3), image_limits())
                .map_err(error)?;
            f.display
                .enable_scene_assets(scope(4), image_limits())
                .map_err(error)?;
            f.key = png_key();
            update(f, true)?;
            status(f, "owner-replaced-requested")
        })
    }
    #[wasm_bindgen]
    pub fn switch_concept_scene() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let f = slot
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture missing"))?;
            let old_player = image(f.player.root())?;
            let old_display = image(f.display.root())?;
            let root = f.player.root().clone();
            let input = root
                .query_selector("input")?
                .ok_or_else(|| JsValue::from_str("draft missing"))?;
            f.stale_concepts = vec![old_player.clone(), old_display.clone()];
            f.scene = if f.scene == ConceptScene::Harbor {
                ConceptScene::Tavern
            } else {
                ConceptScene::Harbor
            };
            update(f, false)?;
            require(
                !image(f.player.root())?.is_same_node(Some(&old_player))
                    && !image(f.display.root())?.is_same_node(Some(&old_display)),
                "new concept reused old image request",
            )?;
            require(
                f.player.root().is_same_node(Some(&root))
                    && root
                        .query_selector("input")?
                        .is_some_and(|node| node.is_same_node(Some(&input))),
                "concept change remounted draft",
            )?;
            fallback(f)?;
            verify_offer(f)?;
            status(f, "concept-changed")
        })
    }
    #[wasm_bindgen]
    pub fn retain_image_change_concept() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let f = slot
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture missing"))?;
            let player = image(f.player.root())?;
            let display = image(f.display.root())?;
            require(
                player
                    .get_attribute("src")
                    .is_some_and(|src| src.starts_with("blob:"))
                    && display
                        .get_attribute("src")
                        .is_some_and(|src| src.starts_with("blob:")),
                "both generated images must be displayed first",
            )?;
            f.scene = if f.scene == ConceptScene::Harbor {
                ConceptScene::Tavern
            } else {
                ConceptScene::Harbor
            };
            update(f, true)?;
            require(
                f.player_fetch.is_none()
                    && f.display_fetch.is_none()
                    && image(f.player.root())?.is_same_node(Some(&player))
                    && image(f.display.root())?.is_same_node(Some(&display)),
                "concept change replaced retained generated key",
            )?;
            verify_offer(f)?;
            status(f, "retained-image-concept-changed")
        })
    }
    #[wasm_bindgen]
    pub fn stale_concept_error() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let slot = slot.borrow();
            let f = slot
                .as_ref()
                .ok_or_else(|| JsValue::from_str("fixture missing"))?;
            let player_class = f.player.root().class_name();
            let display_class = f.display.root().class_name();
            let player_source = image(f.player.root())?.get_attribute("src");
            let display_source = image(f.display.root())?.get_attribute("src");
            require(
                !f.stale_concepts.is_empty(),
                "no prior concept request retained",
            )?;
            for node in &f.stale_concepts {
                node.dispatch_event(&Event::new("error")?)?;
                node.dispatch_event(&Event::new("load")?)?;
            }
            require(
                f.player.root().class_name() == player_class
                    && f.display.root().class_name() == display_class
                    && image(f.player.root())?.get_attribute("src") == player_source
                    && image(f.display.root())?.get_attribute("src") == display_source,
                "old concept event altered current artwork",
            )?;
            verify_offer(f)?;
            status(f, "old-concept-events-fenced")
        })
    }
    #[wasm_bindgen]
    pub fn verify_concept_failure() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let slot = slot.borrow();
            let f = slot
                .as_ref()
                .ok_or_else(|| JsValue::from_str("fixture missing"))?;
            fallback(f)?;
            for root in [f.player.root(), f.display.root()] {
                let image = image(root)?.dyn_into::<web_sys::HtmlImageElement>()?;
                require(
                    image.complete()
                        && image.natural_width() == 0
                        && root
                            .class_name()
                            .split_whitespace()
                            .any(|name| name == "exploration-art-failed"),
                    "intentional concept404 not observed",
                )?;
            }
            verify_offer(f)?;
            status(f, "concept404-input-usable")
        })
    }
    #[wasm_bindgen]
    pub fn verify_concept_recovery() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let slot = slot.borrow();
            let f = slot
                .as_ref()
                .ok_or_else(|| JsValue::from_str("fixture missing"))?;
            fallback(f)?;
            for root in [f.player.root(), f.display.root()] {
                let image = image(root)?.dyn_into::<web_sys::HtmlImageElement>()?;
                require(
                    image.complete()
                        && image.natural_width() > 0
                        && !root
                            .class_name()
                            .split_whitespace()
                            .any(|name| name == "exploration-art-failed"),
                    "new concept resource failed recovery",
                )?;
            }
            verify_offer(f)?;
            status(f, "concept-resource-recovered")
        })
    }
    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), JsValue> {
        let document = web_sys::window()
            .ok_or_else(|| JsValue::from_str("window missing"))?
            .document()
            .ok_or_else(|| JsValue::from_str("document missing"))?;
        let body = document
            .body()
            .ok_or_else(|| JsValue::from_str("body missing"))?;
        let notice = document.create_element("p")?;
        notice.set_text_content(Some("Synthetic PNG bytes through the actual player/display mounts; no provider or RPC authority is exercised."));
        body.append_child(&notice)?;
        let inputs = Rc::new(RefCell::new(0usize));
        let player_inputs = inputs.clone();
        let display_inputs = inputs.clone();
        let player_parent = document.create_element("section")?;
        player_parent.set_attribute("data-client", "player")?;
        body.append_child(&player_parent)?;
        let display_parent = document.create_element("section")?;
        display_parent.set_attribute("data-client", "shared-display")?;
        body.append_child(&display_parent)?;
        let revision = SessionRevision::new(
            RecoveryEpoch::new(1).map_err(|e| JsValue::from_str(&format!("{e:?}")))?,
            0,
        );
        let player = PlayerExploration::mount(
            &document,
            &player_parent,
            "image-player-draft",
            &view(scope(3).binding, revision, false, ConceptScene::Harbor),
            limits(),
            move |_| *player_inputs.borrow_mut() += 1,
        )
        .map_err(error)?;
        let display = DisplayExploration::mount(
            &document,
            &display_parent,
            "image-display-draft",
            &view(scope(4).binding, revision, true, ConceptScene::Harbor),
            limits(),
            move |_| *display_inputs.borrow_mut() += 1,
        )
        .map_err(error)?;
        player
            .enable_scene_assets(scope(3), image_limits())
            .map_err(error)?;
        display
            .enable_scene_assets(scope(4), image_limits())
            .map_err(error)?;
        let status_node = document.create_element("output")?;
        status_node.set_id("scene-image-status");
        body.append_child(&status_node)?;
        let controls = document.create_element("nav")?;
        controls.set_attribute("aria-label", "Scene image fixture controls")?;
        body.insert_before(&controls, Some(&player_parent))?;
        let mut fixture = Fixture {
            player,
            display,
            revision,
            key: png_key(),
            scene: ConceptScene::Harbor,
            stale_concepts: Vec::new(),
            player_fetch: None,
            display_fetch: None,
            status: status_node,
            controls: Vec::new(),
            inputs,
        };
        for (id, text, operation) in [
            (
                "request-image",
                "Request supplied PNG",
                request_scene_image as fn() -> Result<(), JsValue>,
            ),
            (
                "complete-image",
                "Deliver complete PNG",
                complete_scene_image,
            ),
            (
                "verify-image",
                "Verify both displayed PNGs",
                verify_scene_image_visible,
            ),
            ("retain-image", "Retain unchanged image", retain_scene_image),
            ("revoke-image", "Revoke image", revoke_scene_image),
            (
                "corrupt-image",
                "Deliver corrupt bytes",
                corrupt_scene_image,
            ),
            (
                "truncated-image",
                "Deliver truncated bytes",
                truncated_scene_image,
            ),
            (
                "oversized-image",
                "Exceed encoded byte bound",
                oversized_scene_image,
            ),
            (
                "header-image",
                "Reject malformed PNG header",
                malformed_scene_image,
            ),
            (
                "dimensions-image",
                "Reject oversized dimensions",
                dimension_scene_image,
            ),
            (
                "decode-image",
                "Deliver verified undecodable PNG",
                undecodable_scene_image,
            ),
            (
                "verify-fallback",
                "Verify fallback and input",
                verify_scene_image_fallback,
            ),
            (
                "replace-image",
                "Replace and fence old image",
                replace_scene_image,
            ),
            (
                "reset-scope-image",
                "Replace private image owner",
                reset_scene_image_scope,
            ),
            (
                "invalid-image",
                "Reject invalid snapshot",
                invalid_scene_image_snapshot,
            ),
            (
                "recover-image",
                "Recover image ownership",
                recover_scene_image,
            ),
            (
                "legacy-image",
                "Return to legacy fallback",
                legacy_scene_image,
            ),
            (
                "switch-concept",
                "Change concept resource",
                switch_concept_scene,
            ),
            (
                "retained-concept",
                "Change detached fallback behind retained PNG",
                retain_image_change_concept,
            ),
            (
                "stale-concept",
                "Fence delayed concept events",
                stale_concept_error,
            ),
            (
                "concept-failure",
                "Verify intentional concept404",
                verify_concept_failure,
            ),
            (
                "concept-recovery",
                "Verify concept recovery",
                verify_concept_recovery,
            ),
            ("dispose-image", "Dispose both mounts", dispose_scene_image),
        ] {
            let control = ControlledAction::create(
                &document,
                ActionView {
                    label: text,
                    enabled: true,
                    pending: false,
                },
            )
            .map_err(error)?;
            control.element().set_id(id);
            control
                .on_activate(move || {
                    if let Err(e) = operation() {
                        FIXTURE.with(|slot| {
                            if let Some(f) = slot.borrow().as_ref() {
                                f.status
                                    .set_text_content(Some(&format!("fixture failure: {e:?}")));
                            }
                        });
                    }
                })
                .map_err(error)?;
            controls.append_child(control.element())?;
            fixture.controls.push(control);
        }
        status(&fixture, "ready")?;
        FIXTURE.with(|slot| *slot.borrow_mut() = Some(fixture));
        Ok(())
    }
    // The exact encoded PNG and its SHA-256 are appended below; no browser URL enters props.
    const PNG: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 1, 64, 0, 0, 0, 160, 8,
        6, 0, 0, 0, 125, 163, 181, 156, 0, 0, 5, 175, 73, 68, 65, 84, 120, 218, 237, 218, 201, 118,
        80, 69, 16, 128, 225, 251, 20, 110, 61, 71, 209, 64, 32, 36, 16, 18, 102, 112, 96, 6, 231,
        121, 214, 149, 207, 224, 155, 249, 70, 62, 70, 219, 199, 5, 27, 84, 34, 118, 229, 86, 223,
        250, 22, 223, 61, 189, 34, 93, 73, 245, 191, 98, 121, 227, 226, 189, 6, 80, 209, 226, 151,
        0, 8, 32, 64, 185, 0, 238, 223, 111, 0, 21, 45, 111, 246, 15, 64, 69, 2, 8, 20, 14, 224,
        193, 131, 6, 80, 145, 0, 2, 117, 3, 120, 166, 127, 0, 42, 90, 206, 92, 122, 216, 0, 42, 18,
        64, 64, 0, 1, 202, 5, 240, 173, 203, 143, 26, 64, 69, 2, 8, 8, 32, 64, 189, 0, 30, 62, 110,
        0, 21, 45, 111, 247, 15, 64, 69, 2, 8, 20, 14, 224, 149, 39, 13, 160, 34, 1, 4, 234, 6,
        112, 167, 127, 0, 42, 90, 118, 142, 158, 54, 128, 138, 4, 16, 16, 64, 128, 114, 1, 60, 123,
        252, 172, 1, 84, 36, 128, 128, 0, 2, 212, 11, 224, 213, 15, 26, 64, 69, 203, 185, 254, 1,
        168, 72, 0, 129, 194, 1, 188, 246, 97, 3, 168, 72, 0, 129, 186, 1, 220, 237, 31, 128, 138,
        150, 221, 235, 31, 53, 128, 138, 4, 16, 16, 64, 128, 114, 1, 60, 127, 227, 227, 6, 80, 145,
        0, 2, 2, 8, 80, 47, 128, 55, 63, 105, 0, 21, 45, 23, 250, 7, 160, 34, 1, 4, 10, 7, 240,
        214, 167, 13, 160, 34, 1, 4, 234, 6, 112, 175, 127, 0, 42, 90, 246, 110, 127, 214, 0, 42,
        18, 64, 64, 0, 71, 219, 121, 253, 92, 90, 254, 240, 192, 95, 1, 188, 120, 231, 243, 22, 33,
        115, 0, 163, 102, 6, 230, 34, 128, 128, 0, 10, 32, 80, 47, 128, 119, 191, 104, 17, 82, 7,
        48, 104, 102, 96, 46, 203, 126, 255, 68, 200, 28, 192, 168, 153, 129, 185, 8, 32, 80, 56,
        128, 239, 124, 217, 34, 164, 14, 96, 208, 204, 192, 92, 4, 16, 168, 27, 192, 131, 254, 137,
        144, 57, 128, 81, 51, 3, 115, 89, 14, 222, 253, 170, 69, 72, 29, 192, 160, 153, 161, 146,
        45, 188, 97, 1, 4, 4, 80, 0, 129, 114, 1, 188, 244, 222, 215, 45, 66, 230, 0, 70, 205, 12,
        149, 108, 225, 13, 11, 32, 32, 128, 2, 8, 212, 11, 224, 251, 223, 180, 8, 169, 3, 24, 52,
        51, 84, 178, 133, 55, 188, 92, 238, 159, 8, 153, 3, 24, 53, 243, 73, 253, 241, 251, 119,
        47, 88, 251, 78, 238, 236, 206, 51, 189, 241, 81, 51, 8, 96, 130, 5, 207, 190, 232, 238,
        236, 206, 219, 13, 224, 189, 111, 91, 132, 212, 1, 12, 154, 249, 101, 254, 109, 193, 159,
        47, 250, 74, 119, 115, 103, 119, 158, 233, 141, 143, 154, 65, 0, 45, 185, 59, 187, 115,
        221, 0, 30, 246, 79, 132, 204, 1, 140, 154, 249, 101, 78, 178, 228, 107, 221, 205, 157,
        221, 121, 166, 55, 62, 106, 134, 229, 240, 126, 255, 197, 6, 72, 29, 192, 160, 153, 95,
        230, 68, 75, 126, 63, 23, 119, 118, 231, 140, 111, 124, 212, 12, 2, 88, 105, 201, 159, 253,
        248, 159, 157, 232, 206, 175, 240, 239, 70, 26, 114, 103, 1, 20, 64, 1, 60, 221, 69, 15,
        255, 249, 1, 65, 201, 22, 191, 97, 119, 174, 182, 27, 85, 3, 120, 229, 193, 247, 45, 66,
        230, 0, 70, 205, 124, 82, 127, 183, 224, 209, 63, 51, 34, 40, 89, 227, 55, 234, 206, 85,
        118, 99, 198, 55, 62, 106, 6, 1, 44, 34, 123, 172, 50, 170, 184, 39, 2, 40, 128, 2, 136, 0,
        150, 9, 224, 195, 31, 90, 132, 212, 1, 12, 154, 57, 43, 49, 251, 31, 17, 44, 182, 43, 179,
        188, 241, 81, 51, 44, 71, 253, 19, 33, 115, 0, 163, 102, 206, 74, 200, 94, 93, 181, 93,
        153, 229, 141, 143, 154, 65, 0, 5, 16, 1, 44, 28, 192, 71, 253, 143, 28, 32, 117, 0, 7,
        207, 250, 219, 175, 143, 33, 141, 168, 55, 157, 233, 141, 143, 154, 65, 0, 5, 16, 1, 172,
        27, 192, 227, 254, 137, 144, 57, 128, 163, 103, 245, 232, 200, 36, 234, 77, 103, 122, 227,
        163, 102, 88, 142, 31, 255, 212, 34, 164, 14, 224, 224, 89, 61, 58, 82, 5, 48, 232, 77,
        103, 122, 227, 163, 102, 16, 64, 1, 68, 0, 5, 80, 0, 5, 16, 1, 44, 23, 192, 171, 79, 126,
        110, 17, 50, 7, 112, 244, 172, 30, 29, 153, 68, 189, 233, 76, 111, 124, 212, 12, 2, 40,
        128, 8, 160, 0, 10, 224, 182, 248, 79, 204, 227, 85, 216, 155, 122, 1, 124, 250, 75, 139,
        144, 58, 128, 65, 51, 103, 33, 86, 129, 17, 220, 248, 238, 204, 242, 198, 71, 205, 176, 92,
        235, 159, 8, 153, 3, 24, 53, 115, 22, 66, 21, 103, 235, 187, 51, 203, 27, 31, 53, 131, 0,
        10, 32, 2, 88, 55, 128, 175, 157, 221, 105, 17, 50, 7, 48, 106, 230, 12, 68, 42, 222, 150,
        247, 103, 150, 55, 62, 106, 134, 197, 31, 82, 0, 17, 64, 1, 68, 0, 17, 192, 114, 1, 60,
        215, 15, 108, 130, 56, 157, 98, 4, 237, 219, 186, 1, 28, 52, 131, 0, 10, 32, 2, 40, 128, 8,
        32, 2, 88, 47, 128, 187, 253, 192, 244, 68, 105, 133, 8, 22, 223, 185, 85, 3, 56, 104, 6,
        1, 20, 64, 4, 80, 0, 17, 64, 4, 176, 94, 0, 207, 247, 3, 83, 19, 163, 21, 35, 104, 255,
        166, 38, 128, 2, 136, 0, 10, 32, 2, 136, 0, 214, 11, 224, 133, 126, 96, 90, 34, 148, 32,
        130, 246, 112, 90, 2, 40, 128, 8, 160, 0, 34, 128, 8, 96, 189, 0, 238, 245, 3, 83, 18, 159,
        68, 17, 180, 143, 83, 18, 64, 1, 68, 0, 5, 16, 1, 68, 0, 235, 5, 240, 98, 63, 48, 29, 209,
        73, 24, 65, 123, 57, 29, 1, 20, 64, 4, 80, 0, 17, 64, 4, 176, 94, 0, 247, 251, 129, 169,
        136, 77, 226, 8, 218, 207, 169, 8, 160, 0, 34, 128, 2, 136, 0, 34, 128, 245, 2, 120, 208,
        15, 76, 67, 100, 38, 136, 160, 61, 157, 134, 0, 10, 32, 2, 40, 128, 8, 32, 2, 88, 47, 128,
        151, 250, 129, 41, 136, 203, 68, 17, 180, 175, 83, 16, 64, 1, 68, 0, 5, 16, 1, 68, 0, 235,
        5, 240, 114, 63, 144, 158, 168, 76, 24, 65, 123, 155, 158, 0, 10, 32, 2, 40, 128, 8, 32, 2,
        88, 47, 128, 135, 253, 64, 106, 98, 50, 113, 4, 237, 111, 106, 2, 40, 128, 8, 160, 0, 34,
        128, 8, 96, 189, 0, 94, 233, 7, 210, 18, 145, 13, 68, 208, 30, 167, 37, 128, 2, 136, 0, 10,
        32, 2, 136, 0, 214, 11, 224, 81, 63, 144, 146, 120, 108, 40, 130, 246, 57, 37, 1, 20, 64,
        4, 80, 0, 17, 64, 4, 176, 94, 0, 143, 251, 129, 116, 68, 99, 131, 17, 180, 215, 233, 8,
        160, 0, 34, 128, 2, 136, 0, 34, 128, 245, 2, 120, 181, 31, 72, 69, 44, 54, 28, 65, 251,
        157, 138, 0, 10, 32, 2, 40, 128, 8, 32, 2, 88, 47, 128, 215, 250, 129, 52, 68, 162, 64, 4,
        237, 121, 26, 2, 40, 128, 8, 160, 0, 34, 128, 8, 96, 189, 0, 94, 239, 7, 82, 16, 135, 66,
        17, 180, 239, 41, 8, 160, 0, 34, 128, 2, 136, 0, 34, 128, 245, 2, 120, 163, 31, 88, 157,
        40, 20, 140, 160, 189, 95, 157, 0, 10, 32, 2, 40, 128, 8, 32, 2, 88, 47, 128, 55, 251, 129,
        85, 137, 65, 225, 8, 218, 255, 85, 9, 160, 0, 34, 128, 2, 136, 0, 34, 128, 245, 2, 120,
        171, 31, 88, 141, 8, 224, 29, 172, 71, 0, 5, 16, 1, 20, 64, 4, 16, 1, 172, 23, 192, 219,
        253, 192, 42, 60, 126, 158, 71, 208, 123, 88, 133, 0, 10, 32, 2, 40, 128, 8, 32, 2, 88, 47,
        128, 119, 250, 129, 83, 231, 209, 243, 66, 4, 189, 139, 83, 39, 128, 2, 136, 0, 10, 32, 2,
        136, 0, 214, 11, 224, 221, 126, 224, 84, 121, 236, 252, 99, 4, 189, 143, 83, 245, 39, 58,
        14, 177, 224, 254, 131, 186, 140, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];
    fn png_key() -> CacheKey {
        CacheKey {
            version: RevisionLabel::new(Some("synthetic-river-png-v1")).unwrap(),
            bytes: AssetManifest {
                byte_len: PNG.len() as u64,
                sha256: [
                    218, 100, 113, 230, 117, 231, 209, 190, 209, 78, 145, 31, 107, 15, 18, 170,
                    252, 11, 123, 131, 6, 234, 39, 232, 29, 186, 178, 181, 70, 236, 105, 42,
                ],
            },
        }
    }
}
