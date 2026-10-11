//! Authored presentation preview, not a gameplay server or an authoritative session.
//! The finite scene script supplies separate role views and never resolves rules,
//! creates receipts, persists a character, or dispatches media providers.

#[cfg(target_arch = "wasm32")]
mod gameplay_scene_art_catalog;

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::gameplay_scene_art_catalog::GameplaySceneArt;
    use std::cell::RefCell;

    use df_client::revisions::ViewAcceptance;
    use df_types::{ClientBindingId, RecoveryEpoch, RevisionLabel, SessionRevision};
    use df_ui::{
        ActionView, AftermathNextScene, AftermathPartyMember, CampaignLimits, CampaignMember,
        CampaignView, CampfireAction, CampfireMember, CampfireOffer, CampfireView,
        CharacterSheetView, CombatActor, CombatArt, CombatFeedback, CombatLimits, CombatOffer,
        CombatOfferKind, CombatPhaseView, ConceptScene, ControlledAction, EncounterAftermathView,
        ExplorationChoice, ExplorationInput, ExplorationLimits, ExplorationNpc,
        ExplorationPortrait, ExplorationView, SceneTransitionView, SessionConnection, SheetField,
        SheetLabels, SheetOwnerGeneration, SheetRow, SheetSection, SheetTab,
    };
    use df_web::{DisplayPhase, PlayerPhase, RoleInput, RolePhase, RoleShell};
    use wasm_bindgen::{JsValue, prelude::wasm_bindgen};
    use web_sys::{Document, Element};

    const NOTICE: &str = "Interactive scene preview · authored views · production gameplay pending";
    const PRIVATE_NOTE: &str =
        "Mara's private journal: the wax seal matches your old harbor letter.";
    const HARBOR_CHOICES: [ExplorationChoice<'static>; 2] = [
        ExplorationChoice {
            id: "preview:talk-vell",
            label: "Speak with Vell",
            detail: "Ask the dockkeeper what happened before the lanterns went out.",
            disabled_reason: None,
        },
        ExplorationChoice {
            id: "preview:inspect-seal",
            label: "Inspect the broken seal",
            detail: "Study the wax fragment caught between the wet planks.",
            disabled_reason: None,
        },
    ];
    const STORY_CHOICES: [ExplorationChoice<'static>; 2] = [
        ExplorationChoice {
            id: "preview:inspect-seal",
            label: "Examine the clue",
            detail: "A lantern-shaped impression points toward the abandoned loading pier.",
            disabled_reason: None,
        },
        ExplorationChoice {
            id: "preview:enter-encounter",
            label: "Approach the loading pier",
            detail: "Continue this authored scene to its dockside confrontation.",
            disabled_reason: None,
        },
    ];
    const PARTY: [CampaignMember<'static>; 1] = [CampaignMember {
        key: "mara",
        name: "Mara",
        role: "Your company · harbor traveler",
        sigil: "M",
    }];
    const ACTORS: [CombatActor<'static>; 2] = [
        CombatActor {
            key: "mara",
            name: "Mara",
            initiative: "Current preview turn",
            status: "Ready for your intent",
            resources: &[],
            hit_points: None,
            portrait: None,
        },
        CombatActor {
            key: "lookout",
            name: "Dockside lookout",
            initiative: "Waiting",
            status: "Beside the sealed crate",
            resources: &[],
            hit_points: None,
            portrait: None,
        },
    ];
    const FIELDS: [SheetField<'static>; 2] = [
        SheetField {
            label: "Appearance",
            value: "Silver braid · navy coat · scar over the left eye",
        },
        SheetField {
            label: "Origin",
            value: "Greyhaven harbor",
        },
    ];
    const CHARACTER_ROWS: [SheetRow<'static>; 1] = [SheetRow {
        key: "mara-identity",
        title: "Mara",
        value: "Harbor traveler",
        summary: "A familiar face beneath unfamiliar lanterns.",
        details: &FIELDS,
        offers: &[],
    }];
    const EQUIPMENT_ROWS: [SheetRow<'static>; 1] = [SheetRow {
        key: "lantern",
        title: "Storm lantern",
        value: "Carried",
        summary: "A brass lantern for the fogbound quay.",
        details: &[],
        offers: &[],
    }];
    const JOURNAL_ROWS: [SheetRow<'static>; 1] = [SheetRow {
        key: "private-letter",
        title: "The harbor letter",
        value: "Personal note",
        summary: PRIVATE_NOTE,
        details: &[],
        offers: &[],
    }];
    const SHEET_SECTIONS: [SheetSection<'static>; 3] = [
        SheetSection {
            key: "character",
            tab: SheetTab::Character,
            title: "Your character",
            caption: "Authored preview identity",
            rows: &CHARACTER_ROWS,
        },
        SheetSection {
            key: "equipment",
            tab: SheetTab::Equipment,
            title: "Equipment",
            caption: "Preview inventory · no mechanical values inferred",
            rows: &EQUIPMENT_ROWS,
        },
        SheetSection {
            key: "journal",
            tab: SheetTab::Journal,
            title: "Journal",
            caption: "Personal presentation · excluded from the shared display",
            rows: &JOURNAL_ROWS,
        },
    ];

    #[derive(Clone, Copy, Eq, PartialEq)]
    enum Scene {
        Harbor,
        Conversation,
        Clue,
        Encounter,
        Aftermath,
        Campfire,
        Transition,
    }

    const AFTERMATH_PARTY: [AftermathPartyMember<'static>; 1] = [AftermathPartyMember {
        key: "mara",
        name: "Mara",
        summary: "Watching the lanterns along the road inland.",
        hit_points: None,
        portrait: None,
    }];
    const PUBLIC_CAMP: [CampfireMember<'static>; 1] = [CampfireMember {
        key: "mara",
        name: "Mara",
        condition: "By the fire",
        detail: "Keeping watch over the misty valley.",
    }];
    const PLAYER_CAMP: [CampfireMember<'static>; 1] = [CampfireMember {
        key: "mara",
        name: "Mara",
        condition: "By the fire",
        detail: "Your folded harbor letter rests safely inside your coat.",
    }];

    struct Preview {
        root: Element,
        player: RoleShell,
        display: RoleShell,
        player_binding: ClientBindingId,
        display_binding: ClientBindingId,
        revision: SessionRevision,
        scene: Scene,
        sheet: bool,
        disconnected: bool,
        input_count: u64,
        encounter_intent_seen: bool,
        rest_requested: bool,
        feedback: &'static str,
        status: Element,
        controls: Vec<ControlledAction>,
    }
    thread_local! { static PREVIEW: RefCell<Option<Preview>> = const { RefCell::new(None) }; }

    fn error(failure: impl std::fmt::Display) -> JsValue {
        JsValue::from_str(&failure.to_string())
    }
    fn binding(byte: u8) -> Result<ClientBindingId, JsValue> {
        ClientBindingId::from_bytes(&[byte; 16])
            .map_err(|failure| JsValue::from_str(&format!("preview binding: {failure:?}")))
    }
    fn campaign_limits() -> CampaignLimits {
        CampaignLimits {
            max_members: 8,
            max_objectives: 8,
            max_text_bytes: 4096,
        }
    }
    fn exploration_limits() -> ExplorationLimits {
        ExplorationLimits {
            campaign: campaign_limits(),
            max_choices: 8,
            max_text_bytes: 4096,
            max_identifier_bytes: 128,
        }
    }
    fn combat_limits() -> CombatLimits {
        CombatLimits {
            max_actors: 8,
            max_offers: 8,
            max_rolls: 8,
            max_resources_per_actor: 8,
            max_text_bytes: 4096,
            max_total_text_bytes: 32768,
        }
    }

    impl Preview {
        fn campaign(&self) -> CampaignView<'static> {
            CampaignView {
                scene: ConceptScene::MaraHarbor,
                chapter: "Chapter one · The Drowned Lantern",
                title: match self.scene {
                    Scene::Harbor => "Lanterns in the fog",
                    Scene::Conversation => "The dockkeeper's warning",
                    Scene::Clue => "A seal in the rain",
                    Scene::Encounter => "At the loading pier",
                    Scene::Aftermath => "Beyond the loading pier",
                    Scene::Campfire => "Beneath the stars",
                    Scene::Transition => "The road to Greyhaven",
                },
                description: match self.scene {
                    Scene::Harbor => {
                        "Rain traces silver lines across the quay. A single lantern burns where the last boat vanished."
                    }
                    Scene::Conversation => {
                        "Vell lowers his voice as footsteps cross the bridge above."
                    }
                    Scene::Clue => {
                        "Dark wax clings to the timber. Its lantern-shaped mark is still sharp."
                    }
                    Scene::Encounter => {
                        "A lookout steps between your company and the sealed cargo."
                    }
                    Scene::Aftermath => "The authored story turns toward the road inland.",
                    Scene::Campfire => {
                        "A warm fire holds the darkness at the edge of the clearing."
                    }
                    Scene::Transition => "Morning waits beyond the mist.",
                },
                location: "Greyhaven · Lantern quay",
                scene_label: "The harbor at midnight",
                narration: match self.scene {
                    Scene::Harbor => {
                        "The tide carries a distant bell. Vell waits beneath the archway, one hand resting on an unlit lantern."
                    }
                    Scene::Conversation => {
                        "“That boat never reached the far bank,” Vell says. “Follow the broken seal, but mind the loading pier.”"
                    }
                    Scene::Clue => {
                        "The fragment bears the same lantern sigil painted on a crate at the far end of the pier."
                    }
                    Scene::Encounter => "The loading pier disappears into fog beyond the lookout.",
                    Scene::Aftermath => {
                        "This authored scene continues without resolving a production encounter."
                    }
                    Scene::Campfire => "The company gathers beneath the stars.",
                    Scene::Transition => "Follow the next authored chapter back to the harbor.",
                },
                connection: if self.disconnected {
                    "Preview connection paused"
                } else {
                    "Local authored scene preview"
                },
                notice: NOTICE,
                members: &PARTY,
                objectives: &[],
            }
        }
        fn exploration(&self, public: bool) -> ExplorationView<'static> {
            ExplorationView {
                binding: if public { self.display_binding } else { self.player_binding },
                revision: self.revision,
                campaign: self.campaign(),
                npc: (self.scene == Scene::Conversation).then_some(ExplorationNpc { name: "Vell", context: "Dockkeeper · Greyhaven quay", dialogue: "“I saw someone carry a sealed crate toward the old pier. They left this behind.”", portrait: Some(ExplorationPortrait::Vell) }),
                heading: if public { "The company explores" } else { "What do you do?" },
                choices: if public { &[] } else if self.scene == Scene::Harbor { &HARBOR_CHOICES } else { &STORY_CHOICES },
                draft_label: "Describe another approach",
                submit_label: "Send your intent",
                draft_offer_id: if public { None } else { Some("preview:free-intent") },
                pending: None,
                rejection: None,
            }
        }
        fn combat<'a>(&self, offers: &'a [CombatOffer<'a>]) -> CombatPhaseView<'a> {
            CombatPhaseView {
                art: CombatArt::Harbor,
                chapter: "Chapter one · The loading pier",
                title: "A shadow across the quay",
                location: "Greyhaven · Abandoned loading pier",
                narration: "A lookout raises a lantern beside the sealed crate. The company holds its ground as fog rolls across the planks.",
                connection: "Local authored scene preview",
                notice: NOTICE,
                turn_label: "Mara · current preview turn",
                active_actor: Some("mara"),
                actors: &ACTORS,
                action_heading: "Choose your intent and target",
                action_empty: "The player's current offers appear on their phone",
                offers,
                roll_heading: "Resolved rolls",
                roll_empty: "No roll or game outcome is resolved by this preview",
                rolls: &[],
                reaction: None,
                draft: None,
                feedback: CombatFeedback::None,
            }
        }
        fn sheet_view(&self) -> CharacterSheetView<'static> {
            CharacterSheetView {
                owner: SheetOwnerGeneration(1),
                revision: self.revision.sequence(),
                name: "Mara",
                identity: "Harbor traveler",
                subtitle: "Your story, equipment and private journal",
                connection: "Local authored scene preview",
                notice: NOTICE,
                labels: SheetLabels {
                    tabs: ["Character", "Equipment", "Spells", "Journal", "Progression"],
                    navigation: "Personal character navigation",
                    filter: "Search your sheet",
                    no_matches: "No matching preview entries",
                    art_fallback: "Character portrait unavailable",
                },
                sections: &SHEET_SECTIONS,
            }
        }
        fn art(&self) -> GameplaySceneArt {
            match self.scene {
                Scene::Harbor | Scene::Conversation | Scene::Clue => {
                    GameplaySceneArt::HarborExploration
                }
                Scene::Encounter | Scene::Aftermath => GameplaySceneArt::HarborEncounter,
                Scene::Campfire | Scene::Transition => GameplaySceneArt::CampfireRest,
            }
        }
        fn aftermath_view(&self, public: bool) -> EncounterAftermathView<'static> {
            EncounterAftermathView {
                generation: 1,
                revision: self.revision.sequence(),
                chapter: "Chapter one · Beyond the loading pier",
                title: "Lanterns along the road",
                narrative: "The harbor falls behind the company. Beyond the last lantern, a clearing opens beneath the stars.",
                party_heading: "The company",
                party: &AFTERMATH_PARTY,
                next_heading: "Beyond the harbor",
                next_context: "Gather around the campfire before the next chapter.",
                next_empty: "The shared display follows the company's story.",
                next_scene: if public {
                    None
                } else {
                    Some(AftermathNextScene {
                        key: "preview:campfire",
                        label: "Gather by the campfire",
                        enabled: true,
                        pending: false,
                    })
                },
                feedback: Some(
                    "Authored scene continuation · no encounter resolution, reward or receipt is confirmed.",
                ),
            }
        }
        fn campfire_view(&self, public: bool) -> CampfireView<'static> {
            CampfireView {
                owner_generation: 1,
                revision: self.revision.sequence(),
                title: GameplaySceneArt::CampfireRest.selection().title,
                location: "Above Greyhaven · The ridge clearing",
                narration: "A small fire warms the stones. Below the ridge, the harbor lights flicker through the mist. The road will still be there at dawn.",
                party_label: "Around the fire",
                notice: if self.rest_requested {
                    self.feedback
                } else {
                    NOTICE
                },
                connected: !self.disconnected,
                members: if public { &PUBLIC_CAMP } else { &PLAYER_CAMP },
                rest: if public {
                    None
                } else {
                    Some(CampfireOffer {
                        id: "preview:rest",
                        label: "Rest by the fire",
                        enabled: true,
                        pending: false,
                    })
                },
                continue_action: if public {
                    None
                } else {
                    Some(CampfireOffer {
                        id: "preview:continue-road",
                        label: "Continue the journey",
                        enabled: true,
                        pending: false,
                    })
                },
            }
        }
        fn transition_view(&self, public: bool) -> SceneTransitionView<'static> {
            SceneTransitionView {
                scene: ConceptScene::Campfire,
                location: "The road to Greyhaven",
                title: "Where the lanterns lead",
                narration: "The fire settles to embers. With the first light, the company follows the lantern road back toward Greyhaven.",
                continue_label: if public {
                    "The company chooses its next scene"
                } else {
                    "Return to Greyhaven"
                },
                continue_enabled: !public && !self.disconnected,
                pending: false,
            }
        }
        fn inline_feedback(&self, shell: &RoleShell) -> Result<(), JsValue> {
            let Some(actions) = shell
                .root()
                .query_selector(".combat-action-rail, .exploration-actions, .camp-actions, .aftermath-panel:last-child, .transition-stage")?
            else {
                return Ok(());
            };
            let output = match actions.query_selector("[data-preview-inline-feedback]")? {
                Some(output) => output,
                None => {
                    let document = actions
                        .owner_document()
                        .ok_or_else(|| JsValue::from_str("preview feedback document missing"))?;
                    let output = document.create_element("output")?;
                    output.set_attribute("data-preview-inline-feedback", "")?;
                    output.set_attribute("aria-live", "polite")?;
                    output.set_attribute("role", "status")?;
                    output.set_attribute("style", "display:block;margin:0 0 14px;padding:12px 14px;border-left:2px solid #c9a76a;background:#262b20;color:#ecd5a9;font:13px/1.65 system-ui,sans-serif;overflow-wrap:anywhere")?;
                    actions.insert_before(&output, actions.first_child().as_ref())?;
                    output
                }
            };
            output.set_text_content(Some(self.feedback));
            Ok(())
        }
        fn show(&mut self) -> Result<(), JsValue> {
            self.revision = self
                .revision
                .next_sequence()
                .map_err(|failure| JsValue::from_str(&format!("preview revision: {failure:?}")))?;
            let action = RevisionLabel::new(Some("preview:combat-intent"))
                .map_err(|failure| JsValue::from_str(&format!("preview label: {failure:?}")))?;
            let lookout = RevisionLabel::new(Some("preview:lookout"))
                .map_err(|failure| JsValue::from_str(&format!("preview label: {failure:?}")))?;
            let aftermath = RevisionLabel::new(Some("preview:aftermath"))
                .map_err(|failure| JsValue::from_str(&format!("preview label: {failure:?}")))?;
            let crate_target = RevisionLabel::new(Some("preview:crate"))
                .map_err(|failure| JsValue::from_str(&format!("preview label: {failure:?}")))?;
            let offers = [
                CombatOffer {
                    key: "challenge-lookout",
                    offer: &action,
                    option: Some(&lookout),
                    kind: CombatOfferKind::Action,
                    label: "Challenge the lookout",
                    explanation: "Intent: confront · Target: dockside lookout",
                    enabled: true,
                    pending: false,
                },
                CombatOffer {
                    key: "inspect-crate",
                    offer: &action,
                    option: Some(&crate_target),
                    kind: CombatOfferKind::Action,
                    label: "Inspect the sealed crate",
                    explanation: "Intent: investigate · Target: sealed crate",
                    enabled: true,
                    pending: false,
                },
                CombatOffer {
                    key: "preview-next-scene",
                    offer: &action,
                    option: Some(&aftermath),
                    kind: CombatOfferKind::Action,
                    label: "Preview the next scene",
                    explanation: "Authored story continuation · no encounter outcome is resolved",
                    enabled: self.encounter_intent_seen,
                    pending: false,
                },
            ];
            let sheet = self.sheet_view();
            let private_exploration = self.exploration(false);
            let public_exploration = self.exploration(true);
            let private_combat = self.combat(&offers);
            let public_combat = self.combat(&[]);
            let private_aftermath = self.aftermath_view(false);
            let public_aftermath = self.aftermath_view(true);
            let private_campfire = self.campfire_view(false);
            let public_campfire = self.campfire_view(true);
            let private_transition = self.transition_view(false);
            let public_transition = self.transition_view(true);
            let player_phase = if self.sheet {
                PlayerPhase::Sheet(&sheet)
            } else {
                match self.scene {
                    Scene::Encounter => PlayerPhase::Combat {
                        view: &private_combat,
                        limits: combat_limits(),
                    },
                    Scene::Aftermath => PlayerPhase::Aftermath(&private_aftermath),
                    Scene::Campfire => PlayerPhase::Campfire(&private_campfire),
                    Scene::Transition => PlayerPhase::Transition {
                        view: &private_transition,
                        generation: 1,
                    },
                    _ => PlayerPhase::Exploration {
                        view: &private_exploration,
                        limits: exploration_limits(),
                    },
                }
            };
            let player_result = self
                .player
                .present(
                    self.player_binding,
                    self.revision,
                    RolePhase::Player(player_phase),
                    None,
                )
                .map_err(error)?;
            let display_phase = match self.scene {
                Scene::Encounter => DisplayPhase::Combat {
                    view: &public_combat,
                    limits: combat_limits(),
                },
                Scene::Aftermath => DisplayPhase::Aftermath(&public_aftermath),
                Scene::Campfire => DisplayPhase::Campfire(&public_campfire),
                Scene::Transition => DisplayPhase::Transition {
                    view: &public_transition,
                    generation: 1,
                },
                _ => DisplayPhase::Exploration {
                    view: &public_exploration,
                    limits: exploration_limits(),
                },
            };
            let display_result = self
                .display
                .present(
                    self.display_binding,
                    self.revision,
                    RolePhase::Display(display_phase),
                    None,
                )
                .map_err(error)?;
            if player_result != ViewAcceptance::Applied || display_result != ViewAcceptance::Applied
            {
                return Err(JsValue::from_str("preview projection was not applied"));
            }
            let art = self.art().selection();
            self.root
                .set_attribute("data-preview-art", art.asset_path)?;
            self.root
                .set_attribute("data-preview-art-title", art.title)?;
            self.root
                .set_attribute("data-preview-art-description", art.image_description)?;
            self.root
                .set_attribute("data-preview-art-fallback", art.fallback_label)?;
            self.status.set_text_content(Some(self.feedback));
            self.inline_feedback(&self.player)?;
            self.inline_feedback(&self.display)?;
            self.root.set_attribute(
                "data-scene",
                match self.scene {
                    Scene::Harbor => "harbor",
                    Scene::Conversation => "conversation",
                    Scene::Clue => "clue",
                    Scene::Encounter => "encounter",
                    Scene::Aftermath => "aftermath",
                    Scene::Campfire => "campfire",
                    Scene::Transition => "transition",
                },
            )?;
            self.root
                .set_attribute("data-sheet", if self.sheet { "open" } else { "closed" })?;
            self.root.set_attribute(
                "data-preview-sequence",
                &self.revision.sequence().to_string(),
            )?;
            self.root
                .set_attribute("data-input-count", &self.input_count.to_string())?;
            self.root.set_attribute(
                "data-connection",
                if self.disconnected {
                    "paused"
                } else {
                    "connected"
                },
            )?;
            if self
                .display
                .root()
                .text_content()
                .is_some_and(|text| text.contains(PRIVATE_NOTE))
            {
                return Err(JsValue::from_str(
                    "private journal entered the display projection",
                ));
            }
            Ok(())
        }
        fn input(&mut self, input: RoleInput) -> Result<(), JsValue> {
            if self.disconnected || self.sheet {
                return Ok(());
            }
            self.input_count = self
                .input_count
                .checked_add(1)
                .ok_or_else(|| JsValue::from_str("preview input count exhausted"))?;
            match input {
                RoleInput::PlayerExploration(ExplorationInput::Choice { id, revision })
                    if revision == self.revision =>
                {
                    match (self.scene, id.as_str()) {
                        (Scene::Harbor, "preview:talk-vell") => {
                            self.scene = Scene::Conversation;
                            self.feedback = "Scene preview · Vell shares his warning. Your two clients stay mounted.";
                        }
                        (
                            Scene::Harbor | Scene::Conversation | Scene::Clue,
                            "preview:inspect-seal",
                        ) => {
                            self.scene = Scene::Clue;
                            self.feedback =
                                "Scene preview · the lantern sigil leads to the loading pier.";
                        }
                        (Scene::Conversation | Scene::Clue, "preview:enter-encounter") => {
                            self.scene = Scene::Encounter;
                            self.feedback = "Scene preview · choose an advertised intent and target on the player client.";
                        }
                        _ => {
                            self.feedback =
                                "That preview choice is no longer offered. No outcome was applied.";
                            return self.show();
                        }
                    }
                }
                RoleInput::PlayerExploration(ExplorationInput::Draft { id, revision, .. })
                    if revision == self.revision
                        && id == "preview:free-intent"
                        && matches!(
                            self.scene,
                            Scene::Harbor | Scene::Conversation | Scene::Clue
                        ) =>
                {
                    self.feedback = "Your preview intent was received. The draft remains editable; no game outcome is confirmed.";
                }
                RoleInput::PlayerCombat(intent)
                    if self.scene == Scene::Encounter
                        && intent.binding == self.player_binding
                        && intent.revision == self.revision
                        && intent.kind == CombatOfferKind::Action
                        && intent.offer.as_str() == "preview:combat-intent" =>
                {
                    self.feedback = match intent.option.as_ref().map(RevisionLabel::as_str) {
                        Some("preview:lookout") => {
                            self.encounter_intent_seen = true;
                            "Preview intent received · confront the dockside lookout. No attack, roll or damage was resolved."
                        }
                        Some("preview:crate") => {
                            self.encounter_intent_seen = true;
                            "Preview intent received · investigate the sealed crate. No check or discovery was resolved."
                        }
                        Some("preview:aftermath") if self.encounter_intent_seen => {
                            self.scene = Scene::Aftermath;
                            "Authored chapter cut · the company gathers on the road inland. No production encounter outcome or reward was applied."
                        }
                        _ => "That preview target is not offered. No outcome was applied.",
                    };
                }
                RoleInput::PlayerAftermath(selection)
                    if self.scene == Scene::Aftermath
                        && selection.generation == 1
                        && selection.revision == self.revision.sequence()
                        && selection.key == "preview:campfire" =>
                {
                    self.scene = Scene::Campfire;
                    self.feedback = "Authored scene preview · the company gathers by the campfire. No travel time or resources were changed.";
                }
                RoleInput::PlayerCampfire(selection)
                    if self.scene == Scene::Campfire
                        && selection.owner_generation == 1
                        && selection.revision == self.revision.sequence() =>
                {
                    match (selection.action, selection.offer_id.as_str()) {
                        (CampfireAction::Rest, "preview:rest") => {
                            self.rest_requested = true;
                            self.feedback = "Preview rest intent received · Mara settles beside the fire. No time, hit points, spells or other resources were changed.";
                        }
                        (CampfireAction::Continue, "preview:continue-road") => {
                            self.scene = Scene::Transition;
                            self.feedback = "Authored scene preview · follow the lantern road toward Greyhaven. No persisted journey was advanced.";
                        }
                        _ => {
                            self.feedback = "That preview campfire offer is no longer current. No outcome was applied.";
                        }
                    }
                }
                RoleInput::PlayerTransition(revision)
                    if self.scene == Scene::Transition && revision == self.revision =>
                {
                    self.scene = Scene::Harbor;
                    self.encounter_intent_seen = false;
                    self.rest_requested = false;
                    self.feedback = "Returned to the lantern quay in this authored preview. Your role shells remain mounted; no persisted campaign was reset.";
                }
                _ => {
                    self.feedback =
                        "That preview input is stale or not offered. No outcome was applied.";
                }
            }
            self.show()?;
            // Keep the authored response beside the current controls and visible
            // after a scrolled phone selection without changing keyboard focus.
            if let Some(output) = self
                .player
                .root()
                .query_selector("[data-preview-inline-feedback]")?
            {
                output.scroll_into_view_with_bool(false);
            }
            Ok(())
        }
    }

    fn operate(operation: impl FnOnce(&mut Preview) -> Result<(), JsValue>) {
        PREVIEW.with(|slot| {
            if let Some(preview) = slot.borrow_mut().as_mut()
                && let Err(failure) = operation(preview)
            {
                preview
                    .status
                    .set_text_content(Some(&format!("Scene preview unavailable: {failure:?}")));
            }
        });
    }

    #[wasm_bindgen]
    pub fn dispose_gameplay_preview() -> Result<(), JsValue> {
        PREVIEW.with(|slot| {
            if let Some(mut preview) = slot.borrow_mut().take() {
                let player = preview.player.dispose();
                let display = preview.display.dispose();
                preview.controls.clear();
                preview.root.remove();
                player.map_err(error)?;
                display.map_err(error)?;
            }
            Ok(())
        })
    }

    fn section(
        document: &Document,
        parent: &Element,
        role: &str,
        label: &str,
    ) -> Result<Element, JsValue> {
        let section = document.create_element("section")?;
        section.set_attribute("data-preview-client", role)?;
        let heading = document.create_element("h2")?;
        heading.set_text_content(Some(label));
        section.append_child(&heading)?;
        parent.append_child(&section)?;
        Ok(section)
    }

    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), JsValue> {
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| JsValue::from_str("preview document missing"))?;
        let body = document
            .body()
            .ok_or_else(|| JsValue::from_str("preview body missing"))?;
        let root = document.create_element("div")?;
        root.set_id("gameplay-scene-preview");
        root.set_attribute("data-layout", "both")?;
        let style = document.create_element("style")?;
        style.set_text_content(Some(r#"
body{margin:0;background:#121311;color:#eadfc9}#gameplay-scene-preview{max-width:1920px;margin:auto}#gameplay-scene-preview>header{padding:18px 4%;display:flex;gap:18px;align-items:center;flex-wrap:wrap;border-bottom:1px solid #bda16a55;background:#171913}#gameplay-scene-preview>header h1{font:24px Georgia,serif;color:#e5c990;margin:0}#gameplay-scene-preview>header p{font:11px/1.6 system-ui,sans-serif;margin:0;color:#b3b5a5}#gameplay-scene-preview>nav{display:flex;gap:8px;flex-wrap:wrap;padding:12px 4%;background:#191c16}#gameplay-scene-preview>nav button{min-height:44px;padding:10px 14px;color:#efddba;background:#292c22;border:1px solid #bda16a55;border-radius:4px;font:12px system-ui,sans-serif;cursor:pointer}#gameplay-scene-preview>nav button:hover{background:#414331}#gameplay-scene-preview>nav button:focus-visible{outline:3px solid #efd39b;outline-offset:3px}#gameplay-scene-preview>output{display:block;padding:12px 4%;font:12px/1.7 system-ui,sans-serif;color:#ddc599;border-top:1px solid #bda16a33;background:#1d2018}#gameplay-scene-preview>section>h2{font:11px/1.6 system-ui,sans-serif;color:#c7ad78;letter-spacing:2px;text-transform:uppercase;padding:18px 4%;margin:0}#gameplay-scene-preview [data-preview-client=player]{max-width:440px;margin:28px auto;border:1px solid #bda16a55;box-shadow:0 15px 80px #0005}#gameplay-scene-preview[data-layout=tv] [data-preview-client=player],#gameplay-scene-preview[data-layout=phone] [data-preview-client=display]{display:none}#gameplay-scene-preview[data-layout=phone] [data-preview-client=player]{margin:0 auto;border:0;box-shadow:none}#gameplay-scene-preview [data-preview-client=player] .topbar{padding:13px 5%;gap:8px 12px;flex-wrap:wrap}#gameplay-scene-preview [data-preview-client=player] .brand{font-size:22px;flex:0 0 auto;white-space:nowrap}#gameplay-scene-preview [data-preview-client=player] .preview-badge{flex:0 0 100%;max-width:none;box-sizing:border-box;font-size:9px;line-height:1.45;letter-spacing:.4px;padding:0 0 0 10px}#gameplay-scene-preview [data-preview-client=player] .df-exploration .stage{display:contents;min-height:0;padding:0}#gameplay-scene-preview [data-preview-client=player] .df-exploration .stage h1{font-size:32px;line-height:1.1;letter-spacing:-.7px}#gameplay-scene-preview [data-preview-client=player] .exploration-interaction{grid-template-columns:1fr;width:90%;gap:16px}#gameplay-scene-preview [data-preview-client=player] .lower{width:90%}#gameplay-scene-preview [data-preview-client=player] .df-exploration .scene-art{top:68px;height:480px}#gameplay-scene-preview [data-preview-client=player] .df-exploration .description{font-size:14px;line-height:1.45}#gameplay-scene-preview [data-preview-client=player] .exploration-npc{gap:8px;padding:10px}#gameplay-scene-preview [data-preview-client=player] .exploration-dialogue{font-size:15px;line-height:1.5}#gameplay-scene-preview [data-preview-client=player] .exploration-portrait{width:48px;height:60px}#gameplay-scene-preview [data-preview-client=player] .party{grid-template-columns:1fr}#gameplay-scene-preview [data-preview-client=player] .scene-art{object-position:28% center}#gameplay-scene-preview [data-preview-client=player] .df-combat{display:flex;flex-direction:column}#gameplay-scene-preview [data-preview-client=player] .combat-topbar{order:0;padding:17px 5% 0;gap:7px 13px;flex-wrap:wrap}#gameplay-scene-preview [data-preview-client=player] .combat-brand{font-size:22px}#gameplay-scene-preview [data-preview-client=player] .combat-location{order:3;flex-basis:100%;font-size:9px}#gameplay-scene-preview [data-preview-client=player] .combat-notice{max-width:130px;font-size:8px}#gameplay-scene-preview [data-preview-client=player] .combat-stage{display:contents}#gameplay-scene-preview [data-preview-client=player] .combat-story{order:1;width:90%;margin:0 auto;padding:155px 0 22px}#gameplay-scene-preview [data-preview-client=player] .combat-story h1{font-size:37px;letter-spacing:-1px}#gameplay-scene-preview [data-preview-client=player] .combat-narration{font-size:14px;line-height:1.65}#gameplay-scene-preview [data-preview-client=player] .combat-lower{order:2;width:90%;grid-template-columns:1fr;gap:16px;margin:0 auto;padding-bottom:18px}#gameplay-scene-preview [data-preview-client=player] .combat-offers{grid-template-columns:1fr}#gameplay-scene-preview [data-preview-client=player] .combat-initiative{order:3;width:90%;box-sizing:border-box;margin:0 auto 18px;padding:13px 15px}#gameplay-scene-preview [data-preview-client=player] .combat-connection{order:4}#gameplay-scene-preview [data-preview-client=player] .combat-art{height:460px;object-position:58% center}#gameplay-scene-preview [data-preview-client=player] .df-combat:before{background:linear-gradient(180deg,#14131050,transparent 19%,#141310b3 32%,#141310 460px)}#gameplay-scene-preview [data-preview-client=player] .sheet-header{padding:15px 16px;gap:14px}#gameplay-scene-preview [data-preview-client=player] .sheet-brand{font-size:22px}#gameplay-scene-preview [data-preview-client=player] .sheet-hero{padding:27px 0 21px 16px;width:90%}#gameplay-scene-preview [data-preview-client=player] .sheet-hero h1{font-size:35px}#gameplay-scene-preview [data-preview-client=player] .sheet-workspace{width:94%;padding:16px 14px 22px}#gameplay-scene-preview [data-preview-client=player] .sheet-tabs{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:6px}#gameplay-scene-preview [data-preview-client=player] .sheet-tabs button{padding:10px 6px;font-size:12px}#gameplay-scene-preview [data-preview-client=player] .sheet-rows{grid-template-columns:1fr}#gameplay-scene-preview [data-preview-client=player] .sheet-subtitle{font-size:14px}#gameplay-scene-preview [data-preview-client=player] .sheet-row-title{font-size:17px}#gameplay-scene-preview [data-preview-client=player] .df-exploration{display:flex;flex-direction:column}#gameplay-scene-preview [data-preview-client=player] .df-exploration .hero{order:2;width:90%;margin:0 auto;padding-top:56px}#gameplay-scene-preview [data-preview-client=player] .df-exploration .chapter{font-size:8px;letter-spacing:1.5px;margin-bottom:5px;gap:8px}#gameplay-scene-preview [data-preview-client=player] .df-exploration .scene-panel{margin:4px 0 6px}#gameplay-scene-preview [data-preview-client=player] .df-exploration .scene-panel h2{font-size:15px}#gameplay-scene-preview [data-preview-client=player] .df-exploration .exploration-interaction{display:contents}#gameplay-scene-preview [data-preview-client=player] .df-exploration .exploration-npc{order:3;width:90%;margin:0 auto 8px}#gameplay-scene-preview [data-preview-client=player] .df-exploration .exploration-actions{order:5;display:block;width:90%;max-width:90%;margin:0 auto;padding:10px}#gameplay-scene-preview [data-preview-client=player] .df-exploration .exploration-heading{font-size:19px;margin-bottom:7px;padding-bottom:6px}#gameplay-scene-preview [data-preview-client=player] .df-exploration .exploration-choices{grid-template-columns:repeat(2,minmax(0,1fr));gap:8px}#gameplay-scene-preview [data-preview-client=player] .df-exploration .df-ui-field{margin-top:13px}#gameplay-scene-preview [data-preview-client=player] .df-exploration .exploration-submit{width:100%;margin-top:10px}#gameplay-scene-preview [data-preview-client=player] .df-exploration .stage>.narration{order:4;width:90%;margin:8px auto;max-width:none;padding:8px 10px;gap:8px}#gameplay-scene-preview [data-preview-client=player] .df-exploration .narration blockquote{font-size:14px;line-height:1.45;margin-top:4px}#gameplay-scene-preview [data-preview-client=player] .df-exploration .narrator{width:28px;height:28px}#gameplay-scene-preview [data-preview-client=player] .df-exploration .lower{order:6;width:90%;margin:4px auto 0}#gameplay-scene-preview [data-preview-client=player] .df-exploration .connection{order:7}#gameplay-scene-preview [data-preview-client=player] .df-exploration .exploration-art-fallback{order:1;width:90%}#gameplay-scene-preview [data-preview-client=player] .df-exploration.exploration-art-failed .hero{padding-top:0}#gameplay-scene-preview [data-preview-client=player] .df-exploration:before{background:linear-gradient(180deg,#14131026,transparent 12%,#14131080 31%,#141310 548px)}#gameplay-scene-preview [data-preview-client=player] .df-exploration.exploration-art-failed:before{background:linear-gradient(140deg,#342c1d66,#141310 65%)}#gameplay-scene-preview [data-preview-client=player] .df-exploration [data-preview-inline-feedback]{margin-bottom:7px!important;padding:6px 8px!important;font-size:11px!important;line-height:1.5!important}#gameplay-scene-preview [data-preview-client=player] .df-exploration .location{margin-top:6px}#gameplay-scene-preview [data-preview-client=player] .df-exploration .stage h1{margin-bottom:6px}#gameplay-scene-preview [data-preview-client=player] .df-exploration .exploration-npc{display:grid;grid-template-columns:36px minmax(0,1fr);column-gap:8px}#gameplay-scene-preview [data-preview-client=player] .df-exploration .exploration-npc-text{display:contents}#gameplay-scene-preview [data-preview-client=player] .df-exploration .exploration-portrait{grid-column:1;grid-row:1/3;width:36px;height:46px}#gameplay-scene-preview [data-preview-client=player] .df-exploration .exploration-npc-name{grid-column:2;font-size:20px;line-height:1.2}#gameplay-scene-preview [data-preview-client=player] .df-exploration .exploration-context{grid-column:2;margin:2px 0 0;font-size:9px;line-height:1.4}#gameplay-scene-preview [data-preview-client=player] .df-exploration .exploration-dialogue{grid-column:1/-1;margin-top:6px;font-size:14px;line-height:1.45}#gameplay-scene-preview [data-preview-client=player] .df-exploration .exploration-choice button{padding:8px 10px;min-height:44px;font-size:14px;line-height:1.45}#gameplay-scene-preview [data-preview-client=player] .df-exploration .exploration-choice-detail,#gameplay-scene-preview [data-preview-client=player] .df-exploration .exploration-choice-reason{padding:0 10px;margin-bottom:6px;font-size:11px;line-height:1.5}@media(max-width:360px){#gameplay-scene-preview [data-preview-client=player] .df-exploration .hero{padding-top:52px}#gameplay-scene-preview [data-preview-client=player] .df-exploration .stage h1{font-size:29px}#gameplay-scene-preview [data-preview-client=player] .df-exploration .exploration-choices{grid-template-columns:1fr}}@media(min-width:1100px){#gameplay-scene-preview [data-preview-client=display] .df-exploration .stage>.hero{width:48%;max-width:560px;margin-left:auto}#gameplay-scene-preview [data-preview-client=display] .df-exploration:before{background:linear-gradient(180deg,#12100a80,transparent 22%,#14131018 42%,#141310b8 74%,#141310 100%),linear-gradient(90deg,#11100d66,transparent 35%,#11100d99 100%)}}@media(min-width:1100px){#gameplay-scene-preview[data-layout=both]{display:grid;grid-template-columns:minmax(0,1fr) 440px;column-gap:20px}#gameplay-scene-preview[data-layout=both]>header,#gameplay-scene-preview[data-layout=both]>nav,#gameplay-scene-preview[data-layout=both]>output{grid-column:1/-1}#gameplay-scene-preview[data-layout=both]>section{min-width:0}#gameplay-scene-preview[data-layout=both] [data-preview-client=player]{margin:0;border-top:0}}
@media(max-width:600px){#gameplay-scene-preview[data-layout=both] [data-preview-client=display]{display:none}#gameplay-scene-preview [data-preview-client=player]{margin:0 auto;border:0}#gameplay-scene-preview>nav{gap:6px}#gameplay-scene-preview>nav button{padding:10px}}@media(prefers-reduced-motion:reduce){#gameplay-scene-preview *{scroll-behavior:auto!important}}
"#));
        root.append_child(&style)?;
        let header = document.create_element("header")?;
        let title = document.create_element("h1")?;
        title.set_text_content(Some("DungeonFlux · The Drowned Lantern"));
        header.append_child(&title)?;
        let notice = document.create_element("p")?;
        notice.set_text_content(Some(NOTICE));
        header.append_child(&notice)?;
        root.append_child(&header)?;
        let nav = document.create_element("nav")?;
        nav.set_attribute("aria-label", "Scene preview navigation")?;
        root.append_child(&nav)?;
        let status = document.create_element("output")?;
        status.set_id("gameplay-preview-feedback");
        status.set_attribute("aria-live", "polite")?;
        root.append_child(&status)?;
        let display_parent = section(&document, &root, "display", "Shared display · the company")?;
        let player_parent = section(&document, &root, "player", "Player · Mara")?;
        body.append_child(&root)?;
        let player_binding = binding(71)?;
        let display_binding = binding(72)?;
        let player = RoleShell::mount(
            &document,
            &player_parent,
            player_binding,
            "scene-preview-player",
            |input| operate(|preview| preview.input(input)),
        )
        .map_err(error)?;
        let display = RoleShell::mount(
            &document,
            &display_parent,
            display_binding,
            "scene-preview-display",
            |input| operate(|preview| preview.input(input)),
        )
        .map_err(error)?;
        let mut preview = Preview {
            root,
            player,
            display,
            player_binding,
            display_binding,
            revision: SessionRevision::new(
                RecoveryEpoch::new(1)
                    .map_err(|failure| JsValue::from_str(&format!("preview epoch: {failure:?}")))?,
                0,
            ),
            scene: Scene::Harbor,
            sheet: false,
            disconnected: false,
            input_count: 0,
            encounter_intent_seen: false,
            rest_requested: false,
            feedback: "Begin at the lantern quay. Speak with Vell or inspect the broken seal on your player client.",
            status,
            controls: Vec::new(),
        };
        for (id, label, operation) in [
            ("preview-sheet", "Character sheet", 0u8),
            ("preview-return", "Return to the scene", 1),
            ("preview-harbor", "Back to the harbor", 2),
            ("preview-connection", "Pause / resume connection", 3),
            ("preview-tv", "TV", 4),
            ("preview-phone", "Phone", 5),
            ("preview-both", "Both clients", 6),
        ] {
            let control = ControlledAction::create(
                &document,
                ActionView {
                    label,
                    enabled: true,
                    pending: false,
                },
            )
            .map_err(error)?;
            control.element().set_id(id);
            control.on_activate(move || operate(|preview| {
                match operation {
                    0 => { preview.sheet = true; preview.feedback = "Your personal sheet is open. The shared display retains the public scene."; }
                    1 => { preview.sheet = false; preview.feedback = "Returned to the current scene. No game action was submitted by navigation."; }
                    2 => { preview.sheet = false; preview.scene = Scene::Harbor; preview.encounter_intent_seen = false; preview.rest_requested = false; preview.feedback = "Authored preview restarted at the harbor. No persisted campaign was reset."; }
                    3 => {
                        preview.disconnected = !preview.disconnected;
                        if preview.disconnected {
                            preview.player.suspend(SessionConnection::Reconnecting).map_err(error)?;
                            preview.display.suspend(SessionConnection::Reconnecting).map_err(error)?;
                            preview.root.set_attribute("data-connection", "paused")?;
                            preview.status.set_text_content(Some("Preview connection paused · scene inputs are suspended; no live connection was changed."));
                            return Ok(());
                        }
                        preview.feedback = "Preview resumed with a newer supplied snapshot. No page reload or duplicate join.";
                    }
                    4 => { return preview.root.set_attribute("data-layout", "tv"); }
                    5 => { return preview.root.set_attribute("data-layout", "phone"); }
                    6 => { return preview.root.set_attribute("data-layout", "both"); }
                    _ => { return Err(JsValue::from_str("unknown preview navigation")); }
                }
                if preview.disconnected { return Ok(()); }
                preview.show()
            })).map_err(error)?;
            nav.append_child(control.element())?;
            preview.controls.push(control);
        }
        preview.show()?;
        PREVIEW.with(|slot| *slot.borrow_mut() = Some(preview));
        Ok(())
    }
}
