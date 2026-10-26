//! The campaign screens: the create wizard, a campaign's overview, joining a table, the lobby and
//! the TV's pairing screen. Each is a function of the screen's state; the server decided every
//! permission and the page only shows what S1 returned (plan 9.4-9.6; S1 plan 7 and 8).

mod join;
mod lobby;
mod overview;
mod wizard;

pub(super) use join::join;
pub(super) use lobby::lobby;
pub(super) use overview::overview;
pub(super) use wizard::wizard;

use super::ButtonKind;
use crate::{
    node::{Element, Node, el},
    state::{Intent, Remote, Submit},
};

/// A control must wait while its request is out, and until the screen holds the one-use
/// operation id the request needs (a click without one would do nothing).
fn is_busy(submit: Submit, operation: &Remote<String>) -> bool {
    submit.is_sending() || operation.ready().is_none()
}

fn panel(title: &str) -> Element {
    el("section").class("panel").child(el("h2").text(title))
}

/// A word that carries a state (never colour alone); `tone` is one of the `pill-*` classes.
fn pill(label: &str, tone: &str) -> Node {
    el("span").class(&format!("pill {tone}")).text(label).into()
}

fn actions(buttons: Vec<Node>) -> Node {
    el("div").class("actions").children(buttons).into()
}

/// A row action repeated down a list. The visible text is the action; the accessible name starts
/// with it and adds what it acts on, so a screen reader can tell the rows apart.
fn row_button(
    label: &str,
    accessible_name: String,
    intent: Intent,
    kind: ButtonKind,
    is_busy: bool,
) -> Node {
    let mut control = el("button")
        .class(&format!("{} btn-small", kind.class()))
        .attr("type", "button")
        .attr("aria-label", accessible_name)
        .flag("disabled", is_busy)
        .on_click(intent)
        .text(label);
    if is_busy {
        control = control.attr("aria-busy", "true");
    }
    control.into()
}

#[cfg(test)]
mod campaign_tests;
