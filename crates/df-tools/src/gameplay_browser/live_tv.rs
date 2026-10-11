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
use super::*;
use web_sys::Event;

pub(super) const GENERATED: &str = "/assets/ui/generated";
const PARTY_SLOTS: usize = 2;
const TAGLINE: &str = "AI dungeon master for fifth-edition fantasy";
const HERO_ROLE: &str = "Dwarf Fighter";

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
    pub(super) portrait: Option<&'static str>,
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
    pub(super) art: &'static str,
    pub(super) art_fallback: &'static str,
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

/// The generated painting is the top layer; the authored fallback shows where it is absent.
fn image_layers(stem: &str, fallback: &str) -> String {
    format!("background-image:url({GENERATED}/{stem}),url({fallback})")
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
        let art = node(document, &root, "div", "tv-art", "")?;
        node(document, &root, "div", "tv-scrim", "")?;
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
        let roster_slot = node(document, &roster, "div", "tv-roster-slot", "")?;

        let stagebox = node(document, &safe, "section", "tv-stagebox", "")?;
        let narration = node(document, &stagebox, "p", "tv-narration", "")?;
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
        Ok(Self {
            root,
            art,
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

    /// The container for recovery controls; it is emptied and refilled on every render.
    pub(super) fn dock(&self) -> &Element {
        &self.dock
    }

    pub(super) fn update(&self, model: &TvModel) -> Result<(), JsValue> {
        let lobby = model.mode == TvMode::Lobby;
        self.root
            .set_attribute("data-tv-mode", if lobby { "lobby" } else { "scene" })?;
        self.art
            .set_attribute("style", &image_layers(model.art, model.art_fallback))?;
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
        if !self
            .party
            .parent_node()
            .is_some_and(|parent| parent.is_same_node(Some(home.as_ref())))
        {
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
                        .map(|stem| format!("background-image:url({GENERATED}/{stem})"))
                        .unwrap_or_default(),
                )?;
            }
            _ => set_hidden(&self.speaker, true)?,
        }
        for (index, card) in self.cards.iter().enumerate() {
            match model.party.get(index) {
                Some(member) => {
                    card.root.remove_attribute("data-empty")?;
                    card.root
                        .set_attribute("data-ready", if member.ready { "true" } else { "false" })?;
                    card.name.set_text_content(Some(&member.name));
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
        Ok(())
    }

    pub(super) fn dispose(self) -> Result<(), JsValue> {
        let removed = self.wordmark.image.remove_event_listener_with_callback(
            "load",
            self.wordmark.load.as_ref().unchecked_ref(),
        );
        self.wordmark.text.remove_attribute("hidden")?;
        self.root.remove();
        removed
    }
}
