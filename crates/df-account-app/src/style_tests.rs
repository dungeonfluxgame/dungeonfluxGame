//! The app's stylesheet follows the public site: the same tokens, the same two typefaces, and no
//! screen file that restates a token's value as a literal (coding style 10).

const SITE: &str = include_str!("../../../docs/styles.css");
const BASE: &str = include_str!("../web/css/base.css");
const PARTIALS: [(&str, &str); 3] = [
    ("customer.css", include_str!("../web/css/customer.css")),
    ("campaign.css", include_str!("../web/css/campaign.css")),
    ("admin.css", include_str!("../web/css/admin.css")),
];

/// The value of `--name` in the first `:root` block of `css`.
fn token(css: &str, name: &str) -> Option<String> {
    let root = css.split(":root").nth(1)?;
    let block = root.split('}').next()?;
    let declaration = block
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with(&format!("{name}:")))?;
    // `--x: value;` or several declarations on one line (the site writes them on separate lines).
    let value = declaration.split_once(':')?.1.split(';').next()?;
    Some(value.trim().to_owned())
}

const SHARED_TOKENS: [&str; 11] = [
    "--soot",
    "--black",
    "--smoke",
    "--smoke-2",
    "--ash",
    "--parchment",
    "--tallow",
    "--lantern",
    "--lantern-deep",
    "--tide",
    "--tide-dim",
];

#[test]
fn the_app_uses_the_public_sites_colour_tokens_unchanged() {
    for name in SHARED_TOKENS {
        let site = token(SITE, name);
        assert!(site.is_some(), "{name} is missing from docs/styles.css");
        assert_eq!(token(BASE, name), site, "{name}");
    }
}

#[test]
fn the_app_speaks_in_the_sites_two_typefaces() {
    for name in ["--serif", "--sans"] {
        assert_eq!(token(BASE, name), token(SITE, name), "{name}");
    }
    assert!(token(BASE, "--serif").is_some_and(|value| value.contains("Spectral")));
    assert!(token(BASE, "--sans").is_some_and(|value| value.contains("Barlow Semi Condensed")));
}

#[test]
fn screen_files_use_the_tokens_instead_of_restating_their_colours() {
    for (file, css) in PARTIALS {
        for name in SHARED_TOKENS {
            let Some(value) = token(BASE, name) else {
                continue;
            };
            assert!(
                !css.to_lowercase().contains(&value.to_lowercase()) || value == "#000",
                "{file} restates {name} ({value}); use var({name})"
            );
        }
    }
}

#[test]
fn no_stylesheet_forces_a_rule_with_important_unless_it_says_why() {
    for (file, css) in std::iter::once(("base.css", BASE)).chain(PARTIALS) {
        let lines: Vec<&str> = css.lines().collect();
        for (index, line) in lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.contains("!important"))
        {
            // The reason sits on the line or in the block just above it.
            let nearby = lines[index.saturating_sub(3)..=index].join(
                "
",
            );
            assert!(nearby.contains("/*"), "{file}: {line}");
        }
    }
}

#[test]
fn the_page_background_and_text_are_the_sites_soot_and_parchment() {
    assert!(BASE.contains("background: var(--soot);"));
    assert!(BASE.contains("color: var(--parchment);"));
    assert!(BASE.contains("color-scheme: dark;"));
}

#[test]
fn every_interactive_control_keeps_a_visible_keyboard_focus_ring() {
    assert!(BASE.contains(":focus-visible"));
    assert!(!BASE.contains("outline: none;\n}\n\nbutton"));
}

#[test]
fn reduced_motion_removes_every_animation() {
    assert!(BASE.contains("@media (prefers-reduced-motion: reduce)"));
    assert!(BASE.contains("animation: none !important; /* "));
}
