//! The heads-up display over the battlefield canvas, as plain DOM: scene title and whose turn it
//! is, the round and initiative chips, party plates on the left, the enemy card on the right,
//! floating numbers, and the letterboxed finishing-blow card. It only shows what the stage state
//! says; the stylesheet (`live_battlefield.css`) sizes everything from the display's `--u` unit.

use std::collections::{BTreeMap, BTreeSet};

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use super::{
    math::Vec3,
    model::{FloatTone, Side, TokenId},
    stage::{FigureView, FrameView, Stage},
};

const GENERATED: &str = "/assets/ui/generated";

struct Plate {
    root: Element,
    name: Element,
    portrait: Element,
    hp_text: Element,
    hp_fill: Element,
}

pub struct Hud {
    document: Document,
    root: Element,
    title: Element,
    turn: Element,
    round: Element,
    chips: Element,
    party: Element,
    enemies: Element,
    floats: Element,
    roll: Element,
    roll_value: Element,
    roll_label: Element,
    roll_verdict: Element,
    roll_text: Element,
    finish_label: Element,
    finish_title: Element,
    finish_names: Element,
    plates: BTreeMap<TokenId, Plate>,
    chip_signature: String,
    float_elements: BTreeMap<u64, Element>,
}

fn element(
    document: &Document,
    parent: &Element,
    tag: &str,
    class: &str,
    text: &str,
) -> Result<Element, JsValue> {
    let node = document.create_element(tag)?;
    node.set_class_name(class);
    if !text.is_empty() {
        node.set_text_content(Some(text));
    }
    parent.append_child(&node)?;
    Ok(node)
}

fn initial(name: &str) -> String {
    name.chars()
        .next()
        .map_or_else(|| "?".to_owned(), |first| first.to_uppercase().collect())
}

/// Asset stems are lowercase words and dashes; anything else is not turned into a CSS url.
fn portrait_style(stem: Option<&str>) -> String {
    match stem {
        Some(stem)
            if !stem.is_empty()
                && stem.bytes().all(|byte| {
                    byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                }) =>
        {
            format!("background-image:url({GENERATED}/{stem})")
        }
        _ => String::new(),
    }
}

impl Hud {
    pub fn mount(document: &Document, container: &Element, title: &str) -> Result<Self, JsValue> {
        let root = element(document, container, "div", "bf-hud", "")?;
        let top = element(document, &root, "header", "bf-top", "")?;
        let title_element = element(document, &top, "h1", "bf-title", title)?;
        let turn = element(document, &top, "p", "bf-turn", "")?;
        let order = element(document, &top, "div", "bf-order", "")?;
        let round = element(document, &order, "span", "bf-round", "")?;
        let chips = element(document, &order, "span", "bf-chips", "")?;
        let party = element(document, &root, "aside", "bf-party", "")?;
        let enemies = element(document, &root, "aside", "bf-enemies", "")?;
        let floats = element(document, &root, "div", "bf-floats", "")?;
        let roll = element(document, &root, "section", "bf-roll", "")?;
        let roll_value = element(document, &roll, "div", "bf-roll-die", "")?;
        let roll_words = element(document, &roll, "div", "bf-roll-words", "")?;
        let roll_label = element(document, &roll_words, "h2", "bf-roll-label", "")?;
        let roll_verdict = element(document, &roll_words, "p", "bf-roll-verdict", "")?;
        let roll_text = element(document, &roll_words, "p", "bf-roll-text", "")?;
        element(document, &root, "div", "bf-bar bf-bar-top", "")?;
        element(document, &root, "div", "bf-bar bf-bar-bottom", "")?;
        let finish = element(document, &root, "section", "bf-finish", "")?;
        let finish_label = element(document, &finish, "p", "bf-finish-label", "Finishing blow")?;
        let finish_title = element(
            document,
            &finish,
            "h2",
            "bf-finish-title",
            "Enemy vanquished",
        )?;
        let finish_names = element(document, &finish, "p", "bf-finish-names", "")?;
        Ok(Self {
            document: document.clone(),
            root,
            title: title_element,
            turn,
            round,
            chips,
            party,
            enemies,
            floats,
            roll,
            roll_value,
            roll_label,
            roll_verdict,
            roll_text,
            finish_label,
            finish_title,
            finish_names,
            plates: BTreeMap::new(),
            chip_signature: String::new(),
            float_elements: BTreeMap::new(),
        })
    }

    fn ensure_plate(&mut self, figure: &FigureView) -> Result<(), JsValue> {
        if self.plates.contains_key(&figure.id) {
            return Ok(());
        }
        let parent = match figure.side {
            Side::Hero => &self.party,
            Side::Enemy => &self.enemies,
        };
        let root = element(&self.document, parent, "article", "bf-plate", "")?;
        let portrait = element(&self.document, &root, "div", "bf-portrait", "")?;
        let words = element(&self.document, &root, "div", "bf-plate-words", "")?;
        let name = element(&self.document, &words, "div", "bf-plate-name", "")?;
        let hp_text = element(&self.document, &words, "div", "bf-plate-hp", "")?;
        let bar = element(&self.document, &words, "div", "bf-hp", "")?;
        let hp_fill = element(&self.document, &bar, "div", "bf-hp-fill", "")?;
        self.plates.insert(
            figure.id.clone(),
            Plate {
                root,
                name,
                portrait,
                hp_text,
                hp_fill,
            },
        );
        Ok(())
    }

    /// Brings the DOM in line with the stage for this frame.
    pub fn update(
        &mut self,
        stage: &Stage,
        frame: &FrameView<'_>,
        aspect: f32,
    ) -> Result<(), JsValue> {
        self.title.set_text_content(Some(&stage.scene().title));
        let active = frame.figures.iter().find(|figure| figure.active);
        let turn = active.map_or_else(String::new, |figure| {
            format!("{}\u{2019}s turn", figure.name)
        });
        if self.turn.text_content().as_deref() != Some(turn.as_str()) {
            self.turn.set_text_content(Some(&turn));
        }
        let round = if frame.hud.round > 0 {
            format!("Round {}", frame.hud.round)
        } else {
            String::new()
        };
        if self.round.text_content().as_deref() != Some(round.as_str()) {
            self.round.set_text_content(Some(&round));
        }

        let mut order: Vec<&FigureView> = frame.figures.iter().collect();
        order.sort_by_key(|figure| {
            let position = frame
                .hud
                .initiative
                .iter()
                .position(|id| *id == figure.id)
                .unwrap_or(usize::MAX);
            (position, figure.side == Side::Enemy, figure.id.clone())
        });
        let signature: String = order
            .iter()
            .map(|figure| format!("{}:{}:{}|", figure.id.0, figure.active, figure.alpha < 0.6))
            .collect();
        if signature != self.chip_signature {
            self.chips.set_text_content(None);
            for figure in &order {
                let chip = element(&self.document, &self.chips, "span", "bf-chip", "")?;
                chip.set_attribute(
                    "data-side",
                    if figure.side == Side::Hero {
                        "hero"
                    } else {
                        "enemy"
                    },
                )?;
                if figure.active {
                    chip.set_attribute("data-active", "")?;
                }
                if figure.alpha < 0.6 {
                    chip.set_attribute("data-down", "")?;
                }
                chip.set_attribute("style", &portrait_style(figure.portrait.as_deref()))?;
                if figure.portrait.is_none() {
                    element(
                        &self.document,
                        &chip,
                        "span",
                        "bf-chip-initial",
                        &initial(&figure.name),
                    )?;
                }
            }
            self.chip_signature = signature;
        }

        let live: BTreeSet<TokenId> = frame
            .figures
            .iter()
            .map(|figure| figure.id.clone())
            .collect();
        self.plates.retain(|id, plate| {
            let keep = live.contains(id);
            if !keep {
                plate.root.remove();
            }
            keep
        });
        for figure in &order {
            self.ensure_plate(figure)?;
            let Some(plate) = self.plates.get(&figure.id) else {
                continue;
            };
            if plate.name.text_content().as_deref() != Some(figure.name.as_str()) {
                plate.name.set_text_content(Some(&figure.name));
            }
            plate
                .portrait
                .set_attribute("style", &portrait_style(figure.portrait.as_deref()))?;
            let glyph = if figure.portrait.is_some() {
                String::new()
            } else {
                initial(&figure.name)
            };
            plate.portrait.set_text_content(Some(&glyph));
            let (text, fraction) = match figure.hit_points {
                Some(hp) => (format!("HP {}/{}", hp.current, hp.maximum), hp.fraction()),
                None => (String::new(), 0.0),
            };
            if plate.hp_text.text_content().as_deref() != Some(text.as_str()) {
                plate.hp_text.set_text_content(Some(&text));
            }
            plate
                .hp_fill
                .set_attribute("style", &format!("width:{:.1}%", fraction * 100.0))?;
            plate
                .root
                .set_attribute("data-active", if figure.active { "true" } else { "false" })?;
            plate.root.set_attribute(
                "data-down",
                if figure.alpha < 0.6 { "true" } else { "false" },
            )?;
        }

        match &frame.hud.roll_card {
            Some(card) => {
                self.roll.set_attribute("data-tone", &card.tone)?;
                self.roll_value.set_text_content(Some(&card.value));
                self.roll_label.set_text_content(Some(&card.label));
                self.roll_verdict.set_text_content(Some(&card.verdict));
                self.roll_text.set_text_content(Some(&card.explanation));
                self.roll.set_attribute("data-open", "")?;
            }
            None => self.roll.remove_attribute("data-open")?,
        }
        self.update_floats(stage, frame, aspect)?;
        match &frame.hud.finishing_blow {
            Some(blow) => {
                self.root.set_attribute("data-finish", "")?;
                self.finish_names
                    .set_text_content(Some(&format!("{} \u{00b7} {}", blow.hero, blow.enemy)));
                self.finish_label.set_text_content(Some("Finishing blow"));
                self.finish_title.set_text_content(Some("Enemy vanquished"));
            }
            None => self.root.remove_attribute("data-finish")?,
        }
        Ok(())
    }

    fn update_floats(
        &mut self,
        stage: &Stage,
        frame: &FrameView<'_>,
        aspect: f32,
    ) -> Result<(), JsValue> {
        let live: BTreeSet<u64> = frame.floats.iter().map(|text| text.id).collect();
        self.float_elements.retain(|id, node| {
            let keep = live.contains(id);
            if !keep {
                node.remove();
            }
            keep
        });
        for text in frame.floats {
            let Some(figure) = frame.figures.iter().find(|figure| figure.id == text.token) else {
                continue;
            };
            let rise = 0.25 + text.age_s * 0.6;
            let anchor = figure.position + Vec3::new(0.0, figure.height_m * 0.95 + rise, 0.0);
            let Some((x, y)) = stage.project(anchor, aspect) else {
                continue;
            };
            if !self.float_elements.contains_key(&text.id) {
                let tone = match text.tone {
                    FloatTone::Damage => "damage",
                    FloatTone::Critical => "critical",
                    FloatTone::Miss => "miss",
                    FloatTone::Heal => "heal",
                };
                let node = element(&self.document, &self.floats, "div", "bf-float", &text.text)?;
                node.set_attribute("data-tone", tone)?;
                self.float_elements.insert(text.id, node);
            }
            if let Some(node) = self.float_elements.get(&text.id) {
                let fade = (1.0 - (text.age_s / super::model::FLOAT_LIFETIME_S - 0.55) / 0.45)
                    .clamp(0.0, 1.0);
                node.set_attribute(
                    "style",
                    &format!(
                        "left:{:.2}%;top:{:.2}%;opacity:{fade:.2}",
                        x * 100.0,
                        y * 100.0
                    ),
                )?;
            }
        }
        Ok(())
    }

    pub fn dispose(self) {
        self.root.remove();
    }
}
