//! Play-session history and generated assets.

use crate::view::{ButtonKind, InputKind, TextField, button, link, page_wide, remote, text_field};
use crate::{
    copy, money,
    node::{Node, el},
    route::Route,
    state::{AssetsScreen, Field, Intent, PlaySessionsScreen},
    wire::{AssetKind, AssetRow, PlaySessionRow},
};

pub(in crate::view) fn play_sessions(screen: &PlaySessionsScreen, offset: i32) -> Node {
    page_wide(
        "Play sessions",
        Some("Every table session you took part in, newest first."),
    )
    .child(remote(
        &screen.sessions,
        "Loading your play sessions",
        |rows| sessions_content(screen, rows, offset),
    ))
    .into()
}

fn sessions_content(screen: &PlaySessionsScreen, rows: &[PlaySessionRow], offset: i32) -> Node {
    if rows.is_empty() {
        return el("p")
            .class("empty")
            .text("You have not played a session yet.")
            .into();
    }
    let list = el("ul").class("rows").children(rows.iter().map(|row| {
        let ended = row.ended_at_ms.map_or_else(
            || "still playing".to_owned(),
            |at| money::date_time(at, offset),
        );
        el("li")
            .child(el("span").class("row-title").text(format!(
                "{} — started {}",
                row.campaign_name,
                money::date_time(row.started_at_ms, offset)
            )))
            .child(el("span").class("row-meta").text(format!(
                "Ended {ended}. Length {}. {} players. Spend {}.",
                money::duration(row.duration_seconds),
                row.players,
                money::charge(row.spent_micro_dollars)
            )))
            .child(if row.ended_at_ms.is_some() {
                link(
                    "Share session",
                    Route::Share {
                        session_id: row.session_id.clone(),
                    },
                    "button",
                )
            } else {
                el("span").into()
            })
            .into()
    }));
    let mut content = el("div").child(list);
    if screen.next_cursor.is_some() {
        let sending = screen.more.is_sending();
        content = content.child(el("p").class("actions").child(button(
            if sending {
                "Loading older sessions"
            } else {
                "Show older"
            },
            Some(Intent::LoadOlderPlaySessions),
            ButtonKind::Secondary,
            sending,
        )));
    }
    content.into()
}

pub(in crate::view) fn assets(screen: &AssetsScreen, offset: i32) -> Node {
    let mut page = page_wide(
        "Assets",
        Some("Pictures, music and video made for your tables."),
    );
    let filters = el("div")
        .class("actions")
        .attr("aria-label", "Filter assets by kind")
        .child(filter_button(screen, "All kinds", None))
        .child(filter_button(
            screen,
            "Portraits",
            Some(AssetKind::Portrait),
        ))
        .child(filter_button(
            screen,
            "Scene pictures",
            Some(AssetKind::Scene),
        ))
        .child(filter_button(screen, "Music", Some(AssetKind::Music)))
        .child(filter_button(screen, "Video", Some(AssetKind::Video)));
    page = page.child(filters);
    page = page.child(
        el("div")
            .class("actions")
            .child(text_field(
                TextField {
                    field: Field::AssetCampaignFilter,
                    label: "Campaign id",
                    value: &screen.campaign,
                    kind: InputKind::Text,
                    autocomplete: "off",
                    hint: Some("Leave empty for every campaign."),
                },
                None,
            ))
            .child(button(
                "Show this campaign",
                Some(Intent::SetAssetKind(screen.kind)),
                ButtonKind::Secondary,
                false,
            )),
    );
    page.child(remote(&screen.assets, "Loading your assets", |rows| {
        assets_content(screen, rows, offset)
    }))
    .into()
}

fn filter_button(screen: &AssetsScreen, label: &str, kind: Option<AssetKind>) -> Node {
    button(
        label,
        Some(Intent::SetAssetKind(kind)),
        if screen.kind == kind {
            ButtonKind::Primary
        } else {
            ButtonKind::Secondary
        },
        false,
    )
}

fn assets_content(screen: &AssetsScreen, rows: &[AssetRow], offset: i32) -> Node {
    if rows.is_empty() {
        return el("p")
            .class("empty")
            .text("No assets match. Generate a portrait or scene picture at a table and it appears here.")
            .into();
    }
    let mut content = el("div").child(
        el("p")
            .class("fine")
            .text(format!("{} assets in total.", screen.total)),
    );
    let list = el("ul")
        .class("rows")
        .children(rows.iter().map(|row| asset_row(row, offset)));
    content = content.child(list);
    if screen.next_cursor.is_some() {
        let sending = screen.more.is_sending();
        content = content.child(el("p").class("actions").child(button(
            if sending {
                "Loading more assets"
            } else {
                "Show more"
            },
            Some(Intent::LoadOlderAssets),
            ButtonKind::Secondary,
            sending,
        )));
    }
    content.into()
}

fn asset_row(row: &AssetRow, offset: i32) -> Node {
    let mut main = el("div").class("row-main").child(
        el("span")
            .class("row-title")
            .text(copy::asset_kind_name(row.kind)),
    );
    if let Some(url) = &row.thumbnail_url {
        main = main.child(
            el("img")
                .class("asset-thumb")
                .attr("src", url.clone())
                .attr("alt", "")
                .attr("width", "96")
                .attr("height", "96"),
        );
    }
    let campaign = row.campaign_name.as_deref().unwrap_or("No campaign");
    main = main.child(el("span").class("row-meta").text(format!(
        "{campaign}. Made {}. {} bytes. {}.",
        money::date_time(row.created_at_ms, offset),
        row.size_bytes,
        copy::asset_hold_name(row.hold_state)
    )));
    el("li").child(main).into()
}

pub(in crate::view) fn assets_strip(rows: &[AssetRow]) -> Node {
    let mut strip = el("ul").class("rows");
    for row in rows.iter().take(5) {
        strip = strip.child(asset_row(row, 0));
    }
    strip
        .child(el("li").child(link(
            "See all assets",
            Route::Assets,
            "btn btn-outline btn-small",
        )))
        .into()
}
