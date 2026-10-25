//! The customer statements screen.

use super::inline_problem;
use crate::view::{ButtonKind, InputKind, TextField, button, page_wide, remote, text_field};
use crate::{
    copy, money,
    node::{Node, el},
    state::{Field, Intent, StatementsScreen, Submit},
    wire::{Direction, LineStatus, StatementFilter, StatementLine},
};

pub(in crate::view) fn statements(screen: &StatementsScreen, offset_east_minutes: i32) -> Node {
    let filter_button = |label: &str, filter: StatementFilter| {
        button(
            label,
            Some(Intent::SetStatementFilter(filter)),
            if screen.filter == filter {
                ButtonKind::Primary
            } else {
                ButtonKind::Secondary
            },
            false,
        )
    };
    page_wide(
        "Statements",
        Some("Every charge and credit on your account, newest first."),
    )
    .child(
        el("div")
            .class("actions")
            .attr("aria-label", "Filter statements")
            .child(filter_button("All", StatementFilter::All))
            .child(filter_button("Top-ups only", StatementFilter::TopUps))
            .child(filter_button("Spend only", StatementFilter::Spend))
            .child(button(
                "Download CSV",
                Some(Intent::DownloadStatementsCsv),
                ButtonKind::Secondary,
                false,
            )),
    )
    .child(
        el("div")
            .class("actions")
            .child(text_field(
                TextField {
                    field: Field::StatementFrom,
                    label: "From (year-month-day)",
                    value: &screen.from_text,
                    kind: InputKind::Text,
                    autocomplete: "off",
                    hint: None,
                },
                None,
            ))
            .child(text_field(
                TextField {
                    field: Field::StatementTo,
                    label: "To (year-month-day)",
                    value: &screen.to_text,
                    kind: InputKind::Text,
                    autocomplete: "off",
                    hint: None,
                },
                None,
            ))
            .child(button(
                "Show this range",
                Some(Intent::SetStatementFilter(screen.filter)),
                ButtonKind::Secondary,
                false,
            )),
    )
    .child(remote(&screen.lines, "Loading your statement", |lines| {
        statement_content(screen, lines, offset_east_minutes)
    }))
    .into()
}

fn summaries(screen: &StatementsScreen) -> Node {
    let mut content = el("div");
    if let Some(totals) = screen.totals {
        content = content.child(
            el("dl")
                .class("ledger")
                .child(summary_item(
                    "Topped up",
                    money::balance(totals.topped_up_micro_dollars),
                ))
                .child(summary_item(
                    "Free and promotional credit granted",
                    money::balance(totals.promotional_granted_micro_dollars),
                ))
                .child(summary_item(
                    "Spent",
                    money::charge(totals.spent_micro_dollars),
                ))
                .child(summary_item(
                    "Refunded",
                    money::balance(totals.refunded_micro_dollars),
                )),
        );
    }
    if !screen.monthly.is_empty() {
        let header = |label: &str| el("th").attr("scope", "col").text(label);
        content = content.child(
            el("table")
                .class("data data-stack")
                .child(el("caption").text("By month"))
                .child(
                    el("thead").child(
                        el("tr")
                            .child(header("Month"))
                            .child(header("Topped up"))
                            .child(header("Promotional"))
                            .child(header("Spent"))
                            .child(header("Refunded")),
                    ),
                )
                .child(el("tbody").children(screen.monthly.iter().map(|month| {
                    el("tr")
                        .child(el("td").text(month.month.clone()))
                        .child(
                            el("td")
                                .class("num")
                                .text(money::balance(month.topped_up_micro_dollars)),
                        )
                        .child(
                            el("td")
                                .class("num")
                                .text(money::balance(month.promotional_granted_micro_dollars)),
                        )
                        .child(
                            el("td")
                                .class("num")
                                .text(money::charge(month.spent_micro_dollars)),
                        )
                        .child(
                            el("td")
                                .class("num")
                                .text(money::balance(month.refunded_micro_dollars)),
                        )
                        .into()
                }))),
        );
    }
    if !screen.per_campaign.is_empty() {
        content = content.child(
            el("ul")
                .class("rows")
                .children(screen.per_campaign.iter().map(|row| {
                    el("li")
                        .child(el("span").class("row-title").text(format!(
                            "{}: {}",
                            row.campaign_name,
                            money::charge(row.spent_micro_dollars)
                        )))
                        .into()
                })),
        );
    }
    content.into()
}

fn summary_item(label: &str, value: String) -> Node {
    el("div")
        .class("ledger-item")
        .child(el("dt").text(label))
        .child(el("dd").text(value))
        .into()
}

fn statement_content(
    screen: &StatementsScreen,
    lines: &[StatementLine],
    offset_east_minutes: i32,
) -> Node {
    if lines.is_empty() {
        return el("p")
            .class("empty")
            .text("No charges yet. Your first session will appear here")
            .into();
    }
    let mut content = el("div").child(summaries(screen));
    if lines
        .iter()
        .any(|line| line.status == LineStatus::Provisional)
    {
        content = content.child(el("p").class("fine").text(
            "Lines marked Provisional can only go down: the final amount is never higher than shown.",
        ));
    }
    let header = |label: &str, class: &str| el("th").attr("scope", "col").class(class).text(label);
    let table = el("table")
        .class("data data-stack")
        .child(el("caption").text("Charges and credits"))
        .child(
            el("thead").child(
                el("tr")
                    .child(header("When", ""))
                    .child(header("What", ""))
                    .child(header("Status", ""))
                    .child(header("Amount", "num")),
            ),
        )
        .child(
            el("tbody").children(
                lines
                    .iter()
                    .map(|line| statement_row(line, offset_east_minutes)),
            ),
        );
    content = content.child(el("div").class("data-wrap").child(table));
    if screen.next_cursor.is_some() {
        let is_sending = screen.more.is_sending();
        if let Submit::Refused(error) = screen.more {
            content = content.child(inline_problem(error));
        }
        content = content.child(el("p").class("actions").child(button(
            if is_sending {
                "Loading older lines"
            } else {
                "Show older"
            },
            Some(Intent::LoadOlderStatements),
            ButtonKind::Secondary,
            is_sending,
        )));
    }
    content.into()
}

fn statement_row(line: &StatementLine, offset_east_minutes: i32) -> Node {
    let mut what = el("td")
        .attr("data-label", "What")
        .child(el("span").text(copy::category_name(line.category)));
    if let Some(campaign) = &line.campaign_name {
        what = what.child(
            el("span")
                .class("line-sub")
                .text(format!("Campaign: {campaign}")),
        );
    }
    if let Some(quantity) = copy::quantity(line.quantity) {
        what = what.child(el("span").class("line-sub").text(quantity));
    }
    let status_class = match line.status {
        LineStatus::Settled => "pill pill-ok",
        LineStatus::Provisional => "pill pill-warn",
        LineStatus::Pending => "pill pill-wait",
        LineStatus::Refunded => "pill",
    };
    // The direction is written as a word and a sign, never colour alone; a charge rounds up and a
    // credit rounds down (plan 6.4).
    let (direction, amount) = match line.direction {
        Direction::Charge => (
            "Charge",
            format!("-{}", money::charge(line.amount_micro_dollars)),
        ),
        Direction::Credit => (
            "Credit",
            format!("+{}", money::balance(line.amount_micro_dollars)),
        ),
    };
    el("tr")
        .child(
            el("td")
                .attr("data-label", "When")
                .text(money::date_time(line.at_ms, offset_east_minutes)),
        )
        .child(what)
        .child(
            el("td").attr("data-label", "Status").child(
                el("span")
                    .class(status_class)
                    .text(copy::line_status(line.status)),
            ),
        )
        .child(
            el("td")
                .class("num")
                .attr("data-label", "Amount")
                .child(el("span").class("direction").text(direction))
                .text(" ")
                .text(amount),
        )
        .into()
}
