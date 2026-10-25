//! The speech bubble over the battlefield, as plain DOM: the name of whoever speaks and their
//! line in a bubble placed by [`speech::place`] with a tail to the speaker's head. It only shows
//! what the stage says is being spoken; the stylesheet (`live_battlefield.css`) sizes it from the
//! display's `--u` unit.

use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Document, Element, HtmlElement};

use super::{
    math::Vec3,
    speech::{self, Rect},
    stage::{FrameView, Stage},
};

/// Everything on the picture a bubble must stay off, by selector: the party and enemy plates, the
/// title and round strip, the dice card and the banner while they are shown.
const AVOID: &str = ".bf-plate, .bf-top, .bf-roll[data-open], .bf-banner[data-open], .bf-float";
const SVG_NS: &str = "http://www.w3.org/2000/svg";

pub struct SpeechBubble {
    root: Element,
    bubble: HtmlElement,
    name: Element,
    text: Element,
    tail: Element,
    shown: Option<u64>,
    /// The bubble's real size in screen fractions once it has been laid out.
    measured: Option<(f32, f32)>,
}

impl SpeechBubble {
    pub fn mount(document: &Document, container: &Element) -> Result<Self, JsValue> {
        let root = document.create_element("div")?;
        root.set_class_name("bf-speech");
        let bubble: HtmlElement = document.create_element("div")?.dyn_into()?;
        bubble.set_class_name("bf-speech-bubble");
        let name = document.create_element("b")?;
        name.set_class_name("bf-speech-name");
        let text = document.create_element("span")?;
        text.set_class_name("bf-speech-text");
        bubble.append_child(&name)?;
        bubble.append_child(&text)?;
        let svg = document.create_element_ns(Some(SVG_NS), "svg")?;
        svg.set_attribute("class", "bf-speech-tail")?;
        svg.set_attribute("viewBox", "0 0 100 100")?;
        svg.set_attribute("preserveAspectRatio", "none")?;
        let tail = document.create_element_ns(Some(SVG_NS), "polygon")?;
        svg.append_child(&tail)?;
        root.append_child(&svg)?;
        root.append_child(&bubble)?;
        container.append_child(&root)?;
        Ok(Self {
            root,
            bubble,
            name,
            text,
            tail,
            shown: None,
            measured: None,
        })
    }

    pub fn update(
        &mut self,
        container: &Element,
        stage: &Stage,
        frame: &FrameView<'_>,
        aspect: f32,
    ) -> Result<(), JsValue> {
        let Some(line) = &frame.speech else {
            if self.shown.take().is_some() {
                self.root.remove_attribute("data-open")?;
            }
            return Ok(());
        };
        let Some(figure) = frame.figures.iter().find(|figure| figure.id == *line.token) else {
            return Ok(());
        };
        let head = figure.position + Vec3::new(0.0, figure.height_m * 1.02, 0.0);
        let (Some(head), Some(feet)) = (
            stage.project(head, aspect),
            stage.project(figure.position, aspect),
        ) else {
            return Ok(());
        };
        if self.shown != Some(line.line_id) {
            self.name.set_text_content(Some(line.name));
            self.text.set_text_content(Some(line.text));
            self.root.set_attribute(
                "data-side",
                if figure.side == super::model::Side::Enemy {
                    "enemy"
                } else {
                    "hero"
                },
            )?;
            self.shown = Some(line.line_id);
            self.measured = None;
        }
        let (mut width, mut height) = speech::estimate_size(line.text, aspect);
        if let Some(measured) = self.measured {
            (width, height) = measured;
        }
        self.bubble
            .style()
            .set_property("width", &format!("{:.2}%", width * 100.0))?;
        // What the bubble must stay off: everything the page draws over the picture, and the
        // speaker's own body.
        let area = container.get_bounding_client_rect();
        let mut avoid = obstacles(container, &area)?;
        let body_half = ((feet.1 - head.1).abs() * 0.22).max(0.025);
        avoid.push(Rect {
            left: head.0 - body_half,
            top: head.1 - 0.01,
            right: head.0 + body_half,
            bottom: feet.1.max(head.1) + 0.01,
        });
        let speaker = speech::Speaker {
            head,
            mouth: (head.0, head.1 + (feet.1 - head.1).abs() * 0.12),
            reach: body_half,
        };
        let placed = speech::place(speaker, (width, height), &avoid);
        let style = self.bubble.style();
        style.set_property("left", &format!("{:.2}%", placed.rect.left * 100.0))?;
        style.set_property("top", &format!("{:.2}%", placed.rect.top * 100.0))?;
        let points = placed
            .tail
            .iter()
            .map(|(x, y)| format!("{:.2},{:.2}", x * 100.0, y * 100.0))
            .collect::<Vec<_>>()
            .join(" ");
        self.tail.set_attribute("points", &points)?;
        self.root.set_attribute("data-open", "")?;
        // Next frame the placement uses the real size of the laid-out bubble.
        if area.width() > 1.0 && area.height() > 1.0 {
            self.measured = Some((
                self.bubble.offset_width() as f32 / area.width() as f32,
                self.bubble.offset_height() as f32 / area.height() as f32,
            ));
        }
        Ok(())
    }

    pub fn dispose(&self) {
        self.root.remove();
    }
}

fn obstacles(container: &Element, area: &web_sys::DomRect) -> Result<Vec<Rect>, JsValue> {
    let mut obstacle_rects = Vec::new();
    if area.width() < 1.0 || area.height() < 1.0 {
        return Ok(obstacle_rects);
    }
    let nodes = container.query_selector_all(AVOID)?;
    for index in 0..nodes.length() {
        let Some(node) = nodes
            .item(index)
            .and_then(|node| node.dyn_into::<Element>().ok())
        else {
            continue;
        };
        let rect = node.get_bounding_client_rect();
        if rect.width() < 1.0 || rect.height() < 1.0 {
            continue;
        }
        obstacle_rects.push(Rect {
            left: ((rect.left() - area.left()) / area.width()) as f32,
            top: ((rect.top() - area.top()) / area.height()) as f32,
            right: ((rect.right() - area.left()) / area.width()) as f32,
            bottom: ((rect.bottom() - area.top()) / area.height()) as f32,
        });
    }
    Ok(obstacle_rects)
}
