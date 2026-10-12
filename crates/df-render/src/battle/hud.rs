//! The heads-up display over the battlefield canvas, as plain DOM: scene title and whose turn it
//! is, the round and initiative chips, party plates on the left, the enemy card on the right,
//! floating numbers, and the letterboxed finishing-blow card. It only shows what the stage state
//! says; the stylesheet (`live_battlefield.css`) sizes everything from the display's `--u` unit.

use std::collections::{BTreeMap, BTreeSet};

use js_sys::{Array, Function, Object, Reflect};
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Document, Element};

use super::{
    float_text,
    math::Vec3,
    model::{FloatTone, Side, TokenId, ordinal},
    speech::Rect,
    stage::{FigureView, FrameView, Stage},
    status::{Status, icon_statuses},
};

const GENERATED: &str = "/assets/ui/generated";

struct Plate {
    root: Element,
    name: Element,
    portrait: Element,
    hp_text: Element,
    hp_fill: Element,
    /// Beneath the fill, as wide as the hit points were before a blow: the lost segment that
    /// lingers in a bright colour and then drains.
    hp_chip: Element,
    /// The bar itself, for the heal flash.
    hp_bar: Element,
    /// The hit points last shown on this plate.
    last_hp: Option<(u32, u32)>,
    status: Element,
    status_signature: String,
}

/// How long the lost segment lingers (after the fill's own transition) and how long it drains.
const CHIP_HOLD_MS: f64 = 350.0;
const CHIP_DRAIN_MS: f64 = 600.0;
const HEAL_FLASH_MS: f64 = 700.0;

fn set_property(object: &Object, key: &str, value: impl Into<JsValue>) {
    // A property that cannot be set only leaves that keyframe plain.
    let _set = Reflect::set(object, &JsValue::from_str(key), &value.into());
}

/// Starts a Web Animation on `element`; a browser without it simply shows no animation.
fn animate(element: &Element, frames: &Array, options: &Object) {
    let target: &JsValue = element.as_ref();
    if let Some(method) = Reflect::get(target, &JsValue::from_str("animate"))
        .ok()
        .and_then(|method| method.dyn_into::<Function>().ok())
    {
        let _started = method.call2(target, frames, options);
    }
}

/// The lost segment of the bar lingers in the chip colour and drains; a heal flashes the bar.
fn bar_changed(plate: &Plate, before: (u32, u32), now: (u32, u32)) {
    let share = |hp: (u32, u32)| f64::from(hp.0.min(hp.1)) * 100.0 / f64::from(hp.1.max(1));
    if now.0 < before.0 {
        let (from, to) = (
            format!("{:.1}%", share(before)),
            format!("{:.1}%", share(now)),
        );
        let total = CHIP_HOLD_MS + CHIP_DRAIN_MS;
        let frames = Array::new();
        for (offset, width) in [(0.0, &from), (CHIP_HOLD_MS / total, &from), (1.0, &to)] {
            let frame = Object::new();
            set_property(&frame, "offset", offset);
            set_property(&frame, "width", width.as_str());
            frames.push(&frame);
        }
        let options = Object::new();
        set_property(&options, "duration", total);
        set_property(&options, "easing", "ease-in");
        animate(&plate.hp_chip, &frames, &options);
    } else if now.0 > before.0 {
        let frames = Array::new();
        for (offset, glow, brightness) in [
            (0.0, "#7dffa000", 1.0),
            (0.25, "#7dffa0e6", 1.7),
            (1.0, "#7dffa000", 1.0),
        ] {
            let frame = Object::new();
            set_property(&frame, "offset", offset);
            set_property(&frame, "boxShadow", format!("0 0 14px 4px {glow}"));
            set_property(&frame, "filter", format!("brightness({brightness})"));
            frames.push(&frame);
        }
        let options = Object::new();
        set_property(&options, "duration", HEAL_FLASH_MS);
        set_property(&options, "easing", "ease-out");
        animate(&plate.hp_bar, &frames, &options);
    }
}

/// A group of status icons: the signature says which are shown, so the DOM is only rebuilt when
/// the set changes.
struct PipRow {
    root: Element,
    signature: String,
}

/// Most icons shown over a figure's head before the rest collapse into "+N".
const MAX_PIPS: usize = 4;

/// The share of the picture's width the side plates take (a floating result stays clear of them).
const PLATE_EDGE: f32 = 0.23;

/// A floating number never rises above this share of the picture height (the round strip is above).
const FLOAT_TOP_LIMIT: f32 = 0.27;

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
    chip_known: BTreeSet<TokenId>,
    float_elements: BTreeMap<u64, Element>,
    banner: Element,
    overlays: Element,
    target_tag: Element,
    pip_rows: BTreeMap<TokenId, PipRow>,
    dice: BTreeMap<TokenId, Element>,
    badges: BTreeMap<TokenId, Element>,
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

/// One status icon: the art, and while it is missing a labelled chip (the first letters) in its
/// own colour, so a status is never shown as nothing.
fn pip(document: &Document, parent: &Element, status: Status) -> Result<(), JsValue> {
    let node = element(document, parent, "span", "bf-pip", "")?;
    node.set_attribute("data-status", status.id())?;
    node.set_attribute("title", status.label())?;
    node.set_attribute("style", &format!("--chip:{}", status.chip_colour()))?;
    let image = document.create_element("img")?;
    image.set_attribute("src", &format!("{GENERATED}/{}", status.icon_stem()))?;
    image.set_attribute("alt", status.initials())?;
    image.set_attribute("draggable", "false")?;
    node.append_child(&image)?;
    Ok(())
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
        let overlays = element(document, &root, "div", "bf-overlays", "")?;
        let target_tag = element(document, &overlays, "div", "bf-target-tag", "")?;
        let banner = element(document, &root, "div", "bf-banner", "")?;
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
            chip_known: BTreeSet::new(),
            float_elements: BTreeMap::new(),
            banner,
            overlays,
            target_tag,
            pip_rows: BTreeMap::new(),
            dice: BTreeMap::new(),
            badges: BTreeMap::new(),
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
        let hp_chip = element(&self.document, &bar, "div", "bf-hp-chip", "")?;
        let hp_fill = element(&self.document, &bar, "div", "bf-hp-fill", "")?;
        element(&self.document, &bar, "div", "bf-hp-tick", "")?;
        let status = element(&self.document, &words, "div", "bf-plate-status", "")?;
        self.plates.insert(
            figure.id.clone(),
            Plate {
                root,
                name,
                portrait,
                hp_text,
                hp_fill,
                hp_chip,
                hp_bar: bar,
                last_hp: None,
                status,
                status_signature: String::new(),
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
        // While the roll-call runs the tracker holds only those it has visited, in order.
        let chip_order: Vec<&FigureView> = match &frame.hud.roll_for_initiative.tracker {
            Some(tracker) => tracker
                .iter()
                .filter_map(|id| frame.figures.iter().find(|figure| figure.id == *id))
                .collect(),
            None => order.clone(),
        };
        let signature: String = chip_order
            .iter()
            .map(|figure| format!("{}:{}:{}|", figure.id.0, figure.active, figure.downed))
            .collect();
        if signature != self.chip_signature {
            self.chips.set_text_content(None);
            let known = std::mem::take(&mut self.chip_known);
            for figure in &chip_order {
                self.chip_known.insert(figure.id.clone());
                let chip = element(&self.document, &self.chips, "span", "bf-chip", "")?;
                if !known.contains(&figure.id) && !known.is_empty() {
                    chip.set_attribute("data-pop", "")?;
                }
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
                if figure.downed {
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
            match figure.variant.as_deref() {
                // The health variant's face: the head and shoulders of the same full-body picture.
                Some(variant) => {
                    plate
                        .portrait
                        .set_attribute("style", &format!("background-image:url({variant})"))?;
                    plate.portrait.set_attribute("data-crop", "face")?;
                }
                None => {
                    plate
                        .portrait
                        .set_attribute("style", &portrait_style(figure.portrait.as_deref()))?;
                    plate.portrait.remove_attribute("data-crop")?;
                }
            }
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
            // The width the chip rests at, so it only shows while it animates from the old one.
            let width = format!("width:{:.1}%", fraction * 100.0);
            plate.hp_fill.set_attribute("style", &width)?;
            plate.hp_chip.set_attribute("style", &width)?;
            match figure.tone {
                Some(tone) => plate.root.set_attribute("data-health-tier", tone.label())?,
                None => plate.root.remove_attribute("data-health-tier")?,
            }
            let now = figure.hit_points.map(|hp| (hp.current, hp.maximum));
            if let (Some(before), Some(now)) = (plate.last_hp, now) {
                bar_changed(plate, before, now);
            }
            plate
                .root
                .set_attribute("data-active", if figure.active { "true" } else { "false" })?;
            plate
                .root
                .set_attribute("data-down", if figure.downed { "true" } else { "false" })?;
            plate.root.set_attribute(
                "data-bloodied",
                if figure.statuses.contains(Status::Bloodied) {
                    "true"
                } else {
                    "false"
                },
            )?;
            let icons = icon_statuses(figure.statuses);
            let signature: String = icons
                .iter()
                .map(|status| status.id())
                .collect::<Vec<_>>()
                .join(",");
            if signature != plate.status_signature {
                plate.status.set_text_content(None);
                for status in &icons {
                    pip(&self.document, &plate.status, *status)?;
                }
            }
            if let Some(plate) = self.plates.get_mut(&figure.id) {
                plate.status_signature = signature;
                plate.last_hp = figure.hit_points.map(|hp| (hp.current, hp.maximum));
            }
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
        self.update_overlays(stage, frame, aspect)?;
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
        // Results already placed this frame, oldest first: a newer one that would sit on an older
        // one is stacked clear of it (enemies acting in a row), never drawn over it.
        let mut placed: Vec<Rect> = Vec::new();
        for text in frame.floats {
            let Some(figure) = frame.figures.iter().find(|figure| figure.id == text.token) else {
                continue;
            };
            let anchor = figure.position
                + Vec3::new(
                    0.0,
                    figure.height_m * 0.95 + float_text::rise_m(text.age_s),
                    0.0,
                );
            let Some((x, y)) = stage.project(anchor, aspect) else {
                continue;
            };
            // Kept under the round strip and clear of the side cards, however high the figure
            // stands in the picture.
            let (width, height) = float_text::estimate_size(&text.text, aspect);
            // The party plates take the left edge and the enemy cards the right: the whole result
            // stays between them.
            let (x, y) = (
                x.clamp(PLATE_EDGE + width * 0.5, 1.0 - PLATE_EDGE - width * 0.5),
                y.clamp(FLOAT_TOP_LIMIT, 0.92),
            );
            let wanted = Rect {
                left: x - width * 0.5,
                top: y - height * 0.5,
                right: x + width * 0.5,
                bottom: y + height * 0.5,
            };
            let shift =
                float_text::clear_shift(&placed, wanted, height * 1.1, FLOAT_TOP_LIMIT, 0.92);
            let y = y + shift;
            placed.push(Rect {
                top: wanted.top + shift,
                bottom: wanted.bottom + shift,
                ..wanted
            });
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
                let fade = float_text::opacity(text.age_s, text.hold_s);
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

    /// Everything anchored to a figure: its status icons, the target tag, and the initiative
    /// dice, order badges and banner.
    fn update_overlays(
        &mut self,
        stage: &Stage,
        frame: &FrameView<'_>,
        aspect: f32,
    ) -> Result<(), JsValue> {
        let anchor = |figure: &FigureView, lift: f32| {
            let head = figure.height_m * (1.0 - 0.45 * figure.slump) + lift;
            stage.project(figure.position + Vec3::new(0.0, head, 0.0), aspect)
        };
        // Status icons over the heads of affected figures.
        let live: BTreeSet<TokenId> = frame
            .figures
            .iter()
            .filter(|figure| !icon_statuses(figure.statuses).is_empty())
            .map(|figure| figure.id.clone())
            .collect();
        self.pip_rows.retain(|id, row| {
            let keep = live.contains(id);
            if !keep {
                row.root.remove();
            }
            keep
        });
        for figure in &frame.figures {
            let icons = icon_statuses(figure.statuses);
            if icons.is_empty() {
                continue;
            }
            let signature: String = icons
                .iter()
                .map(|status| status.id())
                .collect::<Vec<_>>()
                .join(",");
            if !self.pip_rows.contains_key(&figure.id) {
                let root = element(&self.document, &self.overlays, "div", "bf-pips", "")?;
                self.pip_rows.insert(
                    figure.id.clone(),
                    PipRow {
                        root,
                        signature: String::new(),
                    },
                );
            }
            let Some(row) = self.pip_rows.get_mut(&figure.id) else {
                continue;
            };
            if row.signature != signature {
                row.root.set_text_content(None);
                for status in icons.iter().take(MAX_PIPS) {
                    pip(&self.document, &row.root, *status)?;
                }
                if icons.len() > MAX_PIPS {
                    element(
                        &self.document,
                        &row.root,
                        "span",
                        "bf-pip-more",
                        &format!("+{}", icons.len() - MAX_PIPS),
                    )?;
                }
                row.signature = signature;
            }
            match anchor(figure, 0.12) {
                Some((x, y)) => row.root.set_attribute(
                    "style",
                    &format!("left:{:.2}%;top:{:.2}%", x * 100.0, y * 100.0),
                )?,
                None => row.root.set_attribute("style", "display:none")?,
            }
        }
        // The name tag on the figure being aimed at.
        match frame.figures.iter().find(|figure| figure.targeted) {
            Some(figure) => {
                let text = format!("Target \u{00b7} {}", figure.name);
                if self.target_tag.text_content().as_deref() != Some(text.as_str()) {
                    self.target_tag.set_text_content(Some(&text));
                }
                let lift = if icon_statuses(figure.statuses).is_empty() {
                    0.3
                } else {
                    0.7
                };
                match anchor(figure, lift) {
                    Some((x, y)) => self.target_tag.set_attribute(
                        "style",
                        &format!("left:{:.2}%;top:{:.2}%;opacity:1", x * 100.0, y * 100.0),
                    )?,
                    None => self.target_tag.set_attribute("style", "opacity:0")?,
                }
            }
            None => self.target_tag.set_attribute("style", "opacity:0")?,
        }
        // The initiative moment.
        let show = &frame.hud.roll_for_initiative;
        if show.banner.is_some() || !show.dice.is_empty() || !show.badges.is_empty() {
            self.root.set_attribute("data-initiative", "")?;
        } else {
            self.root.remove_attribute("data-initiative")?;
        }
        match &show.banner {
            Some(text) => {
                if self.banner.text_content().as_deref() != Some(text.as_str()) {
                    self.banner.set_text_content(Some(text));
                    self.banner.remove_attribute("data-open")?;
                }
                self.banner.set_attribute("data-open", "")?;
            }
            None => self.banner.remove_attribute("data-open")?,
        }
        let live_dice: BTreeSet<&TokenId> = show.dice.iter().map(|die| &die.token).collect();
        self.dice.retain(|id, node| {
            let keep = live_dice.contains(id);
            if !keep {
                node.remove();
            }
            keep
        });
        for die in &show.dice {
            let Some(figure) = frame.figures.iter().find(|figure| figure.id == die.token) else {
                continue;
            };
            if !self.dice.contains_key(&die.token) {
                let node = element(&self.document, &self.overlays, "div", "bf-die", "")?;
                self.dice.insert(die.token.clone(), node);
            }
            let Some(node) = self.dice.get(&die.token) else {
                continue;
            };
            let text = if die.landed {
                let sign = if die.modifier >= 0 { "+" } else { "\u{2212}" };
                format!(
                    "{}{}{}{}{}",
                    die.face,
                    sign,
                    die.modifier.unsigned_abs(),
                    " = ",
                    die.total
                )
            } else {
                // The faces flicker by while it tumbles.
                (((die.age_s * 38.0) as u32 * 7 + die.face * 3) % 20 + 1).to_string()
            };
            if node.text_content().as_deref() != Some(text.as_str()) {
                node.set_text_content(Some(&text));
            }
            node.set_attribute("data-landed", if die.landed { "true" } else { "false" })?;
            node.set_attribute(
                "data-side",
                if figure.side == Side::Hero {
                    "hero"
                } else {
                    "enemy"
                },
            )?;
            let tumble = if die.landed {
                0.0
            } else {
                (die.age_s / die.tumble_s.max(0.01)).clamp(0.0, 1.0)
            };
            match anchor(figure, 0.55 + 0.35 * (1.0 - tumble) * (1.0 - tumble)) {
                Some((x, y)) => node.set_attribute(
                    "style",
                    &format!(
                        "left:{:.2}%;top:{:.2}%;--spin:{:.0}deg",
                        x * 100.0,
                        y * 100.0,
                        tumble * 720.0
                    ),
                )?,
                None => node.set_attribute("style", "display:none")?,
            }
        }
        let live_badges: BTreeSet<&TokenId> =
            show.badges.iter().map(|badge| &badge.token).collect();
        self.badges.retain(|id, node| {
            let keep = live_badges.contains(id);
            if !keep {
                node.remove();
            }
            keep
        });
        for badge in &show.badges {
            let Some(figure) = frame.figures.iter().find(|figure| figure.id == badge.token) else {
                continue;
            };
            if !self.badges.contains_key(&badge.token) {
                let node = element(&self.document, &self.overlays, "div", "bf-badge", "")?;
                element(
                    &self.document,
                    &node,
                    "span",
                    "bf-badge-order",
                    &ordinal(badge.order),
                )?;
                element(
                    &self.document,
                    &node,
                    "span",
                    "bf-badge-name",
                    &format!("{} \u{2014} {}", badge.name, badge.total),
                )?;
                node.set_attribute(
                    "data-side",
                    if figure.side == Side::Hero {
                        "hero"
                    } else {
                        "enemy"
                    },
                )?;
                self.badges.insert(badge.token.clone(), node);
            }
            if let Some(node) = self.badges.get(&badge.token) {
                match stage.project(
                    figure.position + Vec3::new(0.0, figure.height_m * 0.62, 0.0),
                    aspect,
                ) {
                    Some((x, y)) => node.set_attribute(
                        "style",
                        &format!("left:{:.2}%;top:{:.2}%", x * 100.0, y * 100.0),
                    )?,
                    None => node.set_attribute("style", "display:none")?,
                }
            }
        }
        Ok(())
    }

    pub fn dispose(self) {
        self.root.remove();
    }
}
