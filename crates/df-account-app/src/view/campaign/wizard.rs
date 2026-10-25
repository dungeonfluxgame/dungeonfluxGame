//! The create-campaign wizard at `/app/campaigns/new` (plan 9.5; S1 8.1): five steps, one saved
//! draft, and a summary before the free "Create campaign".

use super::pill;
use crate::{
    copy, money,
    node::{Node, el},
    route::Route,
    state::{
        Field, Form, Intent, MAXIMUM_CAMPAIGN_NAME_CHARS, Model, Remote, WIZARD_STEPS, WizardScreen,
    },
    view::{
        ButtonKind, InputKind, RadioChoice, TextField, button, checkbox, error_summary, fact,
        facts, failed, field_error, form, link, loading, page, radio_group, submit_button,
        text_field,
    },
    wire::{ApiError, PacePreset},
    wire_campaign::{CampaignDraft, FundingModel, JoinMode, PictureDefault, WizardCatalog},
};

pub(in crate::view) fn wizard(screen: &WizardScreen, model: &Model) -> Node {
    let is_verified = model.account().is_some_and(|account| account.is_verified);
    if !is_verified {
        return unverified();
    }
    let shell = page("Create a campaign", None);
    let catalog = match (&screen.catalog, &screen.draft_id) {
        (Remote::Failed(error), _) | (_, Remote::Failed(error)) => {
            return shell.child(failed(*error)).into();
        }
        (Remote::Ready(catalog), Remote::Ready(_)) => catalog,
        _ => return shell.child(loading("Getting your campaign ready")).into(),
    };
    let step = screen.step.clamp(1, WIZARD_STEPS);
    let error = screen.save.error();
    let fields = match step {
        1 => basics(screen, catalog, error),
        2 => heroes(),
        3 => look_and_pace(screen, catalog, error),
        4 => joining_and_funding(&screen.draft, error),
        _ => summary(&screen.draft, catalog),
    };
    let (submitted, label) = if step == WIZARD_STEPS {
        (Form::CreateCampaign, "Create campaign")
    } else {
        (Form::WizardStep, "Campaign details")
    };
    shell
        .child(progress(step))
        .child(
            form(submitted, label)
                .child(error_summary(error))
                .child(el("h2").text(step_title(step)))
                .children(fields)
                .child(buttons(screen, step)),
        )
        .into()
}

fn unverified() -> Node {
    page(
        "Create a campaign",
        Some("Verify your email to create a campaign"),
    )
    .child(el("p").text(
        "Open the link we emailed you, or send a new one from your dashboard. You can \
             already join a table you were invited to.",
    ))
    .child(el("p").class("actions").child(link(
        "Go to your dashboard",
        Route::Dashboard,
        "btn btn-gilt",
    )))
    .into()
}

const fn step_title(step: u8) -> &'static str {
    match step {
        1 => "Basics",
        2 => "Heroes",
        3 => "Voice, pictures and pace",
        4 => "Joining and funding",
        _ => "Summary",
    }
}

fn progress(current: u8) -> Node {
    let mut steps = el("ol").class("wizard-steps");
    for step in 1..=WIZARD_STEPS {
        let title = step_title(step);
        let item = if step == current {
            el("li")
                .class("wizard-step is-current")
                .attr("aria-current", "step")
                .text(format!("Step {step} of {WIZARD_STEPS}: {title}"))
        } else if step < current {
            el("li")
                .class("wizard-step")
                .text(format!("{step}. {title} (done)"))
        } else {
            el("li")
                .class("wizard-step")
                .text(format!("{step}. {title}"))
        };
        steps = steps.child(item);
    }
    el("nav").attr("aria-label", "Progress").child(steps).into()
}

fn buttons(screen: &WizardScreen, step: u8) -> Node {
    let mut row = el("div").class("actions");
    if step > 1 {
        row = row.child(button(
            "Back",
            Some(Intent::WizardBack),
            ButtonKind::Secondary,
            screen.save.is_sending(),
        ));
    }
    if step < WIZARD_STEPS {
        return row
            .child(submit_button(
                "Save and continue",
                "Saving...",
                screen.save.is_sending(),
            ))
            .into();
    }
    let creation = match &screen.operation {
        Remote::Failed(error) => failed(*error),
        Remote::Loading => button("Create campaign", None, ButtonKind::Primary, true),
        Remote::Ready(_) => {
            submit_button("Create campaign", "Creating...", screen.save.is_sending())
        }
    };
    row.child(creation).into()
}

fn basics(screen: &WizardScreen, catalog: &WizardCatalog, error: Option<ApiError>) -> Vec<Node> {
    let draft = &screen.draft;
    let name_hint = format!("Up to {MAXIMUM_CAMPAIGN_NAME_CHARS} characters.");
    vec![
        text_field(
            TextField {
                field: Field::CampaignName,
                label: "Campaign name",
                value: &draft.name,
                kind: InputKind::Text,
                autocomplete: "off",
                hint: Some(&name_hint),
            },
            error,
        ),
        adventure_choice(draft, catalog, error),
        rating_choice(draft, catalog, error),
        seat_select(draft.seat_count, catalog.maximum_seats, error),
    ]
}

fn adventure_choice(
    draft: &CampaignDraft,
    catalog: &WizardCatalog,
    error: Option<ApiError>,
) -> Node {
    if catalog.adventures.is_empty() {
        return el("p")
            .class("hint")
            .text("No adventures are available right now. Try again later.")
            .into();
    }
    let choices = catalog
        .adventures
        .iter()
        .map(|adventure| RadioChoice {
            field: Field::Adventure,
            value: &adventure.adventure_id,
            title: adventure.name.clone(),
            detail: format!("{} Rules: {}", adventure.summary, adventure.rules_text),
            is_checked: draft.adventure_id.as_deref() == Some(adventure.adventure_id.as_str()),
        })
        .collect();
    radio_group("Adventure", Field::Adventure, choices, error)
}

/// Only the ratings the chosen adventure allows can be picked.
fn rating_choice(draft: &CampaignDraft, catalog: &WizardCatalog, error: Option<ApiError>) -> Node {
    let chosen = draft.adventure_id.as_deref().and_then(|adventure_id| {
        catalog
            .adventures
            .iter()
            .find(|adventure| adventure.adventure_id == adventure_id)
    });
    let Some(adventure) = chosen else {
        return el("p")
            .class("hint")
            .attr("id", Field::Rating.name())
            .text("Choose an adventure to see the content ratings it allows.")
            .into();
    };
    let choices = adventure
        .ratings
        .iter()
        .map(|rating| RadioChoice {
            field: Field::Rating,
            value: copy::rating_value(*rating),
            title: copy::rating_name(*rating).to_owned(),
            detail: String::new(),
            is_checked: draft.content_rating == Some(*rating),
        })
        .collect();
    radio_group("Content rating", Field::Rating, choices, error)
}

fn seat_select(seat_count: u8, maximum_seats: u8, error: Option<ApiError>) -> Node {
    let id = Field::SeatCount.name();
    let hint_id = format!("{id}-hint");
    let message = field_error(Field::SeatCount, error);
    let mut described = hint_id.clone();
    let mut select = el("select")
        .attr("id", id)
        .attr("name", id)
        .bind(Field::SeatCount);
    let is_in_range = (1..=maximum_seats).contains(&seat_count);
    if !is_in_range {
        // A select with no matching option would show the first number while the draft holds
        // another; the placeholder makes the person pick one.
        select = select.child(
            el("option")
                .attr("value", "")
                .flag("selected", true)
                .flag("disabled", true)
                .text("Choose a number"),
        );
    }
    for seats in 1..=maximum_seats {
        let label = if seats == 1 {
            "1 person (just me)".to_owned()
        } else {
            format!("{seats} people")
        };
        select = select.child(
            el("option")
                .attr("value", seats.to_string())
                .flag("selected", is_in_range && seats == seat_count)
                .text(label),
        );
    }
    let mut wrapper = el("div")
        .class(if message.is_some() {
            "field has-error"
        } else {
            "field"
        })
        .child(
            el("label")
                .attr("for", id)
                .text("How many people will play, including you?"),
        )
        .child(el("p").class("hint").attr("id", hint_id).text(format!(
            "Choose from 1 to {maximum_seats}. You count as one."
        )));
    if let Some(message) = message {
        let error_id = format!("{id}-error");
        described.push(' ');
        described.push_str(&error_id);
        select = select.attr("aria-invalid", "true");
        wrapper = wrapper.child(
            el("p")
                .class("field-error")
                .attr("id", error_id)
                .text(message),
        );
    }
    wrapper
        .child(select.attr("aria-describedby", described))
        .into()
}

fn heroes() -> Vec<Node> {
    vec![
        el("div")
            .class("panel")
            .child(el("h3").text("Choose my hero in the lobby"))
            .child(el("p").text(
                "Each player picks or creates their hero in the lobby, before the session starts. \
                 Choosing from saved heroes comes later.",
            ))
            .into(),
    ]
}

fn look_and_pace(
    screen: &WizardScreen,
    catalog: &WizardCatalog,
    error: Option<ApiError>,
) -> Vec<Node> {
    let draft = &screen.draft;
    let mut nodes = vec![
        checkbox(
            Field::HasVoice,
            "Use spoken narration and character voices by default",
            draft.has_voice,
            error,
        ),
        radio_group(
            "Pictures",
            Field::Pictures,
            [PictureDefault::Automatic, PictureDefault::ExistingArtOnly]
                .into_iter()
                .map(|pictures| RadioChoice {
                    field: Field::Pictures,
                    value: picture_value(pictures),
                    title: picture_title(pictures).to_owned(),
                    detail: picture_detail(pictures).to_owned(),
                    is_checked: draft.pictures == pictures,
                })
                .collect(),
            error,
        ),
    ];
    nodes.push(pace_note(catalog));
    nodes.push(radio_group(
        "Table pace",
        Field::PacePreset,
        catalog
            .presets
            .iter()
            .map(|offer| RadioChoice {
                field: Field::PacePreset,
                value: copy::preset_value(offer.preset),
                title: copy::preset_name(offer.preset).to_owned(),
                detail: preset_detail(offer.preset, offer.ceiling_cents_per_hour),
                is_checked: draft.pace == offer.preset,
            })
            .collect(),
        error,
    ));
    if draft.pace == PacePreset::Custom {
        let hint = format!(
            "Between {} and {} an hour.",
            money::price(catalog.custom_minimum_cents_per_hour),
            money::price(catalog.custom_maximum_cents_per_hour)
        );
        nodes.push(text_field(
            TextField {
                field: Field::CustomCeiling,
                label: "Most the table may spend in an hour, in dollars",
                value: &screen.custom_ceiling,
                kind: InputKind::Dollars,
                autocomplete: "off",
                hint: Some(&hint),
            },
            error,
        ));
    }
    nodes
}

fn pace_note(catalog: &WizardCatalog) -> Node {
    let mut note = el("p").class("pace-note");
    if catalog.is_proposed_pricing {
        note = note
            .child(pill("Proposed pilot pricing", "pill-proposed"))
            .text(" ");
    }
    note.text("Each amount is the most the table may spend in an hour of play.")
        .into()
}

fn preset_detail(preset: PacePreset, ceiling_cents_per_hour: Option<u32>) -> String {
    let description = copy::preset_description(preset);
    match ceiling_cents_per_hour {
        Some(cents) => format!("{description} At most {} an hour.", money::price(cents)),
        None => format!("{description} You choose the hourly maximum below."),
    }
}

const fn picture_value(pictures: PictureDefault) -> &'static str {
    match pictures {
        PictureDefault::Automatic => "automatic",
        PictureDefault::ExistingArtOnly => "existing",
    }
}

const fn picture_title(pictures: PictureDefault) -> &'static str {
    match pictures {
        PictureDefault::Automatic => "Make new pictures as the story needs them",
        PictureDefault::ExistingArtOnly => "Use existing art only",
    }
}

const fn picture_detail(pictures: PictureDefault) -> &'static str {
    match pictures {
        PictureDefault::Automatic => {
            "New scene and character pictures appear within the pace you choose."
        }
        PictureDefault::ExistingArtOnly => {
            "No new pictures are made; the table uses art that already exists."
        }
    }
}

fn joining_and_funding(draft: &CampaignDraft, error: Option<ApiError>) -> Vec<Node> {
    vec![
        el("fieldset")
            .class("choices")
            .child(el("legend").text("What new co-hosts can do"))
            .child(checkbox(
                Field::CoHostsCanInvite,
                "Co-hosts can invite people",
                draft.co_hosts_can_invite,
                error,
            ))
            .child(checkbox(
                Field::CoHostsCanStart,
                "Co-hosts can start sessions",
                draft.co_hosts_can_start,
                error,
            ))
            .child(checkbox(
                Field::CoHostsCanExport,
                "Co-hosts can export the campaign",
                draft.co_hosts_can_export,
                error,
            ))
            .into(),
        radio_group(
            "How people join",
            Field::JoinModeChoice,
            [JoinMode::LinkOnly, JoinMode::LinkAndRoomCode]
                .into_iter()
                .map(|mode| RadioChoice {
                    field: Field::JoinModeChoice,
                    value: join_value(mode),
                    title: join_title(mode).to_owned(),
                    detail: join_detail(mode).to_owned(),
                    is_checked: draft.join_mode == mode,
                })
                .collect(),
            error,
        ),
        radio_group(
            "Who pays for the table",
            Field::FundingChoice,
            [FundingModel::Sponsor, FundingModel::SharedPot]
                .into_iter()
                .map(|funding| RadioChoice {
                    field: Field::FundingChoice,
                    value: funding_value(funding),
                    title: funding_title(funding).to_owned(),
                    detail: funding_detail(funding).to_owned(),
                    is_checked: draft.funding == funding,
                })
                .collect(),
            error,
        ),
    ]
}

const fn join_value(mode: JoinMode) -> &'static str {
    match mode {
        JoinMode::LinkOnly => "link",
        JoinMode::LinkAndRoomCode => "code",
    }
}

const fn join_title(mode: JoinMode) -> &'static str {
    match mode {
        JoinMode::LinkOnly => "Invitation link only",
        JoinMode::LinkAndRoomCode => "Invitation link and room code",
    }
}

const fn join_detail(mode: JoinMode) -> &'static str {
    match mode {
        JoinMode::LinkOnly => "Only people you send an invitation link to can join.",
        JoinMode::LinkAndRoomCode => {
            "People can also type a room code to ask to join. A room code always needs your approval."
        }
    }
}

const fn funding_value(funding: FundingModel) -> &'static str {
    match funding {
        FundingModel::Sponsor => "sponsor",
        FundingModel::SharedPot => "shared",
    }
}

const fn funding_title(funding: FundingModel) -> &'static str {
    match funding {
        FundingModel::Sponsor => "I will sponsor this table",
        FundingModel::SharedPot => "Shared pot",
    }
}

const fn funding_detail(funding: FundingModel) -> &'static str {
    match funding {
        FundingModel::Sponsor => "Your credit pays for play, and players join without paying.",
        FundingModel::SharedPot => {
            "Players and contributors add their own credit to one pot for the table."
        }
    }
}

fn summary(draft: &CampaignDraft, catalog: &WizardCatalog) -> Vec<Node> {
    let adventure = draft.adventure_id.as_deref().and_then(|adventure_id| {
        catalog
            .adventures
            .iter()
            .find(|adventure| adventure.adventure_id == adventure_id)
    });
    let rows = vec![
        fact("Campaign name", draft.name.trim()),
        fact(
            "Adventure",
            adventure.map_or("Not chosen", |adventure| adventure.name.as_str()),
        ),
        fact(
            "Content rating",
            draft.content_rating.map_or("Not chosen", copy::rating_name),
        ),
        fact(
            "People playing",
            format!("{}, including you", draft.seat_count),
        ),
        fact("Heroes", "Each player chooses a hero in the lobby"),
        fact(
            "Voice",
            if draft.has_voice {
                "Spoken narration and character voices"
            } else {
                "Text only"
            },
        ),
        fact("Pictures", picture_title(draft.pictures)),
        fact("Table pace", pace_summary(draft, catalog)),
        fact("Co-hosts can", co_host_summary(draft)),
        fact("How people join", join_title(draft.join_mode)),
        fact("Who pays", funding_title(draft.funding)),
    ];
    vec![
        facts(rows),
        el("p")
            .class("lede")
            .text("Creating a campaign is free. Nothing is charged until a session starts.")
            .into(),
    ]
}

fn pace_summary(draft: &CampaignDraft, catalog: &WizardCatalog) -> String {
    let ceiling = match draft.pace {
        PacePreset::Custom => draft.custom_ceiling_cents_per_hour,
        preset => catalog
            .presets
            .iter()
            .find(|offer| offer.preset == preset)
            .and_then(|offer| offer.ceiling_cents_per_hour),
    };
    let name = copy::preset_name(draft.pace);
    let label = match ceiling {
        Some(cents) => format!("{name}: at most {} an hour", money::price(cents)),
        None => name.to_owned(),
    };
    if catalog.is_proposed_pricing {
        format!("{label} (proposed pilot pricing)")
    } else {
        label
    }
}

fn co_host_summary(draft: &CampaignDraft) -> String {
    let allowed: Vec<&str> = [
        (draft.co_hosts_can_invite, "invite people"),
        (draft.co_hosts_can_start, "start sessions"),
        (draft.co_hosts_can_export, "export the campaign"),
    ]
    .into_iter()
    .filter_map(|(is_allowed, what)| is_allowed.then_some(what))
    .collect();
    if allowed.is_empty() {
        "Nothing until you allow more".to_owned()
    } else {
        allowed.join(", ")
    }
}
