//! The `/dev/media/locate` routes: scan a picture, show or label its stored layout, identify its
//! characters.
//!
//! They are merged into the host's one router before the layers, so they sit behind the same guard
//! as every route. Each answers a `LocateReply`: the value, or a typed [`LocateError`] with its
//! status. Scan, label and identify are POSTs of JSON; show is a GET with `?picture=`. Blocking
//! work runs on the blocking pool, never on a runtime worker, and each outcome is one `LOCATE`
//! audit line of names, counts and times; a refusal before any work is `LOCATE refused verb=..
//! picture=.. error=<key>`. Once a scan or an identification has begun, a failure is instead
//! `LOCATE failed verb=.. picture=.. error=<key> cause=<key|-> stage=<stage> duration_ms=..`
//! (with `operation=` once the text-service operation is known), and each identification stage
//! that holds is `LOCATE identify_stage picture=.. stage=.. duration_ms=..`.

use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::{get, post};
use tokio::time::Instant;

use super::request::{
    IdentifiedLayout, IdentifyRequest, LabelRequest, LocateError, LocateRequest, LocateStage,
    LocateStageError, LocatedLayout, ShowQuery, StoredLayout,
};
use super::scan::{ScanSupplier, scan_stages};
use super::storage::lock_layout_writes;
use super::{check_picture_name, layout_file_name, relabel, write_layout};
use crate::dev_http::{parse_body, reply};
use crate::media_library::media_directory;
use crate::state::AppState;
use crate::text::TextRuntime;

/// The addresses of the locate dev API.
pub mod path {
    /// Scans a picture (POST a `LocateRequest`).
    pub const LOCATE: &str = "/dev/media/locate";
    /// The stored layout of a picture (GET with `?picture=`).
    pub const LOCATE_SHOW: &str = "/dev/media/locate/show";
    /// Names or clears a region (POST a `LabelRequest`).
    pub const LOCATE_LABEL: &str = "/dev/media/locate/label";
    /// Names regions by asking the text service (POST an `IdentifyRequest`).
    pub const LOCATE_IDENTIFY: &str = "/dev/media/locate/identify";
}

fn layouts(app: &AppState) -> PathBuf {
    app.playground()
        .files()
        .assets()
        .join(super::LAYOUT_DIRECTORY)
}

fn media(app: &AppState) -> PathBuf {
    media_directory(app.playground().files())
}

/// The picture as a line writes it: its name when it is a valid library name, else `-`, so other
/// request text never reaches an audit line.
fn checked_picture(picture: &str) -> &str {
    if check_picture_name(picture).is_ok() {
        picture
    } else {
        "-"
    }
}

/// Why a locate request ended without its value: refused before any work began, or failed after
/// it began (its `LOCATE failed` line is already written).
enum Ended {
    Refused(LocateError),
    Failed(LocateError),
}

impl From<LocateError> for Ended {
    fn from(error: LocateError) -> Self {
        Self::Refused(error)
    }
}

/// Answers `result`, recording a refusal first; a failure after work began was recorded where it
/// happened.
fn answer<T: serde::Serialize>(
    app: &AppState,
    verb: &str,
    picture: &str,
    result: Result<T, Ended>,
) -> Response {
    let result = result.map_err(|ended| match ended {
        Ended::Refused(error) => {
            app.record_line(format!(
                "LOCATE refused verb={verb} picture={} error={}",
                checked_picture(picture),
                error.key()
            ));
            error
        }
        Ended::Failed(error) => error,
    });
    reply(result)
}

/// A failure of work that had begun at `started`, recorded as one `LOCATE failed` line.
struct StageFailure<'a> {
    verb: &'a str,
    picture: &'a str,
    started: Instant,
    operation: Option<&'a str>,
}

impl StageFailure<'_> {
    /// Records `failed` and gives it back as the request's end.
    fn record(&self, app: &AppState, failed: LocateStageError) -> Ended {
        app.record_line(failed_line(
            self.verb,
            self.picture,
            &failed,
            self.started.elapsed().as_millis(),
            self.operation,
        ));
        Ended::Failed(failed.error)
    }
}

/// The `LOCATE failed` line of `failed`: closed keys, the checked picture, milliseconds, a scan's
/// send state and the text-service operation when there is one.
pub(super) fn failed_line(
    verb: &str,
    picture: &str,
    failed: &LocateStageError,
    duration_ms: u128,
    operation: Option<&str>,
) -> String {
    let mut line = format!(
        "LOCATE failed verb={verb} picture={} error={} cause={} stage={} duration_ms={duration_ms}",
        checked_picture(picture),
        failed.error.key(),
        failed.error.cause_key().unwrap_or("-"),
        failed.stage.key()
    );
    if let LocateError::ScanFailed { send_state, .. } = &failed.error {
        line.push_str(&format!(" send_state={}", send_state.key()));
    }
    if let Some(operation) = operation {
        line.push_str(&format!(" operation={operation}"));
    }
    line
}

/// Runs locate work on the blocking pool; a task that stopped without an answer is `Stopped`.
async fn run_locate_task<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, LocateError> + Send + 'static,
) -> Result<T, LocateError> {
    tokio::task::spawn_blocking(work)
        .await
        .unwrap_or(Err(LocateError::Stopped))
}

/// Runs a paid scan on a task of its own and waits for it, so the scan and the layout it stores
/// run to the end even when the request is dropped (the client left, or shutdown's grace ended).
/// A task that stopped without an answer is `Stopped` while scanning.
pub(super) async fn run_scan_task<T: Send + 'static>(
    scan: impl Future<Output = Result<T, LocateStageError>> + Send + 'static,
) -> Result<T, LocateStageError> {
    tokio::spawn(scan).await.unwrap_or_else(|_| {
        Err(LocateStageError {
            stage: LocateStage::Scan,
            error: LocateError::Stopped,
        })
    })
}

async fn locate(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    scan_answer(&app, &bytes, ScanSupplier::from_environment()).await
}

/// Answers the scan request `bytes`, scanning through `supplier`.
pub(super) async fn scan_answer(app: &AppState, bytes: &[u8], supplier: ScanSupplier) -> Response {
    let request: LocateRequest = match parse_body(bytes) {
        Ok(request) => request,
        Err(error) => {
            let error = LocateError::from(error);
            return answer::<LocatedLayout>(app, "scan", "-", Err(error.into()));
        }
    };
    let picture = request.picture.clone();
    let result = scan(app, request, supplier).await;
    answer(app, "scan", &picture, result)
}

/// Checks a scan request, runs the scan through `supplier` and records what it stored.
async fn scan(
    app: &AppState,
    request: LocateRequest,
    supplier: ScanSupplier,
) -> Result<LocatedLayout, Ended> {
    // The audit line names only checked values: the picture's library name and the prompt.
    check_picture_name(&request.picture)?;
    let masks = super::check_masks(request.masks)?;
    let picture = request.picture.clone();
    app.record_line(format!(
        "LOCATE started picture={picture} prompt={} masks={masks}",
        request.prompt.unwrap_or_default().key()
    ));
    let failure = StageFailure {
        verb: "scan",
        picture: &picture,
        started: Instant::now(),
        operation: None,
    };
    let (directory, stored_in) = (media(app), layouts(app));
    let scanned =
        run_scan_task(async move { scan_stages(&directory, &stored_in, &request, supplier).await })
            .await
            .map_err(|failed| failure.record(app, failed))?;
    app.record_line(stored_line(&picture, &scanned));
    Ok(LocatedLayout {
        layout_file: layout_file_name(&picture),
        picture,
        duration_ms: scanned.duration_ms,
        kept_labels: scanned.kept_labels,
        layout: scanned.layout,
    })
}

/// The `LOCATE stored` line of a scan of `picture`: names, counts, the picture's size, the whole
/// scan's time and the imagery API's time for each phase.
pub(super) fn stored_line(picture: &str, scanned: &super::Scanned) -> String {
    format!(
        concat!(
            "LOCATE stored picture={} file={} model={} prompt={} regions={} left_out={} ",
            "kept_labels={} old_layout={} size={}x{} duration_ms={} prepare_ms={} submit_ms={} ",
            "resume_ms={}"
        ),
        picture,
        layout_file_name(picture),
        scanned.layout.model(),
        scanned.layout.prompt(),
        scanned.layout.regions().len(),
        scanned.left_out,
        scanned.kept_labels,
        if scanned.old_layout_unreadable {
            "unreadable"
        } else {
            "read"
        },
        scanned.layout.picture().width_px,
        scanned.layout.picture().height_px,
        scanned.duration_ms,
        scanned.phases.prepare_ms,
        scanned.phases.submit_ms,
        scanned.phases.resume_ms
    )
}

fn stored(
    picture: &str,
    found: Option<df_scene_layout::SceneLayout>,
) -> Result<df_scene_layout::SceneLayout, LocateError> {
    found.ok_or_else(|| LocateError::NoLayout {
        picture: picture.to_owned(),
    })
}

async fn show(
    State(app): State<Arc<AppState>>,
    query: Result<Query<ShowQuery>, QueryRejection>,
) -> Response {
    let Query(query) = match query {
        Ok(query) => query,
        Err(rejection) => {
            let error = LocateError::InvalidQuery {
                detail: rejection.body_text(),
            };
            return answer::<StoredLayout>(&app, "show", "-", Err(error.into()));
        }
    };
    let picture = query.picture.clone();
    let (media_folder, directory) = (media(&app), layouts(&app));
    let read = run_locate_task(move || {
        check_picture_name(&query.picture)?;
        stored(
            &query.picture,
            super::current_layout(&media_folder, &directory, &query.picture)?,
        )
    })
    .await;
    let result = read.map(|layout| StoredLayout {
        layout_file: layout_file_name(&picture),
        picture: picture.clone(),
        layout,
    });
    answer(&app, "show", &picture, result.map_err(Ended::from))
}

async fn label(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let request: LabelRequest = match parse_body(&bytes) {
        Ok(request) => request,
        Err(error) => {
            let error = LocateError::from(error);
            return answer::<StoredLayout>(&app, "label", "-", Err(error.into()));
        }
    };
    let picture = request.picture.clone();
    let result = relabel_stored(&app, request).await;
    answer(&app, "label", &picture, result.map_err(Ended::from))
}

/// Names or clears one region of the stored layout under the layout write lock, and records it.
async fn relabel_stored(
    app: &AppState,
    request: LabelRequest,
) -> Result<StoredLayout, LocateError> {
    let (picture, region, name) = (
        request.picture.clone(),
        request.region.clone(),
        request.label.clone(),
    );
    let (media_folder, directory) = (media(app), layouts(app));
    let layout = run_locate_task(move || {
        check_picture_name(&request.picture)?;
        let _writing = lock_layout_writes();
        let mut layout = stored(
            &request.picture,
            super::current_layout(&media_folder, &directory, &request.picture)?,
        )?;
        relabel(
            &mut layout,
            df_scene_layout::RegionRef::parse(&request.region),
            request.label.as_deref(),
        )?;
        write_layout(&directory, &request.picture, &layout)?;
        Ok(layout)
    })
    .await?;
    app.record_line(format!(
        "LOCATE labelled picture={picture} region={region} label={}",
        name.as_deref().unwrap_or("-")
    ));
    Ok(StoredLayout {
        layout_file: layout_file_name(&picture),
        picture,
        layout,
    })
}

/// The host's one text surface, built from the environment. The text service may record after a
/// request was answered, so its lines are echoed as they come.
pub fn text_runtime(app: &Arc<AppState>) -> Arc<TextRuntime> {
    let recorder = Arc::clone(app);
    Arc::new(TextRuntime::from_environment(Arc::new(move |line| {
        recorder.record_line_and_echo(line);
    })))
}

/// The locate routes, with a text surface of their own built from the environment.
pub fn router(app: Arc<AppState>) -> Router {
    let text = text_runtime(&app);
    router_with(app, text)
}

/// The locate routes, asking identity questions of `text`.
pub fn router_with(app: Arc<AppState>, text: Arc<TextRuntime>) -> Router {
    Router::new()
        .route(path::LOCATE, post(locate))
        .route(path::LOCATE_SHOW, get(show))
        .route(
            path::LOCATE_IDENTIFY,
            post(move |State(app): State<Arc<AppState>>, bytes: Bytes| {
                let text = Arc::clone(&text);
                async move { identify(app, text, bytes).await }
            }),
        )
        .route(path::LOCATE_LABEL, post(label))
        .with_state(app)
}

/// Names the regions of a picture's layout: the imagery API prepares the question, the host's one
/// text service asks it, the imagery API checks the answer, and the sure matches are stored. Each
/// outcome is one `LOCATE identified` line of names, region numbers and confidences, never the
/// descriptions.
async fn identify(app: Arc<AppState>, text: Arc<TextRuntime>, bytes: Bytes) -> Response {
    let request: IdentifyRequest = match parse_body(&bytes) {
        Ok(request) => request,
        Err(error) => {
            let error = LocateError::from(error);
            return answer::<IdentifiedLayout>(&app, "identify", "-", Err(error.into()));
        }
    };
    let picture = request.picture.clone();
    let result = identify_characters(&app, &text, request).await;
    answer(&app, "identify", &picture, result)
}

/// Records that `stage` of the identification of `picture`, begun at `started`, held.
fn record_stage(app: &AppState, picture: &str, stage: LocateStage, started: Instant) {
    app.record_line(format!(
        "LOCATE identify_stage picture={picture} stage={} duration_ms={}",
        stage.key(),
        started.elapsed().as_millis()
    ));
}

/// Prepares, asks and stores one identification, and records its outcome.
async fn identify_characters(
    app: &AppState,
    text: &TextRuntime,
    request: IdentifyRequest,
) -> Result<IdentifiedLayout, Ended> {
    check_picture_name(&request.picture)?;
    let picture = request.picture.clone();
    app.record_line(format!(
        "LOCATE identify picture={picture} characters={}",
        request.characters.len()
    ));
    let (directory, stored_in) = (media(app), layouts(app));
    let started = Instant::now();
    let mut failure = StageFailure {
        verb: "identify",
        picture: &picture,
        started,
        operation: None,
    };
    let (prepare_media, prepare_in) = (directory.clone(), stored_in.clone());
    let preparing = Instant::now();
    let prepared =
        run_locate_task(move || super::prepare_identity(&prepare_media, &prepare_in, &request))
            .await
            .map_err(|error| {
                failure.record(app, LocateStageError::at(LocateStage::Prepare)(error))
            })?;
    record_stage(app, &picture, LocateStage::Prepare, preparing);
    let asking = Instant::now();
    let at_ask = LocateStageError::at(LocateStage::Ask);
    let submission = super::identify::identity_submission(text, &prepared)
        .map_err(|error| failure.record(app, at_ask(error)))?;
    failure.operation = Some(submission.key.operation());
    let answered = super::ask_identity(text, &prepared, &submission)
        .await
        .map_err(|error| failure.record(app, at_ask(error)))?;
    record_stage(app, &picture, LocateStage::Ask, asking);
    let storing = Instant::now();
    let store_picture = picture.clone();
    let (layout, rows) = run_locate_task(move || {
        super::store_identification(
            &directory,
            &stored_in,
            &store_picture,
            &prepared.layout,
            &answered.found,
            prepared.min_confidence,
        )
    })
    .await
    .map_err(|error| failure.record(app, LocateStageError::at(LocateStage::Store)(error)))?;
    record_stage(app, &picture, LocateStage::Store, storing);
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let profile = text.identity_profile().as_str().to_owned();
    app.record_line(format!(
        concat!(
            "LOCATE identified picture={} profile={} replayed={} duration_ms={} ",
            "results={}"
        ),
        picture,
        profile,
        answered.replayed,
        duration_ms,
        identity_summary(&rows)
    ));
    Ok(IdentifiedLayout {
        picture,
        profile,
        duration_ms,
        replayed: answered.replayed,
        rows,
        layout,
    })
}

/// The rows of an identification as the audit line writes them: `name:region:confidence:outcome`.
fn identity_summary(rows: &[super::IdentityRow]) -> String {
    rows.iter()
        .map(|row| {
            format!(
                "{}:{}:{}:{}",
                row.name,
                row.region
                    .map_or_else(|| "-".to_owned(), |region| region.to_string()),
                row.confidence_permille,
                row.outcome.key()
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}
