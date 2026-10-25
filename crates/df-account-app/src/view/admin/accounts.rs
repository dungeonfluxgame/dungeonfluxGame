//! Account search and the account page: facts, the full ledger, refunds and goodwill credits.

use super::parts::{
    GOODWILL_FIELDS, Input, REFUND_FIELDS, Refused, Tone, can, column, data_table, empty,
    exact_dollars, input, last_result, multiplier, named, number_column, operation_problem, pill,
    reason_input,
};
use crate::{
    copy, money,
    node::{Element, Node, el, text},
    route::{AdminRoute, Route},
    state::{AccountScreen, CreditGrantForm, Field, Form, Intent, RefundForm, SearchScreen},
    view::{
        ButtonKind, InputKind, TextField, button, fact, facts, form, link, page_wide, remote,
        submit_button, text_field,
    },
    wire::{
        AdminAccountDetail, AdminAccountRow, LedgerEntry, LineStatus, OperatorCapability,
        OperatorView, RefundableLot, Standing,
    },
};

fn account_route(account_id: &str) -> Route {
    Route::Admin(AdminRoute::Account {
        account_id: account_id.to_owned(),
    })
}

fn standing_words(standing: Standing) -> &'static str {
    match standing {
        Standing::Active => "Active",
        Standing::Restricted => "Restricted: no new credit or paid play",
        Standing::Closing => "Closing",
    }
}

fn standing_pill(standing: Standing) -> Node {
    match standing {
        Standing::Active => pill(Tone::Ok, "Active"),
        Standing::Restricted => pill(Tone::Warn, "Restricted"),
        Standing::Closing => pill(Tone::Wait, "Closing"),
    }
}

fn verified_pill(is_verified: bool) -> Node {
    if is_verified {
        pill(Tone::Ok, "Verified")
    } else {
        pill(Tone::Warn, "Not verified")
    }
}

pub(super) fn search(screen: &SearchScreen) -> Node {
    let results = match &screen.results {
        None => el("p")
            .class("hint")
            .text("Results appear here after you search.")
            .into(),
        Some(results) => remote(results, "Searching", |rows| account_table(rows)),
    };
    let choice = |label: &str, intent: Intent, is_current: bool| {
        button(
            label,
            Some(intent),
            if is_current {
                ButtonKind::Primary
            } else {
                ButtonKind::Secondary
            },
            false,
        )
    };
    page_wide(
        "Find an account",
        Some("Look up one customer to see their balance and ledger."),
    )
    .child(
        el("div").class("admin-narrow").child(
            form(Form::Search, "Search accounts")
                .child(text_field(
                    TextField {
                        field: Field::SearchQuery,
                        label: "Email, account id or campaign",
                        value: &screen.query,
                        kind: InputKind::Text,
                        autocomplete: "off",
                        hint: Some("Search by email, account id or campaign name."),
                    },
                    None,
                ))
                .child(submit_button(
                    "Search accounts",
                    "Searching",
                    screen.submit.is_sending(),
                )),
        ),
    )
    .child(
        el("div")
            .class("actions")
            .attr("aria-label", "Account filters")
            .child(choice(
                "Any standing",
                Intent::SetAdminStanding(String::new()),
                screen.standing.is_empty(),
            ))
            .child(choice(
                "Active",
                Intent::SetAdminStanding("active".to_owned()),
                screen.standing == "active",
            ))
            .child(choice(
                "Closing",
                Intent::SetAdminStanding("closing".to_owned()),
                screen.standing == "closing",
            ))
            .child(choice(
                "Verified",
                Intent::SetAdminVerified("yes".to_owned()),
                screen.verified == "yes",
            ))
            .child(choice(
                "Not verified",
                Intent::SetAdminVerified("no".to_owned()),
                screen.verified == "no",
            ))
            .child(choice(
                "Has balance",
                Intent::SetAdminHasBalance("yes".to_owned()),
                screen.has_balance == "yes",
            ))
            .child(choice(
                "Sort by balance",
                Intent::SetAdminSort("balance".to_owned()),
                screen.sort == "balance",
            ))
            .child(choice(
                "Sort by last activity",
                Intent::SetAdminSort("last_activity".to_owned()),
                screen.sort == "last_activity",
            ))
            .child(choice(
                "Sort by lifetime top-ups",
                Intent::SetAdminSort("lifetime_topups".to_owned()),
                screen.sort == "lifetime_topups",
            ))
            .child(choice(
                "Sort by lifetime spend",
                Intent::SetAdminSort("lifetime_spend".to_owned()),
                screen.sort == "lifetime_spend",
            )),
    )
    .child(el("div").class("section").child(results))
    .into()
}

fn account_table(rows: &[AdminAccountRow]) -> Node {
    if rows.is_empty() {
        return empty(
            "No accounts match that search. Check the spelling, or search with the full account id.",
        );
    }
    let columns = [
        column("Email"),
        column("Name"),
        column("Email check"),
        column("Standing"),
        number_column("Balance"),
        number_column("Lifetime top-ups"),
        number_column("Lifetime spend"),
    ];
    let body = rows
        .iter()
        .map(|row| {
            vec![
                link(&row.email_masked, account_route(&row.account_id), ""),
                text(row.display_name.as_deref().unwrap_or("Not set")),
                verified_pill(row.is_verified),
                standing_pill(row.standing),
                text(money::balance(row.balance_micro_dollars)),
                text(money::balance(row.lifetime_topups_micro_dollars)),
                text(money::charge(row.lifetime_spend_micro_dollars)),
            ]
        })
        .collect();
    data_table("Accounts found", &columns, body)
}

fn section(title: &str) -> Element {
    el("section").class("section").child(el("h2").text(title))
}

pub(super) fn detail(
    screen: &AccountScreen,
    operator: &OperatorView,
    offset_east_minutes: i32,
) -> Node {
    let masked = screen
        .detail
        .ready()
        .map(|detail| detail.account.email_masked.as_str());
    let mut page = page_wide("Account", masked).child(el("p").class("aside").child(link(
        "Back to account search",
        Route::Admin(AdminRoute::Accounts),
        "",
    )));
    if let Some(last) = &screen.last {
        page = page.child(last_result(last));
    }
    page.child(remote(&screen.detail, "Loading the account", |detail| {
        account_body(screen, detail, operator, offset_east_minutes)
    }))
    .into()
}

fn tab_rows(rows: Vec<String>) -> Node {
    el("ul")
        .class("rows")
        .children(rows.into_iter().map(|row| {
            el("li")
                .child(el("span").class("row-title").text(row))
                .into()
        }))
        .into()
}

fn tab_button(label: &str, tab: crate::wire::AdminTab, current: crate::wire::AdminTab) -> Node {
    button(
        label,
        Some(Intent::SelectAdminTab(tab)),
        if tab == current {
            ButtonKind::Primary
        } else {
            ButtonKind::Secondary
        },
        false,
    )
}

fn account_body(
    screen: &AccountScreen,
    detail: &AdminAccountDetail,
    operator: &OperatorView,
    offset_east_minutes: i32,
) -> Node {
    let account = &detail.account;
    let tabs = el("div")
        .class("actions")
        .attr("aria-label", "Account sections")
        .child(tab_button(
            "Details",
            crate::wire::AdminTab::Details,
            screen.tab,
        ))
        .child(tab_button(
            "Sign-in sessions",
            crate::wire::AdminTab::SignInSessions,
            screen.tab,
        ))
        .child(tab_button(
            "Play sessions",
            crate::wire::AdminTab::PlaySessions,
            screen.tab,
        ))
        .child(tab_button(
            "Assets",
            crate::wire::AdminTab::Assets,
            screen.tab,
        ))
        .child(tab_button(
            "Funding",
            crate::wire::AdminTab::Funding,
            screen.tab,
        ))
        .child(tab_button(
            "History",
            crate::wire::AdminTab::History,
            screen.tab,
        ))
        .child(tab_button(
            "Refunds and disputes",
            crate::wire::AdminTab::RefundsDisputes,
            screen.tab,
        ))
        .child(tab_button(
            "Audit",
            crate::wire::AdminTab::Audit,
            screen.tab,
        ));
    let mut body = el("div").class("stack").child(tabs);
    body = match screen.tab {
        crate::wire::AdminTab::Details => {
            let mut details = el("div").class("stack").child(facts(vec![
                fact("Account id", account.account_id.clone()),
                fact(
                    "Email",
                    detail.email.as_deref().unwrap_or(&account.email_masked),
                ),
                fact(
                    "Display name",
                    account.display_name.as_deref().unwrap_or("Not set"),
                ),
                fact(
                    "Email verified",
                    if account.is_verified {
                        "Yes"
                    } else {
                        "Not yet"
                    },
                ),
                fact("Standing", standing_words(account.standing)),
                fact("Balance", money::balance(account.balance_micro_dollars)),
                fact(
                    "Account created",
                    if detail.created_at_ms == 0 {
                        "Not recorded".to_owned()
                    } else {
                        money::date_time(detail.created_at_ms, offset_east_minutes)
                    },
                ),
                fact(
                    "Terms",
                    detail.terms_version.as_deref().unwrap_or("Not recorded"),
                ),
                fact(
                    "Account state",
                    match detail.closure_state {
                        crate::wire::ClosureState::Open => "Open",
                        crate::wire::ClosureState::Closing => "Closing",
                    },
                ),
                fact(
                    "Sign-in methods",
                    if detail
                        .sign_in_methods
                        .contains(&crate::wire::SignInMethod::Google)
                    {
                        "Password, Google".to_owned()
                    } else {
                        "Password".to_owned()
                    },
                ),
            ]));
            if can(operator, OperatorCapability::Refund) {
                details =
                    details.child(refund_section(&screen.refund, detail, offset_east_minutes));
            }
            if can(operator, OperatorCapability::GoodwillCredit) {
                details = details.child(goodwill_section(&screen.credit));
            }
            body.child(details.child(ledger_section(&detail.ledger, offset_east_minutes)))
        }
        crate::wire::AdminTab::SignInSessions => {
            if detail.sessions.is_empty() {
                body.child(empty("No sign-in sessions are recorded for this account."))
            } else {
                body.child(tab_rows(
                    detail
                        .sessions
                        .iter()
                        .map(|session| {
                            format!(
                                "{} — signed in {}",
                                copy::device_name(session.device),
                                money::date_time(session.signed_in_at_ms, offset_east_minutes)
                            )
                        })
                        .collect(),
                ))
            }
        }
        crate::wire::AdminTab::PlaySessions => {
            if detail.play_sessions.is_empty() {
                body.child(empty("No play sessions yet."))
            } else {
                body.child(tab_rows(
                    detail
                        .play_sessions
                        .iter()
                        .map(|row| {
                            format!(
                                "{} — {}, {} players, {}",
                                row.campaign_name,
                                money::date_time(row.started_at_ms, offset_east_minutes),
                                row.players,
                                money::charge(row.spent_micro_dollars)
                            )
                        })
                        .collect(),
                ))
            }
        }
        crate::wire::AdminTab::Assets => {
            if detail.assets.is_empty() {
                body.child(empty("No assets yet."))
            } else {
                body.child(tab_rows(
                    detail
                        .assets
                        .iter()
                        .map(|row| {
                            format!(
                                "{} — {}, {}",
                                copy::asset_kind_name(row.kind),
                                row.campaign_name.as_deref().unwrap_or("No campaign"),
                                copy::asset_hold_name(row.hold_state)
                            )
                        })
                        .collect(),
                ))
            }
        }
        crate::wire::AdminTab::Funding => {
            let mut funding = el("div");
            if let Some(balance) = detail.balance {
                funding = funding.child(facts(vec![
                    fact(
                        "Purchased, available",
                        money::balance(balance.purchased_available_micro_dollars),
                    ),
                    fact(
                        "Promotional, available",
                        money::balance(balance.promotional_available_micro_dollars),
                    ),
                    fact(
                        "Set aside for campaigns",
                        money::balance(balance.allocated_micro_dollars),
                    ),
                    fact("Held", money::balance(balance.reserved_micro_dollars)),
                    fact(
                        "Used this month",
                        money::charge(balance.consumed_this_month_micro_dollars),
                    ),
                ]));
            }
            for campaign in &detail.campaign_funding {
                funding = funding.child(el("p").text(format!(
                    "{}: {} per hour ({}).",
                    campaign.name,
                    money::cents(u64::from(campaign.ceiling_cents_per_hour)),
                    copy::preset_name(campaign.preset)
                )));
            }
            body.child(funding)
        }
        crate::wire::AdminTab::History => body
            .child(facts(vec![
                fact(
                    "Topped up in range",
                    money::balance(detail.statement_totals.topped_up_micro_dollars),
                ),
                fact(
                    "Promotional granted in range",
                    money::balance(detail.statement_totals.promotional_granted_micro_dollars),
                ),
                fact(
                    "Spent in range",
                    money::charge(detail.statement_totals.spent_micro_dollars),
                ),
                fact(
                    "Refunded in range",
                    money::balance(detail.statement_totals.refunded_micro_dollars),
                ),
            ]))
            .child(ledger_section(&detail.ledger, offset_east_minutes)),
        crate::wire::AdminTab::RefundsDisputes => {
            let mut section_body = el("div");
            if can(operator, OperatorCapability::Refund) {
                section_body =
                    section_body.child(refund_section(&screen.refund, detail, offset_east_minutes));
            }
            if can(operator, OperatorCapability::GoodwillCredit) {
                section_body = section_body.child(goodwill_section(&screen.credit));
            }
            if detail.disputes.is_empty() {
                section_body = section_body.child(empty("No disputes on this account."));
            } else {
                section_body = section_body.child(el("ul").class("rows").children(
                    detail.disputes.iter().map(|dispute| {
                        el("li")
                            .child(el("span").class("row-title").text(format!(
                                "{} — {}",
                                money::cents(u64::from(dispute.amount_cents)),
                                copy::dispute_status(dispute.status)
                            )))
                            .into()
                    }),
                ));
            }
            body.child(section_body)
        }
        crate::wire::AdminTab::Audit => {
            if detail.audit.is_empty() {
                body.child(empty("No operator actions on this account yet."))
            } else {
                body.child(
                    el("ul")
                        .class("rows")
                        .children(detail.audit.iter().map(|row| {
                            el("li")
                                .child(el("span").class("row-title").text(format!(
                                    "{} by {}: {}",
                                    copy::audit_action(row.action),
                                    row.operator,
                                    row.reason
                                )))
                                .into()
                        })),
                )
            }
        }
    };
    body.into()
}

fn refund_section(
    form_state: &RefundForm,
    detail: &AdminAccountDetail,
    offset_east_minutes: i32,
) -> Node {
    let section = section("Refund to the card").child(el("p").text(
        "A refund goes back to the card that paid for the purchase, up to what is still unspent \
         on it. The processor confirms it afterwards.",
    ));
    let lots = &detail.refundable_lots;
    if lots.is_empty() {
        return section
            .child(empty(
                "Nothing on this account can be refunded to the card.",
            ))
            .into();
    }
    let chosen = form_state
        .lot_id
        .as_deref()
        .and_then(|lot_id| lots.iter().find(|lot| lot.lot_id == lot_id));
    section
        .child(
            el("ul").class("rows").children(
                lots.iter()
                    .map(|lot| lot_row(lot, chosen, offset_east_minutes)),
            ),
        )
        .child(refund_form(form_state, chosen))
        .into()
}

fn lot_row(lot: &RefundableLot, chosen: Option<&RefundableLot>, offset_east_minutes: i32) -> Node {
    let purchased = money::date_time(lot.purchased_at_ms, offset_east_minutes);
    let action = if chosen.is_some_and(|chosen| chosen.lot_id == lot.lot_id) {
        pill(Tone::Wait, "Selected")
    } else {
        named(
            button(
                "Choose this purchase",
                Some(Intent::ChooseLot {
                    lot_id: lot.lot_id.clone(),
                }),
                ButtonKind::Secondary,
                false,
            ),
            format!("Choose the purchase of {purchased}"),
        )
    };
    el("li")
        .child(
            el("div")
                .class("row-main")
                .child(
                    el("span")
                        .class("row-title")
                        .text(format!("Purchased {purchased}")),
                )
                .child(el("span").class("row-meta").text(format!(
                    "Card ending {}. Up to {} can be refunded.",
                    lot.card_suffix,
                    money::cents(u64::from(lot.refundable_cents))
                ))),
        )
        .child(el("div").class("row-actions").child(action))
        .into()
}

fn refund_form(form_state: &RefundForm, chosen: Option<&RefundableLot>) -> Node {
    let refused = Refused::new(form_state.submit.error(), REFUND_FIELDS);
    let amount_hint = match chosen {
        Some(lot) => format!(
            "Dollars, for example 12.50. At most {} for the chosen purchase.",
            money::cents(u64::from(lot.refundable_cents))
        ),
        None => "Choose a purchase above first.".to_owned(),
    };
    form(Form::Refund, "Refund to the card")
        .child(refused.summary())
        .children(operation_problem(&form_state.operation))
        .child(input(
            Input {
                field: Field::RefundAmount,
                label: "Amount to refund (dollars)",
                value: &form_state.amount,
                kind: InputKind::Dollars,
                hint: amount_hint,
            },
            &refused,
        ))
        .child(reason_input(
            Field::RefundReason,
            &form_state.reason,
            &refused,
        ))
        .child(submit_button(
            "Refund to the card",
            "Sending the refund",
            form_state.submit.is_sending(),
        ))
        .into()
}

fn goodwill_section(form_state: &CreditGrantForm) -> Node {
    let refused = Refused::new(form_state.submit.error(), GOODWILL_FIELDS);
    section("Goodwill credit")
        .child(el("p").text("Promotional credit, no cash value. There is no way to set a balance."))
        .child(
            form(Form::Goodwill, "Grant credit")
                .child(refused.summary())
                .children(operation_problem(&form_state.operation))
                .child(input(
                    Input {
                        field: Field::CreditAmount,
                        label: "Credit amount (dollars)",
                        value: &form_state.amount,
                        kind: InputKind::Dollars,
                        hint: "Dollars, for example 5.00.".to_owned(),
                    },
                    &refused,
                ))
                .child(input(
                    Input {
                        field: Field::SupportCase,
                        label: "Support case",
                        value: &form_state.support_case,
                        kind: InputKind::Text,
                        hint: "The case or ticket this credit belongs to.".to_owned(),
                    },
                    &refused,
                ))
                .child(reason_input(
                    Field::CreditReason,
                    &form_state.reason,
                    &refused,
                ))
                .child(submit_button(
                    "Grant credit",
                    "Granting credit",
                    form_state.submit.is_sending(),
                )),
        )
        .into()
}

fn ledger_section(ledger: &[LedgerEntry], offset_east_minutes: i32) -> Node {
    let section = section("Ledger");
    if ledger.is_empty() {
        return section
            .child(empty("There are no ledger lines yet."))
            .into();
    }
    let columns = [
        column("When"),
        column("Kind"),
        column("Campaign"),
        number_column("Retail"),
        number_column("Table fee"),
        number_column("Multiplier"),
        column("Provider"),
        number_column("Provider cost"),
        column("Price version"),
        column("Status"),
    ];
    let rows = ledger
        .iter()
        .map(|entry| ledger_row(entry, offset_east_minutes))
        .collect();
    section
        .child(data_table(
            "Every ledger line with its full price breakdown",
            &columns,
            rows,
        ))
        .into()
}

fn ledger_row(entry: &LedgerEntry, offset_east_minutes: i32) -> Vec<Node> {
    vec![
        text(money::date_time(entry.at_ms, offset_east_minutes)),
        text(copy::ledger_kind(entry.kind)),
        text(entry.campaign_name.as_deref().unwrap_or("None")),
        text(exact_dollars(entry.retail_micro_dollars)),
        text(exact_dollars(entry.table_fee_micro_dollars)),
        text(multiplier(entry.multiplier_permille)),
        text(entry.provider.as_deref().unwrap_or("None")),
        text(exact_dollars(entry.provider_cost_micro_dollars)),
        text(entry.price_version.clone()),
        pill(line_tone(entry.status), copy::line_status(entry.status)),
    ]
}

const fn line_tone(status: LineStatus) -> Tone {
    match status {
        LineStatus::Settled => Tone::Ok,
        LineStatus::Provisional | LineStatus::Pending => Tone::Wait,
        LineStatus::Refunded => Tone::Plain,
    }
}
