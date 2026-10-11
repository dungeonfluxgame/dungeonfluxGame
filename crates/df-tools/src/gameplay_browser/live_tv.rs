//! The shared display's 16:9 stage for the lobby and the exploration scenes.
//!
//! One persistent DOM is built on first use and updated in place for every accepted view;
//! it holds no game state and sends nothing. Optional generated art is an alternate
//! `url()` layered above the authored fallback, so a missing file shows the fallback.
use super::*;
use web_sys::Event;

pub(super) const GENERATED: &str = "/assets/ui/generated";
const PARTY_SLOTS: usize = 2;

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
    pub(super) eyebrow: String,
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
    eyebrow: Element,
    title: Element,
    lede: Element,
    narration: Element,
    speaker: Element,
    speaker_glyph: Element,
    speaker_portrait: Element,
    speaker_name: Element,
    speaker_line: Element,
    join: Element,
    code: Element,
    link: Element,
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
        let load = {
            let (image, text) = (image.clone(), text.clone());
            Closure::wrap(Box::new(move |_: Event| {
                let _ = image.remove_attribute("hidden");
                let _ = text.set_attribute("hidden", "");
            }) as Box<dyn FnMut(Event)>)
        };
        image.add_event_listener_with_callback("load", load.as_ref().unchecked_ref())?;
        image.set_attribute("src", &format!("{GENERATED}/wordmark"))?;
        let eyebrow = node(document, &top, "div", "tv-eyebrow", "")?;

        let headline = node(document, &safe, "section", "tv-headline", "")?;
        let intro = node(document, &headline, "div", "tv-intro", "")?;
        let title = node(document, &intro, "h1", "tv-title", "")?;
        node(document, &intro, "div", "tv-divider", "")?;
        let lede = node(document, &intro, "p", "tv-lede", "")?;
        let join = node(document, &headline, "aside", "tv-panel tv-join", "")?;
        node(document, &join, "div", "tv-eyebrow", "Players join here")?;
        let code = node(document, &join, "div", "tv-code room-code", "")?;
        let link = node(document, &join, "a", "tv-url join-link", "")?;
        link.set_attribute("target", "_blank")?;
        node(
            document,
            &join,
            "p",
            "tv-hint",
            "Open the link on your phone to join",
        )?;

        let band = node(document, &safe, "section", "tv-band", "")?;
        let narration_panel = node(document, &band, "div", "tv-panel", "")?;
        let narration = node(document, &narration_panel, "p", "tv-narration", "")?;
        let speaker = node(document, &band, "div", "tv-panel tv-speaker", "")?;
        let portrait = node(document, &speaker, "div", "tv-portrait", "")?;
        let speaker_glyph = node(document, &portrait, "span", "tv-portrait-glyph", "")?;
        let speaker_portrait = node(document, &portrait, "div", "tv-portrait-img", "")?;
        let words = node(document, &speaker, "div", "", "")?;
        let speaker_name = node(document, &words, "h2", "tv-speaker-name", "")?;
        let speaker_line = node(document, &words, "blockquote", "tv-quote", "")?;

        let party = node(document, &safe, "section", "tv-party party", "")?;
        let mut cards = Vec::with_capacity(PARTY_SLOTS);
        for _ in 0..PARTY_SLOTS {
            let root = node(document, &party, "article", "tv-panel tv-card", "")?;
            let portrait = node(document, &root, "div", "tv-portrait", "")?;
            let glyph = node(document, &portrait, "span", "tv-portrait-glyph", "")?;
            let image = node(document, &portrait, "div", "tv-portrait-img", "")?;
            let words = node(document, &root, "div", "", "")?;
            let name = node(document, &words, "div", "tv-member-name", "")?;
            let status = node(document, &words, "div", "tv-member-status", "")?;
            cards.push(Card {
                root,
                glyph,
                portrait: image,
                name,
                status,
            });
        }
        let dock = node(document, &root, "div", "tv-dock", "")?;
        let legal = node(
            document,
            &root,
            "p",
            "tv-legal",
            "This work includes material from the System Reference Document 5.2.1 by Wizards of the Coast LLC, available at ",
        )?;
        let source = node(document, &legal, "a", "", "dndbeyond.com/srd")?;
        source.set_attribute("href", "https://www.dndbeyond.com/srd")?;
        node(
            document,
            &legal,
            "span",
            "",
            ", licensed under the Creative Commons Attribution 4.0 International License. Authored concept art illustrates the setting.",
        )?;
        Ok(Self {
            root,
            art,
            eyebrow,
            title,
            lede,
            narration,
            speaker,
            speaker_glyph,
            speaker_portrait,
            speaker_name,
            speaker_line,
            join,
            code,
            link,
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
        self.eyebrow.set_text_content(Some(&model.eyebrow));
        self.title.set_text_content(Some(&model.title));
        self.lede.set_text_content(Some(&model.lede));
        self.narration.set_text_content(Some(&model.narration));
        self.narration.set_attribute("title", &model.narration)?;
        set_hidden(&self.join, !lobby)?;
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
                    card.name.set_text_content(Some("An open seat"));
                    card.status
                        .set_text_content(Some("Waiting for a hero to join"));
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
