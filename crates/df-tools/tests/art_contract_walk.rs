//! The art contract, checked over the whole repository (`df_render::art_contract`): no content
//! file and no Rust source names a painted depiction that was removed, every content art
//! reference is a reviewed photograph, a role token or live imagery, and every library asset a
//! content file names exists on disk.
#![cfg(not(target_arch = "wasm32"))]

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use df_render::art_contract::{self, Reference};

/// Every painted depiction stem that was deleted from the library, apart from the identity keys
/// (`portrait-hero-1`, `sprite-ant-wolf`, ...), which name subjects and never files.
const REMOVED: &str = include_str!("removed_painted_stems.txt");

/// Source files that name a removed stem on purpose: they test that the stem is refused, or that
/// a name never decides anything.
const REFUSAL_FIXTURES: [&str; 11] = [
    "crates/df-render/src/art_contract.rs",
    "crates/df-render/src/battle/scene_file.rs",
    "crates/df-tools/src/gameplay/imagery/references.rs",
    "crates/df-tools/src/gameplay/startup_configuration.rs",
    "crates/df-tools/src/gameplay_browser/live_screens.rs",
    "crates/df-tools/src/gameplay_browser/live_stage.rs",
    "crates/df-tools/src/gameplay/place_music_tests.rs",
    "crates/df-tools/src/music_score.rs",
    "crates/df-tools/src/gameplay/place_music.rs",
    "crates/df-tools/tests/art_contract_walk.rs",
    "crates/df-tools/tests/removed_painted_stems.txt",
];

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn files_under(directory: &Path, extensions: &[&str], found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if path.is_dir() {
            if !matches!(
                name.to_string_lossy().as_ref(),
                "target" | "artifacts" | ".git"
            ) {
                files_under(&path, extensions, found);
            }
        } else if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extensions.contains(&extension))
        {
            found.push(path);
        }
    }
}

fn relative(path: &Path) -> String {
    path.strip_prefix(repository())
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// The stem-shaped words of a text: runs of lowercase letters, digits and dashes.
fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'))
        .filter(|word| !word.is_empty())
}

#[test]
fn no_content_or_source_file_names_a_removed_painted_stem() {
    let removed: BTreeSet<&str> = REMOVED.lines().filter(|line| !line.is_empty()).collect();
    assert!(removed.len() > 150, "the removed-stem list is loaded");
    let mut files = Vec::new();
    for root in ["crates", "content"] {
        files_under(
            &repository().join(root),
            &["rs", "json", "css", "toml"],
            &mut files,
        );
    }
    let mut offenders = Vec::new();
    for file in files {
        let name = relative(&file);
        if REFUSAL_FIXTURES.contains(&name.as_str()) {
            continue;
        }
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        for word in words(&text) {
            if removed.contains(word) {
                offenders.push(format!("{name}: {word}"));
            }
        }
    }
    offenders.sort();
    offenders.dedup();
    assert!(
        offenders.is_empty(),
        "painted depiction stems are referenced:\n{}",
        offenders.join("\n")
    );
}

fn strings_of(value: &serde_json::Value, found: &mut Vec<String>) {
    match value {
        serde_json::Value::String(text) => found.push(text.clone()),
        serde_json::Value::Array(items) => items.iter().for_each(|item| strings_of(item, found)),
        serde_json::Value::Object(map) => map.values().for_each(|item| strings_of(item, found)),
        _ => {}
    }
}

#[test]
fn every_content_art_reference_is_a_reviewed_photograph_a_role_token_or_live_and_exists() {
    let mut files = Vec::new();
    files_under(&repository().join("content"), &["json"], &mut files);
    files_under(
        &repository().join("crates/df-tools/src/gameplay/fixtures"),
        &["json"],
        &mut files,
    );
    let library = repository().join("assets/ui/generated");
    let exists = |stem: &str| {
        ["webp", "png", "jpg"]
            .iter()
            .any(|extension| library.join(format!("{stem}.{extension}")).is_file())
    };
    let mut problems = Vec::new();
    let mut checked = 0;
    for file in files {
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let mut strings = Vec::new();
        strings_of(&value, &mut strings);
        for string in strings {
            // The legacy concept-art scenes (`assets/concept-art`, `assets/ui/scenes`) are a
            // separate served surface that the library contract does not cover yet; they are
            // listed as remaining work in the hand-off.
            if string.starts_with("assets/concept-art/") || string.starts_with("assets/ui/scenes/")
            {
                continue;
            }
            // Only values shaped like an art reference: a generated-art path, or a bare stem.
            let looks_like_art = string.starts_with("assets/ui/generated/")
                || (string.len() < 80
                    && !string.contains(' ')
                    && art_contract::class_of(art_contract::stem_of(&string))
                        == art_contract::ArtClass::Depiction);
            if !looks_like_art {
                continue;
            }
            checked += 1;
            let name = relative(&file);
            match art_contract::classify(&string) {
                Reference::Reviewed(photo) => {
                    if !exists(photo) {
                        problems.push(format!("{name}: {string} is not on disk"));
                    }
                }
                Reference::Chrome(stem) if string.starts_with("assets/ui/generated/") => {
                    if !string.contains("/sfx/") && !exists(stem) {
                        problems.push(format!("{name}: {string} is not on disk"));
                    }
                }
                Reference::Token(_)
                | Reference::Live(_)
                | Reference::Empty
                | Reference::Chrome(_) => {}
                Reference::Identity { .. } | Reference::Painted(_) => {
                    problems.push(format!(
                        "{name}: {string} is painted art or an identity key"
                    ));
                }
            }
        }
    }
    assert!(
        checked > 20,
        "the walk found the art references ({checked})"
    );
    problems.sort();
    problems.dedup();
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_library_holds_only_chrome_and_reviewed_photographs() {
    let mut files = Vec::new();
    files_under(
        &repository().join("assets/ui/generated"),
        &["webp", "png", "jpg"],
        &mut files,
    );
    let reviewed: BTreeSet<&str> = art_contract::REVIEWED_PHOTOS.into_iter().collect();
    let mut painted = Vec::new();
    for file in files {
        let Some(stem) = file.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        // Dice stills live in a subdirectory and are chrome.
        if file.parent().is_some_and(|parent| parent.ends_with("dice")) {
            continue;
        }
        if art_contract::class_of(stem) == art_contract::ArtClass::Depiction
            && !reviewed.contains(stem)
        {
            painted.push(relative(&file));
        }
    }
    assert!(
        painted.is_empty(),
        "painted depictions on disk: {painted:?}"
    );
}

#[test]
fn every_manifest_entry_names_a_file_that_exists() {
    let library = repository().join("assets/ui/generated");
    let manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(library.join("source-manifest.json")).expect("source manifest"),
    )
    .expect("source manifest is JSON");
    for asset in manifest["assets"].as_array().expect("assets") {
        let path = repository().join(asset["path"].as_str().expect("path"));
        assert!(path.is_file(), "{} is listed but missing", path.display());
    }
    let reviewed: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(library.join("documentary-photo-v2.assets.json"))
            .expect("reviewed manifest"),
    )
    .expect("reviewed manifest is JSON");
    for asset in reviewed["assets"].as_array().expect("assets") {
        let file = asset["file"].as_str().expect("file");
        assert!(library.join(file).is_file(), "{file} is listed but missing");
    }
}
