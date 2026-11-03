#[cfg(target_arch = "wasm32")]
mod browser {
    use df_ui::{
        ActionView, CampaignLimits, CampaignMember, CampaignObjective, CampaignSurface,
        CampaignView, ConceptScene, ControlledAction, ObjectiveState,
    };
    use std::{cell::RefCell, rc::Rc};
    use wasm_bindgen::{JsValue, prelude::wasm_bindgen};
    use web_sys::{Document, Element};

    const MEMBERS: [CampaignMember<'static>; 3] = [
        CampaignMember {
            key: "corin",
            name: "Corin Vale",
            role: "Human · Fighter",
            sigil: "CV",
        },
        CampaignMember {
            key: "lyria",
            name: "Lyria Thalen",
            role: "Elf · Rogue",
            sigil: "LT",
        },
        CampaignMember {
            key: "brom",
            name: "Brom Stonebeard",
            role: "Dwarf · Barbarian",
            sigil: "BS",
        },
    ];
    const OBJECTIVES: [CampaignObjective<'static>; 3] = [
        CampaignObjective {
            label: "Arrive at the Drowned Lantern",
            state: ObjectiveState::Active,
        },
        CampaignObjective {
            label: "Learn about the missing lamplighter",
            state: ObjectiveState::Pending,
        },
        CampaignObjective {
            label: "Follow the light beneath the harbor",
            state: ObjectiveState::Pending,
        },
    ];
    struct Fixture {
        surface: Rc<CampaignSurface>,
        controls: Vec<Rc<ControlledAction>>,
    }
    thread_local! { static FIXTURE:RefCell<Option<Fixture>>=const {RefCell::new(None)}; }
    fn error(e: impl std::fmt::Display) -> JsValue {
        JsValue::from_str(&e.to_string())
    }
    fn campaign_view(scene: ConceptScene, include_members: bool) -> CampaignView<'static> {
        let (chapter, title, description, location, scene_label, narration) = match scene {
            ConceptScene::Harbor | ConceptScene::MaraHarbor => (
                "Act I · The Lamplighter",
                "The Drowned Lantern",
                "A flooded town. A missing lamplighter. And questions that don’t like the light.",
                "Greyhaven · The old harbor",
                "A town beneath the tide",
                "Rain draws silver lines across the harbor. Beyond the stone bridge, a lone lantern burns in the window of a tavern. Someone inside has been waiting for you.",
            ),
            ConceptScene::LobbyHarbor => (
                "Act I Â· The Lamplighter",
                "The waiting harbor",
                "A rain-soaked harbor. A tavern light. A quiet welcome to Greyhaven.",
                "Greyhaven Â· The old harbor",
                "Lanterns across the harbor",
                "Rain beads on the wharf stones while a lantern glows beyond the bridge.",
            ),
            ConceptScene::WharfArrival => (
                "Act I Â· The Lamplighter",
                "A broken seal",
                "A hooded courier waits with a packet whose seal has been broken.",
                "Greyhaven Â· The old wharf",
                "The courier's arrival",
                "A hooded courier holds the packet close as rain sweeps across the wharf.",
            ),
            ConceptScene::WhisperInTheRain => (
                "Act I Â· The Lamplighter",
                "A warning in the rain",
                "A whispered warning reaches the party on the rain-soaked wharf.",
                "Greyhaven Â· The old wharf",
                "The whispered warning",
                "The rain blurs the harbor lights as a warning passes between companions.",
            ),
            ConceptScene::CourierRoad => (
                "Act I Â· Beyond the harbor",
                "The road after the fight",
                "The party leaves the harbor behind and follows the courier's road.",
                "Greyhaven Â· The lantern road",
                "The road beyond the harbor",
                "A lantern marks the road ahead as the harbor falls behind the party.",
            ),
            ConceptScene::Tavern => (
                "Act I · The Lamplighter",
                "Salt, smoke & secrets",
                "Pull up a chair. Every stranger here has a story. Not every story wants to be told.",
                "Greyhaven · The Drowned Lantern",
                "At the barkeeper’s table",
                "Rain drums against the warped roof. Vell sets down a glass and lowers his voice. ‘You’re asking about the lamplighter? Best keep that question away from the windows.’",
            ),
            ConceptScene::Mountain => (
                "Act II · Beyond the harbor",
                "Where the old gods sleep",
                "Above the clouds, the mountain keeps a thousand years of silence.",
                "The northern ridge · Forgotten ruins",
                "The road into the snow",
                "The last lights of Greyhaven disappear behind you. Ahead, broken pillars rise from the snow. The wind carries a sound that might be a voice, or might be something older.",
            ),
            ConceptScene::SunkenHall => (
                "Act I · Beneath Greyhaven",
                "The sunken halls",
                "A flame in the darkness. A door beneath the water. A promise waiting to be broken.",
                "Below the harbor · The drowned temple",
                "Beyond the waterline",
                "Your torch catches the edge of a carved archway. Water trembles around your boots. Far inside the hall, something answers the sound of your footsteps.",
            ),
            ConceptScene::Campfire => (
                "Interlude · The long road",
                "Under the same stars",
                "The world can wait until morning. Tonight, there is a fire, and there are friends.",
                "The old road · A sheltered clearing",
                "A moment between adventures",
                "Sparks drift into a sky full of stars. For a little while, no one speaks of the dark water or the missing lantern. The fire is warm, and the road can wait.",
            ),
        };
        CampaignView {
            scene,
            chapter,
            title,
            description,
            location,
            scene_label,
            narration,
            connection: "Visual design preview · Synthetic party and story · No gameplay session connected",
            notice: "Design preview · Original concept art",
            members: if include_members { &MEMBERS } else { &[] },
            objectives: &OBJECTIVES,
        }
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
    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), JsValue> {
        if FIXTURE.with(|slot| slot.borrow().is_some()) {
            return Err(JsValue::from_str("preview already mounted"));
        }
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| JsValue::from_str("browser document unavailable"))?;
        let body = document
            .body()
            .ok_or_else(|| JsValue::from_str("browser body unavailable"))?;
        body.set_attribute("style", "margin:0;background:#070d14")?;
        document.set_title("DungeonFlux · The Drowned Lantern · Design preview");
        let surface = Rc::new(
            CampaignSurface::create(
                &document,
                &campaign_view(ConceptScene::Harbor, true),
                CampaignLimits {
                    max_members: 128,
                    max_objectives: 128,
                    max_text_bytes: 4096,
                },
            )
            .map_err(error)?,
        );
        let mut controls = Vec::new();
        for (scene, label) in [
            (ConceptScene::Harbor, "Harbor"),
            (ConceptScene::Tavern, "Tavern"),
            (ConceptScene::Mountain, "Mountain"),
            (ConceptScene::SunkenHall, "Sunken hall"),
            (ConceptScene::Campfire, "Campfire"),
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
                .map_err(error)?,
            );
            control.element().set_attribute(
                "aria-pressed",
                if scene == ConceptScene::Harbor {
                    "true"
                } else {
                    "false"
                },
            )?;
            surface.navigation().append_child(control.element())?;
            controls.push(control);
        }
        for (index, scene) in [
            ConceptScene::Harbor,
            ConceptScene::Tavern,
            ConceptScene::Mountain,
            ConceptScene::SunkenHall,
            ConceptScene::Campfire,
        ]
        .into_iter()
        .enumerate()
        {
            let Some(control) = controls.get(index) else {
                return Err(JsValue::from_str("navigation control unavailable"));
            };
            let owner = Rc::clone(&surface);
            let buttons: Vec<_> = controls.iter().map(|item| item.element().clone()).collect();
            control
                .on_activate(move || {
                    let result = (|| -> Result<(), JsValue> {
                        owner.update(&campaign_view(scene, true)).map_err(error)?;
                        for (position, button) in buttons.iter().enumerate() {
                            button.set_attribute(
                                "aria-pressed",
                                if position == index { "true" } else { "false" },
                            )?;
                        }
                        owner
                            .root()
                            .set_attribute("data-preview-state", scene.description())?;
                        Ok(())
                    })();
                    if result.is_err()
                        && let Ok(Some(notice)) = owner.root().query_selector(".preview-badge")
                    {
                        notice.set_text_content(Some(
                            "Presentation update failed · Preview controls retained",
                        ));
                    }
                })
                .map_err(error)?;
        }
        let checks = child(&document, surface.root(), "details", "art-gallery", None)?;
        child(
            &document,
            &checks,
            "summary",
            "",
            Some("Presentation checks"),
        )?;
        let status = child(
            &document,
            &checks,
            "p",
            "connection",
            Some("Local component checks; these do not exercise gameplay."),
        )?;
        let repeat = Rc::new(
            ControlledAction::create(
                &document,
                ActionView {
                    label: "Exercise 64 mounted updates",
                    enabled: true,
                    pending: false,
                },
            )
            .map_err(error)?,
        );
        let repeated = Rc::clone(&surface);
        let repeated_document = document.clone();
        let repeated_button = repeat.element().clone();
        let repeated_navigation: Vec<_> =
            controls.iter().map(|item| item.element().clone()).collect();
        repeat
            .on_activate(move || {
                let result = (|| -> Result<(), JsValue> {
                    repeated_button.focus()?;
                    let before = repeated_document.active_element();
                    let title_before = repeated.root().query_selector("h1")?;
                    for turn in 0..64 {
                        repeated
                            .update(&campaign_view(
                                if turn % 2 == 0 {
                                    ConceptScene::Tavern
                                } else {
                                    ConceptScene::Harbor
                                },
                                true,
                            ))
                            .map_err(error)?;
                    }
                    for (index, button) in repeated_navigation.iter().enumerate() {
                        button.set_attribute(
                            "aria-pressed",
                            if index == 0 { "true" } else { "false" },
                        )?;
                    }
                    repeated
                        .root()
                        .set_attribute("data-preview-state", ConceptScene::Harbor.description())?;
                    let focus_retained = before.as_ref().is_some_and(|node| {
                        repeated_document
                            .active_element()
                            .as_ref()
                            .is_some_and(|after| node.is_same_node(Some(after)))
                    });
                    let title_retained = title_before.as_ref().is_some_and(|node| {
                        repeated
                            .root()
                            .query_selector("h1")
                            .ok()
                            .flatten()
                            .as_ref()
                            .is_some_and(|after| node.is_same_node(Some(after)))
                    });
                    if !focus_retained || !title_retained {
                        return Err(JsValue::from_str("mounted node or focus changed"));
                    }
                    Ok(())
                })();
                status.set_text_content(Some(if result.is_ok() {
                    "PASS · 64 updates retained the mounted heading and focused button."
                } else {
                    "FAIL · mounted update or focus check failed."
                }));
            })
            .map_err(error)?;
        checks.append_child(repeat.element())?;
        controls.push(repeat);
        let terminal = document.create_element("p")?;
        terminal.set_attribute("hidden", "")?;
        terminal.set_attribute("role", "status")?;
        terminal.set_attribute("data-preview-disposal", "pending")?;
        terminal.set_attribute(
            "style",
            "color:#f5eee0;padding:40px;font:18px Georgia,serif",
        )?;
        let dispose = Rc::new(
            ControlledAction::create(
                &document,
                ActionView {
                    label: "Dispose preview",
                    enabled: true,
                    pending: false,
                },
            )
            .map_err(error)?,
        );
        let terminal_status = terminal.clone();
        let terminal_document = document.clone();
        dispose.on_activate(move || {
            let result = (|| -> Result<(), JsValue> {
                shutdown()?;
                shutdown()?;
                if terminal_document.query_selector(".df-campaign")?.is_some()
                    || FIXTURE.with(|slot| slot.borrow().is_some())
                {
                    return Err(JsValue::from_str("preview scope remains after disposal"));
                }
                terminal_status.remove_attribute("hidden")?;
                terminal_status.set_attribute("data-preview-disposal", "pass")?;
                terminal_status.set_text_content(Some("PASS · Preview disposed twice. The mounted surface and owned controls were removed."));
                Ok(())
            })();
            if result.is_err() {
                terminal_status.set_text_content(Some("FAIL · Preview disposal did not complete."));
                if terminal_status.remove_attribute("hidden").is_err() {
                    terminal_status.set_text_content(Some("FAIL · Preview disposal status could not be shown."));
                }
            }
        }).map_err(error)?;
        checks.append_child(dispose.element())?;
        controls.push(dispose);
        body.append_child(surface.root())?;
        body.append_child(&terminal)?;
        FIXTURE.with(|slot| *slot.borrow_mut() = Some(Fixture { surface, controls }));
        Ok(())
    }
    #[wasm_bindgen]
    pub fn shutdown() -> Result<(), JsValue> {
        FIXTURE.with(|slot| {
            let mut owned = slot.borrow_mut();
            if let Some(fixture) = owned.as_ref() {
                for control in &fixture.controls {
                    control.dispose().map_err(error)?;
                }
                fixture.surface.dispose().map_err(error)?;
                fixture.surface.dispose().map_err(error)?;
            }
            *owned = None;
            Ok(())
        })
    }
}
