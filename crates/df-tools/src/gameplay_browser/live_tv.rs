//! The shared display's 16:9 stage for the lobby and the exploration scenes.
//!
//! One persistent DOM is built on first use and updated in place for every accepted view;
//! it holds no game state and sends nothing. Generated art is an alternate `url()` layered
//! above the authored fallback, so a missing file shows the fallback.
//!
//! Stable shell for other screens (combat): `.tv-stage` is the fixed full-viewport root,
//! `.tv-art` the full-bleed painting, `.tv-scrim` the text gradients, `.tv-safe` the
//! overscan-safe grid (rows: top, headline, spacer, content, party bar), `.tv-title` the scene
//! title, `.tv-stagebox` the centred lower-third and `.tv-partybar` the compact party row.
use super::live_sfx::{self, Cue};
use super::live_stage::{Figures, StageMode};
use super::*;
use crate::presentation::{HERO_ROLE, SceneFamily, StageActor};
use std::{cell::Cell, time::Duration};
use web_sys::Event;

pub(super) const GENERATED: &str = "/assets/ui/generated";
const PARTY_SLOTS: usize = 2;
const TAGLINE: &str = "AI dungeon master for fifth-edition fantasy";
const DM_AVATAR: &str = "/assets/concept-art/dm-avatar.webp";
const GATHERED_SOUND_DELAY: Duration = Duration::from_millis(700);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum TvMode {
    Lobby,
    Scene,
}

pub(super) struct TvMember {
    pub(super) name: String,
    pub(super) status: String,
    pub(super) ready: bool,
}

pub(super) struct TvSpeaker {
    pub(super) name: String,
    pub(super) line: String,
    pub(super) portrait: Option<String>,
}

/// Everything the stage shows, already chosen from the accepted server view.
pub(super) struct TvModel {
    pub(super) mode: TvMode,
    pub(super) title: String,
    pub(super) lede: String,
    pub(super) narration: String,
    pub(super) speaker: Option<TvSpeaker>,
    pub(super) room_code: String,
    pub(super) join_label: String,
    pub(super) join_path: String,
    pub(super) party: Vec<TvMember>,
    pub(super) art: String,
    pub(super) art_fallback: &'static str,
    pub(super) cast: Vec<StageActor>,
    pub(super) family: SceneFamily,
    /// Names the scene for the figures: a non-player character stays until it changes.
    pub(super) scene_key: String,
}

struct WordmarkSlot {
    image: Element,
    text: Element,
    load: Closure<dyn FnMut(Event)>,
}

struct Card {
    root: Element,
    glyph: Element,
    portrait: Element,
    name: Element,
    status: Element,
}

pub(super) struct TvStage {
    root: Element,
    art: Element,
    art_previous: Element,
    figures: Figures,
    roster: Element,
    gathered: Element,
    /// Whether a view has been drawn yet: the first one plays no seat sounds.
    drawn: Cell<bool>,
    title: Element,
    lede: Element,
    lobby_row: Element,
    stagebox: Element,
    narration: Element,
    speaker: Element,
    speaker_glyph: Element,
    speaker_portrait: Element,
    speaker_name: Element,
    speaker_line: Element,
    code: Element,
    link: Element,
    roster_slot: Element,
    party_bar: Element,
    party: Element,
    cards: Vec<Card>,
    dock: Element,
    wordmark: WordmarkSlot,
}

/// The loading wordmark, centred with a subtle shimmer until the first view arrives.
pub(super) fn mount_splash(document: &Document, parent: &Element) -> Result<(), JsValue> {
    let splash = node(document, parent, "div", "tv-splash", "")?;
    node(document, &splash, "div", "tv-splash-mark", "")?;
    Ok(())
}

/// The generated painting is the top layer; the authored fallback shows where it is absent.
pub(super) fn image_layers(stem: &str, fallback: &str) -> String {
    format!("background-image:url({stem}),url({fallback})")
}

fn set_hidden(element: &Element, hidden: bool) -> Result<(), JsValue> {
    if hidden {
        element.set_attribute("hidden", "")
    } else {
        element.remove_attribute("hidden")
    }
}

fn initial(name: &str) -> String {
    name.chars()
        .next()
        .map_or_else(|| "?".to_owned(), |first| first.to_uppercase().collect())
}

fn portrait(
    document: &Document,
    parent: &Element,
    class: &str,
) -> Result<(Element, Element, Element), JsValue> {
    let frame = node(document, parent, "div", class, "")?;
    let glyph = node(document, &frame, "span", "tv-portrait-glyph", "")?;
    let image = node(document, &frame, "div", "tv-portrait-img", "")?;
    Ok((frame, glyph, image))
}

impl TvStage {
    pub(super) fn mount(document: &Document, parent: &Element) -> Result<Self, JsValue> {
        let root = node(document, parent, "div", "tv-stage", "")?;
        if let Some(splash) = document.query_selector(".tv-splash")? {
            splash.set_attribute("data-done", "")?;
        }
        let art_previous = node(document, &root, "div", "tv-art tv-art-previous", "")?;
        let art = node(document, &root, "div", "tv-art", "")?;
        node(document, &root, "div", "tv-scrim", "")?;
        let figures = Figures::mount(document, &root)?;
        let safe = node(document, &root, "div", "tv-safe", "")?;
        let top = node(document, &safe, "header", "tv-top", "")?;
        let brand = node(document, &top, "div", "tv-brand", "")?;
        let image = node(document, &brand, "img", "tv-wordmark", "")?;
        image.set_attribute("alt", "DungeonFlux")?;
        image.set_attribute("hidden", "")?;
        let text = node(document, &brand, "span", "tv-wordmark-text", "Dungeon")?;
        node(document, &text, "b", "", "Flux")?;
        node(document, &brand, "p", "tv-tagline", TAGLINE)?;
        let load = {
            let (image, text) = (image.clone(), text.clone());
            Closure::wrap(Box::new(move |_: Event| {
                let _ = image.remove_attribute("hidden");
                let _ = text.set_attribute("hidden", "");
            }) as Box<dyn FnMut(Event)>)
        };
        image.add_event_listener_with_callback("load", load.as_ref().unchecked_ref())?;
        image.set_attribute("src", &format!("{GENERATED}/wordmark"))?;

        let headline = node(document, &safe, "section", "tv-headline", "")?;
        let title = node(document, &headline, "h1", "tv-title", "")?;
        node(document, &headline, "div", "tv-divider", "")?;
        let lede = node(document, &headline, "p", "tv-lede", "")?;

        let lobby_row = node(document, &safe, "section", "tv-lobby-row", "")?;
        let join = node(document, &lobby_row, "aside", "tv-panel tv-join", "")?;
        node(document, &join, "div", "tv-label", "Players join here")?;
        let code_box = node(document, &join, "div", "tv-code-box", "")?;
        node(
            document,
            &code_box,
            "div",
            "tv-label tv-code-label",
            "Room code",
        )?;
        let code = node(document, &code_box, "div", "tv-code room-code", "")?;
        let link = node(document, &join, "a", "tv-url join-link", "")?;
        link.set_attribute("target", "_blank")?;
        node(
            document,
            &join,
            "p",
            "tv-hint",
            "Open the link on your phone to join",
        )?;
        let roster = node(document, &lobby_row, "section", "tv-panel tv-roster", "")?;
        node(document, &roster, "div", "tv-label", "Your party")?;
        let gathered = node(
            document,
            &roster,
            "div",
            "tv-label tv-gathered",
            "The party is gathered",
        )?;
        gathered.set_attribute("hidden", "")?;
        let roster_slot = node(document, &roster, "div", "tv-roster-slot", "")?;

        let stagebox = node(document, &safe, "section", "tv-stagebox", "")?;
        // The Dungeon Master's caption looks unlike a character's speech bubble.
        let caption = node(document, &stagebox, "div", "tv-dm", "")?;
        let caption_head = node(document, &caption, "div", "tv-dm-head", "")?;
        let avatar = node(document, &caption_head, "img", "tv-dm-avatar", "")?;
        avatar.set_attribute("src", DM_AVATAR)?;
        avatar.set_attribute("alt", "")?;
        node(
            document,
            &caption_head,
            "span",
            "tv-dm-label",
            "Dungeon Master",
        )?;
        let narration = node(document, &caption, "p", "tv-narration", "")?;
        narration.set_attribute("data-voice-role", "narrator")?;
        node(document, &stagebox, "div", "tv-divider", "")?;
        let speaker = node(document, &stagebox, "div", "tv-speaker", "")?;
        let (_, speaker_glyph, speaker_portrait) = portrait(document, &speaker, "tv-portrait")?;
        let words = node(document, &speaker, "div", "tv-speaker-words", "")?;
        let speaker_name = node(document, &words, "h2", "tv-speaker-name", "")?;
        let speaker_line = node(document, &words, "blockquote", "tv-quote", "")?;

        let party_bar = node(document, &safe, "section", "tv-partybar", "")?;
        let party = node(document, &roster_slot, "div", "tv-party party", "")?;
        let mut cards = Vec::with_capacity(PARTY_SLOTS);
        for _ in 0..PARTY_SLOTS {
            let card = node(document, &party, "article", "tv-card", "")?;
            let (_, glyph, image) = portrait(document, &card, "tv-portrait")?;
            let words = node(document, &card, "div", "tv-card-words", "")?;
            let name = node(document, &words, "div", "tv-member-name", "")?;
            node(document, &words, "div", "tv-member-role", HERO_ROLE)?;
            let status = node(document, &words, "div", "tv-member-status", "")?;
            cards.push(Card {
                root: card,
                glyph,
                portrait: image,
                name,
                status,
            });
        }
        let dock = node(document, &root, "div", "tv-dock", "")?;
        node(document, &root, "div", "tv-peek", "")?;
        Ok(Self {
            root,
            art,
            art_previous,
            figures,
            roster,
            gathered,
            drawn: Cell::new(false),
            title,
            lede,
            lobby_row,
            stagebox,
            narration,
            speaker,
            speaker_glyph,
            speaker_portrait,
            speaker_name,
            speaker_line,
            code,
            link,
            roster_slot,
            party_bar,
            party,
            cards,
            dock,
            wordmark: WordmarkSlot { image, text, load },
        })
    }

    /// The stage element, so another screen can mount inside the same persistent shell.
    pub(super) fn root(&self) -> &Element {
        &self.root
    }

    /// The composited characters standing in the scene.
    pub(super) fn figures(&self) -> &Figures {
        &self.figures
    }

    /// Shows `style` as the painting. A different painting leaves the previous one to fade
    /// out underneath while the new one fades in; the same one changes nothing.
    pub(super) fn set_art(&self, style: &str) -> Result<(), JsValue> {
        let before = self.art.get_attribute("style");
        if before.as_deref() == Some(style) {
            return Ok(());
        }
        if let Some(before) = before {
            self.art_previous.set_attribute("style", &before)?;
            let tick = if self.root.get_attribute("data-art-swap").as_deref() == Some("a") {
                "b"
            } else {
                "a"
            };
            self.root.set_attribute("data-art-swap", tick)?;
        }
        self.art.set_attribute("style", style)
    }

    /// Where the director is in a scene change or beat; the stylesheet shows each state.
    pub(super) fn set_stage_state(&self, state: &str) -> Result<(), JsValue> {
        if state.is_empty() {
            self.root.remove_attribute("data-stage")
        } else {
            self.root.set_attribute("data-stage", state)
        }
    }

    /// Restarts the staggered title, narration and figure entrance.
    pub(super) fn begin_entering(&self) -> Result<(), JsValue> {
        let tick = if self.root.get_attribute("data-scene-swap").as_deref() == Some("a") {
            "b"
        } else {
            "a"
        };
        self.root.set_attribute("data-scene-swap", tick)
    }

    /// The container for recovery controls; it is emptied and refilled on every render.
    pub(super) fn dock(&self) -> &Element {
        &self.dock
    }

    pub(super) fn update(&self, model: &TvModel) -> Result<(), JsValue> {
        let lobby = model.mode == TvMode::Lobby;
        self.root
            .set_attribute("data-tv-mode", if lobby { "lobby" } else { "scene" })?;
        self.set_art(&image_layers(&model.art, model.art_fallback))?;
        self.title.set_text_content(Some(&model.title));
        self.lede.set_text_content(Some(&model.lede));
        self.narration.set_text_content(Some(&model.narration));
        self.narration.set_attribute("title", &model.narration)?;
        set_hidden(&self.lobby_row, !lobby)?;
        set_hidden(&self.stagebox, lobby)?;
        // The party cards are one set of nodes that moves with the mode.
        let home = if lobby {
            &self.roster_slot
        } else {
            &self.party_bar
        };
        let moved = !self
            .party
            .parent_node()
            .is_some_and(|parent| parent.is_same_node(Some(home.as_ref())));
        if moved {
            home.append_child(&self.party)?;
        }
        self.code.set_text_content(Some(&model.room_code));
        self.link.set_text_content(Some(&model.join_label));
        self.link.set_attribute("href", &model.join_path)?;
        match &model.speaker {
            Some(speaker) if !lobby => {
                set_hidden(&self.speaker, false)?;
                self.speaker_name.set_text_content(Some(&speaker.name));
                self.speaker_line.set_text_content(Some(&speaker.line));
                self.speaker_glyph
                    .set_text_content(Some(&initial(&speaker.name)));
                self.speaker_portrait.set_attribute(
                    "style",
                    &speaker
                        .portrait
                        .as_deref()
                        .map(|url| format!("background-image:url({url})"))
                        .unwrap_or_default(),
                )?;
            }
            _ => set_hidden(&self.speaker, true)?,
        }
        let announce = self.drawn.get() && lobby;
        let mut ready_now = 0;
        let mut newly_ready = false;
        for (index, card) in self.cards.iter().enumerate() {
            let before = card.root.get_attribute("data-state");
            let state = match model.party.get(index) {
                None => "empty",
                Some(member) if member.ready => "ready",
                Some(_) => "creating",
            };
            if state == "ready" {
                ready_now += 1;
            }
            card.root.set_attribute("data-state", state)?;
            // The one-shot reveal belongs to the lobby, not to a card that moves to a new place.
            if moved || !lobby {
                card.root.remove_attribute("data-reveal")?;
            } else if announce && before.as_deref() == Some("creating") && state == "ready" {
                card.root.set_attribute("data-reveal", "")?;
                newly_ready = true;
            }
            match model.party.get(index) {
                Some(member) => {
                    card.root.remove_attribute("data-empty")?;
                    card.root
                        .set_attribute("data-ready", if member.ready { "true" } else { "false" })?;
                    // The server withholds a name until the hero exists.
                    card.name.set_text_content(Some(if member.ready {
                        &member.name
                    } else {
                        "A hero is being forged"
                    }));
                    card.status.set_text_content(Some(&member.status));
                    card.glyph.set_text_content(Some(&initial(if member.ready {
                        &member.name
                    } else {
                        "?"
                    })));
                    card.portrait.set_attribute(
                        "style",
                        &if member.ready {
                            format!(
                                "background-image:url({GENERATED}/portrait-hero-{})",
                                index + 1
                            )
                        } else {
                            String::new()
                        },
                    )?;
                }
                None => {
                    card.root.set_attribute("data-empty", "")?;
                    card.root.set_attribute("data-ready", "false")?;
                    card.name.set_text_content(Some("Open seat"));
                    card.status.set_text_content(Some("Waiting to join"));
                    card.glyph.set_text_content(Some("+"));
                    card.portrait.set_attribute("style", "")?;
                }
            }
        }
        let gathered = lobby && ready_now == PARTY_SLOTS;
        let was_gathered = self.roster.has_attribute("data-gathered");
        if gathered {
            self.roster.set_attribute("data-gathered", "")?;
        } else {
            self.roster.remove_attribute("data-gathered")?;
        }
        set_hidden(&self.gathered, !gathered)?;
        if newly_ready {
            live_sfx::play(Cue::HeroReady, Duration::ZERO);
        }
        if announce && gathered && !was_gathered {
            live_sfx::play(Cue::PartyGathered, GATHERED_SOUND_DELAY);
        }
        if lobby {
            self.figures
                .update(&[], &model.scene_key, model.family, StageMode::Scene)?;
        } else {
            self.figures.update(
                &model.cast,
                &model.scene_key,
                model.family,
                StageMode::Scene,
            )?;
        }
        self.drawn.set(true);
        Ok(())
    }

    pub(super) fn dispose(self) -> Result<(), JsValue> {
        self.figures.dispose();
        let removed = self.wordmark.image.remove_event_listener_with_callback(
            "load",
            self.wordmark.load.as_ref().unchecked_ref(),
        );
        self.wordmark.text.remove_attribute("hidden")?;
        self.root.remove();
        removed
    }
}
