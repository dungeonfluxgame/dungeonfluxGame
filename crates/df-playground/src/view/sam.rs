//! The SAM layer: the backdrop's scanned layout painted over it, for checking what the scan found.
//!
//! Each region is painted as its own mask (the silhouette SAM returned, in a colour of its own; a
//! named region in gold), with a tag in the middle of the mask carrying its number, its name and
//! the model's score, and a dot at its head point (where an anchored bubble's tail points). A
//! region with no mask falls back to its box outline. The layer sits in the world layer with the
//! backdrop, so it moves with the camera, and is shown or hidden by the `show_sam` control
//! (`playground set sam=true|false`). A pure function of the layout; it never changes it.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, set_style, text_element};
use crate::image_layout::{CharacterPoint, ImageLayout, LayoutRegion, percent_text, point_of};

/// How far apart, in degrees, the colours of neighbouring regions are: a step near the golden
/// angle, so regions next to each other never share a colour.
const HUE_STEP_DEGREES: usize = 137;
/// The colour of a named region.
const NAMED_HUE_DEGREES: usize = 45;

fn hue(number: usize, region: &LayoutRegion) -> usize {
    if region.label().is_some() {
        NAMED_HUE_DEGREES
    } else {
        (170 + number * HUE_STEP_DEGREES) % 360
    }
}

fn box_style(region: &LayoutRegion) -> String {
    format!(
        "left:{}%;top:{}%;width:{}%;height:{}%;",
        percent_text(region.left_permille()),
        percent_text(region.top_permille()),
        percent_text(
            region
                .right_permille()
                .saturating_sub(region.left_permille())
        ),
        percent_text(
            region
                .bottom_permille()
                .saturating_sub(region.top_permille())
        ),
    )
}

/// The layer for `layout`.
pub fn layer(document: &Document, layout: &ImageLayout) -> Result<Element, JsValue> {
    let root = element(document, "div", "sam-layer")?;
    root.set_attribute("aria-hidden", "true")?;
    for (number, region) in layout.regions().iter().enumerate() {
        let colour = format!("--sam-hue:{};", hue(number, region));
        let named = if region.label().is_some() {
            " sam-labelled"
        } else {
            ""
        };
        match region.mask() {
            Some(mask) => {
                let paint = element(document, "div", &format!("sam-mask{named}"))?;
                set_style(
                    &paint,
                    &format!(
                        "{colour}-webkit-mask-image:url(\"{mask}\");mask-image:url(\"{mask}\");"
                    ),
                )?;
                append(&root, &paint)?;
            }
            None => {
                let frame = element(document, "div", &format!("sam-box{named}"))?;
                set_style(&frame, &format!("{colour}{}", box_style(region)))?;
                append(&root, &frame)?;
            }
        }
        let name = match region.label() {
            Some(label) => format!("{number} {label}"),
            None => number.to_string(),
        };
        let score = format!(
            "{}.{:02}",
            region.score_permille() / 1000,
            (region.score_permille() % 1000) / 10
        );
        let tag = text_element(
            document,
            "span",
            &format!("sam-tag{named}"),
            &format!("{name} · {score}"),
        )?;
        // The tag sits in the middle of its own mask: the space above a head is kept for the items
        // anchored to the character (bubbles, thinking indicators, markers).
        let (tag_x, tag_y) = point_of(region, CharacterPoint::Centre);
        set_style(
            &tag,
            &format!(
                "{colour}left:{}%;top:{}%;",
                percent_text(tag_x),
                percent_text(tag_y)
            ),
        )?;
        append(&root, &tag)?;
        let head = element(document, "div", "sam-head")?;
        set_style(
            &head,
            &format!(
                "left:{}%;top:{}%;",
                percent_text(region.head_x_permille()),
                percent_text(region.head_y_permille())
            ),
        )?;
        append(&root, &head)?;
    }
    append(
        &root,
        &text_element(
            document,
            "span",
            "sam-legend",
            &format!(
                "{} · prompt \"{}\" · {} masks · {}x{}",
                layout.model(),
                layout.prompt(),
                layout.regions().len(),
                layout.picture().width_px,
                layout.picture().height_px
            ),
        )?,
    )?;
    Ok(root)
}
