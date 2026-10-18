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
    model::{FloatTone, Side, TokenId, hud_picture, ordinal},
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

struct InitiativeDie {
    root: Element,
    value: Element,
}

/// Most icons shown over a figure's head before the rest collapse into "+N".
const MAX_PIPS: usize = 4;

fn marker_overlaps_figure(marker: Rect, figure: Rect) -> bool {
    marker.overlap(&figure) > 0.0
}

fn projected_figure_rect(stage: &Stage, figure: &FigureView, aspect: f32) -> Option<Rect> {
    let half_width = if figure.downed {
        0.72
    } else {
        figure.height_m * 0.34
    };
    let top = if figure.downed { 0.55 } else { figure.height_m };
    let points = [
        Vec3::new(-half_width, 0.05, -half_width),
        Vec3::new(-half_width, 0.05, half_width),
        Vec3::new(half_width, 0.05, -half_width),
        Vec3::new(half_width, 0.05, half_width),
        Vec3::new(-half_width, top, -half_width),
        Vec3::new(-half_width, top, half_width),
        Vec3::new(half_width, top, -half_width),
        Vec3::new(half_width, top, half_width),
    ];
    let projected: Vec<(f32, f32)> = points
        .iter()
        .filter_map(|point| stage.project(figure.position + *point, aspect))
        .collect();
    if projected.is_empty() {
        return None;
    }
    Some(Rect {
        left: projected
            .iter()
            .map(|point| point.0)
            .fold(f32::INFINITY, f32::min),
        top: projected
            .iter()
            .map(|point| point.1)
            .fold(f32::INFINITY, f32::min),
        right: projected
            .iter()
            .map(|point| point.0)
            .fold(f32::NEG_INFINITY, f32::max),
        bottom: projected
            .iter()
            .map(|point| point.1)
            .fold(f32::NEG_INFINITY, f32::max),
    })
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
    chip_known: BTreeSet<TokenId>,
    float_elements: BTreeMap<u64, Element>,
    banner: Element,
    overlays: Element,
    target_tag: Element,
    pip_rows: BTreeMap<TokenId, PipRow>,
    dice: BTreeMap<TokenId, InitiativeDie>,
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
    node.set_attribute("aria-label", status.label())?;
    node.set_attribute("title", status.label())?;
    node.set_attribute("style", &format!("--chip:{}", status.chip_colour()))?;
    let image = document.create_element("img")?;
    image.set_attribute("src", &format!("{GENERATED}/{}", status.icon_stem()))?;
    image.set_attribute("alt", status.initials())?;
    image.set_attribute("draggable", "false")?;
    node.append_child(&image)?;
    element(document, &node, "span", "bf-pip-label", status.initials())?;
    Ok(())
}

/// The picture URL for a token's portrait under the art contract: a live image, or the reviewed
/// photograph that answers the identity key. A painted stem, a role token or an unknown key has no
/// picture (the token shows its initial). Asset stems are lowercase words and dashes; anything else
/// is not turned into a CSS url.
fn portrait_url(stem: Option<&str>) -> Option<String> {
    let stem = stem.filter(|stem| !stem.is_empty())?;
    let safe = |text: &str| {
        text.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-_/".contains(&byte)
        })
    };
    if stem.starts_with(crate::art_contract::LIVE_IMAGERY_PREFIX) {
        return safe(stem).then(|| stem.to_owned());
    }
    if !safe(stem) {
        return None;
    }
    crate::art_contract::library_url(crate::art_contract::Role::CharacterToken, stem)
}

fn portrait_style(stem: Option<&str>) -> String {
    portrait_url(stem).map_or_else(String::new, |url| format!("background-image:url({url})"))
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
        // The detail line (the dice, the mode and the sum) must read across a room on a
        // TV; the stylesheet's default for it is phone-sized. `--u` is the battlefield's
        // own TV unit, inherited from its container.
        roll_text.set_attribute("style", "font-size:calc(var(--u)*1.95);line-height:1.35")?;
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
        let turn = match (&frame.hud.prompt, active) {
            (Some(prompt), _) => prompt.clone(),
            (None, Some(figure)) => format!("{}\u{2019}s turn", figure.name),
            (None, None) => String::new(),
        };
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
            .map(|figure| {
                format!(
                    "{}:{}:{}:{}:{}|",
                    figure.id.0,
                    figure.active,
                    figure.downed,
                    figure.variant.as_deref().unwrap_or_default(),
                    figure.portrait.as_deref().unwrap_or_default()
                )
            })
            .collect();
        if signature != self.chip_signature {
            self.chips.set_text_content(None);
            let known = std::mem::take(&mut self.chip_known);
            for figure in &chip_order {
                self.chip_known.insert(figure.id.clone());
                let chip = element(&self.document, &self.chips, "span", "bf-chip", "")?;
                let chip_name = if figure.downed {
                    format!("{} (down)", figure.name)
                } else {
                    figure.name.clone()
                };
                chip.set_attribute("aria-label", &chip_name)?;
                chip.set_attribute("title", &chip_name)?;
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
                // The same picture as the HP plate: the health variant first, then the
                // portrait. An initial (or the down marker) only when there is no picture
                // at all, so a photographed fighter never reads as a letter tile.
                let picture = hud_picture(
                    figure.variant.as_deref(),
                    portrait_url(figure.portrait.as_deref()),
                );
                let style = picture
                    .as_ref()
                    .map_or_else(String::new, |url| format!("background-image:url({url})"));
                chip.set_attribute("style", &style)?;
                if picture.is_none() {
                    let glyph = if figure.downed {
                        "↓".to_owned()
                    } else {
                        initial(&figure.name)
                    };
                    element(&self.document, &chip, "span", "bf-chip-initial", &glyph)?;
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
            let glyph = if hud_picture(
                figure.variant.as_deref(),
                portrait_url(figure.portrait.as_deref()),
            )
            .is_some()
            {
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
            // Clamp the whole result box, not just its anchor, into the clear middle.
            let (x, y) = float_text::clamp_to_rest_area(x, y, width, height);
            let wanted = Rect {
                left: x - width * 0.5,
                top: y - height * 0.5,
                right: x + width * 0.5,
                bottom: y + height * 0.5,
            };
            let (_, rest_top, _, rest_bottom) = super::rest_shot::REST_AREA;
            let shift =
                float_text::clear_shift(&placed, wanted, height * 1.1, rest_top, rest_bottom);
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
                Some((x, y)) => {
                    let unit = 0.01_f32.min(0.017_778 * aspect);
                    let width = (icons.len() as f32 * 2.6
                        + icons.len().saturating_sub(1) as f32 * 0.3)
                        * unit;
                    let marker = Rect {
                        left: x - width * 0.5,
                        top: y - 2.6 * unit,
                        right: x + width * 0.5,
                        bottom: y,
                    };
                    let overlaps_figure = frame.figures.iter().any(|other| {
                        other.id != figure.id
                            && projected_figure_rect(stage, other, aspect)
                                .is_some_and(|bounds| marker_overlaps_figure(marker, bounds))
                    });
                    let style = if overlaps_figure {
                        "display:none".to_owned()
                    } else {
                        format!("left:{:.2}%;top:{:.2}%", x * 100.0, y * 100.0)
                    };
                    row.root.set_attribute("style", &style)?;
                }
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
                node.root.remove();
            }
            keep
        });
        for die in &show.dice {
            let Some(figure) = frame.figures.iter().find(|figure| figure.id == die.token) else {
                continue;
            };
            if !self.dice.contains_key(&die.token) {
                let root = element(&self.document, &self.overlays, "div", "bf-die", "")?;
                let value = element(&self.document, &root, "span", "bf-die-value", "")?;
                self.dice
                    .insert(die.token.clone(), InitiativeDie { root, value });
            }
            let Some(die_node) = self.dice.get(&die.token) else {
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
            if die_node.value.text_content().as_deref() != Some(text.as_str()) {
                die_node.value.set_text_content(Some(&text));
            }
            die_node
                .root
                .set_attribute("data-landed", if die.landed { "true" } else { "false" })?;
            die_node.root.set_attribute(
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
                Some((x, y)) => die_node.root.set_attribute(
                    "style",
                    &format!(
                        "left:{:.2}%;top:{:.2}%;--spin:{:.0}deg",
                        x * 100.0,
                        y * 100.0,
                        tumble * 720.0
                    ),
                )?,
                None => die_node.root.set_attribute("style", "display:none")?,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srd_p90_graze_result_boxes_stay_inside_the_clear_middle() {
        let (left, top, right, bottom) = super::super::rest_shot::REST_AREA;
        let (x, y) = float_text::clamp_to_rest_area(0.0, 1.0, 0.3, 0.05);
        let result = Rect {
            left: x - 0.15,
            top: y - 0.025,
            right: x + 0.15,
            bottom: y + 0.025,
        };
        assert!(result.left >= left);
        assert!(result.top >= top);
        assert!(result.right <= right);
        assert!(result.bottom <= bottom);
    }

    #[test]
    fn status_markers_are_hidden_when_another_figure_overlaps_them() {
        let marker = Rect {
            left: 0.45,
            top: 0.35,
            right: 0.55,
            bottom: 0.4,
        };
        let overlapping_figure = Rect {
            left: 0.5,
            top: 0.2,
            right: 0.7,
            bottom: 0.55,
        };
        let clear_figure = Rect {
            left: 0.7,
            top: 0.2,
            right: 0.9,
            bottom: 0.55,
        };

        assert!(marker_overlaps_figure(marker, overlapping_figure));
        assert!(!marker_overlaps_figure(marker, clear_figure));
    }
}
