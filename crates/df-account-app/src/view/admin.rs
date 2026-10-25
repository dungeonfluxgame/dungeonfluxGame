//! The operator console (plan 9.13, 7.2-7.3): a sub-navigation, who is signed in, and one screen
//! per area. Operators may see the full price breakdown (table fee, multiplier, provider).
//!
//! The console shows only the areas and controls the account's capabilities allow. That is
//! presentation: the server checks the capability again on every call.

mod accounts;
mod clock;
mod parts;
mod queues;

use self::parts::{Column, can, column, data_table, empty, exact_dollars, number_column, plural};
use crate::{
    copy, money,
    node::{Node, el, text},
    route::{AdminRoute, Route},
    state::{AdminScreen, Intent, Model, Remote},
    view::{link, message_page, page_wide, remote},
    wire::{AdminSummary, AuditRow, OperatorCapability, OperatorView, ReconciliationView},
};

pub(super) fn console(screen: &AdminScreen, model: &Model) -> Node {
    let Some(operator) = model
        .account()
        .and_then(|account| account.operator.as_ref())
    else {
        return message_page(
            "This page is for DungeonFlux operators",
            "Your account does not have operator access. Your own tables and credit are on your dashboard.",
            Some(("Go to your dashboard", Route::Dashboard)),
        );
    };
    let body = if can(operator, required_capability(screen)) {
        area(screen, operator, model)
    } else {
        message_page(
            "Your operator access does not include this area",
            "Choose another area from the menu above, or ask an owner to widen your access.",
            None,
        )
    };
    el("div")
        .class("admin")
        .child(chrome(screen, operator))
        .child(body)
        .into()
}

fn required_capability(screen: &AdminScreen) -> OperatorCapability {
    match screen {
        AdminScreen::Disputes(_) => OperatorCapability::DisputeResponse,
        AdminScreen::Review(_) => OperatorCapability::ContentReview,
        AdminScreen::Summary(_)
        | AdminScreen::Accounts(_)
        | AdminScreen::Account(_)
        | AdminScreen::Reconciliation(_)
        | AdminScreen::Providers(_)
        | AdminScreen::Audit(_) => OperatorCapability::SupportInspect,
    }
}

fn area(screen: &AdminScreen, operator: &OperatorView, model: &Model) -> Node {
    let offset = model.offset_east_minutes;
    match screen {
        AdminScreen::Summary(summary) => summary_screen(summary, operator),
        AdminScreen::Accounts(search) => accounts::search(search),
        AdminScreen::Account(account) => accounts::detail(account, operator, offset),
        AdminScreen::Disputes(disputes) => {
            queues::disputes(disputes, operator, model.now_ms, offset)
        }
        AdminScreen::Reconciliation(view) => reconciliation_screen(view, offset),
        AdminScreen::Providers(providers) => queues::providers(providers, operator),
        AdminScreen::Review(review) => queues::review(review, model.now_ms, offset),
        AdminScreen::Audit(rows) => audit_screen(rows, offset),
    }
}

/// The sub-navigation entries, each with the capability that shows it.
const AREAS: [(&str, AdminRoute, OperatorCapability); 7] = [
    (
        "Summary",
        AdminRoute::Summary,
        OperatorCapability::SupportInspect,
    ),
    (
        "Accounts",
        AdminRoute::Accounts,
        OperatorCapability::SupportInspect,
    ),
    (
        "Disputes",
        AdminRoute::Disputes,
        OperatorCapability::DisputeResponse,
    ),
    (
        "Reconciliation",
        AdminRoute::Reconciliation,
        OperatorCapability::SupportInspect,
    ),
    (
        "Providers",
        AdminRoute::Providers,
        OperatorCapability::SupportInspect,
    ),
    (
        "Review queue",
        AdminRoute::Review,
        OperatorCapability::ContentReview,
    ),
    (
        "Audit log",
        AdminRoute::Audit,
        OperatorCapability::SupportInspect,
    ),
];

fn current_area(screen: &AdminScreen) -> AdminRoute {
    match screen {
        AdminScreen::Summary(_) => AdminRoute::Summary,
        AdminScreen::Accounts(_) | AdminScreen::Account(_) => AdminRoute::Accounts,
        AdminScreen::Disputes(_) => AdminRoute::Disputes,
        AdminScreen::Reconciliation(_) => AdminRoute::Reconciliation,
        AdminScreen::Providers(_) => AdminRoute::Providers,
        AdminScreen::Review(_) => AdminRoute::Review,
        AdminScreen::Audit(_) => AdminRoute::Audit,
    }
}

fn chrome(screen: &AdminScreen, operator: &OperatorView) -> Node {
    let capabilities = operator
        .capabilities
        .iter()
        .map(|capability| copy::capability_name(*capability))
        .collect::<Vec<_>>()
        .join(", ");
    let current = current_area(screen);
    let items = AREAS
        .into_iter()
        .filter(|(_, _, required)| can(operator, *required))
        .map(|(label, route, _)| {
            let is_current = route == current;
            let mut anchor = el("a")
                .class(if is_current {
                    "admin-link is-current"
                } else {
                    "admin-link"
                })
                .attr("href", Route::Admin(route.clone()).path())
                .on_click(Intent::Go(Route::Admin(route)))
                .text(label);
            if is_current {
                anchor = anchor.attr("aria-current", "page");
            }
            el("li").child(anchor).into()
        });
    el("div")
        .class("admin-chrome")
        .child(el("p").class("admin-who").text(format!(
            "Signed in as operator. Your access: {}.",
            if capabilities.is_empty() {
                "none"
            } else {
                capabilities.as_str()
            }
        )))
        .child(
            el("nav")
                .attr("aria-label", "Operator console")
                .child(el("ul").class("admin-nav").children(items)),
        )
        .into()
}

fn summary_screen(summary: &Remote<AdminSummary>, operator: &OperatorView) -> Node {
    page_wide(
        "Operator summary",
        Some("Totals across every account, and the work waiting in each area."),
    )
    .child(remote(summary, "Loading the summary", |summary| {
        summary_body(summary, operator)
    }))
    .into()
}

fn ledger_item(label: &str, value: String) -> Node {
    el("div")
        .class("ledger-item")
        .child(el("dt").text(label))
        .child(el("dd").text(value))
        .into()
}

fn summary_body(summary: &AdminSummary, operator: &OperatorView) -> Node {
    let areas = [
        (
            AdminRoute::Disputes,
            "Disputes",
            OperatorCapability::DisputeResponse,
            plural(summary.open_disputes, "open dispute", "open disputes"),
        ),
        (
            AdminRoute::Reconciliation,
            "Reconciliation",
            OperatorCapability::SupportInspect,
            plural(
                summary.reconciliation_differences,
                "difference at the last run",
                "differences at the last run",
            ),
        ),
        (
            AdminRoute::Providers,
            "Providers",
            OperatorCapability::SupportInspect,
            plural(
                summary.providers_degraded,
                "degraded provider",
                "degraded providers",
            ),
        ),
        (
            AdminRoute::Review,
            "Review queue",
            OperatorCapability::ContentReview,
            plural(summary.review_pending, "upload waiting", "uploads waiting"),
        ),
        (
            AdminRoute::Audit,
            "Audit log",
            OperatorCapability::SupportInspect,
            "Every operator action with its reason".to_owned(),
        ),
    ];
    let rows = areas
        .into_iter()
        .filter(|(_, _, required, _)| can(operator, *required))
        .map(|(route, label, _, count)| {
            el("li")
                .child(
                    el("div")
                        .class("row-main")
                        .child(link(label, Route::Admin(route), "row-title"))
                        .child(el("span").class("row-meta").text(count)),
                )
                .into()
        });
    el("div")
        .class("stack")
        .child(
            el("dl")
                .class("ledger")
                .child(ledger_item("Accounts", summary.accounts.to_string()))
                .child(ledger_item(
                    "Verified accounts",
                    summary.verified_accounts.to_string(),
                ))
                .child(ledger_item(
                    "Customer liability",
                    money::balance(summary.customer_liability_micro_dollars),
                )),
        )
        .child(
            el("section")
                .class("section")
                .child(el("h2").text("Areas"))
                .child(el("ul").class("rows").children(rows)),
        )
        .into()
}

fn reconciliation_screen(view: &Remote<ReconciliationView>, offset_east_minutes: i32) -> Node {
    page_wide(
        "Reconciliation",
        Some("Differences between our books and the processor, payouts and suppliers."),
    )
    .child(remote(view, "Loading reconciliation", |view| {
        reconciliation_body(view, offset_east_minutes)
    }))
    .into()
}

fn reconciliation_body(view: &ReconciliationView, offset_east_minutes: i32) -> Node {
    let Some(last_run_ms) = view.last_run_at_ms else {
        return empty("Reconciliation has not run yet.");
    };
    let last_run = el("p").text(format!(
        "Last run: {}.",
        money::date_time(last_run_ms, offset_east_minutes)
    ));
    if view.differences.is_empty() {
        return el("div")
            .class("stack")
            .child(last_run)
            .child(empty("No differences at the last run"))
            .into();
    }
    let columns: [Column; 4] = [
        column("Kind"),
        column("Reference"),
        number_column("Difference"),
        column("Found"),
    ];
    let rows = view
        .differences
        .iter()
        .map(|difference| {
            vec![
                text(copy::reconciliation_kind(difference.kind)),
                text(difference.reference.clone()),
                text(exact_dollars(difference.difference_micro_dollars)),
                text(money::date_time(
                    difference.found_at_ms,
                    offset_east_minutes,
                )),
            ]
        })
        .collect();
    el("div")
        .class("stack")
        .child(last_run)
        .child(data_table("Differences found", &columns, rows))
        .into()
}

fn audit_screen(rows: &Remote<Vec<AuditRow>>, offset_east_minutes: i32) -> Node {
    page_wide(
        "Audit log",
        Some("Every operator action, with the reason the operator gave."),
    )
    .child(remote(rows, "Loading the audit log", |rows| {
        audit_body(rows, offset_east_minutes)
    }))
    .into()
}

fn audit_body(rows: &[AuditRow], offset_east_minutes: i32) -> Node {
    if rows.is_empty() {
        return empty("No operator actions have been recorded yet.");
    }
    let columns = [
        column("When"),
        column("Operator"),
        column("Action"),
        column("Target"),
        column("Reason"),
    ];
    let body = rows
        .iter()
        .map(|row| {
            vec![
                text(money::date_time(row.at_ms, offset_east_minutes)),
                text(row.operator.clone()),
                text(copy::audit_action(row.action)),
                text(row.target.clone()),
                text(row.reason.clone()),
            ]
        })
        .collect();
    data_table("Operator actions", &columns, body)
}

#[cfg(test)]
mod admin_tests;
