//! Native checks of the home-screen theme: the mood and the atmosphere on synthetic and real
//! paintings, every location painting that can appear, refusals that name the painting, the
//! documented fallback, and the lobby view carried through the generated protocol.
use super::adventure::setting::SettingKind;
use super::home_theme::analysis::{self, AnalysisError, Measures};
use super::home_theme::{
    Atmosphere, Cache, HomeThemeError, atmosphere_for, lobby_view, theme_for, theme_from_cache,
};
use image::{Rgb, RgbImage};
use prost::Message;
use std::path::{Path, PathBuf};

fn asset_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/ui")
}

/// A painting made of `pixel(x, y)`, sized like the real 16:9 arrival paintings.
fn synthetic(pixel: impl Fn(u32, u32) -> [u8; 3]) -> RgbImage {
    let (width, height) = (640, 360);
    RgbImage::from_fn(width, height, |x, y| Rgb(pixel(x, y)))
}

fn measure(image: &RgbImage) -> Measures {
    analysis::measure(image).expect("a lit synthetic painting measures")
}

#[test]
fn a_flat_black_painting_is_refused_as_unlit() {
    let image = synthetic(|_, _| [0, 0, 0]);
    assert_eq!(analysis::measure(&image).err(), Some(AnalysisError::Flat));
}

#[test]
fn every_location_arrival_painting_gets_a_theme_from_its_pixels() {
    let mut themed = 0;
    for painting in arrival_paintings() {
        let theme = theme_for(&asset_root(), &painting, None)
            .unwrap_or_else(|error| panic!("{painting} should have a theme: {error}"));
        assert!(theme.mood.accent.starts_with('#') && theme.mood.accent.len() == 7);
        themed += 1;
    }
    assert!(
        themed >= 2,
        "both reviewed scene photographs are themed, found {themed}"
    );
}

#[test]
fn a_theme_is_the_same_cold_warm_and_in_any_order_of_paintings() {
    // Arrange: two real paintings, measured in two orders with fresh caches.
    let farm = "assets/ui/generated/scene-salt-quay-arrival-documentary-photo.webp";
    let reedmoor = "assets/ui/generated/scene-salt-quay-candid-documentary-photo.webp";
    let root = asset_root();
    let kind = Some(SettingKind::Farmyard);
    // Act: a cold cache measures the farm, then the same cache answers it again from memory.
    let mut ordered = Cache::default();
    let farm_cold = theme_from_cache(&mut ordered, &root, farm, kind).expect("farm is themed");
    let farm_warm = theme_from_cache(&mut ordered, &root, farm, kind).expect("farm is themed");
    let reedmoor_second = theme_from_cache(&mut ordered, &root, reedmoor, kind).expect("reedmoor");
    // A second fresh cache measures the reedmoor first, then the farm.
    let mut reversed = Cache::default();
    let reedmoor_first = theme_from_cache(&mut reversed, &root, reedmoor, kind).expect("reedmoor");
    let farm_later = theme_from_cache(&mut reversed, &root, farm, kind).expect("farm is themed");
    // Assert: every answer for a painting is the same theme, whatever the cache held.
    assert_eq!(
        farm_cold, farm_warm,
        "a cached answer differs from the cold one"
    );
    assert_eq!(
        farm_cold, farm_later,
        "the farm depends on the order of paintings"
    );
    assert_eq!(
        reedmoor_second, reedmoor_first,
        "reedmoor depends on the order of paintings"
    );
}

#[test]
fn a_real_painting_theme_is_repeatable_from_the_content_hash_cache() {
    let painting = "assets/ui/generated/scene-salt-quay-arrival-documentary-photo.webp";
    let first = theme_for(&asset_root(), painting, None).expect("the quay is themed");
    let second = theme_for(&asset_root(), painting, None).expect("the cached quay is themed");
    assert_eq!(first, second);
}

#[test]
fn atmosphere_follows_the_place_kind_and_the_painting_light() {
    let dim_cool = measure(&synthetic(|_, _| [18, 26, 44]));
    let bright_warm = measure(&synthetic(|_, _| [210, 150, 70]));
    let quay_dim = atmosphere_for(Some(SettingKind::Quay), &dim_cool);
    let quay_bright = atmosphere_for(Some(SettingKind::Quay), &bright_warm);
    assert_eq!(quay_dim, vec![Atmosphere::Mist]);
    assert_eq!(quay_bright, vec![Atmosphere::WaterGlints]);
}

#[test]
fn an_unknown_path_is_refused_with_a_typed_error_that_names_it() {
    let painting = "assets/ui/generated/scene-missing-test-painting.webp";
    let error = theme_for(&asset_root(), painting, None).expect_err("a missing file is refused");
    let text = error.to_string();
    assert!(
        text.contains(painting),
        "the refusal must name the painting: {text}"
    );
    assert!(matches!(error, HomeThemeError::Read { .. }), "{error:?}");
}

#[test]
fn a_painting_outside_the_asset_root_is_refused_before_any_file_is_read() {
    let error = theme_for(&asset_root(), "assets/ui/../../Cargo.toml", None)
        .expect_err("a path that escapes the asset root is refused");
    assert!(
        matches!(error, HomeThemeError::Painting { .. }),
        "{error:?}"
    );
}

#[test]
fn a_file_that_is_not_an_image_is_refused_by_name_as_undecodable() {
    let root = std::env::temp_dir().join(format!("df-home-theme-{}", std::process::id()));
    let generated = root.join("generated");
    std::fs::create_dir_all(&generated).expect("a scratch asset root");
    std::fs::write(
        generated.join("not-a-picture.webp"),
        b"this is not an image",
    )
    .expect("a scratch file");
    let painting = "assets/ui/generated/not-a-picture.webp";
    let error = theme_for(&root, painting, None).expect_err("bytes that are not an image");
    let text = error.to_string();
    assert!(
        text.contains(painting),
        "the refusal must name the painting: {text}"
    );
    assert!(matches!(error, HomeThemeError::Decode { .. }), "{error:?}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn without_an_installed_asset_root_the_lobby_view_is_the_documented_fallback() {
    let view = lobby_view(
        "assets/ui/generated/scene-salt-quay-arrival-documentary-photo.webp",
        None,
    );
    assert!(view.fallback, "a lobby with no installed theme says so");
    assert!(view.atmosphere.is_empty());
    let mood = view.mood.expect("the neutral mood is always sent");
    assert_eq!(mood.accent, "#e8cb94");
    assert_eq!(mood.saturation_permille, 1000);
}

#[test]
fn the_lobby_theme_survives_the_generated_protocol_encoding() {
    let view = super::rpc::HomeTheme {
        location_id: "salt-quay".to_owned(),
        mood: Some(super::rpc::HomeMood {
            accent: "#e8cb94".to_owned(),
            tint: "#0b1623".to_owned(),
            vignette_permille: 550,
            brightness_permille: 900,
            contrast_permille: 1000,
            saturation_permille: 1000,
        }),
        atmosphere: vec!["mist".to_owned(), "water-glints".to_owned()],
        fallback: false,
    };
    let journey = super::rpc::JourneyView {
        home: Some(view.clone()),
        ..Default::default()
    };
    let decoded = super::rpc::JourneyView::decode(journey.encode_to_vec().as_slice())
        .expect("the journey view decodes");
    assert_eq!(decoded.home, Some(view));
}

/// Every library picture of a place: the reviewed scene photographs (the art contract allows no
/// painted one).
fn arrival_paintings() -> Vec<String> {
    let mut paintings: Vec<String> = df_render::art_contract::REVIEWED_PHOTOS
        .iter()
        .filter(|stem| stem.starts_with("scene-"))
        .map(|stem| format!("assets/ui/generated/{stem}.webp"))
        .collect();
    paintings.sort();
    paintings
}
