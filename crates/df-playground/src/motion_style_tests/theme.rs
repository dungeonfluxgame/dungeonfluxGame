//! The theme's motion: everything that animates or glides is switched off for a person who asked
//! for reduced motion, and a figure and its label glide together so the label stays upright.

const THEME: &str = include_str!("../../../../assets/playground/theme.css");

#[derive(Debug)]
struct Rule {
    selector: String,
    body: String,
    reduced_motion: bool,
    keyframes: bool,
}

/// The rules of a style sheet, with at-rule blocks opened. Enough for this theme, which has no
/// braces in strings.
fn rules(css: &str, reduced_motion: bool, keyframes: bool, found: &mut Vec<Rule>) {
    let mut rest = css;
    while let Some(open) = rest.find('{') {
        let selector = rest[..open].trim().to_owned();
        let mut depth = 0_usize;
        let mut close = None;
        for (index, character) in rest[open..].char_indices() {
            match character {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(open + index);
                        break;
                    }
                }
                _ => {}
            }
        }
        let close = close.expect("balanced braces");
        let body = &rest[open + 1..close];
        if selector.starts_with("@media") {
            rules(
                body,
                reduced_motion || selector.contains("prefers-reduced-motion: reduce"),
                keyframes,
                found,
            );
        } else if selector.starts_with("@keyframes") {
            found.push(Rule {
                selector,
                body: body.to_owned(),
                reduced_motion,
                keyframes: true,
            });
        } else {
            found.push(Rule {
                selector,
                body: body.to_owned(),
                reduced_motion,
                keyframes,
            });
        }
        rest = &rest[close + 1..];
    }
}

fn theme_rules() -> Vec<Rule> {
    let without_comments = {
        let mut text = String::new();
        let mut rest = THEME;
        while let Some(start) = rest.find("/*") {
            text.push_str(&rest[..start]);
            let end = rest[start..].find("*/").expect("comment ends") + start + 2;
            rest = &rest[end..];
        }
        text.push_str(rest);
        text
    };
    let mut found = Vec::new();
    rules(&without_comments, false, false, &mut found);
    found
}

fn classes(selector: &str) -> Vec<String> {
    selector
        .split(|character: char| {
            !(character.is_ascii_alphanumeric()
                || character == '-'
                || character == '.'
                || character == '_')
        })
        .filter(|word| word.starts_with('.'))
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_theme_has_its_motion_where_the_test_looks() {
    let rules = theme_rules();
    assert!(
        rules
            .iter()
            .any(|rule| rule.keyframes && rule.selector.contains("card-slide"))
    );
    assert!(
        rules
            .iter()
            .any(|rule| rule.keyframes && rule.selector.contains("camera-move"))
    );
    assert!(rules.iter().any(|rule| rule.reduced_motion));
}

#[test]
fn every_rule_that_animates_or_glides_has_a_reduced_motion_override() {
    let rules = theme_rules();
    let reduced: Vec<String> = rules
        .iter()
        .filter(|rule| rule.reduced_motion)
        .flat_map(|rule| classes(&rule.selector))
        .collect();
    let mut checked = Vec::new();
    for rule in rules
        .iter()
        .filter(|rule| !rule.reduced_motion && !rule.keyframes)
        .filter(|rule| rule.body.contains("animation") || rule.body.contains("transition"))
    {
        for class in classes(&rule.selector) {
            let covered = reduced
                .iter()
                .any(|named| class == *named || class.starts_with(&format!("{named}-")));
            assert!(
                covered,
                "{class} animates or glides ({}) but nothing under prefers-reduced-motion names it",
                rule.selector
            );
            checked.push(class);
        }
    }
    for needed in [".figure", ".figure-label", ".scene-leaving"] {
        assert!(
            checked.iter().any(|class| class == needed),
            "{needed} was not checked"
        );
    }
}

#[test]
fn the_overrides_switch_the_motion_off_and_do_not_just_mention_it() {
    let rules = theme_rules();
    let body_for = |selector: &str| -> String {
        rules
            .iter()
            .filter(|rule| {
                rule.reduced_motion
                    && classes(&rule.selector)
                        .iter()
                        .any(|class| class == selector)
            })
            .map(|rule| rule.body.clone())
            .collect()
    };
    assert!(body_for(".figure").contains("transition: none"));
    assert!(body_for(".figure-label").contains("transition: none"));
    assert!(body_for(".scene-leaving").contains("display: none"));
    assert!(body_for(".tv-card").contains("animation: none"));
    assert!(body_for(".tv-world").contains("animation: none"));
    assert!(body_for(".tv-world").contains("transition: none"));
    // The page writes the card's and the world's animation inline, so only an important rule wins.
    assert!(body_for(".tv-card").contains("!important"));
    assert!(body_for(".tv-world").contains("!important"));
}

/// The part of a transition declaration that is about `transform`: `transform 90ms linear`.
fn transform_transition(body: &str) -> String {
    let start = body.find("transition:").expect("has a transition");
    let declaration = body[start + "transition:".len()..]
        .split(';')
        .next()
        .expect("ends");
    declaration
        .split(',')
        .map(str::trim)
        .find(|part| part.starts_with("transform"))
        .expect("transitions the transform")
        .to_owned()
}

#[test]
fn a_figure_and_its_label_glide_with_the_same_duration_and_easing_so_the_label_stays_upright() {
    let rules = theme_rules();
    let declared = |selector: &str| {
        rules
            .iter()
            .find(|rule| !rule.reduced_motion && !rule.keyframes && rule.selector == selector)
            .map(|rule| transform_transition(&rule.body))
            .unwrap_or_else(|| panic!("no rule for {selector}"))
    };
    assert_eq!(declared(".figure"), declared(".figure-label"));
}

#[test]
fn the_leaving_copy_stops_the_figures_gliding_and_does_not_rely_on_the_page_alone() {
    let rules = theme_rules();
    assert!(rules.iter().any(|rule| {
        !rule.reduced_motion
            && rule.selector == ".scene-leaving .figure"
            && rule.body.contains("transition: none")
    }));
}
