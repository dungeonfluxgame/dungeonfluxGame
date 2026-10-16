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

/// Visually reviewed photographic replacements for the public harbor and merchant-shop cast
/// and backdrops. Rejected earlier `-photo` siblings resolve to the reviewed revision as well.
pub(crate) fn photographic_stem(stem: &str) -> Option<&'static str> {
    let stem = stem
        .strip_suffix(".webp")
        .or_else(|| stem.strip_suffix(".png"))
        .unwrap_or(stem);
    let stem = stem
        .strip_suffix("-documentary-photo")
        .or_else(|| stem.strip_suffix("-photo"))
        .unwrap_or(stem);
    match stem {
        "portrait-hero-1" | "portrait-hero-1-full" | "figure-hero-1" => {
            Some("figure-hero-1-documentary-photo")
        }
        "portrait-hero-2" | "portrait-hero-2-full" | "figure-hero-2" => {
            Some("figure-hero-2-documentary-photo")
        }
        "portrait-hero-3" | "portrait-hero-3-full" | "figure-hero-3" => {
            Some("figure-hero-3-documentary-photo")
        }
        "sprite-hero-1" => Some("sprite-hero-1-documentary-photo"),
        "sprite-hero-2" => Some("sprite-hero-2-documentary-photo"),
        "sprite-hero-3" => Some("sprite-hero-3-documentary-photo"),
        "portrait-courier" | "portrait-courier-full" | "sprite-courier" | "figure-courier" => {
            Some("sprite-courier-documentary-photo")
        }
        "portrait-bandit" | "portrait-bandit-full" | "sprite-bandit" | "figure-bandit" => {
            Some("sprite-bandit-documentary-photo")
        }
        "portrait-patron-merchant" | "portrait-patron-merchant-full" => {
            Some("portrait-patron-merchant-documentary-photo")
        }
        "sprite-patron-merchant" | "figure-patron-merchant" => {
            Some("sprite-patron-merchant-documentary-photo")
        }
        "portrait-shop-adventurer" | "portrait-shop-adventurer-full" => {
            Some("portrait-shop-adventurer-documentary-photo")
        }
        "sprite-shop-adventurer" | "figure-shop-adventurer" => {
            Some("sprite-shop-adventurer-documentary-photo")
        }
        "portrait-shop-soldier" | "portrait-shop-soldier-full" => {
            Some("portrait-shop-soldier-documentary-photo")
        }
        "sprite-shop-soldier" | "figure-shop-soldier" => {
            Some("sprite-shop-soldier-documentary-photo")
        }
        "scene-lobby-harbor"
        | "scene-wharf-arrival"
        | "scene-whisper-in-the-rain"
        | "scene-courier-road"
        | "scene-salt-quay-arrival" => Some("scene-salt-quay-arrival-documentary-photo"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harbor_art_routes_to_reviewed_bytes_including_rejected_photo_aliases() {
        for stem in [
            "portrait-hero-1",
            "figure-hero-1-photo",
            "figure-hero-1-documentary-photo.webp",
        ] {
            assert_eq!(
                photographic_stem(stem),
                Some("figure-hero-1-documentary-photo")
            );
        }
        for stem in [
            "portrait-courier",
            "sprite-courier-photo",
            "sprite-courier-documentary-photo",
        ] {
            assert_eq!(
                photographic_stem(stem),
                Some("sprite-courier-documentary-photo")
            );
        }
        for (stem, expected) in [
            (
                "portrait-patron-merchant",
                "portrait-patron-merchant-documentary-photo",
            ),
            (
                "sprite-shop-adventurer",
                "sprite-shop-adventurer-documentary-photo",
            ),
            (
                "portrait-shop-soldier-full",
                "portrait-shop-soldier-documentary-photo",
            ),
        ] {
            assert_eq!(photographic_stem(stem), Some(expected));
        }
        assert_eq!(
            photographic_stem("scene-courier-road"),
            Some("scene-salt-quay-arrival-documentary-photo")
        );
        assert_eq!(photographic_stem("scene-place-unknown"), None);
    }

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
