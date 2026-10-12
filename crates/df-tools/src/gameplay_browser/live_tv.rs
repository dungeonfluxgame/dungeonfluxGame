//! The shared display's 16:9 stage for the lobby and the exploration scenes.
//!
//! One persistent DOM is built on first use and updated in place for every accepted view;
//! it holds no game state and sends nothing. Generated art is an alternate `url()` layered
//! above the authored fallback, so a missing file shows the fallback.
//!
//! Stable shell for other screens (combat): `.tv-stage` is the fixed full-viewport root,
//! `.tv-art` the full-bleed painting, `.tv-scrim` the text gradients, `.tv-safe` the
//! overscan-safe grid (rows: top, headline, spacer, content, party bar), `.tv-title` the scene
//! title with its `.tv-place` line, `.tv-stagebox` the Dungeon Master's caption and its
//! `.tv-rules` line, `.tv-choices` the read-only row of what the table can do and
//! `.tv-bigcard` the title and closing cards the director shows over the dimmed painting.
use super::live_sfx::{self, Cue};
use super::live_stage::{Figures, StageMode};
use super::*;
use crate::{
    presentation::{Card, CardKind, HERO_ROLE, PlateState, PlateView, SceneFamily, StageActor},
    stage_machines::LineState,
};
use std::{cell::Cell, time::Duration};
use web_sys::Event;

pub(super) const GENERATED: &str = "/assets/ui/generated";
/// The party cards a scene shows, and the least the lobby shows (an open seat each).
// PLAY-Q01: the server's QR code of the join link (see gameplay/join_link.rs).
const JOIN_QR: &str = "/gameplay/join-qr.svg";
const PARTY_SLOTS: usize = 2;
/// The most party cards the lobby draws: two rows of three.
const PARTY_MAX: usize = 6;
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
    pub(super) speaker: Option<TvSpeaker>,
    pub(super) room_code: String,
    pub(super) join_label: String,
    // PLAY-Q01: the host the room code is typed at, named in the join hint.
    pub(super) join_host: String,
    pub(super) join_path: String,
    pub(super) party: Vec<TvMember>,
    pub(super) art: String,
    pub(super) art_fallback: &'static str,
    pub(super) cast: Vec<StageActor>,
    /// The place the scene is set in, shown under its title.
    pub(super) place: String,
    /// The rules wording of the scene, kept apart from the story narration.
    pub(super) rules_note: String,
    /// What the table can do now: labels only, for the read-only row of chips.
    pub(super) choices: Vec<String>,
    pub(super) family: SceneFamily,
    /// Names the scene for the figures: a non-player character stays until it changes.
    pub(super) scene_key: String,
    /// The lobby's home screen from the server; absent outside the lobby and for a fallback.
    pub(super) home: Option<rpc::HomeTheme>,
    /// PLAY-V09: the scene's composed plate, which replaces the painting and the cut-outs.
    pub(super) plate: PlateView,
}

struct WordmarkSlot {
    image: Element,
    text: Element,
    load: Closure<dyn FnMut(Event)>,
}

struct SeatCard {
    root: Element,
    glyph: Element,
    portrait: Element,
    name: Element,
    status: Element,
}

/// The centred card of the story's title and of a fight's close.
struct BigCard {
    root: Element,
    kicker: Element,
    headline: Element,
    line: Element,
    place: Element,
    heroes: Element,
}

/// PLAY-V09: the layer that shows the scene plate: two pictures (the shown one and the one that is
/// loading or leaving), a slow push-in, drifting dust and a breathing vignette. The stylesheet
/// (`live_plate.css`) draws every state from the attributes set here.
type PictureListeners = (Closure<dyn FnMut(Event)>, Closure<dyn FnMut(Event)>);

struct PlateLayer {
    root: Element,
    images: Vec<Element>,
    /// Each picture's `load` and `error` listeners, kept so they can be removed.
    listeners: Vec<PictureListeners>,
    /// The scene whose plate is shown, to tell an edit of it from the plate of a new scene.
    scene: RefCell<String>,
    /// Which of the two pictures is the next to be filled.
    next: Cell<usize>,
}

pub(super) struct TvStage {
    root: Element,
    art: Element,
    art_previous: Element,
    plate: PlateLayer,
    figures: Figures,
    roster: Element,
    gathered: Element,
    /// Whether a view has been drawn yet: the first one plays no seat sounds.
    drawn: Cell<bool>,
    /// The scene just drawn is set in the place of the one before it.
    same_place: Cell<bool>,
    title: Element,
    place: Element,
    lede: Element,
    atmosphere: Element,
    lobby_row: Element,
    stagebox: Element,
    narration: Element,
    rules: Element,
    rules_text: Element,
    choices: Element,
    card: BigCard,
    speaker: Element,
    speaker_glyph: Element,
    speaker_portrait: Element,
    speaker_name: Element,
    speaker_line: Element,
    code: Element,
    link: Element,
    // PLAY-Q01: the join hint, which names the host for the room code.
    hint: Element,
    roster_slot: Element,
    party_bar: Element,
    party: Element,
    cards: Vec<SeatCard>,
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

fn mount_card(document: &Document, parent: &Element) -> Result<BigCard, JsValue> {
    let root = node(document, parent, "div", "tv-bigcard", "")?;
    root.set_attribute("aria-hidden", "true")?;
    let kicker = node(document, &root, "p", "tv-bigcard-kicker", "")?;
    let headline = node(document, &root, "h2", "tv-bigcard-title", "")?;
    node(document, &root, "div", "tv-bigcard-rule", "")?;
    let line = node(document, &root, "p", "tv-bigcard-line", "")?;
    let place = node(document, &root, "p", "tv-bigcard-place", "")?;
    let heroes = node(document, &root, "ul", "tv-bigcard-heroes", "")?;
    Ok(BigCard {
        root,
        kicker,
        headline,
        line,
        place,
        heroes,
    })
}

/// PLAY-V09: builds the plate layer between the painting and the scrim.
fn mount_plate(document: &Document, stage: &Element) -> Result<PlateLayer, JsValue> {
    let root = node(document, stage, "div", "tv-plate", "")?;
    root.set_attribute("aria-hidden", "true")?;
    let zoom = node(document, &root, "div", "tv-plate-zoom", "")?;
    let mut images = Vec::with_capacity(2);
    let mut listeners = Vec::with_capacity(2);
    for index in 0..2 {
        let image = node(document, &zoom, "img", "tv-plate-img", "")?;
        image.set_attribute("alt", "")?;
        image.set_attribute("decoding", "async")?;
        image.set_attribute("data-i", &index.to_string())?;
        let load = {
            let (stage, root, image) = (stage.clone(), root.clone(), image.clone());
            Closure::wrap(Box::new(move |_: Event| {
                // A picture that is no longer the one wanted (a newer plate replaced it while it
                // loaded) changes nothing.
                if root.get_attribute("data-want") != image.get_attribute("data-src") {
                    return;
                }
                let tick = if root.get_attribute("data-push").as_deref() == Some("a") {
                    "b"
                } else {
                    "a"
                };
                // The slow push-in restarts only for a new scene's plate, not for an edit of it.
                if root.get_attribute("data-pull").as_deref() != Some("focus") {
                    let _set = root.set_attribute("data-push", tick);
                }
                let _set = root.set_attribute(
                    "data-active",
                    &image.get_attribute("data-i").unwrap_or_default(),
                );
                let _set = stage.set_attribute("data-plate-loaded", "");
                let _removed = stage.remove_attribute("data-plate-hold");
            }) as Box<dyn FnMut(Event)>)
        };
        let error = {
            let (stage, root, image) = (stage.clone(), root.clone(), image.clone());
            Closure::wrap(Box::new(move |_: Event| {
                if root.get_attribute("data-want") != image.get_attribute("data-src") {
                    return;
                }
                // The plate never loaded: the scene keeps its cut-outs.
                let _set = root.set_attribute(
                    "data-failed",
                    &image.get_attribute("data-src").unwrap_or_default(),
                );
                let _set = stage.set_attribute("data-plate-state", "failed");
                let _removed = stage.remove_attribute("data-plate-loaded");
                let _removed = stage.remove_attribute("data-plate-hold");
            }) as Box<dyn FnMut(Event)>)
        };
        image.add_event_listener_with_callback("load", load.as_ref().unchecked_ref())?;
        image.add_event_listener_with_callback("error", error.as_ref().unchecked_ref())?;
        images.push(image);
        listeners.push((load, error));
    }
    node(document, &root, "div", "tv-plate-dust", "")?;
    node(document, &root, "div", "tv-plate-vignette", "")?;
    Ok(PlateLayer {
        root,
        images,
        listeners,
        scene: RefCell::new(String::new()),
        next: Cell::new(0),
    })
}

impl TvStage {
    pub(super) fn mount(document: &Document, parent: &Element) -> Result<Self, JsValue> {
        let root = node(document, parent, "div", "tv-stage", "")?;
        if let Some(splash) = document.query_selector(".tv-splash")? {
            splash.set_attribute("data-done", "")?;
        }
        let art_previous = node(document, &root, "div", "tv-art tv-art-previous", "")?;
        let art = node(document, &root, "div", "tv-art", "")?;
        let atmosphere = node(document, &root, "div", "tv-atmosphere", "")?;
        let plate = mount_plate(document, &root)?;
        node(document, &root, "div", "tv-scrim", "")?;
        let figures = Figures::mount(document, &root)?;
        let safe = node(document, &root, "div", "tv-safe", "")?;
        // The logo and title travel together as one block a themed lobby can place.
        let title_block = node(document, &safe, "div", "tv-title-block", "")?;
        let top = node(document, &title_block, "header", "tv-top", "")?;
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

        let headline = node(document, &title_block, "section", "tv-headline", "")?;
        let title = node(document, &headline, "h1", "tv-title", "")?;
        node(document, &headline, "div", "tv-divider", "")?;
        let place = node(document, &headline, "p", "tv-place", "")?;
        let lede = node(document, &headline, "p", "tv-lede", "")?;

        let lobby_row = node(document, &safe, "section", "tv-lobby-row", "")?;
        let join = node(document, &lobby_row, "aside", "tv-panel tv-join", "")?;
        node(document, &join, "div", "tv-label", "Players join here")?;
        // PLAY-Q01: the code's QR code sits beside the room code on its own light tile.
        let join_row = node(document, &join, "div", "tv-join-row", "")?;
        let qr = node(document, &join_row, "figure", "tv-qr", "")?;
        let qr_image = node(document, &qr, "img", "tv-qr-image", "")?;
        qr_image.set_attribute("src", JOIN_QR)?;
        qr_image.set_attribute("alt", "QR code: scan it to open the join page")?;
        let code_box = node(document, &join_row, "div", "tv-code-box", "")?;
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
        let hint = node(document, &join, "p", "tv-hint", "")?;
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
        // The rules wording is read, never spoken, and looks quieter than the story.
        let rules = node(document, &stagebox, "p", "tv-rules", "")?;
        node(document, &rules, "span", "tv-rules-tag", "Rules")?;
        let rules_text = node(document, &rules, "span", "tv-rules-text", "")?;
        node(document, &stagebox, "div", "tv-divider", "")?;
        let speaker = node(document, &stagebox, "div", "tv-speaker", "")?;
        let (_, speaker_glyph, speaker_portrait) = portrait(document, &speaker, "tv-portrait")?;
        let words = node(document, &speaker, "div", "tv-speaker-words", "")?;
        let speaker_name = node(document, &words, "h2", "tv-speaker-name", "")?;
        let speaker_line = node(document, &words, "blockquote", "tv-quote", "")?;

        let party_bar = node(document, &safe, "section", "tv-partybar", "")?;
        let party = node(document, &roster_slot, "div", "tv-party party", "")?;
        let mut cards = Vec::with_capacity(PARTY_SLOTS);
        for _ in 0..PARTY_MAX {
            let card = node(document, &party, "article", "tv-card", "")?;
            let (_, glyph, image) = portrait(document, &card, "tv-portrait")?;
            let words = node(document, &card, "div", "tv-card-words", "")?;
            let name = node(document, &words, "div", "tv-member-name", "")?;
            node(document, &words, "div", "tv-member-role", HERO_ROLE)?;
            let status = node(document, &words, "div", "tv-member-status", "")?;
            cards.push(SeatCard {
                root: card,
                glyph,
                portrait: image,
                name,
                status,
            });
        }
        let choices = node(document, &root, "ol", "tv-choices", "")?;
        choices.set_attribute("aria-label", "What the table can do")?;
        let card = mount_card(document, &root)?;
        let dock = node(document, &root, "div", "tv-dock", "")?;
        node(document, &root, "div", "tv-peek", "")?;
        Ok(Self {
            root,
            art,
            art_previous,
            plate,
            figures,
            roster,
            gathered,
            drawn: Cell::new(false),
            same_place: Cell::new(false),
            title,
            place,
            lede,
            atmosphere,
            lobby_row,
            stagebox,
            narration,
            rules,
            rules_text,
            choices,
            card,
            speaker,
            speaker_glyph,
            speaker_portrait,
            speaker_name,
            speaker_line,
            code,
            link,
            hint,
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

    /// Fills the title or closing card; the stylesheet shows it while the director is in the
    /// matching state.
    pub(super) fn show_card(&self, document: &Document, card: &Card) -> Result<(), JsValue> {
        let shown = &self.card;
        shown.root.set_attribute(
            "data-kind",
            match card.kind {
                CardKind::Title => "title",
                CardKind::Victory => "victory",
                CardKind::Defeat => "defeat",
            },
        )?;
        shown.kicker.set_text_content(Some(&card.kicker));
        shown.headline.set_text_content(Some(&card.headline));
        shown.line.set_text_content(Some(&card.subline));
        shown.place.set_text_content(Some(&card.place));
        set_hidden(&shown.place, card.place.is_empty())?;
        set_hidden(&shown.line, card.subline.is_empty())?;
        shown.heroes.set_text_content(None);
        set_hidden(&shown.heroes, card.heroes.is_empty())?;
        for hero in &card.heroes {
            let item = node(document, &shown.heroes, "li", "tv-bigcard-hero", "")?;
            let crest = node(document, &item, "span", "tv-bigcard-crest", "")?;
            crest.set_attribute("style", &format!("background-image:url({})", hero.portrait))?;
            node(document, &item, "b", "tv-bigcard-name", &hero.name)?;
            node(document, &item, "span", "tv-bigcard-role", &hero.role)?;
        }
        Ok(())
    }

    /// Which speaker the stage's words belong to; the stylesheet shows the Dungeon Master's
    /// caption or the character's bubble accordingly, never both.
    pub(super) fn set_speaker(&self, speaker: &str) -> Result<(), JsValue> {
        if speaker.is_empty() {
            self.root.remove_attribute("data-speaker")
        } else {
            self.root.set_attribute("data-speaker", speaker)
        }
    }

    /// Applies one transition of the narrator line's machine to the Dungeon Master's box: the
    /// text is set as the line opens, never earlier. Returns whether the line opened.
    pub(super) fn narrator_line(&self, state: LineState, text: &str) -> Result<bool, JsValue> {
        let opened = matches!(
            state,
            LineState::Opening | LineState::Speaking | LineState::Lingering
        ) && self.narration.text_content().as_deref() != Some(text);
        if opened {
            self.narration.set_text_content(Some(text));
            self.narration.set_attribute("title", text)?;
        }
        if let Some(caption) = self.narration.parent_element() {
            caption.set_attribute("data-line-state", state.name())?;
        }
        match state {
            LineState::Speaking => self.narration.class_list().add_1("speaking")?,
            LineState::Lingering | LineState::Closing | LineState::Done => {
                self.narration.class_list().remove_1("speaking")?;
            }
            _ => {}
        }
        Ok(opened)
    }

    /// Restarts the staggered title, narration and figure entrance.
    pub(super) fn begin_entering(&self) -> Result<(), JsValue> {
        // A beat in the same place only cross-fades its title; a new place replays the entrance.
        self.root.set_attribute(
            "data-title-anim",
            if self.same_place.get() {
                "fade"
            } else {
                "full"
            },
        )?;
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
        live_home::apply(&self.root, &self.atmosphere, model.home.as_ref())?;
        let known_place = self.place.text_content().unwrap_or_default();
        self.same_place
            .set(!model.place.is_empty() && known_place == model.place);
        self.title.set_text_content(Some(&model.title));
        self.place.set_text_content(Some(&model.place));
        set_hidden(&self.place, model.place.is_empty())?;
        self.lede.set_text_content(Some(&model.lede));
        self.rules_text.set_text_content(Some(&model.rules_note));
        set_hidden(&self.rules, model.rules_note.is_empty() || lobby)?;
        self.set_choices(if lobby { &[] } else { &model.choices })?;
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
        // PLAY-Q01
        self.hint.set_text_content(Some(&format!(
            "Scan to join, or enter the room code at {}",
            model.join_host
        )));
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
        // The lobby shows an open seat for each place up to six; a scene keeps its two cards.
        let shown = if lobby {
            model.party.len().clamp(PARTY_SLOTS, PARTY_MAX)
        } else {
            PARTY_SLOTS
        };
        self.roster
            .set_attribute("data-count", &shown.to_string())?;
        let mut newly_ready = false;
        for (index, card) in self.cards.iter().enumerate() {
            set_hidden(&card.root, index >= shown)?;
            let before = card.root.get_attribute("data-state");
            let state = match model.party.get(index) {
                None => "empty",
                Some(member) if member.ready => "ready",
                Some(_) => "creating",
            };
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
        let seats = model.party.len();
        let ready_seats = model.party.iter().filter(|member| member.ready).count();
        let gathered = lobby && seats >= PARTY_SLOTS && ready_seats == seats;
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
        self.set_plate(&model.plate, &model.scene_key)?;
        if lobby {
            self.figures.update(
                &[],
                &model.scene_key,
                &model.art,
                model.family,
                StageMode::Scene,
            )?;
        } else {
            self.figures.update(
                &model.cast,
                &model.scene_key,
                &model.art,
                model.family,
                StageMode::Scene,
            )?;
        }
        self.drawn.set(true);
        Ok(())
    }

    /// The read-only row of numbered chips: what the table can do now. It is replaced only
    /// when the labels change, so a redraw never restarts its animation.
    pub(super) fn set_choices(&self, labels: &[String]) -> Result<(), JsValue> {
        let shown: Vec<&str> = labels.iter().map(String::as_str).collect();
        let signature = shown.join("\u{1f}");
        self.root
            .set_attribute("data-chips", &shown.len().to_string())?;
        if self.choices.get_attribute("data-labels").as_deref() == Some(signature.as_str()) {
            return Ok(());
        }
        self.choices.set_attribute("data-labels", &signature)?;
        self.choices.set_text_content(None);
        let document = self.root.owner_document().ok_or("document absent")?;
        for (number, label) in shown.iter().enumerate() {
            let chip = node(&document, &self.choices, "li", "tv-choice", "")?;
            node(
                &document,
                &chip,
                "span",
                "tv-choice-number",
                &(number + 1).to_string(),
            )?;
            node(&document, &chip, "span", "tv-choice-label", label)?;
        }
        Ok(())
    }

    /// PLAY-V09: shows the scene plate the server made, or leaves the scene as it was.
    ///
    /// `data-plate-state` is the server's word (none, pending, ready, failed; failed also when the
    /// picture does not load) and `data-plate-key` names the plate. The cut-outs go once the
    /// picture has loaded (`data-plate-loaded`), or at once when a new scene arrives with its
    /// plate already ready (`data-plate-hold`), so a pre-made plate never shows the cut-out stage.
    /// A new picture for the scene already on screen (a mood edit) comes in as a focus pull.
    fn set_plate(&self, plate: &PlateView, scene_key: &str) -> Result<(), JsValue> {
        let layer = &self.plate;
        let failed = layer.root.get_attribute("data-failed");
        let usable = match (&plate.state, &plate.url) {
            (PlateState::Ready, Some(url)) if failed.as_deref() != Some(url.as_str()) => Some(url),
            _ => None,
        };
        let state = match (plate.state, usable) {
            (PlateState::Ready, None) => PlateState::Failed,
            (state, _) => state,
        };
        self.root.set_attribute("data-plate-state", state.name())?;
        let Some(url) = usable else {
            self.root.remove_attribute("data-plate-key")?;
            self.root.remove_attribute("data-plate-loaded")?;
            self.root.remove_attribute("data-plate-hold")?;
            layer.root.remove_attribute("data-want")?;
            layer.root.remove_attribute("data-push")?;
            layer.root.remove_attribute("data-pull")?;
            layer.scene.borrow_mut().clear();
            self.figures.set_plate(None)?;
            return Ok(());
        };
        self.figures.set_plate(plate.layout)?;
        if let Some(layout) = plate.layout {
            layer.root.set_attribute(
                "style",
                &format!(
                    "--head-x:{}%;--head-y:{}%",
                    layout.head_x / 10,
                    layout.head_y / 10
                ),
            )?;
        }
        if layer.root.get_attribute("data-want").as_deref() == Some(url.as_str()) {
            return Ok(());
        }
        let same_scene =
            layer.scene.borrow().as_str() == scene_key && layer.root.has_attribute("data-want");
        let next = layer.next.get();
        let Some(image) = layer.images.get(next) else {
            return Ok(());
        };
        layer.next.set(1 - next);
        let key = url.rsplit('/').next().unwrap_or_default();
        self.root.set_attribute("data-plate-key", key)?;
        layer.root.set_attribute("data-want", url)?;
        if same_scene {
            // An edit of the plate on screen: the old one stays until the new one has loaded.
            layer.root.set_attribute("data-pull", "focus")?;
        } else {
            layer.root.remove_attribute("data-pull")?;
            self.root.remove_attribute("data-plate-loaded")?;
            self.root.set_attribute("data-plate-hold", "")?;
            *layer.scene.borrow_mut() = scene_key.to_owned();
        }
        image.set_attribute("data-src", url)?;
        image.set_attribute("src", url)
    }

    pub(super) fn dispose(self) -> Result<(), JsValue> {
        for (image, (load, error)) in self.plate.images.iter().zip(&self.plate.listeners) {
            // Listeners die with their elements; removing them is hygiene.
            let _removed =
                image.remove_event_listener_with_callback("load", load.as_ref().unchecked_ref());
            let _removed =
                image.remove_event_listener_with_callback("error", error.as_ref().unchecked_ref());
        }
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
