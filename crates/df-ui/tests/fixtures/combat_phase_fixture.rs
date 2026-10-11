//! Explicitly synthetic fixture. Toolbar changes supply controlled snapshots;
//! combat buttons only report exact intents and show a pending snapshot. There
//! is no gameplay transport, server acceptance claim, random draw or local rule.
#[cfg(target_arch = "wasm32")]
mod browser {
    use df_client::revisions::ViewAcceptance;
    use df_types::{ClientBindingId, RecoveryEpoch, RevisionLabel, SessionRevision};
    use df_ui::{
        ActionView, CombatActor, CombatArt, CombatDraft, CombatFeedback, CombatIntent,
        CombatLimits, CombatOffer, CombatOfferKind, CombatPhaseSurface, CombatPhaseView,
        CombatReaction, CombatResource, CombatRoll, ControlledAction,
    };
    use std::{
        cell::{Cell, RefCell},
        rc::{Rc, Weak},
    };
    use wasm_bindgen::{JsCast, prelude::*};
    use web_sys::{Document, Element, Event, HtmlButtonElement};

    const RESOURCES: [CombatResource<'static>; 3] = [
        CombatResource {
            label: "Hit points",
            value: "23 / 31",
        },
        CombatResource {
            label: "Movement",
            value: "20 ft remaining",
        },
        CombatResource {
            label: "Reaction",
            value: "Available",
        },
    ];
    const ACTORS: [CombatActor<'static>; 4] = [
        CombatActor {
            key: "lyra",
            name: "Lyra Ashford",
            initiative: "19",
            status: "Your turn",
            resources: &RESOURCES,
            hit_points: None,
            portrait: None,
        },
        CombatActor {
            key: "warden",
            name: "The Tide Warden",
            initiative: "17",
            status: "At the water’s edge",
            resources: &[],
            hit_points: None,
            portrait: None,
        },
        CombatActor {
            key: "orin",
            name: "Orin Stonewake",
            initiative: "14",
            status: "Holding the bridge",
            resources: &[],
            hit_points: None,
            portrait: None,
        },
        CombatActor {
            key: "mira",
            name: "Mira Valewind",
            initiative: "11",
            status: "By the lantern tower",
            resources: &[],
            hit_points: None,
            portrait: None,
        },
    ];
    const HISTORY: [CombatRoll<'static>; 2] = [
        CombatRoll {
            key: "previous-attack",
            label: "Orin · Longsword attack",
            value: "18 · Hit",
            explanation: "D20: 12 + Strength: 3 + Proficiency: 3 = 18. The server supplied this outcome.",
            source: "Synthetic resolved attack · read-only history",
            verdict: None,
        },
        CombatRoll {
            key: "previous-save",
            label: "Mira · Dexterity saving throw",
            value: "16 · Success",
            explanation: "D20: 14 + Dexterity: 2 = 16. DC 13, supplied by the fixture snapshot.",
            source: "Synthetic resolved save · read-only history",
            verdict: None,
        },
    ];
    #[derive(Clone, Copy, Eq, PartialEq)]
    enum Scenario {
        Actions,
        Check,
        Save,
        Reaction,
        Pending,
        Refused,
        History,
        Closed,
    }
    struct Fixture {
        surface: Rc<CombatPhaseSurface>,
        binding: ClientBindingId,
        revision: Cell<SessionRevision>,
        scenario: Cell<Scenario>,
        offer: RevisionLabel,
        replacement: RevisionLabel,
        option: RevisionLabel,
        callbacks: Cell<usize>,
        last_intent: RefCell<Option<CombatIntent>>,
        controls: RefCell<Vec<ControlledAction>>,
        status: Element,
    }
    thread_local! { static FIXTURE: RefCell<Option<Rc<Fixture>>> = const { RefCell::new(None) }; }
    fn error(error: impl std::fmt::Display) -> JsValue {
        JsValue::from_str(&error.to_string())
    }
    fn child(
        document: &Document,
        parent: &Element,
        tag: &str,
        class: &str,
        text: Option<&str>,
    ) -> Result<Element, JsValue> {
        let element = document.create_element(tag)?;
        element.set_class_name(class);
        element.set_text_content(text);
        parent.append_child(&element)?;
        Ok(element)
    }
    fn limits() -> CombatLimits {
        CombatLimits {
            max_actors: 32,
            max_offers: 32,
            max_rolls: 32,
            max_resources_per_actor: 16,
            max_text_bytes: 4096,
            max_total_text_bytes: 65536,
        }
    }
    fn with_view<T>(
        scenario: Scenario,
        offer: &RevisionLabel,
        option: &RevisionLabel,
        operation: impl FnOnce(&CombatPhaseView<'_>) -> T,
    ) -> T {
        let pending = scenario == Scenario::Pending;
        let kind = match scenario {
            Scenario::Check | Scenario::Save => CombatOfferKind::Roll,
            Scenario::Reaction => CombatOfferKind::Reaction,
            _ => CombatOfferKind::Action,
        };
        let (label, explanation) = match scenario {
            Scenario::Check => (
                "Request Athletics check",
                "Athletics · D20 + 5. The server requests and resolves the draw.",
            ),
            Scenario::Save => (
                "Request Dexterity save",
                "Dexterity saving throw · D20 + 2. The server supplies the DC and outcome.",
            ),
            Scenario::Reaction => (
                "Use Shield",
                "Raise your ward. This selection is sent for server validation.",
            ),
            Scenario::Pending => (
                "Awaiting server response",
                "Intent delivered by the synthetic fixture. No outcome has been accepted.",
            ),
            _ => (
                "Longsword · Tide Warden",
                "Target and selection supplied by the fixture. The server validates the action.",
            ),
        };
        let offers = [
            CombatOffer {
                key: "primary",
                offer,
                option: Some(option),
                kind,
                label,
                explanation,
                enabled: !pending,
                pending,
            },
            CombatOffer {
                key: "secondary",
                offer,
                option: None,
                kind: CombatOfferKind::Action,
                label: "Move to the lantern",
                explanation: "The supplied destination is unavailable: the walkway is blocked.",
                enabled: false,
                pending: false,
            },
        ];
        let feedback = if scenario == Scenario::Refused {
            CombatFeedback::Refused(
                "Refused by synthetic server snapshot: this target is no longer available. Your draft is retained.",
            )
        } else if pending {
            CombatFeedback::Pending(
                "Waiting for the server · You can still read the battlefield and previous rolls.",
            )
        } else {
            CombatFeedback::None
        };
        let view = CombatPhaseView {
            art: CombatArt::Harbor,
            chapter: "The Drowned Lantern · Encounter I",
            title: "Hold the harbor",
            location: "Greyhaven · The moonlit docks",
            narration: "The tide surges beneath the bridge. Across the water, the Warden lifts a dripping blade. The lantern behind you is the last light on the harbor.",
            connection: "Synthetic combat preview · Controlled snapshots and real Rust/WASM callbacks · No game session connected",
            notice: "Synthetic fixture · Original concept art",
            turn_label: "Round 3 · Lyra’s turn",
            active_actor: Some("lyra"),
            actors: &ACTORS,
            action_heading: match scenario {
                Scenario::Check => "Athletics check",
                Scenario::Save => "Dexterity saving throw",
                Scenario::Reaction => "A moment to react",
                _ => "Your next move",
            },
            action_empty: "No current legal offers · Waiting for the next server view",
            offers: if matches!(scenario, Scenario::History | Scenario::Closed) {
                &[]
            } else {
                &offers
            },
            roll_heading: "The dice have spoken",
            roll_empty: "No resolved rolls supplied yet",
            rolls: &HISTORY,
            reaction: if scenario == Scenario::Reaction {
                Some(CombatReaction {
                    heading: "The Warden’s blade is coming",
                    timing: "Server window: open · 6 seconds remaining in this supplied snapshot",
                    explanation: "Reaction timing is supplied by the server. This view does not expire or advance the turn locally.",
                })
            } else if scenario == Scenario::Closed {
                Some(CombatReaction {
                    heading: "Reaction window closed",
                    timing: "Server window: closed",
                    explanation: "The current snapshot has no remaining reaction offers.",
                })
            } else {
                None
            },
            draft: if scenario == Scenario::History {
                None
            } else {
                Some(CombatDraft {
                    key: "lyra-action-draft",
                    label: "Describe your intent",
                    enabled: !pending,
                    feedback,
                })
            },
            feedback,
        };
        operation(&view)
    }
    impl Fixture {
        fn publish(&self, scenario: Scenario) -> Result<(), JsValue> {
            let revision = self
                .revision
                .get()
                .next_sequence()
                .map_err(|_| JsValue::from_str("fixture revision exhausted"))?;
            let result = with_view(scenario, &self.offer, &self.option, |view| {
                self.surface.update(self.binding, revision, view)
            })
            .map_err(error)?;
            if result != ViewAcceptance::Applied {
                return Err(JsValue::from_str("fixture snapshot was not applied"));
            }
            self.revision.set(revision);
            self.scenario.set(scenario);
            Ok(())
        }
        fn report(&self, result: Result<(), JsValue>, success: &str) {
            match result {
                Ok(()) => {
                    self.status.set_text_content(Some(success));
                    if self.status.set_attribute("data-check", "pass").is_err() {
                        self.status
                            .set_text_content(Some("FAIL · Fixture status could not be published"));
                    }
                }
                Err(_) => {
                    self.status
                        .set_text_content(Some("FAIL · Controlled fixture operation failed"));
                    if self.status.set_attribute("data-check", "fail").is_err() {
                        self.status
                            .set_text_content(Some("FAIL · Status publication also failed"));
                    }
                }
            }
        }
        fn receive(&self, intent: CombatIntent) {
            self.callbacks.set(self.callbacks.get().saturating_add(1));
            let summary = format!(
                "Intent callback {} · {:?} · offer {} · option {} · revision {}:{} · draft {} UTF-16 units. This is input delivery, not an accepted game outcome.",
                self.callbacks.get(),
                intent.kind,
                intent.offer.as_str(),
                intent.option.as_ref().map_or("none", RevisionLabel::as_str),
                intent.revision.epoch().get(),
                intent.revision.sequence(),
                intent
                    .draft
                    .as_ref()
                    .map_or(0, |draft| draft.encode_utf16().count())
            );
            *self.last_intent.borrow_mut() = Some(intent);
            self.report(self.publish(Scenario::Pending), &summary);
        }
        fn button(&self, selector: &str) -> Result<HtmlButtonElement, JsValue> {
            self.surface
                .root()
                .query_selector(selector)?
                .ok_or_else(|| JsValue::from_str("fixture button unavailable"))?
                .dyn_into::<HtmlButtonElement>()
                .map_err(|_| JsValue::from_str("fixture node is not a button"))
        }
        fn publish_offer_order(&self, reversed: bool) -> Result<(), JsValue> {
            let revision = self
                .revision
                .get()
                .next_sequence()
                .map_err(|_| JsValue::from_str("fixture revision exhausted"))?;
            let mut offers = [
                CombatOffer {
                    key: "alpha",
                    offer: &self.offer,
                    option: None,
                    kind: CombatOfferKind::Action,
                    label: "Offer A",
                    explanation: "Synthetic supplied offer A",
                    enabled: true,
                    pending: false,
                },
                CombatOffer {
                    key: "beta",
                    offer: &self.replacement,
                    option: Some(&self.option),
                    kind: CombatOfferKind::Action,
                    label: "Offer B",
                    explanation: "Synthetic supplied offer B",
                    enabled: true,
                    pending: false,
                },
            ];
            if reversed {
                offers.swap(0, 1);
            }
            let outcome = with_view(Scenario::Actions, &self.offer, &self.option, |view| {
                let mut ordered = *view;
                ordered.offers = &offers;
                self.surface.update(self.binding, revision, &ordered)
            })
            .map_err(error)?;
            if outcome != ViewAcceptance::Applied {
                return Err(JsValue::from_str(
                    "offer-order fixture snapshot was not applied",
                ));
            }
            self.revision.set(revision);
            self.scenario.set(Scenario::Actions);
            Ok(())
        }
        fn exercise_offer_reorder(&self) -> Result<(), JsValue> {
            self.publish_offer_order(false)?;
            let alpha = self.button("[data-combat-offer=alpha]")?;
            let beta = self.button("[data-combat-offer=beta]")?;
            beta.focus()?;
            self.publish_offer_order(true)?;
            let document = self
                .surface
                .root()
                .owner_document()
                .ok_or_else(|| JsValue::from_str("document unavailable"))?;
            let current_alpha = self.button("[data-combat-offer=alpha]")?;
            let current_beta = self.button("[data-combat-offer=beta]")?;
            if !alpha.is_same_node(Some(&current_alpha)) || !beta.is_same_node(Some(&current_beta))
            {
                return Err(JsValue::from_str("offer reorder replaced a keyed button"));
            }
            if !document
                .active_element()
                .is_some_and(|active| active.is_same_node(Some(&beta)))
            {
                return Err(JsValue::from_str("offer reorder lost native button focus"));
            }
            let first_offer = self
                .surface
                .root()
                .query_selector(".combat-offers")?
                .and_then(|parent| parent.first_element_child())
                .ok_or_else(|| JsValue::from_str("reordered offer container is empty"))?;
            if first_offer
                .query_selector("button")?
                .and_then(|button| button.get_attribute("data-combat-offer"))
                .as_deref()
                != Some("beta")
            {
                return Err(JsValue::from_str(
                    "offer DOM order differs from supplied order",
                ));
            }
            let revision = self.revision.get();
            let before = self.callbacks.get();
            beta.click();
            if self.callbacks.get() != before + 1
                || !self.last_intent.borrow().as_ref().is_some_and(|intent| {
                    intent.offer == self.replacement
                        && intent.option.as_ref() == Some(&self.option)
                        && intent.revision == revision
                })
            {
                return Err(JsValue::from_str(
                    "reordered button lost its exact callback identity",
                ));
            }
            self.publish(Scenario::Actions)?;
            Ok(())
        }
        fn exercise_implicit_drop(&self) -> Result<(), JsValue> {
            let document = self
                .surface
                .root()
                .owner_document()
                .ok_or_else(|| JsValue::from_str("document unavailable"))?;
            let body = document
                .body()
                .ok_or_else(|| JsValue::from_str("body unavailable"))?;
            let surface = with_view(Scenario::Actions, &self.offer, &self.option, |view| {
                CombatPhaseSurface::create(
                    &document,
                    "combat-implicit-drop-draft",
                    self.binding,
                    self.revision.get(),
                    view,
                    limits(),
                )
            })
            .map_err(error)?;
            body.append_child(surface.root())?;
            let retained_root = surface.root().clone();
            let retained_art = surface
                .root()
                .query_selector(".combat-art")?
                .ok_or_else(|| JsValue::from_str("implicit-drop artwork unavailable"))?;
            let retained_button = surface
                .root()
                .query_selector("[data-combat-offer=primary]")?
                .ok_or_else(|| JsValue::from_str("implicit-drop button unavailable"))?
                .dyn_into::<HtmlButtonElement>()
                .map_err(|_| JsValue::from_str("implicit-drop node is not a button"))?;
            let retained_draft = surface
                .draft_input()
                .ok_or_else(|| JsValue::from_str("implicit-drop draft unavailable"))?;
            let input_callbacks = Rc::new(Cell::new(0usize));
            let input_errors = Rc::new(Cell::new(0usize));
            let changes = Rc::clone(&input_callbacks);
            let failures = Rc::clone(&input_errors);
            retained_draft
                .on_change(move |result| {
                    if result.is_ok() {
                        changes.set(changes.get().saturating_add(1));
                    } else {
                        failures.set(failures.get().saturating_add(1));
                    }
                })
                .map_err(error)?;
            let intent_callbacks = Rc::new(Cell::new(0usize));
            let intents = Rc::clone(&intent_callbacks);
            surface
                .on_intent(move |_| {
                    intents.set(intents.get().saturating_add(1));
                })
                .map_err(error)?;
            retained_draft
                .input()
                .set_value("Private synthetic draft before implicit drop");
            retained_draft
                .input()
                .dispatch_event(&Event::new("input")?)?;
            if retained_draft.draft() != "Private synthetic draft before implicit drop"
                || input_callbacks.get() != 1
                || input_errors.get() != 0
            {
                return Err(JsValue::from_str(
                    "implicit-drop fixture did not receive the mounted native input",
                ));
            }
            // No explicit disposal: the external Rc to the field stays alive.
            drop(surface);
            if !retained_draft.draft().is_empty()
                || !retained_draft.input().value().is_empty()
                || !retained_draft.input().read_only()
                || retained_draft.root().parent_node().is_some()
                || retained_root.parent_node().is_some()
                || retained_root.first_child().is_some()
                || retained_art.has_attribute("src")
            {
                return Err(JsValue::from_str(
                    "implicit drop retained private draft, mounted nodes or artwork",
                ));
            }
            retained_draft
                .input()
                .set_value("Synthetic input after drop");
            retained_draft
                .input()
                .dispatch_event(&Event::new("input")?)?;
            retained_button.dispatch_event(&Event::new("click")?)?;
            if !retained_draft.draft().is_empty()
                || input_callbacks.get() != 1
                || input_errors.get() != 0
                || intent_callbacks.get() != 0
            {
                return Err(JsValue::from_str(
                    "implicit drop left a draft or intent listener active",
                ));
            }
            // Retained public handles also remain terminal under ordinary updates.
            if !matches!(
                retained_draft.update(
                    df_ui::TextInputView {
                        label: "Synthetic retained field",
                        enabled: true,
                        feedback: df_ui::InputFeedback::None
                    },
                    df_ui::DraftUpdate::Replace("New draft")
                ),
                Err(df_ui::ControlError::Draft(df_ui::DraftError::Disposed))
            ) {
                return Err(JsValue::from_str(
                    "implicit drop did not return the retained draft's typed disposal error",
                ));
            }
            retained_draft.dispose().map_err(error)?;
            Ok(())
        }
        fn exercise(&self) -> Result<(), JsValue> {
            self.publish(Scenario::Actions)?;
            let field = self
                .surface
                .draft_input()
                .ok_or_else(|| JsValue::from_str("draft unavailable"))?;
            field.input().focus()?;
            field.input().set_value("Keep the lantern behind me");
            field.input().dispatch_event(&Event::new("input")?)?;
            let original = field.input().clone();
            for _ in 0..64 {
                self.publish(Scenario::Actions)?;
            }
            let current = self
                .surface
                .draft_input()
                .ok_or_else(|| JsValue::from_str("draft disappeared"))?;
            if current.draft() != "Keep the lantern behind me"
                || !original.is_same_node(Some(current.input()))
            {
                return Err(JsValue::from_str("draft or input node was replaced"));
            }
            let document = self
                .surface
                .root()
                .owner_document()
                .ok_or_else(|| JsValue::from_str("document unavailable"))?;
            if !document
                .active_element()
                .is_some_and(|element| element.is_same_node(Some(current.input())))
            {
                return Err(JsValue::from_str("focused draft was not retained"));
            }
            self.publish(Scenario::Refused)?;
            if current.draft() != "Keep the lantern behind me" {
                return Err(JsValue::from_str("refusal cleared draft"));
            }
            let selected = self.button("[data-combat-offer=primary]")?;
            selected.focus()?;
            self.publish(Scenario::Actions)?;
            if !document
                .active_element()
                .is_some_and(|element| element.is_same_node(Some(&selected)))
            {
                return Err(JsValue::from_str("stable offer focus changed"));
            }
            let before = self.callbacks.get();
            selected.click();
            if self.callbacks.get() != before + 1 || self.scenario.get() != Scenario::Pending {
                return Err(JsValue::from_str(
                    "real action callback did not deliver exactly once",
                ));
            }
            if !self.last_intent.borrow().as_ref().is_some_and(|intent| {
                intent.offer == self.offer
                    && intent.option.as_ref() == Some(&self.option)
                    && intent.draft.as_deref() == Some("Keep the lantern behind me")
            }) {
                return Err(JsValue::from_str(
                    "callback selection differs from supplied offer",
                ));
            }
            selected.dispatch_event(&Event::new("click")?)?;
            if self.callbacks.get() != before + 1 {
                return Err(JsValue::from_str("pending offer submitted twice"));
            }
            self.publish(Scenario::Actions)?;
            let revision = self
                .revision
                .get()
                .next_sequence()
                .map_err(|_| JsValue::from_str("revision exhausted"))?;
            with_view(Scenario::Actions, &self.replacement, &self.option, |view| {
                self.surface.update(self.binding, revision, view)
            })
            .map_err(error)?;
            self.revision.set(revision);
            selected.click();
            if !self
                .last_intent
                .borrow()
                .as_ref()
                .is_some_and(|intent| intent.offer == self.replacement)
            {
                return Err(JsValue::from_str("changed offer kept obsolete intent"));
            }
            self.publish(Scenario::History)?;
            let after_removed = self.callbacks.get();
            selected.dispatch_event(&Event::new("click")?)?;
            if self.callbacks.get() != after_removed {
                return Err(JsValue::from_str("removed offer callback survived"));
            }
            if self
                .surface
                .root()
                .query_selector(".combat-roll button")?
                .is_some()
            {
                return Err(JsValue::from_str("past roll became interactive"));
            }
            let stale = SessionRevision::new(
                RecoveryEpoch::new(1).map_err(|_| JsValue::from_str("epoch invalid"))?,
                0,
            );
            let outcome = with_view(Scenario::Save, &self.offer, &self.option, |view| {
                self.surface.update(self.binding, stale, view)
            })
            .map_err(error)?;
            if outcome
                != (ViewAcceptance::Stale {
                    current: self.revision.get(),
                })
            {
                return Err(JsValue::from_str("stale result was accepted"));
            }
            if self
                .surface
                .root()
                .query_selector("[data-combat-offer]")?
                .is_some()
            {
                return Err(JsValue::from_str("stale offer was rendered"));
            }
            self.publish(Scenario::Reaction)?;
            self.publish(Scenario::Closed)?;
            if self
                .surface
                .root()
                .query_selector("[data-combat-offer]")?
                .is_some()
            {
                return Err(JsValue::from_str("closed reaction kept offers"));
            }
            self.publish(Scenario::Actions)?;
            if self
                .surface
                .draft_input()
                .is_none_or(|field| !field.draft().is_empty())
            {
                return Err(JsValue::from_str("removed draft scope was retained"));
            }
            self.exercise_offer_reorder()?;
            self.exercise_implicit_drop()?;
            Ok(())
        }
    }
    fn add_control(
        document: &Document,
        parent: &Element,
        fixture: &Rc<Fixture>,
        label: &str,
        operation: impl Fn(&Fixture) + 'static,
    ) -> Result<(), JsValue> {
        let control = ControlledAction::create(
            document,
            ActionView {
                label,
                enabled: true,
                pending: false,
            },
        )
        .map_err(error)?;
        let owner: Weak<Fixture> = Rc::downgrade(fixture);
        control
            .on_activate(move || {
                if let Some(owner) = owner.upgrade() {
                    operation(&owner);
                }
            })
            .map_err(error)?;
        parent.append_child(control.element())?;
        fixture.controls.borrow_mut().push(control);
        Ok(())
    }
    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), JsValue> {
        if FIXTURE.with(|slot| slot.borrow().is_some()) {
            return Err(JsValue::from_str("combat fixture already mounted"));
        }
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| JsValue::from_str("document unavailable"))?;
        let body = document
            .body()
            .ok_or_else(|| JsValue::from_str("body unavailable"))?;
        body.set_attribute("style", "margin:0;background:#070d14")?;
        document.set_title("DungeonFlux · Hold the harbor · Synthetic combat fixture");
        let binding = ClientBindingId::from_bytes(&[9; 16])
            .map_err(|_| JsValue::from_str("fixture binding invalid"))?;
        let revision = SessionRevision::new(
            RecoveryEpoch::new(1).map_err(|_| JsValue::from_str("fixture epoch invalid"))?,
            1,
        );
        let offer = RevisionLabel::new(Some("synthetic-offer-01"))
            .map_err(|_| JsValue::from_str("fixture offer invalid"))?;
        let replacement = RevisionLabel::new(Some("synthetic-offer-02"))
            .map_err(|_| JsValue::from_str("replacement offer invalid"))?;
        let option = RevisionLabel::new(Some("synthetic-target-tide-warden"))
            .map_err(|_| JsValue::from_str("fixture option invalid"))?;
        let surface = Rc::new(
            with_view(Scenario::Actions, &offer, &option, |view| {
                CombatPhaseSurface::create(
                    &document,
                    "combat-fixture-draft",
                    binding,
                    revision,
                    view,
                    limits(),
                )
            })
            .map_err(error)?,
        );
        let tools = child(
            &document,
            surface.root(),
            "details",
            "combat-fixture-tools",
            None,
        )?;
        child(
            &document,
            &tools,
            "summary",
            "",
            Some("Controlled fixture · Scenarios & lifecycle checks"),
        )?;
        let status = child(
            &document,
            &tools,
            "p",
            "combat-fixture-status",
            Some(
                "Controls below replace synthetic supplied views. Battle actions deliver intents only.",
            ),
        )?;
        status.set_attribute("role", "status")?;
        let navigation = child(&document, &tools, "nav", "", None)?;
        navigation.set_attribute("aria-label", "Synthetic combat scenarios")?;
        let fixture = Rc::new(Fixture {
            surface,
            binding,
            revision: Cell::new(revision),
            scenario: Cell::new(Scenario::Actions),
            offer,
            replacement,
            option,
            callbacks: Cell::new(0),
            last_intent: RefCell::new(None),
            controls: RefCell::new(Vec::new()),
            status,
        });
        let owner = Rc::downgrade(&fixture);
        fixture
            .surface
            .on_intent(move |intent| {
                if let Some(owner) = owner.upgrade() {
                    owner.receive(intent);
                }
            })
            .map_err(error)?;
        for (scenario, label) in [
            (Scenario::Actions, "Legal actions"),
            (Scenario::Check, "Ability check"),
            (Scenario::Save, "Saving throw"),
            (Scenario::Reaction, "Reaction open"),
            (Scenario::Closed, "Reaction closed"),
            (Scenario::Refused, "Server refusal"),
            (Scenario::History, "Past rolls only"),
        ] {
            add_control(&document, &navigation, &fixture, label, move |owner| {
                owner.report(
                    owner.publish(scenario),
                    "Synthetic snapshot supplied · No game mutation performed",
                );
            })?;
        }
        add_control(
            &document,
            &navigation,
            &fixture,
            "Exercise lifecycle",
            |owner| {
                owner.report(owner.exercise(), "PASS · 64 updates retained focus/draft; exact callbacks, changed offers, pending, refusal, stale results, removed callbacks, read-only history, keyed offer reorder/focus and implicit-drop retained draft/listener cleanup verified.");
            },
        )?;
        add_control(
            &document,
            &navigation,
            &fixture,
            "Offer reorder & focus",
            |owner| {
                owner.report(owner.exercise_offer_reorder(), "PASS · Offer order A,B → B,A retained keyed button identity, B focus and the exact B selection callback.");
            },
        )?;
        add_control(
            &document,
            &navigation,
            &fixture,
            "Implicit drop cleanup",
            |owner| {
                owner.report(owner.exercise_implicit_drop(), "PASS · Implicit drop removed the mounted surface and artwork, cleared the externally retained draft, fenced native input/click events and prevented retained draft reactivation.");
            },
        )?;
        add_control(&document, &navigation, &fixture, "Crypt artwork", |owner| {
            let result = (|| -> Result<(), JsValue> {
                let revision = owner
                    .revision
                    .get()
                    .next_sequence()
                    .map_err(|_| JsValue::from_str("fixture revision exhausted"))?;
                with_view(owner.scenario.get(), &owner.offer, &owner.option, |view| {
                    let mut art_view = *view;
                    art_view.art = CombatArt::Crypt;
                    owner.surface.update(owner.binding, revision, &art_view)
                })
                .map_err(error)?;
                owner.revision.set(revision);
                Ok(())
            })();
            owner.report(
                result,
                "Synthetic art variant supplied · The combat offers and results are unchanged",
            );
        })?;
        let terminal = document.create_element("p")?;
        terminal.set_attribute("hidden", "")?;
        terminal.set_attribute("role", "status")?;
        terminal.set_attribute(
            "style",
            "padding:36px;color:#f5eee0;font:18px Georgia,serif",
        )?;
        let terminal_status = terminal.clone();
        add_control(
            &document,
            &navigation,
            &fixture,
            "Dispose fixture",
            move |owner| {
                let result = (|| -> Result<(), JsValue> {
                    let retained = owner
                        .surface
                        .root()
                        .query_selector("[data-combat-offer=primary]")?
                        .map(|node| {
                            node.dyn_into::<HtmlButtonElement>().map_err(|_| {
                                JsValue::from_str("retained fixture node is not a button")
                            })
                        })
                        .transpose()?;
                    let before = owner.callbacks.get();
                    shutdown()?;
                    shutdown()?;
                    if let Some(retained) = retained {
                        retained.dispatch_event(&Event::new("click")?)?;
                    }
                    if owner.callbacks.get() != before
                        || owner.surface.root().parent_node().is_some()
                    {
                        return Err(JsValue::from_str("disposed surface or callback survived"));
                    }
                    terminal_status.remove_attribute("hidden")?;
                    terminal_status.set_attribute("data-combat-disposal", "pass")?;
                    terminal_status.set_text_content(Some("PASS · Combat fixture disposed twice; retained native button cannot deliver another intent."));
                    Ok(())
                })();
                if result.is_err() {
                    terminal_status.set_text_content(Some("FAIL · Combat fixture disposal failed"));
                    if terminal_status.remove_attribute("hidden").is_err() {
                        owner
                            .status
                            .set_text_content(Some("FAIL · Disposal status could not be shown"));
                    }
                }
            },
        )?;
        body.append_child(fixture.surface.root())?;
        body.append_child(&terminal)?;
        FIXTURE.with(|slot| *slot.borrow_mut() = Some(fixture));
        Ok(())
    }
    #[wasm_bindgen]
    pub fn shutdown() -> Result<(), JsValue> {
        let fixture = FIXTURE.with(|slot| slot.borrow_mut().take());
        if let Some(fixture) = fixture {
            for control in fixture.controls.borrow().iter() {
                control.dispose().map_err(error)?;
            }
            fixture.surface.dispose().map_err(error)?;
            fixture.surface.dispose().map_err(error)?;
        }
        Ok(())
    }
}
