//! Native checks of the home-screen theme: the scorer on synthetic paintings with a known focal
//! region, the arrangement geometry at the reference viewports, every location painting that can
//! appear, refusals that name the painting, the documented fallback, and the lobby view carried
//! through the generated protocol.
use super::adventure::setting::SettingKind;
use super::home_theme::analysis::{self, AnalysisError, Measures, VIEWPORTS};
use super::home_theme::{
    ARRANGEMENTS, Arrangement, Atmosphere, HomeThemeError, atmosphere_for, choose_arrangement,
    lobby_view, theme_for,
};
use image::{Rgb, RgbImage};
use prost::Message;
use std::path::{Path, PathBuf};

const PARTY_SIZES: [usize; 3] = [0, 2, 6];

fn asset_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/ui")
}

/// A painting made of `pixel(x, y)`, sized like the real 16:9 arrival paintings.
fn synthetic(pixel: impl Fn(u32, u32) -> [u8; 3]) -> RgbImage {
    let (width, height) = (640, 360);
    RgbImage::from_fn(width, height, |x, y| Rgb(pixel(x, y)))
}

/// Navy night with one pale disc centred at `(cx, cy)` in painting fractions.
fn moon_at(cx: f64, cy: f64) -> RgbImage {
    synthetic(|x, y| {
        let fx = f64::from(x) / 640.0;
        let fy = f64::from(y) / 360.0;
        let distance = ((fx - cx).powi(2) + ((fy - cy) * 0.5625).powi(2)).sqrt();
        if distance < 0.07 {
            [250, 246, 226]
        } else {
            [14, 22, 40]
        }
    })
}

fn measure(image: &RgbImage) -> Measures {
    analysis::measure(image).expect("a lit synthetic painting measures")
}

fn overlaps(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

/// The arrangement's three blocks in pixels at one viewport, as `[left, top, right, bottom]`.
fn blocks_px(arrangement: &Arrangement, viewport: (u32, u32)) -> [[f64; 4]; 3] {
    let (width, height) = (f64::from(viewport.0), f64::from(viewport.1));
    arrangement.boxes().map(|[left, top, w, h]| {
        [
            width * f64::from(left) / 1000.0,
            height * f64::from(top) / 1000.0,
            width * f64::from(left + w) / 1000.0,
            height * f64::from(top + h) / 1000.0,
        ]
    })
}

#[test]
fn a_moon_in_the_upper_right_is_the_focal_region_and_the_chosen_layout_leaves_it_clear() {
    let measures = measure(&moon_at(0.8, 0.25));
    let [left, top, right, bottom] = measures.focal;
    assert!(
        left <= 0.8 && 0.8 <= right && top <= 0.25 && 0.25 <= bottom,
        "focal box {:?} must contain the moon centre",
        measures.focal
    );
    assert!(
        right - left < 0.5,
        "focal box {:?} should be the moon, not the sky",
        measures.focal
    );
    let arrangement = choose_arrangement(&measures).expect("some layout leaves the moon clear");
    let fit = analysis::fit(arrangement.boxes(), &measures);
    assert!(
        fit.is_clear(),
        "{} covers the moon: {fit:?}",
        arrangement.name
    );
}

#[test]
fn a_moon_low_on_the_left_is_found_where_it_is_painted() {
    let measures = measure(&moon_at(0.2, 0.75));
    let [left, top, right, bottom] = measures.focal;
    assert!(
        left <= 0.2 && 0.2 <= right,
        "focal {:?} misses x 0.2",
        measures.focal
    );
    assert!(
        top <= 0.75 && 0.75 <= bottom,
        "focal {:?} misses y 0.75",
        measures.focal
    );
}

#[test]
fn a_layout_that_covers_the_focal_region_is_not_clear_and_scores_it() {
    let mut measures = measure(&moon_at(0.8, 0.25));
    measures.focal = [0.0, 0.0, 0.5, 0.5];
    let title_top_left = ARRANGEMENTS
        .iter()
        .find(|arrangement| arrangement.name == "title-top-left-panels-bottom")
        .expect("the fallback arrangement is defined");
    let fit = analysis::fit(title_top_left.boxes(), &measures);
    assert!(
        !fit.is_clear(),
        "the title block covers the top-left focal region"
    );
    assert!(
        fit.focal_covered_permille
            .iter()
            .all(|covered| *covered > 0)
    );
}

#[test]
fn a_flat_black_painting_has_no_focal_region_and_is_refused_as_unlit() {
    let image = synthetic(|_, _| [0, 0, 0]);
    assert_eq!(analysis::measure(&image).err(), Some(AnalysisError::Flat));
}

#[test]
fn ground_that_is_bright_and_busy_costs_more_than_ground_that_is_dark_and_calm() {
    // The left half is dark and flat; the right half is a bright checker.
    let image = synthetic(|x, y| {
        if x < 320 {
            [12, 18, 30]
        } else if (x / 8 + y / 8) % 2 == 0 {
            [245, 240, 220]
        } else {
            [40, 30, 20]
        }
    });
    let measures = measure(&image);
    let calm = [
        [45, 45, 240, 200],
        [30, 690, 260, 290],
        [650, 690, 330, 290],
    ];
    let busy = [
        [45, 45, 240, 200],
        [600, 690, 260, 290],
        [650, 690, 330, 290],
    ];
    let calm_fit = analysis::fit(calm, &measures);
    let busy_fit = analysis::fit(busy, &measures);
    assert!(
        busy_fit.penalty > calm_fit.penalty,
        "bright busy ground {busy_fit:?} must cost more than dark calm ground {calm_fit:?}"
    );
}

#[test]
fn every_arrangement_keeps_its_blocks_inside_and_apart_at_each_reference_viewport() {
    for arrangement in ARRANGEMENTS {
        for viewport in VIEWPORTS {
            let blocks = blocks_px(&arrangement, viewport);
            for block in blocks {
                assert!(
                    block[0] >= 0.0 && block[1] >= 0.0,
                    "{} leaves the viewport at {viewport:?}",
                    arrangement.name
                );
                assert!(
                    block[2] <= f64::from(viewport.0) && block[3] <= f64::from(viewport.1),
                    "{} leaves the viewport at {viewport:?}",
                    arrangement.name
                );
            }
            for (index, block) in blocks.iter().enumerate() {
                for other in &blocks[index + 1..] {
                    assert!(
                        !overlaps(*block, *other),
                        "{} overlaps its own blocks at {viewport:?}",
                        arrangement.name
                    );
                }
            }
        }
    }
}

#[test]
fn every_arrangement_gives_the_join_block_a_qr_tile_and_the_party_two_rows_of_three() {
    // The join block holds a square QR tile of 22% of the screen height at 1080p beside the room
    // code (PLAY-Q01), so its box must be at least that tall at 1080p. Six heroes take two rows
    // of three, so the party box must hold two rows of cards at the smallest viewport.
    for arrangement in ARRANGEMENTS {
        let join_1080 = blocks_px(&arrangement, (1920, 1080))[1];
        let join_height = join_1080[3] - join_1080[1];
        assert!(
            join_height >= 0.22 * 1080.0,
            "{} join block is {join_height}px tall, short of the QR tile",
            arrangement.name
        );
        let party = blocks_px(&arrangement, (800, 577))[2];
        let width = party[2] - party[0];
        let height = party[3] - party[1];
        assert!(
            width >= 384.0 && height >= 160.0,
            "{} party box {width}x{height} at 800x577 is too small for {:?} heroes",
            arrangement.name,
            PARTY_SIZES
        );
    }
}

#[test]
fn every_location_arrival_painting_gets_a_theme_from_its_pixels() {
    let mut themed = 0;
    for painting in arrival_paintings() {
        let theme = theme_for(&asset_root(), &painting, None)
            .unwrap_or_else(|error| panic!("{painting} should have a theme: {error}"));
        assert!(
            ARRANGEMENTS
                .iter()
                .any(|arrangement| arrangement.name == theme.arrangement.name),
            "{painting} names an undefined arrangement"
        );
        assert!(theme.mood.accent.starts_with('#') && theme.mood.accent.len() == 7);
        themed += 1;
    }
    assert!(
        themed >= 8,
        "the eight arrival paintings are all themed, found {themed}"
    );
}

#[test]
fn a_real_painting_theme_is_repeatable_from_the_content_hash_cache() {
    let painting = "assets/ui/generated/scene-salt-quay-arrival.webp";
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
    let view = lobby_view("assets/ui/generated/scene-salt-quay-arrival.webp", None);
    assert!(view.fallback, "a lobby with no installed theme says so");
    assert_eq!(view.arrangement, "title-top-left-panels-bottom");
    assert!(view.title.is_none() && view.code.is_none() && view.party.is_none());
    assert!(view.atmosphere.is_empty());
    let mood = view.mood.expect("the neutral mood is always sent");
    assert_eq!(mood.accent, "#e8cb94");
    assert_eq!(mood.saturation_permille, 1000);
}

#[test]
fn the_lobby_theme_survives_the_generated_protocol_encoding() {
    let view = super::rpc::HomeTheme {
        location_id: "salt-quay".to_owned(),
        arrangement: "single-bottom-band".to_owned(),
        title: Some(super::rpc::HomeBox {
            left_permille: 30,
            top_permille: 690,
            width_permille: 320,
            height_permille: 290,
        }),
        code: None,
        party: None,
        mood: None,
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

fn arrival_paintings() -> Vec<String> {
    let text = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../content/adventure-tables/art-manifest.json"),
    )
    .expect("the art manifest is readable");
    let entries: Vec<serde_json::Value> =
        serde_json::from_str(&text).expect("the art manifest is JSON");
    let mut paintings: Vec<String> = entries
        .iter()
        .filter_map(|entry| {
            let key = entry["art_key"].as_str()?;
            // A location's home painting is its arrival, or its forecourt where the adventure
            // meets in the open (the library's lobby shows the forecourt).
            let home = key.ends_with("-arrival") || key.ends_with("-forecourt");
            (key.starts_with("loc-") && home).then(|| {
                format!(
                    "assets/ui/generated/{}.webp",
                    entry["file_stem"].as_str().unwrap_or_default()
                )
            })
        })
        .collect();
    paintings.sort();
    paintings.dedup();
    paintings
}

/// Development report, not a gate: writes each location painting's focal box and how every
/// arrangement covers it, to the file named by `DF_HOME_THEME_REPORT`. Run with `--ignored`.
#[test]
#[ignore = "development report; set DF_HOME_THEME_REPORT to a file path"]
fn report_home_theme_focal_and_coverage_for_each_location_painting() {
    let Some(path) = std::env::var_os("DF_HOME_THEME_REPORT") else {
        return;
    };
    let mut report = Vec::new();
    for painting in arrival_paintings() {
        let file = super::home_theme::painting_file(&asset_root(), &painting)
            .expect("the painting file resolves");
        let image = image::open(&file)
            .expect("the painting decodes")
            .into_rgb8();
        let measures = measure(&image);
        let fits: Vec<serde_json::Value> = ARRANGEMENTS
            .iter()
            .map(|arrangement| {
                let fit = analysis::fit(arrangement.boxes(), &measures);
                serde_json::json!({
                    "arrangement": arrangement.name,
                    "covered_permille": fit.focal_covered_permille,
                    "penalty": fit.penalty,
                })
            })
            .collect();
        report.push(serde_json::json!({
            "painting": painting,
            "size": measures.size,
            "focal": measures.focal,
            "mean_luminance": measures.mean_luminance,
            "warm_balance": measures.warm_balance,
            "fits": fits,
        }));
    }
    std::fs::write(
        path,
        serde_json::to_string_pretty(&report).expect("the report serialises"),
    )
    .expect("the report file is writable");
}
