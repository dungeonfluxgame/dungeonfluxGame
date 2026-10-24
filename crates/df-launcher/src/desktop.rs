//! The desktop windows, on tao and wry (the window and WebView2 layers Tauri is built on).
//! The control window shows the launcher's own pages through a private `dungeonflux:` scheme;
//! the display window shows the running game's table display, with sound allowed to start.
use std::{
    borrow::Cow,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use tao::{
    dpi::{LogicalSize, PhysicalPosition},
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
    platform::InstanceGuard,
    settings::{DisplayScreen, FullScreen},
    ui,
};

#[derive(Debug)]
enum UserEvent {
    Shell(ShellEvent),
    ShowControl,
    /// The closing display window's last screenshot is saved; it can go now.
    DropDisplay,
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
    origin: String,
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
    let handler_app = Arc::clone(&app);
    let control_view = WebViewBuilder::new_with_web_context(&mut context)
        .with_custom_protocol("dungeonflux".into(), move |_id, request| {
            respond(&handler_app, request)
        })
        .with_url("dungeonflux://localhost/")
        .with_background_color((9, 19, 29, 255))
        .with_navigation_handler(|url| {
            let ours = url.starts_with(CONTROL_ORIGIN) || url.starts_with("dungeonflux:");
            if !ours && (url.starts_with("http://") || url.starts_with("https://")) {
                let _ = open::that_detached(&url);
            }
            ours
        })
        .with_new_window_req_handler(|url, _features| {
            let _ = open::that_detached(&url);
            NewWindowResponse::Deny
        })
        .with_devtools(cfg!(debug_assertions))
        .build(&control)?;

    // Queued now, handled once the loop runs: the display window opens like any other request.
    if let Some(url) = connect
        && let Err(message) = app.connect_to(&url, true)
    {
        eprintln!("CONNECT refused: {message}");
    }
    if show_display {
        app.open_display();
    }

    let mut display: Option<Display> = None;
    let mut control_visible = true;
    let drop_proxy = event_loop.create_proxy();

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
                    Some(open) if url.starts_with(&open.origin) => {
                        open.window.set_minimized(false);
                        open.window.set_focus();
                    }
                    _ => {
                        display = None;
                        match open_display(
                            target,
                            &mut context,
                            &control,
                            &url,
                            screen,
                            full_screen,
                        ) {
                            Ok(opened) => display = Some(opened),
                            Err(error) => {
                                eprintln!("DISPLAY_WINDOW error={error}");
                                let _ = open::that_detached(&url);
                            }
                        }
                    }
                }
                report_display(&app, display.as_ref());
            }
            Event::UserEvent(UserEvent::Shell(ShellEvent::CloseDisplay)) => {
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
            Event::UserEvent(UserEvent::DropDisplay) => {
                display = None;
                report_display(&app, None);
                if !control_visible {
                    *control_flow = ControlFlow::Exit;
                }
            }
            Event::UserEvent(UserEvent::ShowControl) => {
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
                        control.set_visible(false);
                        control_visible = false;
                    } else {
                        *control_flow = ControlFlow::Exit;
                    }
                } else if let Some(open) = display
                    .as_ref()
                    .filter(|open| open.window.id() == window_id)
                {
                    // Keep a picture of the table for the saved-game card, then close.
                    open.window.set_visible(false);
                    match app.current_screenshot_path() {
                        Some(path) => {
                            let proxy = drop_proxy.clone();
                            capture(
                                &open.view,
                                path,
                                Box::new(move || {
                                    let _ = proxy.send_event(UserEvent::DropDisplay);
                                }),
                            );
                        }
                        None => {
                            let _ = drop_proxy.send_event(UserEvent::DropDisplay);
                        }
                    }
                }
            }
            _ => {}
        }
    });
}

/// Saves the display's picture; `done` runs once saved or once it is clear it cannot be.
#[cfg(windows)]
fn capture(view: &WebView, path: PathBuf, done: Box<dyn FnOnce()>) {
    // A failed request never calls back, so `done` must then run here.
    let shared = std::rc::Rc::new(std::cell::RefCell::new(Some(done)));
    let later = std::rc::Rc::clone(&shared);
    let result = crate::platform::capture_webview(
        view,
        path,
        Box::new(move || {
            if let Some(done) = later.borrow_mut().take() {
                done();
            }
        }),
    );
    if let Err(error) = result {
        eprintln!("SCREENSHOT request_failed error={error}");
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
    let window = builder.build(target)?;
    let origin = url
        .split_once("/gameplay/")
        .map_or(url, |(origin, _)| origin)
        .to_owned();
    let allowed = origin.clone();
    let view = WebViewBuilder::new_with_web_context(context)
        .with_url(url)
        .with_autoplay(true)
        .with_background_color((0, 0, 0, 255))
        .with_navigation_handler(move |next| {
            let ours = next.starts_with(&allowed);
            if !ours && (next.starts_with("http://") || next.starts_with("https://")) {
                let _ = open::that_detached(&next);
            }
            ours
        })
        .with_new_window_req_handler(|next, _features| {
            let _ = open::that_detached(&next);
            NewWindowResponse::Deny
        })
        .with_devtools(cfg!(debug_assertions))
        .build(&window)?;
    Ok(Display {
        view,
        window,
        origin,
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
    let method = request.method().as_str().to_owned();
    // Only the launcher's own pages may act on it.
    let foreign = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|origin| origin.to_str().ok())
        .is_some_and(|origin| origin != CONTROL_ORIGIN && origin != "null");
    if method == "POST" && foreign {
        return plain(403, "Refused");
    }
    let target = request
        .uri()
        .path_and_query()
        .map_or("/", |path| path.as_str())
        .to_owned();
    let page = ui::handle(app, &method, &target, request.body());
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

fn plain(status: u16, text: &'static str) -> Response<Cow<'static, [u8]>> {
    let mut response = Response::new(Cow::Borrowed(text.as_bytes()));
    *response.status_mut() = wry::http::StatusCode::from_u16(status)
        .unwrap_or(wry::http::StatusCode::INTERNAL_SERVER_ERROR);
    response
}
