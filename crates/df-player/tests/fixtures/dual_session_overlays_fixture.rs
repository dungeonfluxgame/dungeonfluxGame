//! Synthetic independent player/public projections; no production RPC, audio or server connected.
#[cfg(target_arch = "wasm32")]
mod browser {
    use df_display::{DisplayCharacterScreen, DisplayExploration, DisplaySessionOverlays};
    use df_player::{
        PlayerCharacterConnection, PlayerCharacterScreen, PlayerExploration, PlayerSessionOverlays,
    };
    use df_types::{ClientBindingId, RecoveryEpoch, SessionRevision};
    use df_ui::{
        ActionView, CampaignLimits, CampaignMember, CampaignView, CharacterAction,
        CharacterActionKind, CharacterDisplayConnection, CharacterDisplayHostOffer,
        CharacterDisplayLimits, CharacterDisplayView, CharacterFact, CharacterGroup,
        CharacterLimits, CharacterOption, CharacterPhaseView, CharacterPortrait,
        CharacterPublicMember, CharacterPublicReadiness, CharacterStatus, ConceptScene,
        ControlledAction, ExplorationChoice, ExplorationLimits, ExplorationNpc,
        ExplorationPortrait, ExplorationView, FeedbackView, SessionBookend, SessionBookendKind,
        SessionConnection, SessionOverlayOffer, SessionOverlaySelection, SessionOverlayView,
    };
    use std::{cell::RefCell, rc::Rc};
    use wasm_bindgen::{JsCast, JsValue, prelude::wasm_bindgen};
    use web_sys::{Document, Element, Event, HtmlButtonElement, HtmlInputElement};
    const PRIVATE_TEXT: &str = "PRIVATE_NPC_STORY_SENTINEL";
    const PRIVATE_DRAFT: &str = "PRIVATE_DRAFT_SENTINEL";
    const PRIVATE_CHOICES: [ExplorationChoice<'static>; 2] = [
        ExplorationChoice {
            id: "offered:player/ask?opaque=1",
            label: "Ask Vell about the lantern",
            detail: "Send the supplied question.",
            disabled_reason: None,
        },
        ExplorationChoice {
            id: "offered:player/listen?opaque=2",
            label: "Listen to the room",
            detail: "Send the supplied listening offer.",
            disabled_reason: None,
        },
    ];
    const REORDERED_CHOICES: [ExplorationChoice<'static>; 2] = [
        ExplorationChoice {
            id: "offered:player/listen?opaque=2",
            label: "Listen to the room",
            detail: "Send the supplied listening offer.",
            disabled_reason: None,
        },
        ExplorationChoice {
            id: "offered:player/ask?opaque=1",
            label: "Ask Vell about the lantern",
            detail: "Send the supplied question.",
            disabled_reason: None,
        },
    ];
    const HOST_CHOICES: [ExplorationChoice<'static>; 1] = [ExplorationChoice {
        id: "offered:public-host/continue?opaque=3",
        label: "Continue the public scene",
        detail: "Fixture-supplied public host offer.",
        disabled_reason: None,
    }];
    const MEMBERS: [CampaignMember<'static>; 1] = [CampaignMember {
        key: "corin",
        name: "Corin Vale",
        role: "Adventurer",
        sigil: "CV",
    }];

    fn error(error: impl std::fmt::Display) -> JsValue {
        JsValue::from_str(&error.to_string())
    }
    fn require(condition: bool, message: &str) -> Result<(), JsValue> {
        if condition {
            Ok(())
        } else {
            Err(JsValue::from_str(message))
        }
    }
    fn binding(byte: u8) -> Result<ClientBindingId, JsValue> {
        ClientBindingId::from_bytes(&[byte; 16])
            .map_err(|failure| JsValue::from_str(&format!("fixture binding rejected: {failure:?}")))
    }
    fn revision(epoch: u64, sequence: u64) -> Result<SessionRevision, JsValue> {
        Ok(SessionRevision::new(
            RecoveryEpoch::new(epoch).map_err(|failure| {
                JsValue::from_str(&format!("fixture epoch rejected: {failure:?}"))
            })?,
            sequence,
        ))
    }
    fn limits() -> ExplorationLimits {
        ExplorationLimits {
            campaign: CampaignLimits {
                max_members: 8,
                max_objectives: 8,
                max_text_bytes: 1024,
            },
            max_choices: 8,
            max_text_bytes: 1024,
            max_identifier_bytes: 256,
        }
    }
    fn public_campaign() -> CampaignView<'static> {
        CampaignView {
            scene: ConceptScene::Tavern,
            chapter: "Act I · The Lamplighter",
            title: "The Drowned Lantern",
            description: "A hearth, a lowered voice, a story waiting to be heard.",
            location: "Greyhaven · The Drowned Lantern",
            scene_label: "At the barkeeper’s table",
            narration: "Rain drums against the roof. Vell sets down a glass beside the lantern.",
            connection: "Synthetic role fixture · No session or provider connected",
            notice: "Caller-filtered presentation boundary · Not an RPC projection",
            members: &MEMBERS,
            objectives: &[],
        }
    }
    // This separately constructed public snapshot never reads a private view.
    fn public_view(
        binding: ClientBindingId,
        revision: SessionRevision,
        host: bool,
        pending: bool,
    ) -> ExplorationView<'static> {
        ExplorationView {
            binding,
            revision,
            campaign: public_campaign(),
            npc: Some(ExplorationNpc {
                name: "Vell",
                context: "Barkeeper · The Drowned Lantern",
                dialogue: "Welcome, travelers. The storm will pass.",
                portrait: Some(ExplorationPortrait::Vell),
            }),
            heading: "The party’s conversation",
            choices: if host { &HOST_CHOICES } else { &[] },
            draft_label: "No public free-text input offered",
            submit_label: "No public draft offered",
            draft_offer_id: None,
            pending: if pending {
                Some("Waiting for a supplied public scene update.")
            } else {
                None
            },
            rejection: None,
        }
    }
    fn private_view(
        binding: ClientBindingId,
        revision: SessionRevision,
        reordered: bool,
        pending: bool,
        refused: bool,
    ) -> ExplorationView<'static> {
        let mut campaign = public_campaign();
        campaign.narration =
            "PRIVATE_NPC_STORY_SENTINEL · You alone recognize the lamplighter’s crest.";
        ExplorationView {
            binding,
            revision,
            campaign,
            npc: Some(ExplorationNpc {
                name: "Vell",
                context: PRIVATE_TEXT,
                dialogue: "PRIVATE_NPC_STORY_SENTINEL · Vell whispers a name only you heard.",
                portrait: Some(ExplorationPortrait::Vell),
            }),
            heading: "Your private conversation",
            choices: if reordered {
                &REORDERED_CHOICES
            } else {
                &PRIVATE_CHOICES
            },
            draft_label: "Your words",
            submit_label: "Send your words",
            draft_offer_id: Some("offered:player/draft?opaque=4"),
            pending: if pending {
                Some("Your input is pending; no outcome is confirmed.")
            } else {
                None
            },
            rejection: if refused {
                Some("The supplied refusal leaves your words editable.")
            } else {
                None
            },
        }
    }

    fn player_limits() -> CharacterLimits {
        CharacterLimits {
            max_groups: 16,
            max_options: 128,
            max_facts: 128,
            max_actions: 16,
            max_text_bytes: 4096,
        }
    }
    fn display_limits() -> CharacterDisplayLimits {
        CharacterDisplayLimits {
            max_members: 64,
            max_host_offers: 16,
            max_text_bytes: 4096,
        }
    }
    fn character_player_view() -> CharacterPhaseView {
        CharacterPhaseView {
            generation: 1,
            owner_key: "synthetic-private-player-one".into(),
            revision: 1,
            chapter: "Prologue · Player character creation".into(),
            title: "Who will you become?".into(),
            description:
                "Choose a story, write a name, and carry a spark of courage into the dark.".into(),
            connection: "Synthetic filtered player projection · Production RPC pending".into(),
            status: CharacterStatus::Editing,
            status_message:
                "Fixture offers illustrate input only; server build validation is not connected."
                    .into(),
            editable: true,
            name: "Mara".into(),
            flavor: "PRIVATE-DRAFT-SENTINEL · A lantern that never goes out".into(),
            appearance: None,
            portrait: Some(CharacterPortrait::Narrator),
            groups: vec![CharacterGroup {
                id: "synthetic-story".into(),
                label: "A story calling".into(),
                description: "Synthetic presentation choices, with no mechanical effect.".into(),
                selected: Some("lantern".into()),
                options: vec![
                    CharacterOption {
                        id: "lantern".into(),
                        label: "Lantern keeper".into(),
                        description: "A light to guide the travellers home.".into(),
                        enabled: true,
                        availability: "Synthetic advertised choice".into(),
                        portrait: Some(CharacterPortrait::Narrator),
                    },
                    CharacterOption {
                        id: "harbor".into(),
                        label: "Harbor witness".into(),
                        description: "A hundred stories waiting to be remembered.".into(),
                        enabled: true,
                        availability: "Synthetic advertised choice".into(),
                        portrait: Some(CharacterPortrait::Vell),
                    },
                ],
            }],
            facts: vec![CharacterFact {
                label: "View source".into(),
                value: "Separate synthetic player projection".into(),
            }],
            actions: vec![CharacterAction {
                id: "synthetic-submit".into(),
                kind: CharacterActionKind::SubmitDraft,
                label: "Send draft · fixture only".into(),
                enabled: true,
            }],
        }
    }
    fn character_display_view() -> CharacterDisplayView {
        CharacterDisplayView {
            generation: 1, public_scope_key: "synthetic-public-room-one".into(), revision: 1,
            chapter: "Prologue · Shared character creation".into(), title: "Every story needs its heroes".into(),
            description: "Your paths are gathering around the fire. A shared adventure waits beyond its light.".into(),
            readiness: CharacterPublicReadiness::Choosing,
            progress_label: "Synthetic server report · Party choosing characters".into(),
            public_notice: "Separate public projection · AI-generated v2 lantern-keeper art · No private form, build choices, or rejection details".into(),
            connection: CharacterDisplayConnection::Connected,
            connection_label: "Synthetic shared display · Production RPC pending".into(),
            members: vec![
                CharacterPublicMember { key: "public-mara".into(), character_name: "Mara".into(),
                    portrait: Some(CharacterPortrait::Narrator), readiness: CharacterPublicReadiness::Choosing,
                    progress_label: "Public report · Choosing".into(), appearance_summary: None },
                CharacterPublicMember { key: "public-elian".into(), character_name: "Elian".into(),
                    portrait: Some(CharacterPortrait::Vell), readiness: CharacterPublicReadiness::Ready,
                    progress_label: "Public report · Ready".into(), appearance_summary: None },
            ],
            host_offers: vec![CharacterDisplayHostOffer { id: "synthetic-host-offer".into(),
                label: "Advertised host offer · fixture only".into(), enabled: true, pending: false }],
        }
    }

    const HOST_INITIAL: [SessionOverlayOffer<'static>; 1] = [SessionOverlayOffer {
        key: "fixture:advertised-host-a",
        label: "Advertised session offer A",
        enabled: true,
        pending: false,
    }];
    const HOST_REPLACED: [SessionOverlayOffer<'static>; 1] = [SessionOverlayOffer {
        key: "fixture:advertised-host-b",
        label: "Advertised session offer B",
        enabled: true,
        pending: false,
    }];
    const HOST_PENDING: [SessionOverlayOffer<'static>; 1] = [SessionOverlayOffer {
        key: "fixture:advertised-host-b",
        label: "Waiting for the supplied update",
        enabled: true,
        pending: true,
    }];
    const PRIVATE_OVERLAY: &str = "PRIVATE_OVERLAY_SENTINEL";

    // Public content is built independently; no private view is accepted or redacted here.
    fn public_overlay(
        generation: u64,
        revision: u64,
        connection: SessionConnection,
        offers: &'static [SessionOverlayOffer<'static>],
    ) -> SessionOverlayView<'static> {
        SessionOverlayView {
            generation,
            revision,
            title: "The party’s session",
            subtitle: "Public supplied status",
            connection,
            connection_label: "Synthetic public connection",
            connection_detail: "No production RPC connected",
            operation: None,
            bookend: Some(SessionBookend {
                key: "public-recap",
                kind: SessionBookendKind::Recap,
                title: "Beneath the lantern",
                context: "Supplied public recap",
                caption: "The company gathered at the inn.",
                attribution: "Synthetic narrator",
                position: "Supplied still caption · No audio timeline",
                still: Some((
                    "assets/concept-art/scene-campfire-under-stars.webp",
                    "Campfire beneath the stars",
                )),
                skip_offer: None,
            }),
            host_context: "Only explicitly supplied public offers",
            host_offers: offers,
            reduced_motion: true,
        }
    }
    fn private_overlay(
        generation: u64,
        revision: u64,
        connection: SessionConnection,
        offers: &'static [SessionOverlayOffer<'static>],
    ) -> SessionOverlayView<'static> {
        SessionOverlayView {
            generation,
            revision,
            title: "Your session",
            subtitle: PRIVATE_OVERLAY,
            connection,
            connection_label: "Synthetic player connection",
            connection_detail: "Private supplied status",
            operation: Some(FeedbackView::Uncertain(PRIVATE_OVERLAY)),
            bookend: Some(SessionBookend {
                key: "private-recap",
                kind: SessionBookendKind::Recap,
                title: "Your remembered whisper",
                context: PRIVATE_OVERLAY,
                caption: PRIVATE_OVERLAY,
                attribution: "Synthetic narrator",
                position: "Supplied still caption · No audio timeline",
                still: None,
                skip_offer: None,
            }),
            host_context: "Only explicitly supplied player offers",
            host_offers: offers,
            reduced_motion: true,
        }
    }
    #[derive(Default)]
    struct Deliveries {
        count: usize,
        last: Option<SessionOverlaySelection>,
    }
    impl Deliveries {
        fn record(&mut self, selection: SessionOverlaySelection) {
            self.count = self.count.saturating_add(1);
            self.last = Some(selection);
        }
    }
    struct Fixture {
        document: Document,
        player_parent: Element,
        display_parent: Element,
        player: PlayerExploration,
        display: DisplayExploration,
        player_character: PlayerCharacterScreen,
        display_character: DisplayCharacterScreen,
        player_overlay: Option<PlayerSessionOverlays>,
        display_overlay: Option<DisplaySessionOverlays>,
        generation: u64,
        revision: u64,
        selections: Rc<RefCell<Deliveries>>,
        public_selections: Rc<RefCell<Deliveries>>,
        status: Element,
        controls: Vec<ControlledAction>,
    }
    thread_local! { static FIXTURE: RefCell<Option<Fixture>> = const { RefCell::new(None) }; }
    fn required(root: &Element, selector: &str) -> Result<Element, JsValue> {
        root.query_selector(selector)?
            .ok_or_else(|| JsValue::from_str("fixture node missing"))
    }
    fn button(root: &Element, selector: &str) -> Result<HtmlButtonElement, JsValue> {
        required(root, selector)?
            .dyn_into()
            .map_err(|_| JsValue::from_str("button missing"))
    }
    fn draft(root: &Element) -> Result<HtmlInputElement, JsValue> {
        required(root, "input")?
            .dyn_into()
            .map_err(|_| JsValue::from_str("draft missing"))
    }
    fn overlays(
        fixture: &Fixture,
    ) -> Result<(&PlayerSessionOverlays, &DisplaySessionOverlays), JsValue> {
        Ok((
            fixture
                .player_overlay
                .as_ref()
                .ok_or_else(|| JsValue::from_str("player overlay missing"))?,
            fixture
                .display_overlay
                .as_ref()
                .ok_or_else(|| JsValue::from_str("display overlay missing"))?,
        ))
    }
    fn mount_overlays(fixture: &mut Fixture) -> Result<(), JsValue> {
        let captured = Rc::clone(&fixture.selections);
        fixture.player_overlay = Some(
            PlayerSessionOverlays::mount(
                &fixture.document,
                &fixture.player_parent,
                fixture.player.root(),
                &private_overlay(
                    fixture.generation,
                    fixture.revision,
                    SessionConnection::Connected,
                    &HOST_INITIAL,
                ),
                move |selection| captured.borrow_mut().record(selection),
            )
            .map_err(error)?,
        );
        let captured = Rc::clone(&fixture.public_selections);
        fixture.display_overlay = Some(
            DisplaySessionOverlays::mount(
                &fixture.document,
                &fixture.display_parent,
                fixture.display.root(),
                &public_overlay(
                    fixture.generation,
                    fixture.revision,
                    SessionConnection::Connected,
                    &HOST_INITIAL,
                ),
                move |selection| captured.borrow_mut().record(selection),
            )
            .map_err(error)?,
        );
        Ok(())
    }
    fn apply(
        fixture: &mut Fixture,
        connection: SessionConnection,
        offers: &'static [SessionOverlayOffer<'static>],
    ) -> Result<(), JsValue> {
        fixture.revision = fixture
            .revision
            .checked_add(1)
            .ok_or_else(|| JsValue::from_str("fixture revision exhausted"))?;
        fixture
            .player_overlay
            .as_mut()
            .ok_or_else(|| JsValue::from_str("player overlay missing"))?
            .update(&private_overlay(
                fixture.generation,
                fixture.revision,
                connection,
                offers,
            ))
            .map_err(error)?;
        fixture
            .display_overlay
            .as_mut()
            .ok_or_else(|| JsValue::from_str("display overlay missing"))?
            .update(&public_overlay(
                fixture.generation,
                fixture.revision,
                connection,
                offers,
            ))
            .map_err(error)?;
        let (_, public) = overlays(fixture)?;
        require(
            !public.root().outer_html().contains(PRIVATE_OVERLAY)
                && !public.root().outer_html().contains(PRIVATE_TEXT)
                && !public.root().outer_html().contains(PRIVATE_DRAFT),
            "public overlay contains private data",
        )
    }
    fn record_result(fixture: &Fixture, result_label: &str) -> Result<(), JsValue> {
        fixture.status.set_text_content(Some(result_label));
        fixture.status.set_attribute("data-result", result_label)
    }
    #[wasm_bindgen]
    pub fn exercise_session_overlays() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let fixture = slot.as_mut().ok_or_else(|| JsValue::from_str("fixture missing"))?;
            fixture.player.root().remove_attribute("hidden")?;
            fixture.display.root().remove_attribute("hidden")?;
            fixture.player_character.root().set_attribute("hidden", "")?;
            fixture.display_character.root().set_attribute("hidden", "")?;
            let (player, public) = overlays(fixture)?;
            let player_root = player.root().clone();
            let public_root = public.root().clone();
            let essential_root = fixture.player.root().clone();
            let input = draft(&essential_root)?;
            input.set_value(PRIVATE_DRAFT);
            input.dispatch_event(&Event::new("input")?)?;
            input.focus()?;
            let open = button(&player_root, ".session-header button")?;
            let close = button(&player_root, ".session-drawer > button")?;
            open.click();
            require(!required(&player_root, ".session-drawer")?.has_attribute("hidden"), "details did not open")?;
            close.click();
            require(required(&player_root, ".session-drawer")?.has_attribute("hidden"), "details did not close")?;
            require(fixture.document.active_element().is_some_and(|active| active.is_same_node(Some(&input))), "details lost essential focus")?;
            let raw = required(&player_root, ".session-subtitle")?.first_child().ok_or_else(|| JsValue::from_str("private Text missing"))?;
            apply(fixture, SessionConnection::Connected, &HOST_REPLACED)?;
            require(player_root.is_same_node(Some(overlays(fixture)?.0.root())) && public_root.is_same_node(Some(overlays(fixture)?.1.root())), "update remounted overlay")?;
            require(input.value() == PRIVATE_DRAFT && input.is_connected(), "update lost essential draft")?;
            require(fixture.document.active_element().is_some_and(|active| active.is_same_node(Some(&input))), "update lost essential focus")?;
            require(raw.node_value().is_none_or(|text| text.is_empty()), "update retained obsolete raw Text")?;
            open.click();
            button(&public_root, ".session-header button")?.click();
            let host = button(&player_root, "[data-offer-key='fixture:advertised-host-b']")?;
            let public_host = button(&public_root, "[data-offer-key='fixture:advertised-host-b']")?;
            host.focus()?;
            let count = fixture.selections.borrow().count;
            host.click(); public_host.click();
            require(fixture.selections.borrow().count == count + 1, "player offer did not emit once")?;
            require(matches!(fixture.selections.borrow().last.as_ref(), Some(selection)
                if selection.key == "fixture:advertised-host-b" && selection.generation == fixture.generation && selection.revision == fixture.revision), "player callback changed offer")?;
            require(matches!(fixture.public_selections.borrow().last.as_ref(), Some(selection)
                if selection.key == "fixture:advertised-host-b" && selection.revision == fixture.revision), "public callback changed offer")?;
            apply(fixture, SessionConnection::Connected, &HOST_REPLACED)?;
            require(fixture.document.active_element().is_some_and(|active| active.is_same_node(Some(&host))), "surviving offer lost focus")?;
            apply(fixture, SessionConnection::Connected, &HOST_PENDING)?;
            let count = fixture.selections.borrow().count;
            host.dispatch_event(&Event::new("click")?)?;
            require(fixture.selections.borrow().count == count, "pending offer emitted")?;
            apply(fixture, SessionConnection::Offline, &HOST_REPLACED)?;
            host.dispatch_event(&Event::new("click")?)?;
            require(fixture.selections.borrow().count == count, "offline offer emitted")?;
            apply(fixture, SessionConnection::Reconnecting, &HOST_REPLACED)?;
            host.dispatch_event(&Event::new("click")?)?;
            require(fixture.selections.borrow().count == count, "reconnecting offer emitted")?;
            apply(fixture, SessionConnection::Connected, &[])?;
            host.dispatch_event(&Event::new("click")?)?;
            require(fixture.selections.borrow().count == count, "revoked host offer emitted")?;
            require(input.value() == PRIVATE_DRAFT, "host lifecycle changed essential draft")?;
            close.click(); input.focus()?;
            record_result(fixture, "exercise-pass")
        })
    }
    #[wasm_bindgen]
    pub fn navigate_session_clients() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let slot = slot.borrow();
            let fixture = slot
                .as_ref()
                .ok_or_else(|| JsValue::from_str("fixture missing"))?;
            for (exploration, character) in [
                (fixture.player.root(), fixture.player_character.root()),
                (fixture.display.root(), fixture.display_character.root()),
            ] {
                if exploration.has_attribute("hidden") {
                    exploration.remove_attribute("hidden")?;
                    character.set_attribute("hidden", "")?;
                } else {
                    exploration.set_attribute("hidden", "")?;
                    character.remove_attribute("hidden")?;
                }
            }
            record_result(fixture, "navigation-pass")
        })
    }
    #[wasm_bindgen]
    pub fn replace_session_overlay_generation() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let fixture = slot
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture missing"))?;
            fixture.player.root().remove_attribute("hidden")?;
            fixture.display.root().remove_attribute("hidden")?;
            let old = overlays(fixture)?.0.root().clone();
            let raw = required(&old, ".bookend-caption")?
                .first_child()
                .ok_or_else(|| JsValue::from_str("raw caption missing"))?;
            let input = draft(fixture.player.root())?;
            input.set_value(PRIVATE_DRAFT);
            input.dispatch_event(&Event::new("input")?)?;
            input.focus()?;
            fixture.generation = fixture
                .generation
                .checked_add(1)
                .ok_or_else(|| JsValue::from_str("fixture generation exhausted"))?;
            require(
                fixture
                    .player_overlay
                    .as_mut()
                    .ok_or_else(|| JsValue::from_str("overlay missing"))?
                    .update(&private_overlay(
                        fixture.generation,
                        0,
                        SessionConnection::Connected,
                        &HOST_INITIAL,
                    ))
                    .is_err(),
                "new generation reused old owner",
            )?;
            require(
                !old.is_connected() && raw.node_value().is_none_or(|text| text.is_empty()),
                "replaced scope retained private overlay",
            )?;
            require(
                input.is_connected() && input.value() == PRIVATE_DRAFT,
                "overlay replacement discarded caller-owned draft",
            )?;
            fixture.player_overlay.take();
            require(
                fixture
                    .display_overlay
                    .as_mut()
                    .ok_or_else(|| JsValue::from_str("display missing"))?
                    .update(&public_overlay(
                        fixture.generation,
                        0,
                        SessionConnection::Connected,
                        &HOST_INITIAL,
                    ))
                    .is_err(),
                "new public generation reused old owner",
            )?;
            fixture.display_overlay.take();
            fixture.revision = 0;
            mount_overlays(fixture)?;
            record_result(fixture, "replacement-pass")
        })
    }
    #[wasm_bindgen]
    pub fn drop_session_overlays() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let fixture = slot
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture missing"))?;
            fixture.player.root().remove_attribute("hidden")?;
            fixture.display.root().remove_attribute("hidden")?;
            apply(fixture, SessionConnection::Connected, &HOST_INITIAL)?;
            let root = overlays(fixture)?.0.root().clone();
            let raw = required(&root, ".bookend-caption")?
                .first_child()
                .ok_or_else(|| JsValue::from_str("raw caption missing"))?;
            let old_host = button(&root, "[data-offer-key='fixture:advertised-host-a']")?;
            let input = draft(fixture.player.root())?;
            input.set_value(PRIVATE_DRAFT);
            input.dispatch_event(&Event::new("input")?)?;
            input.focus()?;
            let count = fixture.selections.borrow().count;
            fixture.player_overlay.take();
            fixture.display_overlay.take();
            old_host.dispatch_event(&Event::new("click")?)?;
            require(
                fixture.selections.borrow().count == count,
                "Drop left active host callback",
            )?;
            require(
                !root.is_connected() && raw.node_value().is_none_or(|text| text.is_empty()),
                "Drop retained private overlay Text",
            )?;
            require(
                input.is_connected() && input.value() == PRIVATE_DRAFT,
                "Drop lost essential draft",
            )?;
            require(
                fixture
                    .document
                    .active_element()
                    .is_some_and(|active| active.is_same_node(Some(&input))),
                "Drop lost essential focus",
            )?;
            require(
                fixture
                    .player
                    .root()
                    .parent_node()
                    .is_some_and(|parent| parent.is_same_node(Some(&fixture.player_parent))),
                "Drop failed to restore essential parent",
            )?;
            record_result(fixture, "drop-pass")
        })
    }
    #[wasm_bindgen]
    pub fn remount_session_overlays() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let fixture = slot
                .as_mut()
                .ok_or_else(|| JsValue::from_str("fixture missing"))?;
            if fixture.player_overlay.is_some() || fixture.display_overlay.is_some() {
                return Err(JsValue::from_str(
                    "existing overlays must be dropped before remount",
                ));
            }
            mount_overlays(fixture)?;
            record_result(fixture, "remount-pass")
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
        notice.set_text_content(Some("Synthetic dual-client overlays mounted over real character/exploration components. Separate public/private projections; no production RPC, gameplay outcome, audio or authorization claim."));
        body.append_child(&notice)?;
        let controls_root = document.create_element("nav")?;
        controls_root.set_attribute("aria-label", "Fixture controls")?;
        body.append_child(&controls_root)?;
        let status = document.create_element("output")?;
        status.set_id("dual-session-overlays-status");
        status.set_attribute("aria-live", "polite")?;
        body.append_child(&status)?;
        let player_parent = document.create_element("section")?;
        player_parent.set_attribute("data-client", "player")?;
        body.append_child(&player_parent)?;
        let display_parent = document.create_element("section")?;
        display_parent.set_attribute("data-client", "shared-display")?;
        body.append_child(&display_parent)?;
        let player = PlayerExploration::mount(
            &document,
            &player_parent,
            "overlay-private-draft",
            &private_view(binding(7)?, revision(1, 0)?, false, false, false),
            limits(),
            |_| {},
        )
        .map_err(error)?;
        let display = DisplayExploration::mount(
            &document,
            &display_parent,
            "overlay-public-draft",
            &public_view(binding(8)?, revision(1, 0)?, false, false),
            limits(),
            |_| {},
        )
        .map_err(error)?;
        let player_character = PlayerCharacterScreen::mount(
            &document,
            &player_parent,
            "overlay-private-character",
            &character_player_view(),
            player_limits(),
            PlayerCharacterConnection::Connected,
            |_| {},
        )
        .map_err(error)?;
        let display_character = DisplayCharacterScreen::mount(
            &document,
            &display_parent,
            &character_display_view(),
            display_limits(),
            CharacterDisplayConnection::Connected,
            |_| {},
        )
        .map_err(error)?;
        player_character.root().set_attribute("hidden", "")?;
        display_character.root().set_attribute("hidden", "")?;
        let mut fixture = Fixture {
            document: document.clone(),
            player_parent,
            display_parent,
            player,
            display,
            player_character,
            display_character,
            player_overlay: None,
            display_overlay: None,
            generation: 1,
            revision: 1,
            selections: Rc::new(RefCell::new(Deliveries::default())),
            public_selections: Rc::new(RefCell::new(Deliveries::default())),
            status,
            controls: Vec::new(),
        };
        mount_overlays(&mut fixture)?;
        for (id, label, operation) in [
            (
                "exercise-session-overlays",
                "Exercise session details, offers and reconnect",
                exercise_session_overlays as fn() -> Result<(), JsValue>,
            ),
            (
                "navigate-session-clients",
                "Navigate creation / exploration in place",
                navigate_session_clients,
            ),
            (
                "replace-session-overlays",
                "Replace overlay generation",
                replace_session_overlay_generation,
            ),
            (
                "drop-session-overlays",
                "Drop both overlays; retain essential input",
                drop_session_overlays,
            ),
            (
                "remount-session-overlays",
                "Remount supplied overlay scope",
                remount_session_overlays,
            ),
        ] {
            let action = ControlledAction::create(
                &document,
                ActionView {
                    label,
                    enabled: true,
                    pending: false,
                },
            )
            .map_err(error)?;
            action.element().set_id(id);
            action
                .on_activate(move || {
                    if let Err(failure) = operation() {
                        FIXTURE.with(|slot| {
                            if let Some(fixture) = slot.borrow().as_ref() {
                                fixture.status.set_text_content(Some(&format!(
                                    "fixture failure: {failure:?}"
                                )));
                                if fixture
                                    .status
                                    .set_attribute("data-result", "failed")
                                    .is_err()
                                {
                                    fixture.status.set_text_content(Some(
                                        "Fixture failure could not update its result attribute.",
                                    ));
                                }
                            }
                        });
                    }
                })
                .map_err(error)?;
            controls_root.append_child(action.element())?;
            fixture.controls.push(action);
        }
        record_result(&fixture, "ready")?;
        FIXTURE.with(|slot| *slot.borrow_mut() = Some(fixture));
        Ok(())
    }
}
