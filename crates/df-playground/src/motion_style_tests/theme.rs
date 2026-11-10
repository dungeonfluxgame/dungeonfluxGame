//! The page's motion: no style sheet honours a reduced-motion setting (the owner's decision, so
//! every animation and transition runs at full motion whatever the system says), and a figure and
//! its label glide together so the label stays upright.

const THEME: &str = include_str!("../../../../assets/playground/theme.css");
const KIT: &str = include_str!("../../../../assets/playground/kit.css");

#[derive(Debug)]
struct Rule {
    selector: String,
    body: String,
    keyframes: bool,
}

/// The rules of a style sheet, with at-rule blocks opened. Enough for this theme, which has no
/// braces in strings.
fn rules(css: &str, keyframes: bool, found: &mut Vec<Rule>) {
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
            rules(body, keyframes, found);
        } else if selector.starts_with("@keyframes") {
            found.push(Rule {
                selector,
                body: body.to_owned(),
                keyframes: true,
            });
        } else {
            found.push(Rule {
                selector,
                body: body.to_owned(),
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
    rules(&without_comments, false, &mut found);
    found
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
}

#[test]
fn no_style_sheet_of_the_page_honours_a_reduced_motion_setting() {
    for (name, css) in [("theme.css", THEME), ("kit.css", KIT)] {
        assert!(
            !css.contains("prefers-reduced-motion"),
            "{name} names prefers-reduced-motion"
        );
    }
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
            .find(|rule| !rule.keyframes && rule.selector == selector)
            .map(|rule| transform_transition(&rule.body))
            .unwrap_or_else(|| panic!("no rule for {selector}"))
    };
    assert_eq!(declared(".figure"), declared(".figure-label"));
}

#[test]
fn the_leaving_copy_stops_the_figures_gliding_and_does_not_rely_on_the_page_alone() {
    let rules = theme_rules();
    assert!(rules.iter().any(|rule| {
        rule.selector == ".scene-leaving .figure" && rule.body.contains("transition: none")
    }));
}
