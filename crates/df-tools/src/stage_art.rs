//! The one rule that names a character cut-out after its portrait.
//!
//! The server allowlists the `sprite-*` twins of the generated portraits by this rule. The
//! display no longer derives a cut-out name: it asks for the `sprite` path the server sends
//! in each scene's cast, so only the server side applies the rule now.

#[cfg(not(target_arch = "wasm32"))]
const PORTRAIT_PREFIX: &str = "portrait-";
#[cfg(not(target_arch = "wasm32"))]
const SPRITE_PREFIX: &str = "sprite-";
#[cfg(not(target_arch = "wasm32"))]
const FULL_SUFFIX: &str = "-full";

/// The transparent cut-out stem for a portrait stem: `portrait-ant-wolf` becomes
/// `sprite-ant-wolf`. A stem that is not a plain portrait (a scene, an icon, a `-full`
/// portrait, an existing sprite) has no sprite.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn sprite_stem(portrait_stem: &str) -> Option<String> {
    let name = portrait_stem.strip_prefix(PORTRAIT_PREFIX)?;
    if name.is_empty() || name.ends_with(FULL_SUFFIX) {
        return None;
    }
    Some(format!("{SPRITE_PREFIX}{name}"))
}

/// The stem of a served generated-art path such as `/assets/ui/generated/portrait-x`.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) fn stem_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn portrait_stems_map_to_sprite_twins() {
        assert_eq!(
            sprite_stem("portrait-hero-1").as_deref(),
            Some("sprite-hero-1")
        );
        assert_eq!(
            sprite_stem("portrait-patron-merchant").as_deref(),
            Some("sprite-patron-merchant")
        );
        assert_eq!(
            sprite_stem("portrait-ant-goblin-warrior").as_deref(),
            Some("sprite-ant-goblin-warrior")
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn non_portrait_stems_have_no_sprite() {
        for stem in [
            "portrait-hero-1-full",
            "scene-courier-road",
            "sprite-hero-1",
            "portrait-",
            "",
        ] {
            assert_eq!(sprite_stem(stem), None, "{stem}");
        }
    }

    #[test]
    fn stem_of_takes_the_last_path_segment() {
        assert_eq!(
            stem_of("/assets/ui/generated/portrait-courier"),
            "portrait-courier"
        );
        assert_eq!(stem_of("portrait-courier"), "portrait-courier");
    }
}
