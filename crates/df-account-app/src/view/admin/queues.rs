//! The lists an operator works through: disputes, providers and the review queue. Each row can
//! open one reason form; the server records the reason in the audit log.

use super::{
    clock::{TimeLeft, time_left},
    parts::{
        ReasonPanel, Tone, can, column, data_table, empty, exact_dollars, last_result, named,
        number_column, pill, rate, reason_panel,
    },
};
use crate::{
    api::ProviderSwitch,
    copy, money,
    node::{Node, el, text},
    route::{AdminRoute, Route},
    state::{ActionTarget, Intent, ReasonAction, RowsScreen},
    view::{ButtonKind, button, link, page_wide, remote},
    wire::{
        DisputeRow, DisputeStatus, OperatorCapability, OperatorView, ProviderHealth, ProviderRow,
        ReviewDecision, ReviewItem, ReviewKind,
    },
};

pub(super) fn disputes(
    screen: &RowsScreen<DisputeRow>,
    operator: &OperatorView,
    now_ms: u64,
    offset_east_minutes: i32,
) -> Node {
    let mut page = page_wide(
        "Disputes",
        Some("Card disputes opened by customers. Send evidence before the due time."),
    );
    if let Some(last) = &screen.last {
        page = page.child(last_result(last));
    }
    if let Some(action) = &screen.action {
        page = page.child(dispute_panel(action));
    }
    page.child(remote(&screen.rows, "Loading disputes", |rows| {
        dispute_table(rows, operator, now_ms, offset_east_minutes)
    }))
    .into()
}

fn dispute_panel(action: &ReasonAction) -> Node {
    let ActionTarget::DisputeEvidence { dispute_id } = &action.target else {
        return el("div").into();
    };
    reason_panel(ReasonPanel {
        title: format!("Send evidence for dispute {dispute_id}"),
        detail: "Say what evidence you are sending to the card company.",
        submit_label: "Send evidence",
        sending_label: "Sending evidence",
        action,
    })
}

fn dispute_table(
    rows: &[DisputeRow],
    operator: &OperatorView,
    now_ms: u64,
    offset_east_minutes: i32,
) -> Node {
    if rows.is_empty() {
        return empty("There are no disputes.");
    }
    let columns = [
        column("Account"),
        number_column("Amount"),
        column("Status"),
        column("Opened"),
        column("Evidence due"),
        column("Action"),
    ];
    let body = rows
        .iter()
        .map(|row| {
            vec![
                account_cell(&row.account_id, operator),
                text(money::cents(u64::from(row.amount_cents))),
                pill(dispute_tone(row.status), copy::dispute_status(row.status)),
                text(money::date_time(row.opened_at_ms, offset_east_minutes)),
                evidence_due(row, now_ms, offset_east_minutes),
                dispute_action(row, operator),
            ]
        })
        .collect();
    data_table("Disputes", &columns, body)
}

/// The account page needs the inspect capability, so without it the id is plain text.
fn account_cell(account_id: &str, operator: &OperatorView) -> Node {
    if can(operator, OperatorCapability::SupportInspect) {
        link(
            account_id,
            Route::Admin(AdminRoute::Account {
                account_id: account_id.to_owned(),
            }),
            "",
        )
    } else {
        text(account_id)
    }
}

const fn dispute_tone(status: DisputeStatus) -> Tone {
    match status {
        DisputeStatus::NeedsResponse => Tone::Warn,
        DisputeStatus::UnderReview => Tone::Wait,
        DisputeStatus::Won => Tone::Ok,
        DisputeStatus::Lost => Tone::Bad,
    }
}

fn evidence_due(row: &DisputeRow, now_ms: u64, offset_east_minutes: i32) -> Node {
    let due = money::date_time(row.evidence_due_at_ms, offset_east_minutes);
    if row.status != DisputeStatus::NeedsResponse {
        return text(due);
    }
    let left = time_left(row.evidence_due_at_ms, now_ms);
    el("div")
        .child(el("div").text(due))
        .child(pill(due_tone(left), left.due()))
        .into()
}

fn due_tone(left: TimeLeft) -> Tone {
    if left.is_overdue() {
        Tone::Bad
    } else if left.is_within_a_day() {
        Tone::Warn
    } else {
        Tone::Wait
    }
}

fn dispute_action(row: &DisputeRow, operator: &OperatorView) -> Node {
    if row.status != DisputeStatus::NeedsResponse
        || !can(operator, OperatorCapability::DisputeResponse)
    {
        return text("No action needed");
    }
    named(
        button(
            "Submit evidence",
            Some(Intent::OpenAction(ActionTarget::DisputeEvidence {
                dispute_id: row.dispute_id.clone(),
            })),
            ButtonKind::Secondary,
            false,
        ),
        format!("Submit evidence for dispute {}", row.dispute_id),
    )
}

pub(super) fn providers(screen: &RowsScreen<ProviderRow>, operator: &OperatorView) -> Node {
    let mut page = page_wide(
        "Providers",
        Some(
            "Pausing a provider stops new paid work for that provider. It does not stop \
             read-only access to games.",
        ),
    );
    if let Some(last) = &screen.last {
        page = page.child(last_result(last));
    }
    if let Some(action) = &screen.action {
        let rows = screen.rows.ready().map(Vec::as_slice).unwrap_or_default();
        page = page.child(provider_panel(action, rows));
    }
    page.child(remote(&screen.rows, "Loading providers", |rows| {
        provider_list(rows, operator)
    }))
    .into()
}

fn provider_panel(action: &ReasonAction, rows: &[ProviderRow]) -> Node {
    let ActionTarget::Provider {
        provider_id,
        switch,
    } = &action.target
    else {
        return el("div").into();
    };
    let name = rows
        .iter()
        .find(|row| &row.provider_id == provider_id)
        .map_or(provider_id.as_str(), |row| row.name.as_str());
    let (title, detail, submit_label, sending_label) = match switch {
        ProviderSwitch::Pause => (
            format!("Pause provider {name}"),
            "New paid work stops using this provider. Game read-only access keeps working.",
            "Pause provider",
            "Pausing provider",
        ),
        ProviderSwitch::Resume => (
            format!("Resume provider {name}"),
            "New paid work may use this provider again.",
            "Resume provider",
            "Resuming provider",
        ),
    };
    reason_panel(ReasonPanel {
        title,
        detail,
        submit_label,
        sending_label,
        action,
    })
}

fn provider_list(rows: &[ProviderRow], operator: &OperatorView) -> Node {
    if rows.is_empty() {
        return empty("No providers are configured.");
    }
    el("ul")
        .class("rows")
        .children(rows.iter().map(|row| provider_row(row, operator)))
        .into()
}

fn provider_row(row: &ProviderRow, operator: &OperatorView) -> Node {
    let mut actions = el("div").class("row-actions").child(pill(
        health_tone(row.health),
        copy::provider_health(row.health),
    ));
    if can(operator, OperatorCapability::ProviderControl) {
        actions = actions.child(provider_button(row));
    }
    el("li")
        .child(
            el("div")
                .class("row-main")
                .child(el("span").class("row-title").text(row.name.clone()))
                .child(el("span").class("row-meta").text(format!(
                    "Error rate {}. Open exposure {}.",
                    rate(row.error_rate_permille),
                    exact_dollars(row.open_exposure_micro_dollars)
                ))),
        )
        .child(actions)
        .into()
}

fn provider_button(row: &ProviderRow) -> Node {
    let (label, switch) = match row.health {
        ProviderHealth::Paused => ("Resume provider", ProviderSwitch::Resume),
        ProviderHealth::Healthy | ProviderHealth::Degraded => {
            ("Pause provider", ProviderSwitch::Pause)
        }
    };
    named(
        button(
            label,
            Some(Intent::OpenAction(ActionTarget::Provider {
                provider_id: row.provider_id.clone(),
                switch,
            })),
            ButtonKind::Secondary,
            false,
        ),
        format!("{label} {}", row.name),
    )
}

const fn health_tone(health: ProviderHealth) -> Tone {
    match health {
        ProviderHealth::Healthy => Tone::Ok,
        ProviderHealth::Degraded => Tone::Warn,
        ProviderHealth::Paused => Tone::Bad,
    }
}

pub(super) fn review(
    screen: &RowsScreen<ReviewItem>,
    now_ms: u64,
    offset_east_minutes: i32,
) -> Node {
    let mut page = page_wide(
        "Review queue",
        Some("Uploaded portraits that the automatic screen flagged, soonest deadline first."),
    )
    .child(
        el("p")
            .class("aside")
            // TODO(O2): show the image preview here once the media gateway serves it to operators.
            .text(
                "The image itself is shown only through the media gateway, not on this screen. \
                 Approving means the portrait may be shown to the campaign. Past its deadline an \
                 upload is rejected unreviewed.",
            ),
    );
    if let Some(last) = &screen.last {
        page = page.child(last_result(last));
    }
    if let Some(action) = &screen.action {
        let rows = screen.rows.ready().map(Vec::as_slice).unwrap_or_default();
        page = page.child(review_panel(action, rows));
    }
    page.child(remote(&screen.rows, "Loading the review queue", |rows| {
        review_list(rows, now_ms, offset_east_minutes)
    }))
    .into()
}

fn review_panel(action: &ReasonAction, rows: &[ReviewItem]) -> Node {
    let ActionTarget::Review { item_id, decision } = &action.target else {
        return el("div").into();
    };
    let campaign = rows
        .iter()
        .find(|row| &row.item_id == item_id)
        .map_or(item_id.as_str(), |row| row.campaign_name.as_str());
    let (verb, detail, sending_label) = match decision {
        ReviewDecision::Approve => (
            "Approve upload",
            "The portrait may be shown to the campaign.",
            "Approving upload",
        ),
        ReviewDecision::Reject => (
            "Reject upload",
            "The portrait is removed and is never shown to the campaign.",
            "Rejecting upload",
        ),
    };
    reason_panel(ReasonPanel {
        title: format!("{verb} for {campaign}"),
        detail,
        submit_label: verb,
        sending_label,
        action,
    })
}

fn review_list(rows: &[ReviewItem], now_ms: u64, offset_east_minutes: i32) -> Node {
    if rows.is_empty() {
        return empty("Nothing is waiting for review");
    }
    let mut sorted: Vec<&ReviewItem> = rows.iter().collect();
    sorted.sort_by(|left, right| {
        left.review_deadline_ms
            .cmp(&right.review_deadline_ms)
            .then_with(|| left.item_id.cmp(&right.item_id))
    });
    el("ul")
        .class("rows")
        .children(
            sorted
                .into_iter()
                .map(|item| review_row(item, now_ms, offset_east_minutes)),
        )
        .into()
}

fn review_row(item: &ReviewItem, now_ms: u64, offset_east_minutes: i32) -> Node {
    let left = time_left(item.review_deadline_ms, now_ms);
    let deadline = money::date_time(item.review_deadline_ms, offset_east_minutes);
    let (tone, state, deadline_words) = if left.is_overdue() {
        (
            Tone::Bad,
            "Past the deadline".to_owned(),
            format!(
                "Review by {deadline}. Past the deadline: the upload will be rejected unreviewed."
            ),
        )
    } else {
        let state = if left.is_within_a_day() {
            "Due soon"
        } else {
            "Waiting"
        };
        (
            if left.is_within_a_day() {
                Tone::Warn
            } else {
                Tone::Wait
            },
            state.to_owned(),
            format!("Review by {deadline} (due in {})", left.span()),
        )
    };
    let kind = match item.kind {
        ReviewKind::UploadedPortrait => "Uploaded portrait",
    };
    el("li")
        .child(
            el("div")
                .class("row-main")
                .child(
                    el("span")
                        .class("row-title")
                        .text(item.campaign_name.clone()),
                )
                .child(el("span").class("row-meta").text(format!(
                    "{kind}: {}. Submitted {}.",
                    copy::review_flag(item.flag),
                    money::date_time(item.submitted_at_ms, offset_east_minutes)
                )))
                .child(
                    el("span")
                        .class("row-meta")
                        .child(pill(tone, state))
                        .child(text(format!(" {deadline_words}"))),
                ),
        )
        .child(
            el("div")
                .class("row-actions")
                .child(decision_button(item, ReviewDecision::Approve))
                .child(decision_button(item, ReviewDecision::Reject)),
        )
        .into()
}

fn decision_button(item: &ReviewItem, decision: ReviewDecision) -> Node {
    let (label, kind) = match decision {
        ReviewDecision::Approve => ("Approve", ButtonKind::Secondary),
        ReviewDecision::Reject => ("Reject", ButtonKind::Destructive),
    };
    named(
        button(
            label,
            Some(Intent::OpenAction(ActionTarget::Review {
                item_id: item.item_id.clone(),
                decision,
            })),
            kind,
            false,
        ),
        format!("{label} the upload for {}", item.campaign_name),
    )
}
