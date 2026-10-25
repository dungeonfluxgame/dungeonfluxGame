//! The shared display's home screen (PLAY-V05, composed by PLAY-V11). The server's theme for this
//! lobby gives the location's mood and its atmosphere effects. This module only writes those
//! values to the stage as custom properties and overlay elements; it never reads the painting,
//! analyses it or places a block. The layout is one fixed composition in the stylesheet. A view
//! without a record keeps the stylesheet's neutral mood.
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

/// Shows the home theme's mood on the stage, or the neutral mood when there is none.
pub(super) fn apply(
    stage: &Element,
    layer: &Element,
    home: Option<&rpc::HomeTheme>,
) -> Result<(), JsValue> {
    let themed = home.filter(|theme| !theme.fallback);
    let Some(theme) = themed else {
        stage.remove_attribute("style")?;
        return set_atmosphere(layer, &[]);
    };
    let Some(mood) = &theme.mood else {
        return Err(JsValue::from_str("home theme is incomplete"));
    };
    for colour in [&mood.accent, &mood.tint] {
        if !is_hex_colour(colour) {
            return Err(JsValue::from_str("home theme colour is not #rrggbb"));
        }
    }
    // The glass behind the panels is darker over a brighter painting: the grade's brightness runs
    // from 0.78 (a bright place) to 0.92 (a dim one), and the glass opacity runs the other way.
    let brightness_permille = f64::from(mood.brightness_permille).clamp(780.0, 920.0);
    let glass_opacity = 0.98 - (brightness_permille - 780.0) / 140.0 * 0.08;
    let style = format!(
        "--home-accent:{};--home-tint:{};--home-vignette:{};\
         --home-brightness:{};--home-contrast:{};--home-saturation:{};--home-glass:{:.3}",
        mood.accent,
        mood.tint,
        f64::from(mood.vignette_permille) / 1000.0,
        f64::from(mood.brightness_permille) / 1000.0,
        f64::from(mood.contrast_permille) / 1000.0,
        f64::from(mood.saturation_permille) / 1000.0,
        glass_opacity,
    );
    stage.set_attribute("style", &style)?;
    let names: Vec<&str> = theme.atmosphere.iter().map(String::as_str).collect();
    set_atmosphere(layer, &names)
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
