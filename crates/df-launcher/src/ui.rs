//! The launcher's pages. Plain server-rendered HTML with ordinary forms and links: the window
//! runs no script. Every POST answers with a redirect to a page, so a refresh never repeats it.
use std::sync::Arc;

use crate::{
    app::{App, Failure, Phase, Recovery, RunRecord, Snapshot, Step, View},
    credentials::{Credential, Source},
    form::Form,
    saves::{Adventure, SavedGame},
    settings::{
        ContentRating, DisplayScreen, DisplayTarget, FieldError, FullScreen, PictureProvider,
        PictureQuality, ServerMode, Settings,
    },
};

pub struct Page {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
    pub location: Option<String>,
    pub cache_control: &'static str,
}

impl Page {
    fn html(status: u16, body: String) -> Self {
        Self {
            status,
            content_type: "text/html; charset=utf-8",
            body: body.into_bytes(),
            location: None,
            cache_control: "no-store",
        }
    }

    /// 303 to a GET page, with a meta refresh as the fallback for webviews that ignore it.
    fn redirect(to: &str) -> Self {
        let to = escape(to);
        Self {
            status: 303,
            content_type: "text/html; charset=utf-8",
            body: format!(
                "<!doctype html><meta http-equiv=\"refresh\" content=\"0; url={to}\"><title>DungeonFlux</title>"
            )
            .into_bytes(),
            location: Some(to),
            cache_control: "no-store",
        }
    }

    fn not_found() -> Self {
        Self::html(
            404,
            shell(
                "Not found",
                "plain-page",
                false,
                "<main class=\"plain\"><h1>That page does not exist.</h1><p><a href=\"/\">Back to DungeonFlux</a></p></main>",
            ),
        )
    }
}

const STYLE: &str = include_str!("ui.css");

/// The game's own artwork, compiled in so the launcher looks right even before it has found
/// the game folder. Names are the launcher's; sources are under `assets/ui`.
const ART: &[(&str, &str, &[u8])] = &[
    (
        "harbor-hall.png",
        "image/png",
        include_bytes!("../../../assets/ui/scenes/mara-harbor-v4.png"),
    ),
    (
        "salt-quay.webp",
        "image/webp",
        include_bytes!(
            "../../../assets/ui/generated/scene-salt-quay-arrival-documentary-photo.webp"
        ),
    ),
    (
        "d20.webp",
        "image/webp",
        include_bytes!("../../../assets/ui/generated/d20.webp"),
    ),
    (
        "wordmark.webp",
        "image/webp",
        include_bytes!("../../../assets/ui/generated/wordmark.webp"),
    ),
    (
        "lantern.svg",
        "image/svg+xml",
        include_bytes!("../../../assets/ui/lantern.svg"),
    ),
    (
        "compass.svg",
        "image/svg+xml",
        include_bytes!("../../../assets/ui/compass.svg"),
    ),
    (
        "connection.svg",
        "image/svg+xml",
        include_bytes!("../../../assets/ui/connection.svg"),
    ),
    (
        "party.svg",
        "image/svg+xml",
        include_bytes!("../../../assets/ui/party.svg"),
    ),
    (
        "spellbook.svg",
        "image/svg+xml",
        include_bytes!("../../../assets/ui/spellbook.svg"),
    ),
    (
        "portrait-frame.svg",
        "image/svg+xml",
        include_bytes!("../../../assets/ui/portrait-frame.svg"),
    ),
    (
        "shield.svg",
        "image/svg+xml",
        include_bytes!("../../../assets/ui/shield.svg"),
    ),
    (
        "die.svg",
        "image/svg+xml",
        include_bytes!("../../../assets/ui/d20.svg"),
    ),
    (
        "sword.svg",
        "image/svg+xml",
        include_bytes!("../../../assets/ui/sword.svg"),
    ),
    (
        "ornament.svg",
        "image/svg+xml",
        include_bytes!("../../../assets/ui/section-ornament.svg"),
    ),
];

pub fn handle(app: &Arc<App>, method: &str, target: &str, body: &[u8]) -> Page {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    match (method, path) {
        ("GET", "/style.css") => Page {
            status: 200,
            content_type: "text/css; charset=utf-8",
            body: STYLE.as_bytes().to_vec(),
            location: None,
            cache_control: "no-store",
        },
        ("GET", art) if art.starts_with("/art/") => {
            match ART
                .iter()
                .find(|(name, _, _)| Some(*name) == art.strip_prefix("/art/"))
            {
                Some((_, content_type, bytes)) => Page {
                    status: 200,
                    content_type,
                    body: bytes.to_vec(),
                    location: None,
                    cache_control: "max-age=86400",
                },
                None => Page::not_found(),
            }
        }
        ("GET", screen) if screen.starts_with("/screen/") => screenshot(app, screen),
        ("GET", "/") => {
            app.viewing(View::Home);
            app.refresh_from_disk();
            Page::html(200, home(&app.snapshot()))
        }
        ("GET", "/settings") => {
            app.viewing(View::Settings);
            let notice = match query {
                "saved=1" => {
                    Some("Settings saved. They apply the next time you start or resume a game.")
                }
                "key=1" => Some("Key saved."),
                "key=0" => Some("Key removed."),
                _ => None,
            };
            Page::html(200, settings_page(&app.snapshot(), &[], notice, None))
        }
        ("POST", _) => post(app, path, body),
        _ => Page::not_found(),
    }
}

fn post(app: &Arc<App>, path: &str, body: &[u8]) -> Page {
    let Some(form) = Form::parse(body) else {
        return Page::html(
            400,
            shell(
                "Not sent",
                "plain-page",
                false,
                "<main class=\"plain\"><h1>That form could not be read.</h1><p><a href=\"/\">Back to DungeonFlux</a></p></main>",
            ),
        );
    };
    match path {
        "/start" => {
            let adventure = match form.get("adventure") {
                Some("random") => Adventure::Random,
                _ => Adventure::Harbor,
            };
            app.start_new(adventure);
        }
        "/resume" => app.resume(form.get("database").unwrap_or("")),
        "/stop" => app.stop(),
        "/force-stop" => app.force_stop(),
        "/retry" => app.retry(),
        "/dismiss" => app.dismiss(),
        "/display/open" => app.open_display(),
        "/display/browser" => app.open_display_in_browser(),
        "/display/full-screen" => app.set_display_full_screen(form.get("full") == Some("on")),
        "/connect" => {
            if let Err(message) = app.connect() {
                let snapshot = app.snapshot();
                return Page::html(200, settings_page(&snapshot, &[], None, Some(&message)));
            }
        }
        "/disconnect" => app.disconnect(),
        "/logs" => app.open_logs(),
        "/data-folder" => {
            app.open_data_folder();
            return Page::redirect("/settings#developer");
        }
        "/config-folder" => {
            app.open_config_folder();
            return Page::redirect("/settings#developer");
        }
        "/settings" => {
            return match app.save_settings(&form) {
                Ok(()) => Page::redirect("/settings?saved=1"),
                Err(errors) => Page::html(422, settings_page(&app.snapshot(), &errors, None, None)),
            };
        }
        "/keys" => {
            let Some(credential) = form.get("credential").and_then(Credential::from_id) else {
                return Page::redirect("/settings#keys");
            };
            let outcome = if form.get("remove") == Some("1") {
                app.remove_key(credential).map(|()| "/settings?key=0#keys")
            } else {
                app.store_key(credential, form.get("value").unwrap_or(""))
                    .map(|()| "/settings?key=1#keys")
            };
            return match outcome {
                Ok(to) => Page::redirect(to),
                Err(message) => {
                    let errors = [FieldError {
                        field: credential.id(),
                        message,
                    }];
                    Page::html(422, settings_page(&app.snapshot(), &errors, None, None))
                }
            };
        }
        _ => return Page::not_found(),
    }
    Page::redirect("/")
}

/// `/screen/<database>.jpg`: the table display's last picture of a saved game.
fn screenshot(app: &App, path: &str) -> Page {
    let picture = path
        .strip_prefix("/screen/")
        .and_then(|name| name.strip_suffix(".jpg"))
        .and_then(|database| app.screenshot_path(database))
        .and_then(|file| std::fs::read(file).ok());
    match picture {
        Some(bytes) => Page {
            status: 200,
            content_type: "image/jpeg",
            body: bytes,
            location: None,
            cache_control: "max-age=86400",
        },
        None => Page::not_found(),
    }
}

/// The picture for a saved game: its last screenshot (versioned so a newer one is never
/// served from cache), else the adventure's own artwork.
fn save_picture(snapshot: &Snapshot, game: &SavedGame) -> String {
    let shot = snapshot
        .data
        .join("screens")
        .join(format!("{}.jpg", game.database));
    let version = std::fs::metadata(&shot)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok());
    match version {
        Some(version) => format!(
            "<span class=\"shot\"><img src=\"/screen/{}.jpg?v={}\" alt=\"The table when {} was last played\" loading=\"lazy\"></span>",
            escape(&game.database),
            version.as_secs(),
            escape(&game.name)
        ),
        None => match game.adventure {
            Adventure::Harbor => {
                "<span class=\"shot art\"><img src=\"/art/salt-quay.webp\" alt=\"\"></span>"
                    .to_owned()
            }
            Adventure::Random => {
                "<span class=\"shot art die\"><img src=\"/art/d20.webp\" alt=\"\"></span>"
                    .to_owned()
            }
        },
    }
}

pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// `refresh` re-reads the page every two seconds while a start or stop is in progress, the
/// script-free way to follow progress (the desktop window also reloads on each change).
fn shell(title: &str, page: &str, refresh: bool, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><meta name=\"color-scheme\" content=\"dark\">{}<title>{}</title><link rel=\"stylesheet\" href=\"/style.css\"></head><body class=\"{page}\">{body}</body></html>",
        if refresh {
            "<meta http-equiv=\"refresh\" content=\"2\">"
        } else {
            ""
        },
        escape(title)
    )
}

fn masthead(current: View) -> String {
    let link = |view: View, href: &str, label: &str| {
        if view == current {
            format!("<a href=\"{href}\" aria-current=\"page\">{label}</a>")
        } else {
            format!("<a href=\"{href}\">{label}</a>")
        }
    };
    format!(
        "<header class=\"masthead\"><a class=\"brand\" href=\"/\"><img src=\"/art/wordmark.webp\" alt=\"DungeonFlux\" width=\"200\" height=\"50\"></a><nav aria-label=\"Launcher\">{}{}</nav></header>",
        link(View::Home, "/", "Play"),
        link(View::Settings, "/settings", "Settings")
    )
}

fn when(seconds: i64) -> String {
    use chrono::{Datelike, Local, TimeZone};
    let Some(moment) = Local.timestamp_opt(seconds, 0).single() else {
        return "at an unknown time".to_owned();
    };
    let today = Local::now().date_naive();
    let day = moment.date_naive();
    let clock = moment.format("%-I:%M %p");
    if day == today {
        format!("today at {clock}")
    } else if today.pred_opt() == Some(day) {
        format!("yesterday at {clock}")
    } else if day.year() == today.year() {
        format!("on {} at {clock}", moment.format("%-d %B"))
    } else {
        format!("on {} at {clock}", moment.format("%-d %B %Y"))
    }
}

/// The lantern beside the state line: unlit, turning while starting, lit while running.
fn lantern(state: &str) -> String {
    format!(
        "<div class=\"lantern\" data-state=\"{state}\" aria-hidden=\"true\"><img src=\"/art/lantern.svg\" alt=\"\" width=\"30\" height=\"30\"></div>"
    )
}

fn button(action: &str, label: &str, class: &str, hidden: &[(&str, &str)]) -> String {
    button_named(action, label, label, class, hidden)
}

/// A button whose spoken name (`name`) says more than its short visible label.
fn button_named(
    action: &str,
    label: &str,
    name: &str,
    class: &str,
    hidden: &[(&str, &str)],
) -> String {
    let fields: String = hidden
        .iter()
        .map(|(field, value)| {
            format!(
                "<input type=\"hidden\" name=\"{}\" value=\"{}\">",
                escape(field),
                escape(value)
            )
        })
        .collect();
    let spoken = if name == label {
        String::new()
    } else {
        format!(" aria-label=\"{}\"", escape(name))
    };
    format!(
        "<form method=\"post\" action=\"{action}\" class=\"inline\">{fields}<button class=\"{class}\"{spoken}>{}</button></form>",
        escape(label)
    )
}

fn home(snapshot: &Snapshot) -> String {
    let state = match &snapshot.phase {
        Phase::Idle { notice } => idle(snapshot, notice.as_deref()),
        Phase::Starting { game, step, detail } => starting(game, *step, detail),
        Phase::Running(run) => running(snapshot, run),
        Phase::Connected { origin } => connected(snapshot, origin),
        Phase::Stopping(run) => format!(
            "<section class=\"state\">{}<div><h1>Saving {}</h1><p class=\"lede\">The game is finishing the current moment and closing the table. This usually takes a few seconds.</p></div></section>",
            lantern("dimming"),
            escape(&run.game.name)
        ),
        Phase::Failed(failure) => failed(failure),
    };
    let saves = saved_games(snapshot);
    let busy = matches!(snapshot.phase, Phase::Starting { .. } | Phase::Stopping(_));
    shell(
        "DungeonFlux",
        "home-page",
        busy,
        &format!(
            "{}<main class=\"home\"><div class=\"play\">{state}</div>{saves}</main>",
            masthead(View::Home)
        ),
    )
}

fn notice_line(notice: Option<&str>) -> String {
    notice
        .map(|text| format!("<p class=\"notice\" role=\"status\">{}</p>", escape(text)))
        .unwrap_or_default()
}

fn joining_line(settings: &Settings) -> &'static str {
    if settings.phones_on_network {
        "Phones on the same Wi-Fi can join. The join code and QR appear on the table display. If Windows asks, allow DungeonFlux on private networks."
    } else {
        "Only this PC can open the game. Turn on phones on this network in Settings to let players join from their phones."
    }
}

fn idle(snapshot: &Snapshot, notice: Option<&str>) -> String {
    let settings = &snapshot.settings;
    let problem = snapshot
        .settings_problem
        .as_deref()
        .map(|problem| {
            format!(
                "<p class=\"alert\" role=\"alert\">{} DungeonFlux is using default settings until you save new ones.</p>",
                escape(problem)
            )
        })
        .unwrap_or_default();
    if settings.server_mode == ServerMode::External {
        return format!(
            "<section class=\"state\">{}<div><h1>No server connected</h1><p class=\"lede\">Developer mode: DungeonFlux connects to the server at <strong>{}</strong> instead of starting its own.</p>{}{}<div class=\"actions\">{}<a class=\"quiet-link\" href=\"/settings#developer\">Change server</a></div></div></section>",
            lantern("unlit"),
            escape(&settings.external_url),
            problem,
            notice_line(notice),
            button("/connect", "Connect", "primary", &[])
        );
    }
    let install = match &snapshot.install {
        Ok(_) => String::new(),
        Err(missing) => format!("<p class=\"alert\" role=\"alert\">{}</p>", escape(missing)),
    };
    format!(
        "<section class=\"state\">{}<div><h1>No game is running</h1><p class=\"lede\">Start a new adventure, or pick up a saved game.</p>{problem}{install}{}</div></section>\
<form method=\"post\" action=\"/start\" class=\"new-game\"><fieldset><legend>Choose your adventure</legend><div class=\"adventures\">\
<label class=\"adventure\"><input type=\"radio\" name=\"adventure\" value=\"harbor\" checked><span class=\"adventure-art\"><img src=\"/art/salt-quay.webp\" alt=\"\"></span><span class=\"adventure-text\"><strong>The harbor adventure</strong><small>The written story on the salt quay. A good first game.</small></span></label>\
<label class=\"adventure\"><input type=\"radio\" name=\"adventure\" value=\"random\"><span class=\"adventure-art die\"><img src=\"/art/d20.webp\" alt=\"\"></span><span class=\"adventure-text\"><strong>A newly rolled adventure</strong><small>Fresh places, people and trouble from the adventure tables.</small></span></label>\
</div></fieldset>{}<button class=\"primary start\">Start new game</button></form>",
        lantern("unlit"),
        notice_line(notice),
        readiness(snapshot)
    )
}

/// What a new game will include, before the player presses Start: each line is either ready
/// or says exactly which setting or key changes it.
fn readiness(snapshot: &Snapshot) -> String {
    let settings = &snapshot.settings;
    let has = |credential| {
        snapshot
            .keys
            .as_ref()
            .is_ok_and(|keys| keys.source(credential) != Source::Missing)
    };
    let mut lines: Vec<(bool, String, &str)> = Vec::new();
    lines.push(match (settings.storyteller, has(Credential::Openai)) {
        (true, true) => (
            true,
            "An AI storyteller narrates, with pictures of each scene".to_owned(),
            "/settings#story",
        ),
        (true, false) => (
            false,
            "Written story only: add an OpenAI key for the AI storyteller and pictures".to_owned(),
            "/settings#keys",
        ),
        (false, _) => (
            false,
            "Written story only: the AI storyteller is off".to_owned(),
            "/settings#story",
        ),
    });
    lines.push(
        match (
            settings.narration_voice || settings.music,
            has(Credential::Elevenlabs),
        ) {
            (true, true) => (
                true,
                "Spoken narration and music on the table display".to_owned(),
                "/settings#story",
            ),
            (true, false) => (
                false,
                "No voices or music: add an ElevenLabs key".to_owned(),
                "/settings#keys",
            ),
            (false, _) => (
                false,
                "No voices or music: both are off".to_owned(),
                "/settings#story",
            ),
        },
    );
    lines.push(if settings.phones_on_network {
        (
            true,
            "Players join from their phones on this Wi-Fi".to_owned(),
            "/settings#network",
        )
    } else {
        (
            false,
            "Only this PC can join: phones on the network are off".to_owned(),
            "/settings#network",
        )
    });
    let screen = match settings.display_screen {
        DisplayScreen::Second => "a second screen when one is connected",
        DisplayScreen::Main => "this screen",
    };
    lines.push(match settings.display {
        DisplayTarget::Window => (
            true,
            format!("The table display opens in its own window, on {screen}"),
            "/settings#display",
        ),
        DisplayTarget::Browser => (
            true,
            "The table display opens in your browser".to_owned(),
            "/settings#display",
        ),
        DisplayTarget::Manual => (
            false,
            "The table display does not open by itself".to_owned(),
            "/settings#display",
        ),
    });
    let items: String = lines
        .iter()
        .map(|(ready, text, link)| {
            format!(
                "<li data-ready=\"{ready}\"><span>{}</span><a href=\"{link}\">Change</a></li>",
                escape(text)
            )
        })
        .collect();
    format!(
        "<div class=\"readiness\"><h2>This game starts with</h2><ul>{items}</ul><p class=\"hint\">If Windows asks, allow DungeonFlux on private networks so phones can reach it.</p></div>"
    )
}

fn starting(game: &SavedGame, current: Step, detail: &str) -> String {
    let steps: String = Step::ALL
        .iter()
        .map(|step| {
            let state = match step.cmp(&current) {
                std::cmp::Ordering::Less => "done",
                std::cmp::Ordering::Equal => "current",
                std::cmp::Ordering::Greater => "waiting",
            };
            let extra = if *step == current && !detail.is_empty() {
                format!("<small>{}…</small>", escape(detail))
            } else {
                String::new()
            };
            let aria = if *step == current {
                " aria-current=\"step\""
            } else {
                ""
            };
            format!(
                "<li data-state=\"{state}\"{aria}>{}{extra}</li>",
                step.label()
            )
        })
        .collect();
    format!(
        "<section class=\"state\">{}<div><h1>Starting {}</h1><p class=\"lede\">{}. The table display opens when the game is ready.</p><ol class=\"steps\">{steps}</ol></div></section>",
        lantern("starting"),
        escape(&game.name),
        escape(game.adventure.label())
    )
}

fn display_controls(snapshot: &Snapshot) -> String {
    let mut actions = Vec::new();
    if snapshot.windowed {
        if snapshot.display_open {
            actions.push(if snapshot.display_full_screen {
                button(
                    "/display/full-screen",
                    "Leave full screen",
                    "secondary",
                    &[("full", "off")],
                )
            } else {
                button(
                    "/display/full-screen",
                    "Show full screen",
                    "secondary",
                    &[("full", "on")],
                )
            });
        } else {
            actions.push(button(
                "/display/open",
                "Show table display",
                "primary",
                &[],
            ));
        }
    }
    actions.push(button(
        "/display/browser",
        "Open display in browser",
        if snapshot.windowed {
            "quiet"
        } else {
            "primary"
        },
        &[],
    ));
    actions.concat()
}

fn running(snapshot: &Snapshot, run: &RunRecord) -> String {
    let display_state = if snapshot.display_open {
        "The table display is open."
    } else {
        "The table display is closed. Players stay connected."
    };
    format!(
        "<section class=\"state\">{}<div><h1>{} is running</h1><p class=\"lede\">{}. {display_state}</p><div class=\"actions\">{}</div><p class=\"hint\">{}</p>\
<div class=\"stop-row\">{}<p class=\"hint\">Closing this window keeps the game running. Stop saves the game and ends it.</p></div></div></section>",
        lantern("lit"),
        escape(&run.game.name),
        escape(run.game.adventure.label()),
        display_controls(snapshot),
        joining_line(&snapshot.settings),
        button("/stop", "Stop and save", "danger", &[])
    )
}

fn connected(snapshot: &Snapshot, origin: &str) -> String {
    format!(
        "<section class=\"state\">{}<div><h1>Connected to a running server</h1><p class=\"lede\">Developer mode: the game at <strong>{}</strong> was started outside DungeonFlux, so it keeps running when you disconnect.</p><div class=\"actions\">{}{}</div></div></section>",
        lantern("lit"),
        escape(origin),
        display_controls(snapshot),
        button("/disconnect", "Disconnect", "quiet", &[])
    )
}

fn failed(failure: &Failure) -> String {
    let primary = match &failure.recovery {
        Recovery::Retry(_) => button("/retry", "Try again", "primary", &[]),
        Recovery::StopAgain(_) => format!(
            "{}{}",
            button("/retry", "Try stopping again", "primary", &[]),
            button("/force-stop", "Force stop", "danger", &[])
        ),
        Recovery::Nothing => String::new(),
    };
    let log = if failure.log_tail.trim().is_empty() {
        String::new()
    } else {
        format!(
            "<details class=\"log\"><summary>What the game printed</summary><pre>{}</pre></details>",
            escape(&failure.log_tail)
        )
    };
    format!(
        "<section class=\"state\">{}<div><h1>{}</h1><p class=\"lede\">{}</p><div class=\"actions\">{primary}{}<a class=\"quiet-link\" href=\"/settings\">Open settings</a></div>{log}{}</div></section>",
        lantern("failed"),
        escape(&failure.title),
        escape(&failure.message),
        button("/dismiss", "Dismiss", "quiet", &[]),
        if failure.log_tail.is_empty() {
            String::new()
        } else {
            button("/logs", "Open log folder", "quiet", &[])
        }
    )
}

fn saved_games(snapshot: &Snapshot) -> String {
    let can_resume = matches!(snapshot.phase, Phase::Idle { .. } | Phase::Failed(_))
        && snapshot.settings.server_mode == ServerMode::Managed;
    let current = match &snapshot.phase {
        Phase::Running(run) | Phase::Stopping(run) => Some(run.game.database.as_str()),
        Phase::Starting { game, .. } => Some(game.database.as_str()),
        _ => None,
    };
    let featured = snapshot
        .saves
        .first()
        .filter(|game| can_resume && current != Some(game.database.as_str()));
    let feature = featured
        .map(|game| {
            format!(
                "<div class=\"continue\">{}<div class=\"continue-text\"><p class=\"continue-label\">Pick up where you left off</p><h3>{}</h3><p>{}, last played {}.</p>{}</div></div>",
                save_picture(snapshot, game),
                escape(&game.name),
                escape(game.adventure.label()),
                when(game.last_played),
                button_named(
                    "/resume",
                    "Continue",
                    &format!("Continue {}", game.name),
                    "primary",
                    &[("database", &game.database)],
                )
            )
        })
        .unwrap_or_default();
    let rows: String = snapshot
        .saves
        .iter()
        .skip(usize::from(featured.is_some()))
        .map(|game| {
            let action = if current == Some(game.database.as_str()) {
                "<span class=\"tag\">Playing now</span>".to_owned()
            } else if can_resume {
                button_named(
                    "/resume",
                    "Resume",
                    &format!("Resume {}", game.name),
                    "secondary",
                    &[("database", &game.database)],
                )
            } else {
                String::new()
            };
            format!(
                "<li>{}<div class=\"save-text\"><h3>{}</h3><p>{}</p><p>Last played {}</p></div>{action}</li>",
                save_picture(snapshot, game),
                escape(&game.name),
                escape(game.adventure.short_label()),
                when(game.last_played)
            )
        })
        .collect();
    let list = if rows.is_empty() && feature.is_empty() {
        "<p class=\"empty\">Games you start appear here, with a picture of the table from when you last played.</p>"
            .to_owned()
    } else if rows.is_empty() {
        feature
    } else {
        format!("{feature}<ul class=\"saves\">{rows}</ul>")
    };
    format!(
        "<aside class=\"ledger\" aria-labelledby=\"saved-heading\"><h2 id=\"saved-heading\">Saved games</h2>{list}</aside>"
    )
}

// ---------------------------------------------------------------------------------- settings

struct Fields<'a> {
    errors: &'a [FieldError],
}

impl Fields<'_> {
    fn error(&self, field: &str) -> String {
        self.errors
            .iter()
            .filter(|error| error.field == field)
            .map(|error| {
                format!(
                    "<p class=\"field-error\" id=\"{field}-error\">{}</p>",
                    escape(&error.message)
                )
            })
            .collect()
    }

    fn described(&self, field: &str) -> String {
        if self.errors.iter().any(|error| error.field == field) {
            format!(" aria-invalid=\"true\" aria-describedby=\"{field}-error\"")
        } else {
            String::new()
        }
    }

    fn check(&self, field: &str, label: &str, help: &str, on: bool) -> String {
        format!(
            "<label class=\"check\"><input type=\"checkbox\" name=\"{field}\"{}><span><strong>{label}</strong><small>{help}</small></span></label>{}",
            if on { " checked" } else { "" },
            self.error(field)
        )
    }

    fn radios(
        &self,
        field: &str,
        legend: &str,
        options: &[(&str, &str, &str)],
        selected: &str,
    ) -> String {
        let options: String = options
            .iter()
            .map(|(value, label, help)| {
                format!(
                    "<label class=\"option\"><input type=\"radio\" name=\"{field}\" value=\"{value}\"{}><span><strong>{label}</strong><small>{help}</small></span></label>",
                    if *value == selected { " checked" } else { "" }
                )
            })
            .collect();
        format!(
            "<fieldset class=\"choices\"><legend>{legend}</legend>{options}</fieldset>{}",
            self.error(field)
        )
    }

    fn text(
        &self,
        field: &str,
        label: &str,
        help: &str,
        value: &str,
        placeholder: &str,
        kind: &str,
    ) -> String {
        format!(
            "<div class=\"field\"><label class=\"text\" for=\"{field}\"><strong>{label}</strong><small>{help}</small></label><input id=\"{field}\" name=\"{field}\" type=\"{kind}\" value=\"{}\" placeholder=\"{}\"{}>{}</div>",
            escape(value),
            escape(placeholder),
            self.described(field),
            self.error(field)
        )
    }
}

fn needs(snapshot: &Snapshot, credentials: &[Credential]) -> String {
    let Ok(keys) = &snapshot.keys else {
        return String::new();
    };
    let missing: Vec<String> = credentials
        .iter()
        .filter(|credential| keys.source(**credential) == Source::Missing)
        .map(|credential| {
            let label = credential.label();
            let article = if label.starts_with(['A', 'E', 'I', 'O', 'U']) {
                "an"
            } else {
                "a"
            };
            format!("{article} {label} key")
        })
        .collect();
    match missing.as_slice() {
        [] => String::new(),
        [one] => format!(" Needs {one}, which is not set."),
        many => format!(" Needs {}, which are not set.", many.join(" and ")),
    }
}

fn settings_page(
    snapshot: &Snapshot,
    errors: &[FieldError],
    notice: Option<&str>,
    alert: Option<&str>,
) -> String {
    let settings = &snapshot.settings;
    let fields = Fields { errors };
    let general_error: String = errors
        .iter()
        .filter(|error| error.field.is_empty())
        .map(|error| {
            format!(
                "<p class=\"alert\" role=\"alert\">{}</p>",
                escape(&error.message)
            )
        })
        .collect();
    let summary = if errors.iter().any(|error| !error.field.is_empty()) {
        "<p class=\"alert\" role=\"alert\">Some settings need attention. Nothing was saved yet; fix the marked fields and save again.</p>"
    } else {
        ""
    };
    let running = matches!(snapshot.phase, Phase::Running(_) | Phase::Starting { .. });
    let running_note = if running {
        "<p class=\"notice\">A game is running. Changes apply the next time you start or resume a game.</p>"
    } else {
        ""
    };
    let alert = alert
        .map(|text| format!("<p class=\"alert\" role=\"alert\">{}</p>", escape(text)))
        .unwrap_or_default();

    let network = format!(
        "<section id=\"network\"><h2>Players and network</h2>{}<div class=\"pair\">{}{}</div></section>",
        fields.check(
            "phones_on_network",
            "Let phones on this network join",
            "Players open the join link on their phones over this PC's Wi-Fi. Turn off to keep the game on this PC only.",
            settings.phones_on_network
        ),
        fields.text("game_port", "Game port", "The port phones and the display connect to.", &settings.game_port.to_string(), "63309", "text"),
        fields.text("database_port", "Database port", "The port of the game's own database on this PC.", &settings.database_port.to_string(), "55517", "text"),
    );
    let display = format!(
        "<section id=\"display\"><h2>Table display</h2>{}{}{}</section>",
        fields.radios(
            "display",
            "When the game is ready, open the table display",
            &[
                (
                    "window",
                    "In a DungeonFlux window",
                    "Its own window, where the browser never blocks the game's sound."
                ),
                (
                    "browser",
                    "In the default browser",
                    "As a browser tab, as before. The browser may ask before playing sound."
                ),
                (
                    "manual",
                    "Don't open it",
                    "Open it yourself from the Play page."
                ),
            ],
            match settings.display {
                DisplayTarget::Window => "window",
                DisplayTarget::Browser => "browser",
                DisplayTarget::Manual => "manual",
            }
        ),
        fields.radios(
            "display_screen",
            "Screen",
            &[
                (
                    "second",
                    "A second screen when one is connected",
                    "Ideal for a TV over HDMI. Uses this screen when there is only one."
                ),
                ("main", "This screen", "The screen the launcher is on."),
            ],
            match settings.display_screen {
                DisplayScreen::Second => "second",
                DisplayScreen::Main => "main",
            }
        ),
        fields.radios(
            "full_screen",
            "Full screen",
            &[
                (
                    "on-second-screen",
                    "Only on a second screen",
                    "A TV fills its screen; on this screen it opens as a window."
                ),
                (
                    "always",
                    "Always",
                    "The display covers its whole screen. Leave full screen from the Play page."
                ),
                ("never", "Never", "Always a normal window."),
            ],
            match settings.full_screen {
                FullScreen::OnSecondScreen => "on-second-screen",
                FullScreen::Always => "always",
                FullScreen::Never => "never",
            }
        ),
    );
    let story = format!(
        "<section id=\"story\"><h2>Story and sound</h2>{}{}{}{}{}{}{}</section>",
        fields.radios("content_rating", "Content rating", &[
            ("mature", "Mature", "For adults: gore, strong language and frank flirting can appear."),
            ("restrained", "Restrained", "Family-friendly: no gore or strong language, and flirting stays light."),
        ], match settings.content_rating {
            ContentRating::Mature => "mature",
            ContentRating::Restrained => "restrained",
        }),
        fields.check("storyteller", "Storyteller", &format!("An AI narrator voices the story in its own words. Without it, the game uses its written lines.{}", needs(snapshot, &[Credential::Openai])), settings.storyteller),
        fields.check("story_memory", "Story memory", &format!("The storyteller remembers earlier scenes and characters.{}", needs(snapshot, &[Credential::Openai])), settings.story_memory),
        fields.check("narration_voice", "Narration voices", &format!("Narration and characters are spoken aloud on the table display.{}", needs(snapshot, &[Credential::Elevenlabs])), settings.narration_voice),
        fields.check("music", "Music", &format!("Music made for each place, playing on the table display.{}", needs(snapshot, &[Credential::Elevenlabs])), settings.music),
        fields.check("speech_input", "Talk to characters", &format!("Players can hold to speak on their phones instead of typing.{}", needs(snapshot, &[Credential::Elevenlabs])), settings.speech_input),
        fields.check("choice_hints", "Numbered choices on the display", "Show the numbered choice chips on the table display. Players can still change this during play.", settings.choice_hints),
    );
    let pictures = format!(
        "<section id=\"pictures\"><h2>Pictures</h2>{}{}{}{}{}{}</section>",
        fields.check("scene_pictures", "Scene pictures", &format!("Pictures of people and places are made as the story goes.{}", needs(snapshot, &[Credential::Openai])), settings.scene_pictures),
        fields.check("scene_plates", "A picture for every scene", "Each story scene gets a composed picture. Needs scene pictures.", settings.scene_plates),
        fields.radios("picture_quality", "Picture quality", &[
            ("low", "Low", "Fastest and cheapest."),
            ("medium", "Medium", "The usual balance."),
            ("high", "High", "Sharper pictures that take longer and cost more."),
        ], match settings.picture_quality {
            PictureQuality::Low => "low",
            PictureQuality::Medium => "medium",
            PictureQuality::High => "high",
        }),
        fields.radios("picture_provider", "Picture service", &[
            ("openai", "OpenAI", "The standard picture service."),
            ("flux", "FLUX through fal", "A developer option. Needs a fal key."),
        ], match settings.picture_provider {
            PictureProvider::Openai => "openai",
            PictureProvider::Flux => "flux",
        }),
        fields.check("creator_portraits", "Character portraits", &format!("Players can generate a portrait of their hero while creating it. Each portrait is a paid picture.{}", needs(snapshot, &[Credential::Openai, Credential::Jev])), settings.creator_portraits),
        fields.check("creator_alternate_route", "Alternate route for mature portraits", &format!("Portraits with adult anatomy, nudity or fantasy violence go to Black Forest Labs. Needs character portraits.{}", needs(snapshot, &[Credential::Bfl])), settings.creator_alternate_route),
    );
    let video = format!(
        "<section id=\"video\"><h2>Video</h2><p class=\"section-note\">Generated video costs much more than pictures, so it is off unless you turn it on.{}</p>{}{}{}</section>",
        needs(snapshot, &[Credential::Fal]),
        fields.check(
            "home_video",
            "Moving title screen",
            "A short looping clip behind the table display's title.",
            settings.home_video
        ),
        fields.check(
            "scene_video",
            "Scene and finishing-blow clips",
            "Short clips for story moments and the last hit of a fight.",
            settings.scene_video
        ),
        fields.check(
            "character_video",
            "Character action clips",
            "Clips of heroes attacking and reacting in fights.",
            settings.character_video
        ),
    );
    let keys_note = fields.check(
        "keys_from_env_file",
        "Also use keys from the game folder's .env file",
        "Keys saved below come first. The .env file is how developers have been providing keys.",
        settings.keys_from_env_file,
    );
    let developer = developer_section(snapshot, &fields);
    let form = format!(
        "<form method=\"post\" action=\"/settings\" class=\"settings-form\">{network}{display}{story}{pictures}{video}<section id=\"keys-source\"><h2>Where keys come from</h2>{keys_note}</section>{developer}<div class=\"save-bar\"><button class=\"primary\">Save settings</button><a class=\"quiet-link\" href=\"/settings\">Discard changes</a></div></form>"
    );
    let keys = keys_section(snapshot, &fields);
    let files = files_section(snapshot);
    let index = "<nav class=\"index\" aria-label=\"Settings sections\"><a href=\"#network\">Players and network</a><a href=\"#display\">Table display</a><a href=\"#story\">Story and sound</a><a href=\"#pictures\">Pictures</a><a href=\"#video\">Video</a><a href=\"#keys\">Keys</a><a href=\"#developer\">Developer</a><a href=\"#files\">Files and command line</a></nav>";
    shell(
        "DungeonFlux settings",
        "settings-page",
        false,
        &format!(
            "{}<main class=\"settings\">{index}<div class=\"settings-body\"><h1>Settings</h1>{}{general_error}{summary}{alert}{running_note}{form}{keys}{files}</div></main>",
            masthead(View::Settings),
            notice_line(notice)
        ),
    )
}

fn developer_section(snapshot: &Snapshot, fields: &Fields<'_>) -> String {
    let settings = &snapshot.settings;
    let detected = match &snapshot.install {
        Ok(install) => format!(
            "<dl class=\"facts\"><dt>Game folder</dt><dd>{}</dd><dt>Game build</dt><dd>{}</dd><dt>PostgreSQL</dt><dd>{}</dd><dt>Game data and logs</dt><dd>{}</dd></dl>",
            escape(&install.root.display().to_string()),
            escape(
                &install
                    .server_exe
                    .parent()
                    .and_then(|debug| debug.parent())
                    .unwrap_or(&install.server_exe)
                    .display()
                    .to_string()
            ),
            escape(&install.postgres_bin.display().to_string()),
            escape(&install.play_root.display().to_string()),
        ),
        Err(missing) => format!("<p class=\"alert\">{}</p>", escape(missing)),
    };
    format!(
        "<section id=\"developer\"><h2>Developer</h2><p class=\"section-note\">For working on DungeonFlux. Players can leave these alone.</p>{}{}{}<h3>Folders</h3><p class=\"section-note\">Leave a folder empty to use the one DungeonFlux finds. Currently in use:</p>{detected}{}{}{}{}</section>",
        fields.radios("server_mode", "Game server", &[
            ("managed", "DungeonFlux starts its own server", "The normal way to play. Start, resume and stop work from the Play page."),
            ("external", "Connect to a server that is already running", "For a server you started yourself, such as development/run-gameplay.sh or dev-gameplay.py. DungeonFlux won't stop it."),
        ], match settings.server_mode {
            ServerMode::Managed => "managed",
            ServerMode::External => "external",
        }),
        fields.text("external_url", "Server address", "Used when connecting to a running server.", &settings.external_url, "http://127.0.0.1:63309", "url"),
        fields.check("dev_control", "Developer control", "Turns on the server's authenticated developer commands (df-transport-fixture dev). Off for normal play.", settings.dev_control),
        fields.text("game_folder", "Game folder", "The folder holding content/ and assets/.", &settings.game_folder, "Found automatically", "text"),
        fields.text("build_folder", "Game build", "A build folder holding debug/df-transport-fixture.exe and play-web/.", &settings.build_folder, "Found automatically", "text"),
        fields.text("postgres_folder", "PostgreSQL folder", "The folder holding pg_ctl.exe and initdb.exe.", &settings.postgres_folder, "Found automatically", "text"),
        fields.text("play_folder", "Game data folder", "Where saved games' database, caches and logs live. Use a separate folder for a different database port.", &settings.play_folder, "Found automatically", "text"),
    )
}

/// Where the configuration lives, and how agents and scripts skip this window.
fn files_section(snapshot: &Snapshot) -> String {
    format!(
        "<section id=\"files\"><h2>Files and command line</h2>\
<dl class=\"facts\"><dt>Configuration</dt><dd>{}</dd><dt>Clean copy in Git</dt><dd>config/dungeonflux.example.toml</dd><dt>Saved games and screenshots</dt><dd>{}</dd></dl>\
<div class=\"actions\">{}{}</div>\
<h3>Skip this window</h3><p class=\"section-note\">Agents and scripts can start, resume and stop games directly, with the same saves and settings:</p>\
<pre class=\"commands\">dungeonflux start harbor --to creation\ndungeonflux resume \"The Iron Map\" --dev-control\ndungeonflux status\ndungeonflux stop</pre>\
<p class=\"section-note\">Run <code>dungeonflux --help</code> for every command and option.</p></section>",
        escape(&snapshot.config_path.display().to_string()),
        escape(&snapshot.data.display().to_string()),
        button(
            "/config-folder",
            "Open configuration folder",
            "secondary",
            &[]
        ),
        button("/data-folder", "Open saved games folder", "quiet", &[]),
    )
}

fn keys_section(snapshot: &Snapshot, fields: &Fields<'_>) -> String {
    let rows: String = Credential::ALL
        .iter()
        .map(|credential| {
            let status = match &snapshot.keys {
                Ok(keys) => match keys.source(*credential) {
                    Source::Config => "<span class=\"tag ok\">Saved in the config file</span>",
                    Source::EnvFile => "<span class=\"tag ok\">From the .env file</span>",
                    Source::Missing => "<span class=\"tag\">Not set</span>",
                },
                Err(_) => "<span class=\"tag warn\">Can't be read</span>",
            };
            let stored = matches!(&snapshot.keys, Ok(keys) if keys.source(*credential) == Source::Config);
            let remove = if stored {
                button("/keys", "Remove", "quiet", &[("credential", credential.id()), ("remove", "1")])
            } else {
                String::new()
            };
            let id = credential.id();
            format!(
                "<li><div class=\"key-head\"><h3>{}</h3>{status}</div><p>{}</p><form method=\"post\" action=\"/keys\" class=\"key-form\"><input type=\"hidden\" name=\"credential\" value=\"{id}\"><label class=\"visually-hidden\" for=\"key-{id}\">New {} key</label><input id=\"key-{id}\" name=\"value\" type=\"password\" autocomplete=\"off\" spellcheck=\"false\" placeholder=\"Paste a new key\"{}><button class=\"secondary\">Save key</button></form>{remove}{}</li>",
                credential.label(),
                credential.purpose(),
                credential.label(),
                fields.described(id),
                fields.error(id)
            )
        })
        .collect();
    let problem = match &snapshot.keys {
        Err(message) => format!("<p class=\"alert\" role=\"alert\">{}</p>", escape(message)),
        Ok(_) => String::new(),
    };
    format!(
        "<section id=\"keys\" class=\"keys\"><h2>Keys</h2><p class=\"section-note\">Keys are saved in <code>config/dungeonflux.toml</code>, which Git ignores. They are never shown again here. Each feature above says which key it needs.</p>{problem}<ul>{rows}</ul></section>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn app(name: &str) -> (Arc<App>, std::path::PathBuf) {
        let data =
            std::env::temp_dir().join(format!("df-launcher-ui-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        std::fs::create_dir_all(&data).unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        (
            App::new(
                data.clone(),
                data.join("dungeonflux.toml"),
                false,
                Box::new(move |event| events.lock().unwrap().push(event)),
            ),
            data,
        )
    }

    fn text(page: &Page) -> String {
        String::from_utf8(page.body.clone()).unwrap()
    }

    #[test]
    fn escape_neutralises_markup() {
        assert_eq!(
            escape("<a href=\"x\">'&'</a>"),
            "&lt;a href=&quot;x&quot;&gt;&#39;&amp;&#39;&lt;/a&gt;"
        );
    }

    #[test]
    fn pages_render_without_script_and_posts_redirect() {
        let (app, data) = app("pages");
        for target in ["/", "/settings", "/settings?saved=1"] {
            let page = handle(&app, "GET", target, b"");
            assert_eq!(page.status, 200, "{target}");
            let body = text(&page);
            for script in ["<script", "onclick", "onsubmit", "onload", "javascript:"] {
                assert!(!body.contains(script), "{target} contains {script}");
            }
        }
        assert!(text(&handle(&app, "GET", "/", b"")).contains("No game is running"));
        let dismissed = handle(&app, "POST", "/dismiss", b"");
        assert_eq!(dismissed.status, 303);
        assert_eq!(dismissed.location.as_deref(), Some("/"));
        assert_eq!(handle(&app, "GET", "/nothing", b"").status, 404);
        assert_eq!(handle(&app, "POST", "/start", b"a=%ZZ").status, 400);
        std::fs::remove_dir_all(data).unwrap();
    }

    #[test]
    fn settings_save_round_trips_and_errors_keep_the_page() {
        let (app, data) = app("settings");
        let body = b"game_port=64001&database_port=55517&display=browser&display_screen=main&full_screen=never&content_rating=restrained&picture_quality=low&picture_provider=openai&server_mode=managed&storyteller=on";
        let saved = handle(&app, "POST", "/settings", body);
        assert_eq!(saved.location.as_deref(), Some("/settings?saved=1"));
        let page = text(&handle(&app, "GET", "/settings", b""));
        assert!(page.contains("value=\"64001\""));
        assert!(page.contains("value=\"browser\" checked"));
        let refused = handle(
            &app,
            "POST",
            "/settings",
            b"game_port=12&database_port=55517",
        );
        assert_eq!(refused.status, 422);
        assert!(text(&refused).contains("Enter a port number"));
        assert_eq!(
            crate::config::load(&data.join("dungeonflux.toml"))
                .unwrap()
                .settings
                .game_port,
            64001
        );
        std::fs::remove_dir_all(data).unwrap();
    }

    #[test]
    fn keys_are_stored_without_ever_rendering_the_value() {
        let (app, data) = app("keys");
        let saved = handle(
            &app,
            "POST",
            "/keys",
            b"credential=openai&value=sk-synthetic-123",
        );
        assert_eq!(saved.location.as_deref(), Some("/settings?key=1#keys"));
        let page = text(&handle(&app, "GET", "/settings", b""));
        assert!(page.contains("Saved in the config file"));
        assert!(!page.contains("sk-synthetic-123"));
        let file = std::fs::read_to_string(data.join("dungeonflux.toml")).unwrap();
        assert!(file.contains("openai = \"sk-synthetic-123\""));
        let refused = handle(&app, "POST", "/keys", b"credential=jev&value=two+words");
        assert_eq!(refused.status, 422);
        assert!(!text(&refused).contains("two words"));
        handle(&app, "POST", "/keys", b"credential=openai&remove=1");
        assert!(!text(&handle(&app, "GET", "/settings", b"")).contains("Saved in the config file"));
        std::fs::remove_dir_all(data).unwrap();
    }
}
