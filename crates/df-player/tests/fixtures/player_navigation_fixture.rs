//! Executable navigation contract over the existing mounted player owner.
//! Synthetic permitted presentation only; native browser input is observed separately.
#![cfg(any(target_arch = "wasm32", test))]

use df_ui::{
    CampaignLimits, CampaignView, ConceptScene, JoinFeedback, JoinPhaseView, JoinStage, JoinStamp,
    LobbyOffer,
};

#[derive(Clone, Copy)]
enum Scenario {
    Invitation,
    Pending,
    Rejected,
    Lobby,
}

fn limits() -> CampaignLimits {
    CampaignLimits {
        max_members: 8,
        max_objectives: 8,
        max_text_bytes: 4096,
    }
}

fn view(stamp: JoinStamp, scenario: Scenario) -> JoinPhaseView<'static> {
    let lobby = matches!(scenario, Scenario::Lobby);
    JoinPhaseView {
        stamp,
        stage: if lobby {
            JoinStage::Lobby
        } else {
            JoinStage::Invitation
        },
        campaign: CampaignView {
            scene: ConceptScene::Harbor,
            chapter: "Navigation contract",
            title: "The Drowned Lantern",
            description: "Your input remains mounted while permitted views change.",
            location: "Greyhaven",
            scene_label: "Harbor",
            narration: "Synthetic private player presentation",
            connection: "Browser fixture · no live session",
            notice: "Native input · intentions only",
            members: &[],
            objectives: &[],
        },
        panel_heading: if lobby {
            "Your party awaits"
        } else {
            "Join the story"
        },
        panel_description: "Use Tab and Shift+Tab to navigate; Enter, Space or pointer to activate.",
        invitation_label: "Invitation",
        name_label: "Player name",
        join_label: "Send join intention",
        join_enabled: true,
        feedback: match scenario {
            Scenario::Pending => JoinFeedback::Pending("Waiting for the server"),
            Scenario::Rejected => JoinFeedback::Rejected("Invitation rejected; edit your draft"),
            _ => JoinFeedback::None,
        },
        room: None,
        roster_heading: "Party",
        empty_roster: "No supplied participants",
        participants: &[],
        lobby_offer: lobby.then_some(LobbyOffer {
            key: "fixture-ready-offer",
            label: "Send readiness intention",
            enabled: true,
            pending: false,
        }),
        pairing_fallback: "Public pairing unavailable in this navigation fixture",
    }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use df_player::{PlayerJoinConnection, PlayerJoinInput, PlayerJoinScreen};
    use df_ui::{ActionView, ControlledAction, JoinPhaseIntent, JoinUpdate};
    use std::{cell::RefCell, rc::Rc};
    use wasm_bindgen::{JsCast, prelude::*};
    use web_sys::{Document, Element, HtmlButtonElement, HtmlInputElement, Node};

    const MAX_INTENTS: u32 = 256;
    #[derive(Default)]
    struct Intents {
        count: u32,
        last: Option<JoinPhaseIntent>,
        capacity_failed: bool,
    }
    struct Fixture {
        screen: PlayerJoinScreen,
        stamp: JoinStamp,
        scenario: Scenario,
        document: Document,
        intents: Rc<RefCell<Intents>>,
        controls: Vec<ControlledAction>,
    }
    #[derive(PartialEq)]
    struct Selection {
        start: Option<u32>,
        end: Option<u32>,
        direction: Option<String>,
    }
    struct Checkpoint {
        root: Element,
        invitation: HtmlInputElement,
        name: HtmlInputElement,
        invitation_value: String,
        name_value: String,
        active: Option<Element>,
        selection: Selection,
    }
    thread_local! {
        static FIXTURE: RefCell<Option<Fixture>> = const { RefCell::new(None) };
    }
    fn error(value: impl std::fmt::Display) -> JsValue {
        JsValue::from_str(&value.to_string())
    }
    fn check(condition: bool, message: &str) -> Result<(), JsValue> {
        if condition {
            Ok(())
        } else {
            Err(JsValue::from_str(message))
        }
    }
    fn required(root: &Element, selector: &str) -> Result<Element, JsValue> {
        root.query_selector(selector)?
            .ok_or_else(|| JsValue::from_str("fixture node unavailable"))
    }
    fn field(root: &Element, suffix: &str) -> Result<HtmlInputElement, JsValue> {
        let feedback = required(root, &format!(".df-ui-feedback[id$='-{suffix}-feedback']"))?;
        let container = feedback
            .parent_element()
            .ok_or_else(|| JsValue::from_str("field unavailable"))?;
        required(&container, "label input")?
            .dyn_into()
            .map_err(|_| JsValue::from_str("input type unavailable"))
    }
    fn selection(input: &HtmlInputElement) -> Result<Selection, JsValue> {
        Ok(Selection {
            start: input.selection_start()?,
            end: input.selection_end()?,
            direction: input.selection_direction()?,
        })
    }
    fn checkpoint(fixture: &Fixture) -> Result<Checkpoint, JsValue> {
        let invitation = field(fixture.screen.root(), "invitation")?;
        let name = field(fixture.screen.root(), "name")?;
        Ok(Checkpoint {
            root: fixture.screen.root().clone(),
            invitation_value: invitation.value(),
            name_value: name.value(),
            selection: selection(&name)?,
            active: fixture.document.active_element(),
            invitation,
            name,
        })
    }
    fn retained(fixture: &Fixture, before: &Checkpoint) -> Result<(), JsValue> {
        check(
            before.root.is_same_node(Some(fixture.screen.root())),
            "root remounted",
        )?;
        let invitation = field(fixture.screen.root(), "invitation")?;
        let name = field(fixture.screen.root(), "name")?;
        check(
            before.invitation.is_same_node(Some(invitation.as_ref())),
            "invitation remounted",
        )?;
        check(
            before.name.is_same_node(Some(name.as_ref())),
            "name remounted",
        )?;
        check(
            before.invitation_value == before.invitation.value()
                && before.name_value == before.name.value(),
            "draft changed",
        )?;
        check(
            selection(&before.name)? == before.selection,
            "selection changed",
        )?;
        match (&before.active, fixture.document.active_element()) {
            (Some(old), Some(current)) => check(old.is_same_node(Some(&current)), "focus changed"),
            (None, None) => Ok(()),
            _ => Err(JsValue::from_str("focus changed")),
        }
    }
    fn advance(fixture: &mut Fixture, scenario: Scenario) -> Result<(), JsValue> {
        fixture.stamp.sequence = fixture
            .stamp
            .sequence
            .checked_add(1)
            .ok_or_else(|| JsValue::from_str("fixture sequence exhausted"))?;
        check(
            fixture
                .screen
                .update(&view(fixture.stamp, scenario))
                .map_err(error)?
                == JoinUpdate::Applied,
            "current view refused",
        )?;
        fixture.scenario = scenario;
        Ok(())
    }
    fn with_fixture<T>(
        action: impl FnOnce(&mut Fixture) -> Result<T, JsValue>,
    ) -> Result<T, JsValue> {
        FIXTURE.with(|slot| {
            let mut owner = slot.borrow_mut();
            action(
                owner
                    .as_mut()
                    .ok_or_else(|| JsValue::from_str("fixture retired"))?,
            )
        })
    }
    fn publish(result: &Result<(), JsValue>) {
        if let Some(document) = web_sys::window().and_then(|window| window.document())
            && let Some(status) = document.get_element_by_id("navigation-status")
        {
            status.set_text_content(Some(if result.is_ok() {
                "PASS · Mounted contract probe completed; native keyboard/device observation is separate"
            } else { "FAIL · Mounted navigation contract probe" }));
        }
    }
    /// Updates while leaving native focus untouched. Call after real browser typing/selection.
    #[wasm_bindgen]
    pub fn fixture_same_phase() -> Result<(), JsValue> {
        with_fixture(|fixture| {
            let before = checkpoint(fixture)?;
            let count = fixture.intents.borrow().count;
            let scenario = fixture.scenario;
            for _ in 0..64 {
                advance(fixture, scenario)?;
            }
            retained(fixture, &before)?;
            check(
                fixture.intents.borrow().count == count,
                "update emitted intention",
            )
        })
    }
    #[wasm_bindgen]
    pub fn fixture_pending() -> Result<(), JsValue> {
        with_fixture(|fixture| {
            let before = checkpoint(fixture)?;
            advance(fixture, Scenario::Pending)?;
            retained(fixture, &before)?;
            check(
                before.name.read_only() && before.invitation.read_only(),
                "pending input editable",
            )
        })
    }
    #[wasm_bindgen]
    pub fn fixture_rejected() -> Result<(), JsValue> {
        with_fixture(|fixture| {
            let before = checkpoint(fixture)?;
            advance(fixture, Scenario::Rejected)?;
            retained(fixture, &before)?;
            check(
                !before.name.read_only() && !before.invitation.read_only(),
                "rejected draft unavailable",
            )
        })
    }
    #[wasm_bindgen]
    pub fn fixture_reconnect() -> Result<(), JsValue> {
        with_fixture(|fixture| {
            let before = checkpoint(fixture)?;
            let count = fixture.intents.borrow().count;
            fixture
                .screen
                .set_connection(PlayerJoinConnection::Offline)
                .map_err(error)?;
            fixture
                .screen
                .set_connection(PlayerJoinConnection::Reconnecting)
                .map_err(error)?;
            // Synthetic activation proves the refusal fence only, never native keyboard behavior.
            required(fixture.screen.root(), ".join-fields button")?
                .dyn_into::<HtmlButtonElement>()
                .map_err(|_| JsValue::from_str("action unavailable"))?
                .click();
            check(
                fixture.intents.borrow().count == count,
                "disconnected intention escaped",
            )?;
            fixture
                .screen
                .set_connection(PlayerJoinConnection::Connected)
                .map_err(error)?;
            retained(fixture, &before)
        })
    }
    #[wasm_bindgen]
    pub fn fixture_refusals() -> Result<(), JsValue> {
        with_fixture(|fixture| {
            let before = checkpoint(fixture)?;
            let count = fixture.intents.borrow().count;
            check(
                fixture
                    .screen
                    .update(&view(fixture.stamp, Scenario::Lobby))
                    .map_err(error)?
                    == JoinUpdate::StaleSequence,
                "stale sequence applied",
            )?;
            let stale = JoinStamp {
                generation: fixture.stamp.generation - 1,
                sequence: u64::MAX,
            };
            check(
                fixture
                    .screen
                    .update(&view(stale, Scenario::Lobby))
                    .map_err(error)?
                    == JoinUpdate::StaleGeneration,
                "stale generation applied",
            )?;
            let mut invalid = view(
                JoinStamp {
                    generation: fixture.stamp.generation,
                    sequence: fixture
                        .stamp
                        .sequence
                        .checked_add(1)
                        .ok_or_else(|| JsValue::from_str("fixture sequence exhausted"))?,
                },
                Scenario::Invitation,
            );
            invalid.panel_heading = "";
            check(
                fixture.screen.update(&invalid).is_err(),
                "invalid view applied",
            )?;
            retained(fixture, &before)?;
            check(
                fixture.intents.borrow().count == count,
                "refusal emitted intention",
            )
        })
    }
    /// Call while a join input/action is actually focused; current owner restores visible heading.
    #[wasm_bindgen]
    pub fn fixture_lobby() -> Result<(), JsValue> {
        with_fixture(|fixture| {
            let form = required(fixture.screen.root(), ".join-fields")?;
            let was_inside = fixture
                .document
                .active_element()
                .as_ref()
                .is_some_and(|active| form.contains(Some(active)));
            advance(fixture, Scenario::Lobby)?;
            check(form.has_attribute("hidden"), "retired stage visible")?;
            if was_inside {
                let heading = required(fixture.screen.root(), ".join-heading")?;
                check(
                    fixture
                        .document
                        .active_element()
                        .as_ref()
                        .is_some_and(|active| heading.is_same_node(Some(active))),
                    "stage focus stranded",
                )?;
            }
            Ok(())
        })
    }
    #[wasm_bindgen]
    pub fn fixture_replace_owner() -> Result<(), JsValue> {
        with_fixture(|fixture| {
            let before = checkpoint(fixture)?;
            let old_stamp = fixture.stamp;
            fixture.stamp = JoinStamp {
                generation: old_stamp
                    .generation
                    .checked_add(1)
                    .ok_or_else(|| JsValue::from_str("fixture generation exhausted"))?,
                sequence: 1,
            };
            fixture
                .screen
                .replace_owner(&view(fixture.stamp, Scenario::Invitation))
                .map_err(error)?;
            fixture.scenario = Scenario::Invitation;
            fixture.intents.borrow_mut().last = None;
            check(
                before.root.is_same_node(Some(fixture.screen.root())),
                "owner replacement remounted shell",
            )?;
            check(
                before.invitation.value().is_empty() && before.name.value().is_empty(),
                "obsolete private draft retained",
            )?;
            check(
                fixture
                    .screen
                    .update(&view(old_stamp, Scenario::Lobby))
                    .map_err(error)?
                    == JoinUpdate::StaleGeneration,
                "retired owner returned",
            )
        })
    }
    /// Count is bounded and contains no private draft text. Keyboard evidence requires real browser input.
    #[wasm_bindgen]
    pub fn fixture_intent_count() -> Result<u32, JsValue> {
        with_fixture(|fixture| {
            let record = fixture.intents.borrow();
            check(!record.capacity_failed, "fixture intent capacity exceeded")?;
            Ok(record.count)
        })
    }
    #[wasm_bindgen]
    pub fn fixture_last_intent_is_current() -> Result<bool, JsValue> {
        with_fixture(|fixture| {
            let expected = match fixture.scenario {
                Scenario::Lobby => JoinPhaseIntent::LobbyAction {
                    stamp: fixture.stamp,
                    offer_key: "fixture-ready-offer".into(),
                },
                _ => JoinPhaseIntent::Join {
                    stamp: fixture.stamp,
                    invitation: field(fixture.screen.root(), "invitation")?.value(),
                    player_name: field(fixture.screen.root(), "name")?.value(),
                },
            };
            Ok(fixture.intents.borrow().last.as_ref() == Some(&expected))
        })
    }
    fn collect(node: &Node, nodes: &mut Vec<Node>) {
        nodes.push(node.clone());
        let mut child = node.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            collect(&current, nodes);
        }
    }
    #[wasm_bindgen]
    pub fn fixture_dispose() -> Result<(), JsValue> {
        let owner = FIXTURE.with(|slot| slot.borrow_mut().take());
        if let Some(mut fixture) = owner {
            let before = checkpoint(&fixture)?;
            let action = required(fixture.screen.root(), ".join-fields button")?
                .dyn_into::<HtmlButtonElement>()
                .map_err(|_| JsValue::from_str("action unavailable"))?;
            let count = fixture.intents.borrow().count;
            let mut nodes = Vec::new();
            collect(fixture.screen.root().as_ref(), &mut nodes);
            let mut failure = None;
            for control in &fixture.controls {
                if let Err(value) = control.dispose() {
                    failure.get_or_insert(error(value));
                }
            }
            for _ in 0..2 {
                if let Err(value) = fixture.screen.revoke() {
                    failure.get_or_insert(error(value));
                }
            }
            action.click();
            if let Some(value) = failure {
                return Err(value);
            }
            check(
                count == fixture.intents.borrow().count,
                "retired intention escaped",
            )?;
            check(
                before.invitation.value().is_empty() && before.name.value().is_empty(),
                "disposed private draft retained",
            )?;
            check(!before.root.is_connected(), "disposed root connected")?;
            for node in nodes {
                if node.node_type() == Node::TEXT_NODE {
                    check(
                        node.node_value().as_deref() == Some(""),
                        "disposed text retained",
                    )?;
                }
            }
        }
        Ok(())
    }
    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), JsValue> {
        check(
            !FIXTURE.with(|slot| slot.borrow().is_some()),
            "fixture already mounted",
        )?;
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| JsValue::from_str("document unavailable"))?;
        let body = document
            .body()
            .ok_or_else(|| JsValue::from_str("body unavailable"))?;
        document.set_title("DungeonFlux · Player navigation contract");
        let slot = document.create_element("main")?;
        slot.set_id("navigation-player");
        slot.set_attribute("aria-label", "Player join navigation")?;
        body.append_child(&slot)?;
        let status = document.create_element("p")?;
        status.set_id("navigation-status");
        status.set_attribute("role", "status")?;
        status.set_attribute("aria-live", "polite")?;
        status.set_text_content(Some(
            "Synthetic navigation fixture · actual keyboard observation pending",
        ));
        let toolbar = document.create_element("nav")?;
        toolbar.set_attribute("aria-label", "Navigation fixture scenarios")?;
        toolbar.set_attribute("style", "padding:16px;display:flex;flex-wrap:wrap;gap:12px")?;
        body.append_child(&status)?;
        body.append_child(&toolbar)?;
        let style = document.create_element("style")?;
        style.set_text_content(Some(r#"#navigation-status{padding:16px;font:16px/1.6 system-ui}nav[aria-label="Navigation fixture scenarios"] button{min-height:44px;min-width:44px;padding:10px 16px}nav[aria-label="Navigation fixture scenarios"] button:focus-visible{outline:2px solid #1577ae;outline-offset:3px}"#));
        toolbar.append_child(&style)?;
        let stamp = JoinStamp {
            generation: 1,
            sequence: 1,
        };
        let intents = Rc::new(RefCell::new(Intents::default()));
        let observed = Rc::clone(&intents);
        let screen = PlayerJoinScreen::mount(
            &document,
            &slot,
            &view(stamp, Scenario::Invitation),
            PlayerJoinInput {
                identifier: "navigation",
                invitation: "",
                player_name: "",
            },
            limits(),
            PlayerJoinConnection::Connected,
            move |intent| {
                let mut record = observed.borrow_mut();
                if record.count >= MAX_INTENTS {
                    record.capacity_failed = true;
                    return;
                }
                record.count = match record.count.checked_add(1) {
                    Some(count) => count,
                    None => {
                        record.capacity_failed = true;
                        return;
                    }
                };
                record.last = Some(intent);
            },
        )
        .map_err(error)?;
        let mut fixture = Fixture {
            screen,
            stamp,
            scenario: Scenario::Invitation,
            document,
            intents,
            controls: Vec::new(),
        };
        type Probe = fn() -> Result<(), JsValue>;
        for (label, probe) in [
            ("64 same-phase updates", fixture_same_phase as Probe),
            ("Pending draft", fixture_pending),
            ("Rejected draft", fixture_rejected),
            ("Reconnect", fixture_reconnect),
            ("Refuse stale and invalid views", fixture_refusals),
            ("Current lobby stage", fixture_lobby),
            ("Replace private owner", fixture_replace_owner),
            ("Dispose", fixture_dispose),
        ] {
            let control = ControlledAction::create(
                &fixture.document,
                ActionView {
                    label,
                    enabled: true,
                    pending: false,
                },
            )
            .map_err(error)?;
            control
                .on_activate(move || publish(&probe()))
                .map_err(error)?;
            toolbar.append_child(control.element())?;
            fixture.controls.push(control);
        }
        FIXTURE.with(|slot| *slot.borrow_mut() = Some(fixture));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use df_ui::{
        JoinPhaseIntent, JoinValidationError, MAX_DRAFT_UTF16_UNITS, validate_join_drafts,
    };

    #[test]
    fn reachable_scenarios_use_the_actual_bounded_join_contract() {
        let stamp = JoinStamp {
            generation: 1,
            sequence: 1,
        };
        for scenario in [
            Scenario::Invitation,
            Scenario::Pending,
            Scenario::Rejected,
            Scenario::Lobby,
        ] {
            assert_eq!(view(stamp, scenario).validate(limits()), Ok(()));
        }
        let mut invalid = view(stamp, Scenario::Invitation);
        invalid.lobby_offer = view(stamp, Scenario::Lobby).lobby_offer;
        assert_eq!(
            invalid.validate(limits()),
            Err(JoinValidationError::InvalidStage)
        );
    }

    #[test]
    fn private_literal_drafts_keep_the_canonical_utf16_capacity_boundary() {
        let exact = "😀".repeat(MAX_DRAFT_UTF16_UNITS / 2);
        assert_eq!(validate_join_drafts(&exact, "<Mira & Aster>"), Ok(()));
        assert_eq!(
            validate_join_drafts(&(exact + "a"), ""),
            Err(JoinValidationError::DraftTooLong)
        );
        assert_eq!(
            validate_join_drafts("opaque?invitation=server-owned", ""),
            Ok(())
        );
    }

    #[test]
    fn navigation_intention_is_not_a_confirmation_or_locally_resolved_action() {
        let stamp = JoinStamp {
            generation: 7,
            sequence: 9,
        };
        let current = view(stamp, Scenario::Lobby);
        let supplied = current
            .lobby_offer
            .as_ref()
            .expect("fixture supplies a bounded offer");
        let intent = JoinPhaseIntent::LobbyAction {
            stamp,
            offer_key: supplied.key.into(),
        };
        assert_eq!(
            intent,
            JoinPhaseIntent::LobbyAction {
                stamp: current.stamp,
                offer_key: "fixture-ready-offer".into(),
            }
        );
        let invitation = view(stamp, Scenario::Invitation);
        assert!(invitation.lobby_offer.is_none());
    }
}
