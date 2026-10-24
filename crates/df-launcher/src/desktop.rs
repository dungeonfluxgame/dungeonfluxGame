//! The desktop windows, on tao and wry (the window and WebView2 layers Tauri is built on).
//! The control window shows the launcher's own pages through a private `dungeonflux:` scheme;
//! the display window shows the running game's table display, with sound allowed to start.
use std::{
    borrow::Cow,
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
};

use tao::{
    dpi::{LogicalPosition, LogicalSize, PhysicalPosition},
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopWindowTarget},
    monitor::MonitorHandle,
    window::{Fullscreen, Icon, Window, WindowBuilder},
};
use wry::{
    NewWindowResponse, WebContext, WebView, WebViewBuilder,
    http::{Request, Response, header},
};

use crate::{
    app::{App, ShellEvent},
    config::switch,
    control::{self, Reply},
    log,
    platform::InstanceGuard,
    settings::{DisplayScreen, FullScreen},
    ui,
};

#[derive(Debug)]
enum UserEvent {
    Shell(ShellEvent),
    ShowControl,
    /// The closing display window (this generation) may go: its last screenshot is saved, or
    /// the capture took too long.
    DropDisplay(u64),
    /// `dungeonflux window` (see control.rs), answered on the channel.
    Window(control::Request, mpsc::Sender<Reply>),
}

/// How long a closing display waits for its last screenshot before closing anyway.
const CAPTURE_WAIT: std::time::Duration = std::time::Duration::from_secs(4);

/// Scheme, host and port of an absolute `http`/`https` URL, compared exactly (never by text
/// prefix). URLs carrying a user name or password are refused.
fn origin_of(url: &str) -> Option<(String, String, u16)> {
    let uri: wry::http::Uri = url.parse().ok()?;
    let scheme = uri.scheme_str()?.to_ascii_lowercase();
    let default_port = match scheme.as_str() {
        "http" => 80,
        "https" => 443,
        _ => return None,
    };
    let authority = uri.authority()?;
    if authority.as_str().contains('@') {
        return None;
    }
    Some((
        scheme,
        authority.host().to_ascii_lowercase(),
        authority.port_u16().unwrap_or(default_port),
    ))
}

/// Hands a link to the system browser, but only a plain `http`/`https` link: never a file,
/// a custom scheme or a program.
fn open_external(url: &str, from: &str) {
    if origin_of(url).is_some() {
        log::event("LINK_OPENED_EXTERNALLY", format!("from={from}"));
        let _ = open::that_detached(url);
    } else {
        log::event("LINK_REFUSED", format!("from={from}"));
    }
}

/// Where the control pages are served. WebView2 maps custom schemes to `http://<scheme>.localhost`.
#[cfg(windows)]
const CONTROL_ORIGIN: &str = "http://dungeonflux.localhost";
#[cfg(not(windows))]
const CONTROL_ORIGIN: &str = "dungeonflux://localhost";

/// Fields drop in order: the webview goes before the window it lives in.
struct Display {
    view: WebView,
    window: Window,
    origin: (String, String, u16),
    generation: u64,
}

pub fn run(
    data: PathBuf,
    config: PathBuf,
    instance: InstanceGuard,
    connect: Option<String>,
    show_display: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = Mutex::new(event_loop.create_proxy());
    let app = App::new(
        data.clone(),
        config,
        true,
        Box::new(move |event| {
            if let Ok(proxy) = proxy.lock() {
                let _ = proxy.send_event(UserEvent::Shell(event));
            }
        }),
    );

    let show_proxy = event_loop.create_proxy();
    std::thread::spawn(move || {
        while instance.wait_for_show_request() {
            if show_proxy.send_event(UserEvent::ShowControl).is_err() {
                return;
            }
        }
    });

    let mut context = WebContext::new(Some(data.join("webview")));
    let control = WindowBuilder::new()
        .with_title("DungeonFlux")
        .with_inner_size(LogicalSize::new(1040.0, 740.0))
        .with_min_inner_size(LogicalSize::new(520.0, 480.0))
        .with_window_icon(icon())
        .build(&event_loop)?;
    log::event("WINDOW_CONTROL_OPENED", "");
    let handler_app = Arc::clone(&app);
    let control_view = WebViewBuilder::new_with_web_context(&mut context)
        .with_custom_protocol("dungeonflux".into(), move |_id, request| {
            respond(&handler_app, request)
        })
        .with_url("dungeonflux://localhost/")
        .with_background_color((9, 19, 29, 255))
        .with_navigation_handler(|url| {
            // A URL that does not parse never counts as ours, even when CONTROL_ORIGIN itself
            // is not an http origin (the custom scheme off Windows).
            let ours = url.starts_with("dungeonflux:")
                || origin_of(&url).is_some_and(|origin| Some(origin) == origin_of(CONTROL_ORIGIN));
            if !ours {
                open_external(&url, "control");
            }
            ours
        })
        .with_new_window_req_handler(|url, _features| {
            open_external(&url, "control");
            NewWindowResponse::Deny
        })
        .with_devtools(cfg!(debug_assertions))
        .build(&control)?;

    // Queued now, handled once the loop runs: the display window opens like any other request.
    if let Some(url) = connect
        && let Err(message) = app.connect_to(&url, true)
    {
        log::event("CONNECT_REFUSED", format!("reason={}", log::value(message)));
    }
    if show_display {
        app.open_display();
    }

    let mut display: Option<Display> = None;
    let mut generation = 0u64;
    let mut control_visible = true;
    let drop_proxy = event_loop.create_proxy();

    let window_proxy = Mutex::new(event_loop.create_proxy());
    let listening = control::listen(&data, move |request, reply| {
        window_proxy
            .lock()
            .is_ok_and(|proxy| proxy.send_event(UserEvent::Window(request, reply)).is_ok())
    })
    .inspect_err(|error| {
        log::event(
            "CONTROL_FAILED",
            format!("error={}", log::value(error.to_string())),
        );
    })
    .ok();

    event_loop.run(move |event, target, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(UserEvent::Shell(ShellEvent::Refresh)) => {
                let _ = control_view.load_url(&format!("{CONTROL_ORIGIN}/"));
            }
            Event::UserEvent(UserEvent::Shell(ShellEvent::OpenDisplay {
                url,
                screen,
                full_screen,
            })) => {
                match &display {
                    Some(open) if origin_of(&url).as_ref() == Some(&open.origin) => {
                        // Also undoes the hiding of a display that was closing.
                        open.window.set_visible(true);
                        open.window.set_minimized(false);
                        open.window.set_focus();
                        log::event("WINDOW_DISPLAY_FOCUSED", "");
                    }
                    _ => {
                        display = None;
                        generation += 1;
                        match open_display(
                            target,
                            &mut context,
                            &control,
                            &url,
                            screen,
                            full_screen,
                            generation,
                        ) {
                            Ok(opened) => display = Some(opened),
                            Err(error) => {
                                log::event(
                                    "WINDOW_DISPLAY_FAILED",
                                    format!("error={}", log::value(error.to_string())),
                                );
                                open_external(&url, "display-fallback");
                            }
                        }
                    }
                }
                report_display(&app, display.as_ref());
            }
            Event::UserEvent(UserEvent::Shell(ShellEvent::CloseDisplay)) => {
                if display.is_some() {
                    log::event("WINDOW_DISPLAY_CLOSED", "reason=game_ended");
                }
                display = None;
                report_display(&app, None);
                if !control_visible {
                    *control_flow = ControlFlow::Exit;
                }
            }
            Event::UserEvent(UserEvent::Shell(ShellEvent::DisplayFullScreen(full))) => {
                if let Some(open) = &display {
                    let monitor = open.window.current_monitor();
                    open.window
                        .set_fullscreen(full.then_some(Fullscreen::Borderless(monitor)));
                    if !full {
                        open.window.set_focus();
                    }
                }
                report_display(&app, display.as_ref());
            }
            Event::UserEvent(UserEvent::Shell(ShellEvent::CaptureDisplay { path })) => {
                if let Some(open) = &display {
                    capture(&open.view, path, Box::new(|| {}));
                }
            }
            Event::UserEvent(UserEvent::DropDisplay(closing)) => {
                // A late timer must not close a display opened since.
                if display
                    .as_ref()
                    .is_some_and(|open| open.generation == closing)
                {
                    log::event("WINDOW_DISPLAY_CLOSED", "reason=closed");
                    display = None;
                    report_display(&app, None);
                    if !control_visible {
                        *control_flow = ControlFlow::Exit;
                    }
                }
            }
            Event::UserEvent(UserEvent::Window(request, reply)) => {
                let answer = match &request {
                    control::Request::Read => Ok(()),
                    control::Request::Set { name, value } => {
                        let mut windows = Windows {
                            control: &control,
                            control_view: &control_view,
                            control_visible: &mut control_visible,
                            display: display.as_ref(),
                        };
                        let outcome = set_property(&app, target, &mut windows, name, value);
                        if outcome.is_ok()
                            && name == "display.open"
                            && value_is_off(value)
                            && let Some(open) = &display
                        {
                            close_display(&app, open, &drop_proxy, "agent");
                        }
                        log::event(
                            "WINDOW_SET",
                            format!("name={} ok={}", log::value(name), outcome.is_ok()),
                        );
                        outcome
                    }
                };
                let answer = answer.map(|()| {
                    properties(
                        target,
                        &control,
                        &control_view,
                        control_visible,
                        display.as_ref(),
                    )
                });
                let _ = reply.send(answer);
            }
            Event::LoopDestroyed => {
                if let Some(listening) = &listening {
                    listening.close();
                }
            }
            Event::UserEvent(UserEvent::ShowControl) => {
                log::event("WINDOW_CONTROL_SHOWN", "reason=launcher_opened_again");
                control.set_visible(true);
                control.set_minimized(false);
                control.set_focus();
                control_visible = true;
                let _ = control_view.load_url(&format!("{CONTROL_ORIGIN}/"));
            }
            Event::WindowEvent {
                window_id,
                event: WindowEvent::CloseRequested,
                ..
            } => {
                if window_id == control.id() {
                    // The game keeps running (REF-005); keep the table display up if it is open.
                    if display.is_some() {
                        log::event("WINDOW_CONTROL_HIDDEN", "reason=display_still_open");
                        control.set_visible(false);
                        control_visible = false;
                    } else {
                        log::event("LAUNCHER_EXIT", "reason=window_closed");
                        *control_flow = ControlFlow::Exit;
                    }
                } else if let Some(open) = display
                    .as_ref()
                    .filter(|open| open.window.id() == window_id)
                {
                    close_display(&app, open, &drop_proxy, "user");
                }
            }
            _ => {}
        }
    });
}

/// Keeps a picture of the table for the saved-game card, then closes the display: when the
/// capture is saved, or after CAPTURE_WAIT if WebView2 never answers.
fn close_display(
    app: &App,
    open: &Display,
    drop_proxy: &tao::event_loop::EventLoopProxy<UserEvent>,
    by: &str,
) {
    log::event("WINDOW_DISPLAY_CLOSING", format!("by={by}"));
    let closing = open.generation;
    match app.current_screenshot_path() {
        Some(path) => {
            log::event(
                "SCREENSHOT_REQUESTED",
                format!("reason=display_closed by={by}"),
            );
            let proxy = drop_proxy.clone();
            capture(
                &open.view,
                path,
                Box::new(move || {
                    let _ = proxy.send_event(UserEvent::DropDisplay(closing));
                }),
            );
            let late = drop_proxy.clone();
            std::thread::spawn(move || {
                std::thread::sleep(CAPTURE_WAIT);
                let _ = late.send_event(UserEvent::DropDisplay(closing));
            });
        }
        None => {
            let _ = drop_proxy.send_event(UserEvent::DropDisplay(closing));
        }
    }
}

/// What `dungeonflux window set` may change.
struct Windows<'a> {
    control: &'a Window,
    control_view: &'a WebView,
    control_visible: &'a mut bool,
    display: Option<&'a Display>,
}

fn value_is_off(value: &str) -> bool {
    switch(value) == Some(false)
}

fn on_off(name: &str, value: &str) -> Result<bool, String> {
    switch(value).ok_or_else(|| format!("{name} takes true or false"))
}

/// `WxH` in logical pixels, each from 200 to 10000.
fn size(name: &str, value: &str) -> Result<LogicalSize<f64>, String> {
    let problem = || format!("{name} takes a size such as 1040x740");
    let (width, height) = value.split_once('x').ok_or_else(problem)?;
    let side = |text: &str| {
        text.trim()
            .parse::<u32>()
            .ok()
            .filter(|side| (200..=10_000).contains(side))
            .ok_or_else(problem)
    };
    Ok(LogicalSize::new(
        f64::from(side(width)?),
        f64::from(side(height)?),
    ))
}

/// `X,Y` in logical pixels on the virtual desktop.
fn position(name: &str, value: &str) -> Result<LogicalPosition<f64>, String> {
    let problem = || format!("{name} takes a position such as 100,80");
    let (x, y) = value.split_once(',').ok_or_else(problem)?;
    let coordinate = |text: &str| {
        text.trim()
            .parse::<i32>()
            .ok()
            .filter(|coordinate| coordinate.abs() <= 100_000)
            .ok_or_else(problem)
    };
    Ok(LogicalPosition::new(
        f64::from(coordinate(x)?),
        f64::from(coordinate(y)?),
    ))
}

/// Sets the properties both windows share.
fn set_shared(window: &Window, name: &str, field: &str, value: &str) -> Result<bool, String> {
    match field {
        "minimized" => window.set_minimized(on_off(name, value)?),
        "maximized" => window.set_maximized(on_off(name, value)?),
        "focused" => {
            if on_off(name, value)? {
                window.set_minimized(false);
                window.set_focus();
            }
        }
        "size" => window.set_inner_size(size(name, value)?),
        "position" => window.set_outer_position(position(name, value)?),
        _ => return Ok(false),
    }
    Ok(true)
}

fn set_property(
    app: &App,
    target: &EventLoopWindowTarget<UserEvent>,
    windows: &mut Windows,
    name: &str,
    value: &str,
) -> Result<(), String> {
    let unknown = || format!("The window has no settable property {name:?}");
    let (window, field) = name.split_once('.').ok_or_else(unknown)?;
    match window {
        "control" => {
            if set_shared(windows.control, name, field, value)? {
                return Ok(());
            }
            match field {
                "visible" => {
                    let visible = on_off(name, value)?;
                    windows.control.set_visible(visible);
                    *windows.control_visible = visible;
                }
                "page" => {
                    // A launcher page only, never another site.
                    if !value.starts_with('/') || value.starts_with("//") || value.contains('\\') {
                        return Err("control.page takes a launcher path such as /settings".into());
                    }
                    windows
                        .control_view
                        .load_url(&format!("{CONTROL_ORIGIN}{value}"))
                        .map_err(|error| error.to_string())?;
                }
                _ => return Err(unknown()),
            }
            Ok(())
        }
        "display" if field == "open" => {
            if on_off(name, value)? {
                if app.display_url().is_none() {
                    return Err("No game is running, so there is no table display.".into());
                }
                app.open_display();
            }
            // Closing is done by the caller, which owns the display.
            Ok(())
        }
        "display" => {
            let open = windows
                .display
                .ok_or("The table display is not open (set display.open true first).")?;
            if set_shared(&open.window, name, field, value)? {
                return Ok(());
            }
            match field {
                "visible" => open.window.set_visible(on_off(name, value)?),
                "full_screen" => {
                    app.set_display_full_screen(on_off(name, value)?);
                }
                "monitor" => {
                    let monitor = value
                        .parse::<usize>()
                        .ok()
                        .and_then(|index| target.available_monitors().nth(index))
                        .ok_or("display.monitor takes a screen number from monitor.N")?;
                    let full = open.window.fullscreen().is_some();
                    let corner = monitor.position();
                    open.window.set_fullscreen(None);
                    open.window
                        .set_outer_position(PhysicalPosition::new(corner.x + 60, corner.y + 60));
                    if full {
                        open.window
                            .set_fullscreen(Some(Fullscreen::Borderless(Some(monitor))));
                    }
                }
                "screenshot" => {
                    let path = PathBuf::from(value);
                    let fits = path.is_absolute()
                        && path
                            .extension()
                            .is_some_and(|extension| extension.eq_ignore_ascii_case("jpg"))
                        && path.parent().is_some_and(std::path::Path::is_dir);
                    if !fits {
                        return Err(
                            "display.screenshot takes the full path of a .jpg file in an existing folder"
                                .into(),
                        );
                    }
                    log::event("SCREENSHOT_REQUESTED", "reason=agent");
                    capture(&open.view, path, Box::new(|| {}));
                }
                _ => return Err(unknown()),
            }
            Ok(())
        }
        _ => Err(unknown()),
    }
}

fn describe_window(window: &Window, prefix: &str, lines: &mut Vec<(String, String)>) {
    let scale = window.scale_factor();
    let inner = window.inner_size().to_logical::<f64>(scale);
    let mut add = |field: &str, value: String| lines.push((format!("{prefix}.{field}"), value));
    add("title", format!("{:?}", window.title()));
    add("visible", window.is_visible().to_string());
    add("minimized", window.is_minimized().to_string());
    add("maximized", window.is_maximized().to_string());
    add("focused", window.is_focused().to_string());
    add("full_screen", window.fullscreen().is_some().to_string());
    add(
        "size",
        format!("{}x{}", inner.width.round(), inner.height.round()),
    );
    if let Ok(outer) = window.outer_position() {
        let outer = outer.to_logical::<f64>(scale);
        add(
            "position",
            format!("{},{}", outer.x.round(), outer.y.round()),
        );
    }
    add("scale", format!("{scale}"));
    add(
        "monitor",
        format!(
            "{:?}",
            window
                .current_monitor()
                .and_then(|monitor| monitor.name())
                .unwrap_or_default()
        ),
    );
}

/// Every property `dungeonflux window` shows.
fn properties(
    target: &EventLoopWindowTarget<UserEvent>,
    control: &Window,
    control_view: &WebView,
    control_visible: bool,
    display: Option<&Display>,
) -> Vec<(String, String)> {
    let mut lines = Vec::new();
    describe_window(control, "control", &mut lines);
    if let Some(entry) = lines.iter_mut().find(|(name, _)| name == "control.visible") {
        entry.1 = control_visible.to_string();
    }
    let page = control_view.url().ok().and_then(|url| {
        url.strip_prefix(CONTROL_ORIGIN)
            .or_else(|| url.strip_prefix("dungeonflux://localhost"))
            .map(str::to_owned)
    });
    lines.push(("control.page".into(), page.unwrap_or_default()));
    lines.push(("display.open".into(), display.is_some().to_string()));
    if let Some(open) = display {
        describe_window(&open.window, "display", &mut lines);
        lines.push(("display.url".into(), open.view.url().unwrap_or_default()));
    }
    for (index, monitor) in target.available_monitors().enumerate() {
        let size = monitor.size();
        let corner = monitor.position();
        lines.push((
            format!("monitor.{index}"),
            format!(
                "{:?} {}x{} at {},{} scale {}",
                monitor.name().unwrap_or_default(),
                size.width,
                size.height,
                corner.x,
                corner.y,
                monitor.scale_factor()
            ),
        ));
    }
    lines
}

/// Saves the display's picture; `done` runs once saved or once it is clear it cannot be.
#[cfg(windows)]
fn capture(view: &WebView, path: PathBuf, done: Box<dyn FnOnce()>) {
    // A failed request never calls back, so `done` must then run here.
    let started = std::time::Instant::now();
    let shared = std::rc::Rc::new(std::cell::RefCell::new(Some(done)));
    let later = std::rc::Rc::clone(&shared);
    let saved_to = path.clone();
    let result = crate::platform::capture_webview(
        view,
        path,
        Box::new(move || {
            let bytes = std::fs::metadata(&saved_to).map_or(0, |meta| meta.len());
            log::event(
                "SCREENSHOT_DONE",
                format!("bytes={bytes} ms={}", started.elapsed().as_millis()),
            );
            if let Some(done) = later.borrow_mut().take() {
                done();
            }
        }),
    );
    if let Err(error) = result {
        log::event(
            "SCREENSHOT_FAILED",
            format!("error={}", log::value(error.to_string())),
        );
        if let Some(done) = shared.borrow_mut().take() {
            done();
        }
    }
}

#[cfg(not(windows))]
fn capture(_view: &WebView, _path: PathBuf, done: Box<dyn FnOnce()>) {
    done();
}

fn report_display(app: &App, display: Option<&Display>) {
    match display {
        Some(open) => app.display_changed(true, open.window.fullscreen().is_some()),
        None => app.display_changed(false, false),
    }
}

/// The screen for the display: another monitor than the launcher's when asked and present.
fn choose_monitor(
    target: &EventLoopWindowTarget<UserEvent>,
    control: &Window,
    screen: DisplayScreen,
) -> (Option<MonitorHandle>, bool) {
    let current = control.current_monitor();
    if screen == DisplayScreen::Second {
        let other = target.available_monitors().find(|monitor| {
            current.as_ref().is_none_or(|current| {
                monitor.name() != current.name() || monitor.position() != current.position()
            })
        });
        if let Some(other) = other {
            return (Some(other), true);
        }
    }
    (current, false)
}

fn open_display(
    target: &EventLoopWindowTarget<UserEvent>,
    context: &mut WebContext,
    control: &Window,
    url: &str,
    screen: DisplayScreen,
    full_screen: FullScreen,
    generation: u64,
) -> Result<Display, Box<dyn std::error::Error>> {
    let (monitor, second) = choose_monitor(target, control, screen);
    let full = match full_screen {
        FullScreen::Always => true,
        FullScreen::OnSecondScreen => second,
        FullScreen::Never => false,
    };
    let mut builder = WindowBuilder::new()
        .with_title("DungeonFlux table display")
        .with_inner_size(LogicalSize::new(1280.0, 720.0))
        .with_min_inner_size(LogicalSize::new(480.0, 270.0))
        .with_window_icon(icon());
    if let Some(monitor) = &monitor {
        let origin = monitor.position();
        builder = builder.with_position(PhysicalPosition::new(origin.x + 60, origin.y + 60));
        if full {
            builder = builder.with_fullscreen(Some(Fullscreen::Borderless(Some(monitor.clone()))));
        }
    }
    let origin = origin_of(url).ok_or("the table display address is not an http address")?;
    let window = builder.build(target)?;
    let allowed = origin.clone();
    let view = WebViewBuilder::new_with_web_context(context)
        .with_url(url)
        .with_autoplay(true)
        .with_background_color((0, 0, 0, 255))
        .with_navigation_handler(move |next| {
            let ours = origin_of(&next).as_ref() == Some(&allowed);
            if !ours {
                open_external(&next, "display");
            }
            ours
        })
        .with_new_window_req_handler(|next, _features| {
            open_external(&next, "display");
            NewWindowResponse::Deny
        })
        .with_devtools(cfg!(debug_assertions))
        .build(&window)?;
    log::event(
        "WINDOW_DISPLAY_OPENED",
        format!(
            "screen={} second_screen={second} full_screen={full}",
            log::value(
                monitor
                    .and_then(|monitor| monitor.name())
                    .unwrap_or_default()
            )
        ),
    );
    Ok(Display {
        view,
        window,
        origin,
        generation,
    })
}

#[cfg(windows)]
fn icon() -> Option<Icon> {
    use tao::platform::windows::IconExtWindows;
    Icon::from_resource(1, None).ok()
}

#[cfg(not(windows))]
fn icon() -> Option<Icon> {
    None
}

fn respond(app: &Arc<App>, request: Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let started = std::time::Instant::now();
    let method = request.method().as_str().to_owned();
    // Only the launcher's own pages may act on it. (Only this webview has the scheme, so a
    // foreign page cannot reach it; the check is a second line.)
    let foreign = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|origin| origin.to_str().ok())
        .is_some_and(|origin| origin != CONTROL_ORIGIN && origin != "null");
    if method == "POST" && foreign {
        log::event("PAGE_REFUSED", "reason=foreign_origin");
        return plain(403, "Refused");
    }
    let target = request
        .uri()
        .path_and_query()
        .map_or("/", |path| path.as_str())
        .to_owned();
    let page = ui::handle(app, &method, &target, request.body());
    log_request(&method, &target, page.status, started);
    let mut builder = Response::builder()
        .status(page.status)
        .header(header::CONTENT_TYPE, page.content_type)
        .header(header::CACHE_CONTROL, page.cache_control);
    if let Some(location) = &page.location {
        builder = builder.header(header::LOCATION, location.as_str());
    }
    builder
        .body(Cow::Owned(page.body))
        .unwrap_or_else(|_| plain(500, "The launcher page could not be built"))
}

/// One line per page or form, without the query string or body (a form may carry a key).
pub fn log_request(method: &str, target: &str, status: u16, started: std::time::Instant) {
    let path = target.split('?').next().unwrap_or("/");
    if path.starts_with("/art/") || path == "/style.css" {
        return;
    }
    log::event(
        "PAGE",
        format!(
            "method={method} path={} status={status} ms={}",
            log::value(path),
            started.elapsed().as_millis()
        ),
    );
}

fn plain(status: u16, text: &'static str) -> Response<Cow<'static, [u8]>> {
    let mut response = Response::new(Cow::Borrowed(text.as_bytes()));
    *response.status_mut() = wry::http::StatusCode::from_u16(status)
        .unwrap_or(wry::http::StatusCode::INTERNAL_SERVER_ERROR);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_compare_exactly_and_refuse_tricks() {
        let game = origin_of("http://127.0.0.1:63419/gameplay/display");
        assert_eq!(game, Some(("http".into(), "127.0.0.1".into(), 63419)));
        assert_eq!(origin_of("http://127.0.0.1:63419/other"), game);
        for other in [
            "http://127.0.0.1:63419.evil.test/gameplay/display",
            "http://127.0.0.1:63420/gameplay/display",
            "https://127.0.0.1:63419/gameplay/display",
        ] {
            assert_ne!(origin_of(other), game, "{other}");
        }
        for refused in [
            "http://user@127.0.0.1:63419/",
            "file:///C:/Windows/System32/calc.exe",
            "ms-settings:privacy",
            "javascript:alert(1)",
            "C:\\Windows\\notepad.exe",
        ] {
            assert_eq!(origin_of(refused), None, "{refused}");
        }
        assert_eq!(
            origin_of("https://Example.org/x"),
            Some(("https".into(), "example.org".into(), 443))
        );
    }
}
