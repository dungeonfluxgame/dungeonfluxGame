//! The customer dashboard.

use super::{empty, plural, section};
use crate::view::{
    ButtonKind, button, error_summary, failed, link, page_wide, remote, status_line,
};
use crate::{
    copy, money,
    node::{Node, el},
    route::Route,
    state::{DashboardScreen, Intent, Model, Remote},
    wire::{OverviewView, TrialView},
    wire_campaign::{CampaignCard, DashboardView, InvitePreview},
};

pub(in crate::view) fn dashboard(screen: &DashboardScreen, model: &Model) -> Node {
    let mut content = page_wide("Your tables", None);
    if let Some(account) = model.account().filter(|account| !account.is_verified) {
        content = content.child(verify_prompt(screen, &account.email));
    }
    content
        .child(error_summary(model.play_now.error()))
        .child(button(
            "Play now",
            Some(Intent::PlayNow),
            ButtonKind::Primary,
            model.play_now.is_sending()
                || model.play_operation.ready().is_none()
                || !model.account().is_some_and(|account| account.is_verified),
        ))
        .child(el("p").class("hint").text(
            "Play now starts a table on your credit, at most the hourly price of its pace. You can change the pace or end the session on the campaign page.",
        ))
        .child(stats_section(screen))
        .child(credit_section(&screen.overview))
        .child(funds_section(screen))
        .child(assets_section(screen))
        .child(account_section_link())
        .child(invitation_section(screen, model.offset_east_minutes))
        .child(campaigns_section(
            &screen.campaigns,
            model.offset_east_minutes,
        ))
        .into()
}

fn stats_section(screen: &DashboardScreen) -> Node {
    section("stats-title", "Your play in numbers")
        .child(remote(&screen.stats, "Loading your numbers", |stats| {
            let hours = |seconds: u64| format!("{:.1}", seconds as f64 / 3600.0);
            let item = |label: &str, value: String| {
                el("div")
                    .class("ledger-item")
                    .child(el("dt").text(label))
                    .child(el("dd").text(value))
            };
            el("dl")
                .class("ledger")
                .child(item("Hours played", hours(stats.played_seconds_total)))
                .child(item(
                    "Hours played this month",
                    hours(stats.played_seconds_this_month),
                ))
                .child(item("Play sessions", stats.play_sessions.to_string()))
                .child(item(
                    "Campaigns, active",
                    stats.campaigns_active.to_string(),
                ))
                .child(item(
                    "Campaigns, archived",
                    stats.campaigns_archived.to_string(),
                ))
                .child(item("Characters", stats.characters.to_string()))
                .child(item("Players hosted", stats.players_hosted.to_string()))
                .child(item("Assets generated", stats.assets_generated.to_string()))
                .child(item(
                    "Spent this month",
                    money::charge(stats.spent_this_month_micro_dollars),
                ))
                .child(item(
                    "Spent in total",
                    money::charge(stats.spent_lifetime_micro_dollars),
                ))
                .child(item(
                    "Average spend per table-hour",
                    money::charge(stats.average_spend_per_table_hour_micro_dollars),
                ))
                .into()
        }))
        .into()
}

fn funds_section(screen: &DashboardScreen) -> Node {
    let mut body = section("funds-title", "Funds")
        .child(el("p").text(
            "Your available credit is above. Add more before a session so the table never stops for funding.",
        ))
        .child(
            el("p")
                .class("actions")
                .child(link("Top up", Route::AddCredit, "btn btn-gilt"))
                .child(link("Full history", Route::Statements, "btn btn-outline")),
        );
    body = match &screen.topups {
        Remote::Ready(lines) if !lines.is_empty() => {
            body.child(el("ul").class("rows").children(lines.iter().map(|line| {
                el("li")
                    .child(el("span").class("row-title").text(format!(
                        "{} {}",
                        copy::category_name(line.category),
                        money::balance(line.amount_micro_dollars)
                    )))
                    .into()
            })))
        }
        _ => body,
    };
    body.into()
}

fn assets_section(screen: &DashboardScreen) -> Node {
    section("dash-assets-title", "Latest assets")
        .child(remote(
            &screen.latest_assets,
            "Loading your assets",
            |view| {
                if view.assets.is_empty() {
                    el("p")
                        .class("empty")
                        .text("Nothing generated yet.")
                        .child(link(" See all assets.", Route::Assets, ""))
                        .into()
                } else {
                    super::assets_strip(&view.assets)
                }
            },
        ))
        .into()
}

fn account_section_link() -> Node {
    section("your-account-title", "Your account")
        .child(el("p").text(
            "Your email, sign-in methods, email choices, data download and closing your account.",
        ))
        .child(
            el("p")
                .class("actions")
                .child(link("Your account", Route::Settings, "btn btn-outline"))
                .child(link("Security", Route::Security, "btn btn-outline")),
        )
        .into()
}

fn verify_prompt(screen: &DashboardScreen, email: &str) -> Node {
    let mut prompt = el("section")
        .class("panel panel-warn verify-prompt")
        .attr("aria-labelledby", "verify-title")
        .child(el("h2").attr("id", "verify-title").text("Verify your email"))
        .child(el("p").text("Verify your email to get $10 of free play."))
        .child(el("p").text(format!(
            "We sent a link to {email}. Open it to finish. If nothing arrived, check your spam folder or ask for a new link."
        )))
        .child(error_summary(screen.resend.error()));
    prompt = if screen.is_resent {
        prompt.child(status_line(
            "A new link is on its way. Older links no longer work.",
        ))
    } else {
        let is_sending = screen.resend.is_sending();
        prompt.child(button(
            if is_sending {
                "Sending a new link"
            } else {
                "Send a new link"
            },
            Some(Intent::ResendVerification),
            ButtonKind::Secondary,
            is_sending,
        ))
    };
    prompt.into()
}

fn credit_section(overview: &Remote<OverviewView>) -> Node {
    section("credit-title", "Your credit")
        .child(remote(overview, "Loading your credit", overview_content))
        .child(
            el("p")
                .class("actions")
                .child(link("Add credit", Route::AddCredit, "btn btn-gilt"))
                .child(link("Statements", Route::Statements, "btn btn-outline")),
        )
        .into()
}

fn overview_content(overview: &OverviewView) -> Node {
    let balance = &overview.balance;
    let item = |label: &str, amount: String, is_quiet: bool| {
        el("div")
            .class("ledger-item")
            .child(el("dt").text(label))
            .child(if is_quiet {
                el("dd").class("is-quiet").text(amount)
            } else {
                el("dd").text(amount)
            })
    };
    el("div")
        .child(el("p").class("trial-line").text(trial_line(overview.trial)))
        .child(
            el("dl")
                .class("ledger")
                .child(item(
                    "Available purchased credit",
                    money::balance(balance.purchased_available_micro_dollars),
                    false,
                ))
                .child(item(
                    "Free play credit",
                    money::balance(balance.promotional_available_micro_dollars),
                    false,
                ))
                .child(item(
                    "Set aside for tables",
                    money::balance(balance.allocated_micro_dollars),
                    true,
                ))
                .child(item(
                    "Held for pending work",
                    money::balance(balance.reserved_micro_dollars),
                    true,
                ))
                .child(item(
                    "Used this month",
                    money::charge(balance.consumed_this_month_micro_dollars),
                    true,
                )),
        )
        .into()
}

fn trial_line(trial: TrialView) -> String {
    match trial {
        TrialView::AwaitingVerification {
            amount_micro_dollars,
        } => format!(
            "Your {} of free play credit arrives when you verify your email.",
            money::balance(amount_micro_dollars)
        ),
        TrialView::Active {
            granted_micro_dollars,
            remaining_micro_dollars,
        } => format!(
            "Free play credit: {} left of {}.",
            money::balance(remaining_micro_dollars),
            money::balance(granted_micro_dollars)
        ),
        TrialView::Spent {
            granted_micro_dollars,
        } => format!(
            "You have used all {} of your free play credit.",
            money::balance(granted_micro_dollars)
        ),
        TrialView::NotEligible => {
            "This email address already received free play credit on another account, so none is added here."
                .to_owned()
        }
    }
}

fn invitation_section(screen: &DashboardScreen, offset_east_minutes: i32) -> Node {
    remote(
        &screen.pending,
        "Checking for an invitation",
        |pending| match &pending.preview {
            Some(preview) => invitation_panel(screen, preview, offset_east_minutes),
            None => empty("invitation-slot"),
        },
    )
}

fn invitation_panel(
    screen: &DashboardScreen,
    preview: &InvitePreview,
    offset_east_minutes: i32,
) -> Node {
    let mut details = format!("Content: {}.", copy::rating_name(preview.content_rating));
    if let Some(at_ms) = preview.scheduled_at_ms {
        details.push_str(&format!(
            " Planned for {}.",
            money::date_time(at_ms, offset_east_minutes)
        ));
    }
    let is_sending = screen.join.is_sending();
    let join = match &screen.join_operation {
        Remote::Failed(error) => failed(*error),
        Remote::Loading => button(
            "Join this table",
            Some(Intent::AcceptPendingInvitation),
            ButtonKind::Primary,
            true,
        ),
        Remote::Ready(_) => button(
            if is_sending {
                "Joining the table"
            } else {
                "Join this table"
            },
            Some(Intent::AcceptPendingInvitation),
            ButtonKind::Primary,
            is_sending,
        ),
    };
    el("section")
        .class("panel panel-warn invitation")
        .attr("aria-labelledby", "invitation-title")
        .child(
            el("h2")
                .attr("id", "invitation-title")
                .text("You have an invitation"),
        )
        .child(el("p").text(format!(
            "You were invited to {} by {}. Join?",
            preview.campaign_name, preview.host_name
        )))
        .child(el("p").class("row-meta").text(details))
        .child(error_summary(screen.join.error()))
        .child(join)
        .into()
}

fn campaigns_section(campaigns: &Remote<DashboardView>, offset_east_minutes: i32) -> Node {
    section("campaigns-title", "Campaigns")
        .child(remote(campaigns, "Loading your campaigns", |view| {
            campaigns_content(view, offset_east_minutes)
        }))
        .into()
}

fn campaigns_content(view: &DashboardView, offset_east_minutes: i32) -> Node {
    let mut content = el("div");
    if view.campaigns.is_empty() {
        content = content.child(
            el("div")
                .class("empty")
                .child(el("p").text("Your first adventure starts with a table."))
                .child(el("p").text("Invited already? ").child(link(
                    "Enter a room code.",
                    Route::Join { token: None },
                    "",
                ))),
        );
    } else {
        content = content.child(
            el("ul").class("rows").children(
                view.campaigns
                    .iter()
                    .map(|card| campaign_row(card, offset_east_minutes)),
            ),
        );
    }
    let mut actions = el("p").class("actions");
    if !view.campaigns.is_empty() {
        actions = actions.child(link(
            "Join with a room code",
            Route::Join { token: None },
            "btn btn-outline",
        ));
    }
    if !actions.children.is_empty() {
        content = content.child(actions);
    }
    if !view.can_create {
        content = content.child(el("p").class("aside").text(
            "Creating a campaign needs a verified email address. You can still join a table with an invitation or a room code.",
        ));
    }
    content.into()
}

fn campaign_row(card: &CampaignCard, offset_east_minutes: i32) -> Node {
    let mut meta = vec![
        format!("Your role: {}.", copy::role_name(card.role)),
        format!("Host: {}.", card.host_name),
        format!("{}.", plural(u32::from(card.party_size), "player")),
    ];
    if let Some(at_ms) = card.last_played_at_ms {
        meta.push(format!(
            "Last played {}.",
            money::date_time(at_ms, offset_east_minutes)
        ));
    }
    if let Some(at_ms) = card.last_saved_at_ms {
        meta.push(format!(
            "Last saved {}.",
            money::date_time(at_ms, offset_east_minutes)
        ));
    }
    if card.last_played_at_ms.is_none() && card.last_saved_at_ms.is_none() {
        meta.push("Not played yet.".to_owned());
    }
    let mut title = el("span").class("row-title").text(card.name.clone());
    if card.is_archived {
        title = title
            .text(" ")
            .child(el("span").class("pill").text("Archived"));
    }
    let action = match &card.live_session_id {
        Some(session_id) => row_link(
            "Resume lobby",
            format!("Resume lobby for {}", card.name),
            Route::Lobby {
                session_id: session_id.clone(),
            },
            "btn btn-gilt btn-small",
        ),
        None => row_link(
            "Open",
            format!("Open {}", card.name),
            Route::Campaign {
                campaign_id: card.campaign_id.clone(),
            },
            "btn btn-outline btn-small",
        ),
    };
    el("li")
        .child(
            el("div")
                .class("row-main")
                .child(title)
                .child(el("span").class("row-meta").text(meta.join(" "))),
        )
        .child(el("div").class("row-actions").child(action))
        .into()
}

/// A link whose accessible name says which row it belongs to.
fn row_link(label: &str, accessible_name: String, route: Route, class: &str) -> Node {
    el("a")
        .class(class)
        .attr("href", route.path())
        .attr("aria-label", accessible_name)
        .on_click(Intent::Go(route))
        .text(label)
        .into()
}
