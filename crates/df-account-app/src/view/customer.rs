//! The customer screens: dashboard, add credit, payment return, table pace, statements and
//! settings (plan 9.4, 9.7-9.9, 11, J13 and J14; decision 26).
//!
//! Customers see only what they pay: balances round down, charges round up, and there is no
//! table fee, multiplier or provider price anywhere (none exists on the customer wire types).
//! Each screen lives in its own file; the helpers they share are here.

mod credit;
mod dashboard;
mod funding;
mod history;
mod settings;
mod statements;

pub(super) use credit::{add_credit, credit_return};
pub(super) use dashboard::dashboard;
pub(super) use funding::funding;
pub(super) use history::{assets, assets_strip, play_sessions};
pub(super) use settings::settings;
pub(super) use statements::statements;

use super::{ButtonKind, button, failed, submit_button};
use crate::{
    copy,
    node::{Element, Node, el},
    state::Remote,
    wire::ApiError,
};

/// An empty placeholder so a screen keeps the same slots in the same order.
fn empty(class: &str) -> Node {
    el("div").class(class).into()
}

fn section(id: &str, title: &str) -> Element {
    el("section")
        .class("section")
        .attr("aria-labelledby", id)
        .child(el("h2").attr("id", id).text(title))
}

fn plural(count: u32, unit: &str) -> String {
    if count == 1 {
        format!("1 {unit}")
    } else {
        format!("{count} {unit}s")
    }
}

/// The submit control of a form whose one-time key may still be loading or may have failed.
fn submit_control(
    operation: &Remote<String>,
    label: &str,
    sending_label: &str,
    is_sending: bool,
) -> Node {
    match operation {
        Remote::Failed(error) => failed(*error),
        Remote::Loading => button(label, None, ButtonKind::Primary, true),
        Remote::Ready(_) => submit_button(label, sending_label, is_sending),
    }
}

/// A failure that sits beside a control the person can press again (unlike `failed`, whose retry
/// reloads the whole screen).
fn inline_problem(error: ApiError) -> Node {
    let problem = copy::problem(error);
    el("div")
        .class("problem")
        .attr("role", "alert")
        .child(el("p").class("problem-what").text(problem.what))
        .child(el("p").text(problem.fix))
        .into()
}

#[cfg(test)]
mod customer_tests;
