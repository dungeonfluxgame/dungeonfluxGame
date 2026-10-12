//! The shared display's home screen (PLAY-V05). The server's theme for this lobby names an
//! arrangement, gives the three block boxes, the location's mood and its atmosphere effects. This
//! module only writes those values to the stage as custom properties and overlay elements; it
//! never reads the painting, analyses it or chooses a layout. A view without a record keeps the
//! stylesheet's default layout and neutral mood.
use super::*;

/// The atmosphere effects the stylesheet draws. The server's closed list must match this one.
const EFFECTS: [&str; 9] = [
    "mist",
    "lantern-glow",
    "fireflies",
    "embers",
    "falling-leaves",
    "rain",
    "dust-motes",
    "water-glints",
    "cloud-shadow",
];

/// Shows the home theme on the stage, or the default layout when there is none.
pub(super) fn apply(
    stage: &Element,
    layer: &Element,
    home: Option<&rpc::HomeTheme>,
) -> Result<(), JsValue> {
    let themed = home.filter(|theme| !theme.fallback);
    let Some(theme) = themed else {
        stage.remove_attribute("data-home")?;
        stage.remove_attribute("style")?;
        return set_atmosphere(layer, &[]);
    };
    let (Some(title), Some(code), Some(party), Some(mood)) =
        (&theme.title, &theme.code, &theme.party, &theme.mood)
    else {
        return Err(JsValue::from_str("home theme is incomplete"));
    };
    for colour in [&mood.accent, &mood.tint] {
        if !is_hex_colour(colour) {
            return Err(JsValue::from_str("home theme colour is not #rrggbb"));
        }
    }
    // The glass behind the panels is darker over a brighter painting: the grade's brightness runs
    // from 0.78 (a bright place) to 0.92 (a dim one), and the glass opacity runs the other way.
    let shade = f64::from(mood.brightness_permille).clamp(780.0, 920.0);
    let glass = 0.98 - (shade - 780.0) / 140.0 * 0.08;
    let style = format!(
        "{};{};{};\
         --home-accent:{};--home-tint:{};--home-vignette:{};\
         --home-brightness:{};--home-contrast:{};--home-saturation:{};--home-glass:{:.3}",
        boxed("title", title),
        boxed("code", code),
        boxed("party", party),
        mood.accent,
        mood.tint,
        f64::from(mood.vignette_permille) / 1000.0,
        f64::from(mood.brightness_permille) / 1000.0,
        f64::from(mood.contrast_permille) / 1000.0,
        f64::from(mood.saturation_permille) / 1000.0,
        glass,
    );
    stage.set_attribute("style", &style)?;
    stage.set_attribute("data-home", &theme.arrangement)?;
    let names: Vec<&str> = theme.atmosphere.iter().map(String::as_str).collect();
    set_atmosphere(layer, &names)
}

/// The block's box as the custom properties the stylesheet positions it with, in percent.
fn boxed(block: &str, value: &rpc::HomeBox) -> String {
    // The percent form places the block; the bare permille numbers let the atmosphere mask cut the
    // same rectangle out of the effects (CSS turns them into viewport lengths).
    format!(
        "--home-{block}-x:{}%;--home-{block}-y:{}%;--home-{block}-w:{}%;--home-{block}-h:{}%;\
         --home-{block}-xp:{};--home-{block}-yp:{};--home-{block}-wp:{};--home-{block}-hp:{}",
        f64::from(value.left_permille) / 10.0,
        f64::from(value.top_permille) / 10.0,
        f64::from(value.width_permille) / 10.0,
        f64::from(value.height_permille) / 10.0,
        value.left_permille,
        value.top_permille,
        value.width_permille,
        value.height_permille,
    )
}

fn is_hex_colour(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value.bytes().skip(1).all(|byte| byte.is_ascii_hexdigit())
}

/// Rebuilds the overlay effects only when the list changes, so a redraw keeps their motion.
fn set_atmosphere(layer: &Element, names: &[&str]) -> Result<(), JsValue> {
    let signature = names.join(",");
    if layer.get_attribute("data-effects").as_deref() == Some(signature.as_str()) {
        return Ok(());
    }
    if let Some(unknown) = names.iter().find(|name| !EFFECTS.contains(name)) {
        return Err(JsValue::from_str(&format!(
            "home atmosphere {unknown} is not drawn"
        )));
    }
    layer.set_attribute("data-effects", &signature)?;
    layer.set_text_content(None);
    let document = layer.owner_document().ok_or("document absent")?;
    for name in names {
        node(&document, layer, "i", &format!("tv-fx tv-fx-{name}"), "")?;
    }
    Ok(())
}
