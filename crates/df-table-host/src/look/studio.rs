//! The look studio's work over one picture generator and one text service: the frames' base poses,
//! each seat's versions (refine, keep, discard, start again) and the lock-in.
//!
//! A refine never replaces the kept version: it adds a pending version, and only the player's keep
//! makes it the kept one. A picture that arrives after a newer refine began is stale and can never
//! be kept (REF-032). Nothing is retried and nothing is counted against a quota (REF-034). A
//! picture is made on its own task, so a phone that stops waiting does not cancel what was paid
//! for. Every step is one `LOOK` line with ids, keys and counts only, never a name or the player's
//! words.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use df_imagery::CreatorCategory;
use df_playground::look::wire::{
    BodyFrame, HeroPack, LockState, LookControls, LookDetails, LookDraft, LookLock, LookLockAsk,
    LookPackAsk, LookRefineAsk, LookStartAsk, LookStartOptions, LookStudioView, LookVersion,
    PackItem, PackStatus, PictureFailure, VersionState,
};
use df_rules::local_journey::valid_character_name;
use df_table::ports::Clock;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::catalog::{PublicHero, Studio, read_studio};
use super::jev::{DetailsRead, held, judge_look, read_details};
use super::request::{
    DraftCause, FramePoses, HeroBind, HeroLook, LookError, PoseCandidates, PoseCandidatesAsk,
    PoseChoose, PoseView, PosesView,
};

/// Put after the lock-in sheet's prompt, before the kept look's details: the three-view template
/// draws a plain figure, so what sets the hero apart is asked for in every view.
const SHEET_DETAILS_LEAD: &str =
    "In all three views, the same in each, the character shows these features of their look: ";
use super::store::{
    StudioRecord, read_picture, read_poses, read_record, write_poses, write_record,
};
use crate::adapters::{EntropySource, SeededEntropy, SystemEntropy};
use crate::blocking::run_blocking;
use crate::imagegen::{
    GenerateError, GenerateRequestError, ImageGenerator, ImageRequest, JobState, JobView, Purpose,
    SheetPick, ThingReference,
};
use crate::media_library::{media_directory, store_bytes};
use crate::observe::WallClock;
use crate::persona::reference_sheet_name;
use crate::seat::book::{Book, HeroRecord};
use crate::seat::request::SeatError;
use crate::state::AppState;
use crate::text::TextRuntime;

/// The hero a seat plays, as the studio needs it.
#[derive(Debug, Clone)]
struct SeatHero {
    hero: u64,
    public: PublicHero,
}

/// How a new version is asked for: by a refine (`locking` false), or by the lock-in of a design
/// with no kept version; with `wait`, the answer comes when its picture ended.
#[derive(Debug, Clone, Copy)]
struct VersionCall {
    seat: u64,
    kept: Option<u64>,
    locking: bool,
    wait: bool,
}

/// What a new version is made from: the version it edits, if any, and the draft.
struct VersionPlan {
    parent: Option<LookVersion>,
    draft: LookDraft,
    changes: Vec<String>,
    prompt: String,
    base: String,
    things: Vec<ThingReference>,
}

/// The look studio of the host.
pub struct LookStudio {
    generator: Arc<ImageGenerator>,
    text: Arc<TextRuntime>,
    /// Each seat's record, read from its file on first use. Held only to read or change a record
    /// and write its file, never across a model call or a picture.
    records: tokio::sync::Mutex<BTreeMap<u64, StudioRecord>>,
    /// Held around a read-then-write of the poses file.
    poses: tokio::sync::Mutex<()>,
    /// The tasks that wait for a picture and record it, owned until they end.
    followers: Mutex<Vec<JoinHandle<()>>>,
}

fn assets(app: &AppState) -> PathBuf {
    app.playground().files().assets().to_path_buf()
}

fn now_ms() -> u64 {
    WallClock.now_ms()
}

fn invalid_request(error: GenerateRequestError) -> LookError {
    LookError::Generate {
        cause: GenerateError::InvalidRequest(error),
    }
}

/// The closed failure of a picture the image service did not store.
pub(crate) fn failure_of(error: &GenerateError) -> PictureFailure {
    match error {
        GenerateError::InvalidBody(_)
        | GenerateError::InvalidRequest(_)
        | GenerateError::NameBusy { .. }
        | GenerateError::Backend { .. } => PictureFailure::Backend,
        GenerateError::InvalidPicture { .. } | GenerateError::StoreFailed { .. } => {
            PictureFailure::NotStored
        }
        GenerateError::BaseUnreadable { .. } | GenerateError::ReferenceUnreadable { .. } => {
            PictureFailure::Unreadable
        }
        GenerateError::Stopped { .. } => PictureFailure::Stopped,
    }
}

/// The stored picture of a job's end, or why there is none.
fn picture_of(
    end: Result<Result<JobView, GenerateError>, oneshot::error::RecvError>,
) -> Result<String, PictureFailure> {
    match end {
        Ok(Ok(JobView {
            state: JobState::Stored { provenance, .. },
            ..
        })) => Ok(provenance.media_file),
        Ok(Ok(JobView {
            state: JobState::Failed { error, .. },
            ..
        })) => Err(failure_of(&error)),
        Ok(Err(error)) => Err(failure_of(&error)),
        Ok(Ok(JobView {
            state: JobState::Running,
            ..
        }))
        | Err(_) => Err(PictureFailure::Stopped),
    }
}

/// Refuses a refine the record does not take: the look is locked in or being locked, or the
/// phone's kept version is not the studio's.
fn check_refinable(
    record: &StudioRecord,
    kept: Option<u64>,
    locking: bool,
) -> Result<(), LookError> {
    match &record.view.lock {
        Some(LookLock {
            hero,
            state: LockState::Locked { .. },
        }) => return Err(LookError::Locked { hero: *hero }),
        Some(_) if !locking => return Err(LookError::LockBusy),
        _ => {}
    }
    // No kept version renders a new person from the base pose, whatever the studio keeps.
    if kept.is_some() && record.view.kept != kept {
        return Err(LookError::KeptChanged {
            kept: record.view.kept,
        });
    }
    Ok(())
}

fn version_mut(record: &mut StudioRecord, version: u64) -> Result<&mut LookVersion, LookError> {
    record
        .view
        .versions
        .iter_mut()
        .find(|known| known.id == version)
        .ok_or(LookError::UnknownVersion { version })
}

/// Records how version `id` ended: its picture, ready, or stale when a newer version was asked
/// for meanwhile (REF-032); or failed.
pub(super) fn arrive(
    record: &mut StudioRecord,
    id: u64,
    end: Result<String, VersionState>,
    took_ms: u64,
) -> Result<LookVersion, LookError> {
    let newer = record.view.versions.iter().any(|known| known.id > id);
    let found = version_mut(record, id)?;
    found.took_ms = Some(took_ms);
    match end {
        Ok(picture) => {
            found.picture = Some(picture);
            found.state = if newer {
                VersionState::Stale
            } else {
                VersionState::Ready
            };
        }
        Err(failed) => found.state = failed,
    }
    Ok(found.clone())
}

/// Makes `version` the kept one, when it may be kept; answers the version kept before.
pub(super) fn keep_in(record: &mut StudioRecord, version: u64) -> Result<Option<u64>, LookError> {
    let state = version_mut(record, version)?.state;
    if !state.keepable() {
        return Err(LookError::NotKeepable { version, state });
    }
    Ok(record.view.kept.replace(version))
}

/// Refuses a draft with a detail that is neither one Jev read for this seat's hero nor one of the
/// studio's rolled details: the words of every detail reach the image prompt.
pub(super) fn check_details(
    studio: &Studio,
    record: &StudioRecord,
    draft: &LookDraft,
) -> Result<(), LookError> {
    let unread = draft
        .details
        .iter()
        .any(|detail| !record.read_details.contains(detail) && !studio.rolls(detail));
    if unread {
        return Err(LookError::InvalidDraft {
            cause: DraftCause::UnreadDetail,
        });
    }
    Ok(())
}

/// The kept version of `record` when it is `kept`, checked as one a new version can edit.
fn parent_of(record: &StudioRecord, kept: Option<u64>) -> Result<Option<LookVersion>, LookError> {
    let Some(kept) = kept else {
        return Ok(None);
    };
    let parent = record
        .view
        .versions
        .iter()
        .find(|known| known.id == kept)
        .ok_or(LookError::UnknownVersion { version: kept })?;
    if parent.picture.is_none() || !parent.state.keepable() {
        return Err(LookError::NotKeepable {
            version: kept,
            state: parent.state,
        });
    }
    Ok(Some(parent.clone()))
}

/// What a new version of `hero` with the settled `draft` is made from: a first render of the
/// whole description on `base_pose`, or an edit of `parent` asking only for what changed, with the
/// base pose shown as a reference.
fn plan_version(
    studio: &Studio,
    hero: &PublicHero,
    draft: LookDraft,
    parent: Option<LookVersion>,
    base_pose: &str,
) -> Result<VersionPlan, LookError> {
    let Some(parent) = parent else {
        let prompt = studio.first_prompt(&studio.description(hero, &draft)?);
        return Ok(VersionPlan {
            parent: None,
            draft,
            changes: Vec::new(),
            prompt,
            base: base_pose.to_owned(),
            things: Vec::new(),
        });
    };
    let changes = studio.changes(&hero.species, &parent.draft, &draft)?;
    if changes.is_empty() {
        return Err(LookError::NothingChanged);
    }
    let base = parent.picture.clone().ok_or(LookError::NotKeepable {
        version: parent.id,
        state: parent.state,
    })?;
    Ok(VersionPlan {
        prompt: studio.edit_prompt(&changes),
        draft,
        changes,
        parent: Some(parent),
        base,
        things: vec![ThingReference {
            picture: base_pose.to_owned(),
            shows: studio.prompts.pose_shows.clone(),
        }],
    })
}

impl VersionPlan {
    fn request(&self, studio: &Studio, name: &str) -> Result<ImageRequest, LookError> {
        ImageRequest::new(
            &self.prompt,
            name,
            Some(Purpose::Figure),
            Some(studio.prompts.pose_aspect),
        )
        .and_then(|request| request.editing(&self.base))
        .and_then(|request| request.showing(&self.things))
        .map_err(invalid_request)
    }
}

/// The media library name of `item` of `hero`'s pack: the sheet is `ref-<hero id>`, as every
/// character's sheet is `ref-<id>`.
fn item_name(item: PackItem, hero: u64) -> String {
    match item {
        PackItem::Sheet => format!("ref-{hero}"),
        PackItem::Headshot => format!("head-{hero}"),
        PackItem::Portrait => format!("portrait-{hero}"),
        PackItem::Ready => format!("ready-{hero}"),
    }
}

fn version_name(seat: u64, version: u64) -> String {
    format!("look-{seat}-v{version}")
}

fn candidate_name(frame: BodyFrame, view: PoseView, number: u64) -> String {
    match view {
        PoseView::Base => format!("pose-{}-{number}", frame.key()),
        PoseView::Sheet => format!("pose-{}-sheet-{number}", frame.key()),
    }
}

fn parent_key(parent: Option<u64>) -> String {
    parent.map_or_else(|| "none".to_owned(), |parent| parent.to_string())
}

impl LookStudio {
    pub fn new(generator: Arc<ImageGenerator>, text: Arc<TextRuntime>) -> Self {
        Self {
            generator,
            text,
            records: tokio::sync::Mutex::new(BTreeMap::new()),
            poses: tokio::sync::Mutex::new(()),
            followers: Mutex::new(Vec::new()),
        }
    }

    /// Keeps a follower task until it ends, dropping those already ended.
    fn keep(&self, task: JoinHandle<()>) {
        let mut followers = self
            .followers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        followers.retain(|follower| !follower.is_finished());
        followers.push(task);
    }

    async fn studio(app: &AppState) -> Result<Studio, LookError> {
        let assets = assets(app);
        run_blocking(move || read_studio(&assets), LookError::Stopped).await
    }

    async fn poses_now(app: &AppState) -> Result<PosesView, LookError> {
        let assets = assets(app);
        run_blocking(move || read_poses(&assets), LookError::Stopped).await
    }

    /// The hero the player at `seat` plays.
    async fn seat_hero(app: &AppState, seat: u64) -> Result<SeatHero, LookError> {
        let mut open = app.seats().open().await?;
        let books = open.books();
        let record = books
            .record_at(seat)
            .ok_or(SeatError::UnknownSeat { seat })?;
        let hero = record.hero.ok_or(LookError::NoHero { seat })?;
        let found = books.hero(hero).ok_or(SeatError::UnknownHero { hero })?;
        Ok(SeatHero {
            hero,
            public: PublicHero::of(found),
        })
    }

    /// Runs `change` on `seat`'s record, read from its file on first use, and writes the file when
    /// the record changed. A change that fails leaves the record as it was.
    pub(super) async fn with_record<T>(
        &self,
        app: &AppState,
        seat: u64,
        change: impl FnOnce(&mut StudioRecord) -> Result<T, LookError>,
    ) -> Result<T, LookError> {
        let mut records = self.records.lock().await;
        let assets = assets(app);
        let current = match records.get(&seat) {
            Some(record) => record.clone(),
            None => {
                let reading = assets.clone();
                let mut record =
                    run_blocking(move || read_record(&reading, seat), LookError::Stopped).await?;
                if record.interrupted() {
                    app.record_line_and_echo(format!("LOOK failed seat={seat} cause=interrupted"));
                }
                record
            }
        };
        let mut changed = current.clone();
        let answer = change(&mut changed)?;
        if changed != current {
            let written = changed.clone();
            run_blocking(
                move || write_record(&assets, seat, &written),
                LookError::Stopped,
            )
            .await?;
        }
        records.insert(seat, changed);
        Ok(answer)
    }

    /// The studio of `seat`.
    pub(crate) async fn view(
        &self,
        app: &AppState,
        seat: u64,
    ) -> Result<LookStudioView, LookError> {
        let hero = Self::seat_hero_id(app, seat).await?;
        self.with_record(app, seat, |record| {
            Ok(match hero {
                Some(hero) if record.hero == Some(hero) => record.view.clone(),
                _ => LookStudioView::default(),
            })
        })
        .await
    }

    /// The hero the player at `seat` plays, if any.
    async fn seat_hero_id(app: &AppState, seat: u64) -> Result<Option<u64>, LookError> {
        let mut open = app.seats().open().await?;
        let record = open
            .books()
            .record_at(seat)
            .ok_or(SeatError::UnknownSeat { seat })?;
        Ok(record.hero)
    }

    /// The controls of the hero at `seat`.
    pub(crate) async fn controls(
        &self,
        app: &AppState,
        seat: u64,
    ) -> Result<LookControls, LookError> {
        let hero = Self::seat_hero(app, seat).await?;
        Self::studio(app)
            .await?
            .controls(&hero.public.species, hero.hero)
    }

    /// The player's `words` read by Jev, for the hero at `seat`: the controls they set and the
    /// details no control covers. One `LOOK details seat= chips= controls= dropped= category=`
    /// line, and one `LOOK details_dropped seat= control= option=` line for each control pair that
    /// is not the studio's.
    pub(crate) async fn details(
        &self,
        app: &AppState,
        seat: u64,
        words: &str,
    ) -> Result<LookDetails, LookError> {
        if words.trim().is_empty() {
            return Err(LookError::EmptyWords);
        }
        let hero = Self::seat_hero(app, seat).await?;
        let studio = Self::studio(app).await?;
        let context = studio.hero_context(&hero.public);
        let controls = studio.word_controls(&hero.public.species)?;
        let DetailsRead {
            values,
            details,
            dropped,
            category,
        } = read_details(&self.text, words, &context, &controls).await?;
        let read = details.clone();
        self.with_record(app, seat, |record| {
            record.for_hero(hero.hero);
            for detail in read {
                if !record.read_details.contains(&detail) {
                    record.read_details.push(detail);
                }
            }
            Ok(())
        })
        .await?;
        for (control, option) in &dropped {
            app.record_line(format!(
                "LOOK details_dropped seat={seat} control={control} option={option}"
            ));
        }
        app.record_line(format!(
            "LOOK details seat={seat} chips={} controls={} dropped={} category={}",
            details.len(),
            values.len(),
            dropped.len(),
            category.key()
        ));
        Ok(LookDetails { values, details })
    }

    /// Makes a new version of the look at `seat` from the phone's draft.
    pub(crate) async fn refine(
        self: &Arc<Self>,
        app: &Arc<AppState>,
        seat: u64,
        ask: LookRefineAsk,
        wait: bool,
    ) -> Result<LookVersion, LookError> {
        let hero = Self::seat_hero(app, seat).await?;
        let call = VersionCall {
            seat,
            kept: ask.kept,
            locking: false,
            wait,
        };
        self.new_version(app, &hero, &ask.draft, call).await
    }

    /// Plans, judges and starts one version.
    async fn new_version(
        self: &Arc<Self>,
        app: &Arc<AppState>,
        hero: &SeatHero,
        draft: &LookDraft,
        call: VersionCall,
    ) -> Result<LookVersion, LookError> {
        let VersionCall {
            seat,
            kept,
            locking,
            wait,
        } = call;
        let studio = Self::studio(app).await?;
        // A control the player left unset is rolled, and the version's draft shows what it was.
        let draft = studio.roll(
            &hero.public.species,
            draft,
            false,
            &mut SystemEntropy::new(),
        )?;
        let draft = studio.settle(&hero.public.species, &draft)?;
        let frame = studio.frame_of(&hero.public.species)?;
        let base_pose = Self::poses_now(app)
            .await?
            .frames
            .get(&frame)
            .and_then(|poses| poses.base.clone())
            .ok_or(LookError::PoseNotChosen { frame })?;
        let parent = self
            .with_record(app, seat, |record| {
                record.for_hero(hero.hero);
                check_refinable(record, kept, locking)?;
                check_details(&studio, record, &draft)?;
                parent_of(record, kept)
            })
            .await?;
        let plan = plan_version(&studio, &hero.public, draft, parent, &base_pose)?;
        // Checked once before Jev is asked, so a request the image service cannot take costs no
        // reading; the stand-in name is replaced by the version's below.
        plan.request(&studio, "look-check")?;
        let category =
            judge_look(&self.text, &studio.description(&hero.public, &plan.draft)?).await?;
        if let Some(cause) = held(category) {
            app.record_line(format!(
                "LOOK held seat={seat} category={} cause={}",
                category.key(),
                cause.key()
            ));
            return Err(LookError::Held { cause });
        }
        let pending = self
            .with_record(app, seat, |record| {
                record.for_hero(hero.hero);
                check_refinable(record, kept, locking)?;
                let id = record.next_version.max(1);
                record.next_version = id + 1;
                let version = LookVersion {
                    id,
                    parent: plan.parent.as_ref().map(|parent| parent.id),
                    draft: plan.draft.clone(),
                    changes: plan.changes.clone(),
                    picture: None,
                    state: VersionState::Pending,
                    asked_ms: now_ms(),
                    took_ms: None,
                };
                record.view.versions.push(version.clone());
                Ok(version)
            })
            .await?;
        let request = plan.request(&studio, &version_name(seat, pending.id))?;
        self.render(app, seat, pending, request, category, wait)
            .await
    }

    /// Starts the picture of `pending` and a follower that records its end.
    async fn render(
        self: &Arc<Self>,
        app: &Arc<AppState>,
        seat: u64,
        pending: LookVersion,
        request: ImageRequest,
        category: CreatorCategory,
        wait: bool,
    ) -> Result<LookVersion, LookError> {
        let started_at = tokio::time::Instant::now();
        let started = match self.generator.start(app, request).await {
            Ok(started) => started,
            Err(error) => {
                let failed = VersionState::Failed {
                    cause: failure_of(&error),
                };
                self.settle(app, seat, &pending, Err(failed), 0, category)
                    .await?;
                return Err(LookError::Generate { cause: error });
            }
        };
        let (answer, answered) = oneshot::channel();
        let studio = Arc::clone(self);
        let owner = Arc::clone(app);
        let version = pending.clone();
        let follower = tokio::spawn(async move {
            let end = picture_of(started.finished.await);
            let took_ms = df_types::saturating_millis(started_at.elapsed());
            let end = end.map_err(|cause| VersionState::Failed { cause });
            let settled = studio
                .settle(&owner, seat, &version, end, took_ms, category)
                .await;
            // A caller that stopped waiting only stopped waiting; the record holds the end.
            let _ = answer.send(settled);
        });
        self.keep(follower);
        if !wait {
            return Ok(pending);
        }
        answered.await.map_err(|_| LookError::Stopped)?
    }

    /// Records how `version` ended: ready, or stale when a newer refine began meanwhile, or
    /// failed. Writes its `LOOK` line.
    async fn settle(
        &self,
        app: &AppState,
        seat: u64,
        version: &LookVersion,
        end: Result<String, VersionState>,
        took_ms: u64,
        category: CreatorCategory,
    ) -> Result<LookVersion, LookError> {
        let id = version.id;
        let settled = self
            .with_record(app, seat, |record| arrive(record, id, end, took_ms))
            .await?;
        let line = match settled.state {
            VersionState::Ready => format!(
                concat!(
                    "LOOK refine seat={} version={} parent={} changes={} category={} ",
                    "ms={}"
                ),
                seat,
                id,
                parent_key(version.parent),
                version.changes.len(),
                category.key(),
                took_ms
            ),
            VersionState::Stale => format!("LOOK stale seat={seat} version={id} ms={took_ms}"),
            VersionState::Failed { cause } => format!(
                "LOOK failed seat={seat} version={id} stage=refine cause={} ms={took_ms}",
                cause.key()
            ),
            VersionState::Pending | VersionState::Discarded => return Ok(settled),
        };
        app.record_line_and_echo(line);
        Ok(settled)
    }

    /// Makes `version` the kept one: the player kept a new version, or went back to an earlier one.
    pub(crate) async fn keep_version(
        &self,
        app: &AppState,
        seat: u64,
        version: u64,
    ) -> Result<LookStudioView, LookError> {
        let (previous, view) = self
            .with_record(app, seat, |record| {
                if let Some(lock) = &record.view.lock {
                    return Err(match lock.state {
                        LockState::Locked { .. } => LookError::Locked { hero: lock.hero },
                        LockState::Portrait | LockState::Sheet { .. } => LookError::LockBusy,
                    });
                }
                let previous = keep_in(record, version)?;
                Ok((previous, record.view.clone()))
            })
            .await?;
        app.record_line(format!(
            "LOOK kept seat={seat} version={version} previous={}",
            parent_key(previous)
        ));
        Ok(view)
    }

    /// The player kept the old version over `version`.
    pub(crate) async fn discard_version(
        &self,
        app: &AppState,
        seat: u64,
        version: u64,
    ) -> Result<LookStudioView, LookError> {
        let view = self
            .with_record(app, seat, |record| {
                let found = version_mut(record, version)?;
                match found.state {
                    VersionState::Ready | VersionState::Discarded => {
                        found.state = VersionState::Discarded;
                    }
                    state => return Err(LookError::NotKeepable { version, state }),
                }
                Ok(record.view.clone())
            })
            .await?;
        app.record_line(format!("LOOK discarded seat={seat} version={version}"));
        Ok(view)
    }

    /// Start again: the next refine renders anew from the base pose. After a lock-in it also
    /// clears the versions, for a new hero; their pictures stay in the media library.
    pub(crate) async fn start_again(
        &self,
        app: &AppState,
        seat: u64,
    ) -> Result<LookStudioView, LookError> {
        let (cleared, view) = self
            .with_record(app, seat, |record| {
                let cleared = match record.view.lock.as_ref().map(|lock| &lock.state) {
                    Some(LockState::Locked { .. }) => {
                        record.view = LookStudioView::default();
                        true
                    }
                    Some(LockState::Portrait | LockState::Sheet { .. }) => {
                        return Err(LookError::LockBusy);
                    }
                    None => {
                        record.view.kept = None;
                        false
                    }
                };
                Ok((cleared, record.view.clone()))
            })
            .await?;
        app.record_line(format!(
            "LOOK again seat={seat} cleared={}",
            if cleared { "yes" } else { "no" }
        ));
        Ok(view)
    }

    /// Locks the look at `seat` in for the seat's hero: the kept version (or one rendered now from
    /// the draft, REF-059) becomes the portrait, and the reference sheet is made from it on the
    /// frame's three-view pose. With `wait`, answers when both pictures ended.
    pub(crate) async fn lock(
        self: &Arc<Self>,
        app: &Arc<AppState>,
        seat: u64,
        ask: LookLockAsk,
        wait: bool,
    ) -> Result<LookStudioView, LookError> {
        let hero = Self::seat_hero(app, seat).await?;
        let (kept, view) = self
            .with_record(app, seat, |record| {
                record.for_hero(hero.hero);
                if let Some(lock) = &record.view.lock {
                    return Err(match lock.state {
                        LockState::Locked { .. } => LookError::Locked { hero: lock.hero },
                        LockState::Portrait | LockState::Sheet { .. } => LookError::LockBusy,
                    });
                }
                let kept = record.view.kept.and_then(|id| {
                    let version = record.view.versions.iter().find(|known| known.id == id)?;
                    Some((version.picture.clone()?, version.draft.clone()))
                });
                let state = match &kept {
                    Some((portrait, _)) => LockState::Sheet {
                        portrait: Some(portrait.clone()),
                    },
                    None => LockState::Portrait,
                };
                record.view.lock = Some(LookLock {
                    hero: hero.hero,
                    state,
                });
                Ok((kept, record.view.clone()))
            })
            .await?;
        let studio = Arc::clone(self);
        let owner = Arc::clone(app);
        let task = tokio::spawn(async move {
            studio.lock_in(&owner, seat, &hero, &ask.draft, kept).await;
        });
        if !wait {
            self.keep(task);
            return Ok(view);
        }
        task.await.map_err(|_| LookError::Stopped)?;
        self.view(app, seat).await
    }

    /// The lock-in's work: the portrait (the kept version, or one rendered now, REF-059), then
    /// the hero's pack: the reference sheet before the lock-in ends, then the headshot, the
    /// half-body portrait and the ready pose. A picture that fails is logged and left out; the
    /// hero locks in all the same.
    async fn lock_in(
        self: &Arc<Self>,
        app: &Arc<AppState>,
        seat: u64,
        hero: &SeatHero,
        draft: &LookDraft,
        kept: Option<(String, LookDraft)>,
    ) {
        let (portrait, look) = match kept {
            Some((portrait, look)) => (Some(portrait), Some(look)),
            None => self.lock_portrait(app, seat, hero, draft).await,
        };
        let locked = async {
            let mut open = app.seats().open().await?;
            open.books().lock_hero(hero.hero, portrait.clone(), look)?;
            open.save(Book::Heroes).await
        }
        .await;
        if let Err(error) = &locked {
            app.record_line_and_echo(format!(
                "LOOK failed seat={seat} hero={} stage=library cause={}",
                hero.hero,
                error.key()
            ));
        }
        let state = LockState::Sheet {
            portrait: portrait.clone(),
        };
        self.set_lock(app, seat, hero.hero, state).await;
        // A failed portrait is made again first, from the look, as every other item follows it.
        let mut items = PackItem::ALL.to_vec();
        if portrait.is_none() {
            items.sort_by_key(|item| *item != PackItem::Portrait);
        }
        for item in items {
            let made = self.make_item(app, hero.hero, item).await;
            if item == PackItem::Sheet {
                let sheet = made;
                let state = LockState::Locked {
                    portrait: self.pack_portrait(app, hero.hero).await,
                    sheet: sheet.clone(),
                };
                self.set_lock(app, seat, hero.hero, state).await;
                app.record_line_and_echo(format!(
                    "LOOK locked seat={seat} hero={} sheet={}",
                    hero.hero,
                    if sheet.is_some() { "ok" } else { "failed" }
                ));
            }
        }
    }

    /// The approved portrait of `hero` as the library holds it now.
    async fn pack_portrait(&self, app: &AppState, hero: u64) -> Option<String> {
        let mut open = app.seats().open().await.ok()?;
        open.books().hero(hero)?.portrait.clone()
    }

    /// Claims `hero`'s pack items to make, marking them pending in one change of the library,
    /// then makes them on a task of its own: `item`, or every item `unmade` takes. Answers the
    /// items claimed; nothing is claimed twice, so nothing is paid for twice.
    pub(crate) async fn make_pack(
        self: &Arc<Self>,
        app: &Arc<AppState>,
        hero: u64,
        item: Option<PackItem>,
        unmade: fn(&PackStatus) -> bool,
    ) -> Result<Vec<PackItem>, LookError> {
        let items = {
            let mut open = app.seats().open().await?;
            let items = open.books().claim_pack(hero, item, unmade)?;
            if !items.is_empty() {
                open.save(Book::Heroes).await?;
            }
            items
        };
        let studio = Arc::clone(self);
        let owner = Arc::clone(app);
        let claimed = items.clone();
        let task = tokio::spawn(async move {
            for item in claimed {
                studio.make_item(&owner, hero, item).await;
            }
        });
        self.keep(task);
        Ok(items)
    }

    async fn pack_of(&self, app: &AppState, hero: u64) -> Result<HeroPack, LookError> {
        let mut open = app.seats().open().await?;
        let found = open
            .books()
            .hero(hero)
            .ok_or(SeatError::UnknownHero { hero })?;
        Ok(found.pack.clone())
    }

    /// Records `status` for `item` of `hero`'s pack; a library that will not take it is logged.
    async fn set_item(&self, app: &AppState, hero: u64, item: PackItem, status: PackStatus) {
        let written = async {
            let mut open = app.seats().open().await?;
            open.books().set_pack(hero, item, status)?;
            open.save(Book::Heroes).await
        }
        .await;
        if let Err(error) = written {
            app.record_line_and_echo(format!(
                "LOOK failed hero={hero} item={} stage=library cause={}",
                item.key(),
                error.key()
            ));
        }
    }

    /// Makes one pack item and records how it ended; its picture when it was made.
    async fn make_item(&self, app: &Arc<AppState>, hero: u64, item: PackItem) -> Option<String> {
        let started = tokio::time::Instant::now();
        let made = self.item_picture(app, hero, item).await;
        let ms = df_types::saturating_millis(started.elapsed());
        let (status, line) = match &made {
            Ok(picture) => (
                PackStatus::Ready {
                    picture: picture.clone(),
                },
                format!("LOOK pack hero={hero} item={} ok ms={ms}", item.key()),
            ),
            Err(cause) => (
                PackStatus::Failed { cause: *cause },
                format!(
                    "LOOK pack hero={hero} item={} failed cause={} ms={ms}",
                    item.key(),
                    cause.key()
                ),
            ),
        };
        self.set_item(app, hero, item, status).await;
        app.record_line_and_echo(line);
        made.ok()
    }

    /// The kept look of the hero `seat` plays, for a runner's scene prompts: the hero, its details
    /// (the chips) and its whole look in the studio's words; none of them for a hero locked with
    /// no design.
    pub(crate) async fn hero_look(app: &AppState, seat: u64) -> Result<HeroLook, LookError> {
        let record = {
            let mut open = app.seats().open().await?;
            let books = open.books();
            let hero = books
                .records()
                .iter()
                .find(|record| record.seat == seat)
                .ok_or(SeatError::UnknownSeat { seat })?
                .hero
                .ok_or(LookError::NoHero { seat })?;
            books
                .hero(hero)
                .cloned()
                .ok_or(SeatError::UnknownHero { hero })?
        };
        let studio = Self::studio(app).await?;
        let details = record
            .look
            .as_ref()
            .and_then(|draft| studio.details_text(draft));
        Ok(HeroLook {
            hero: record.id,
            details,
            look: Self::locked_look(app, &record).await?,
        })
    }

    /// The request for `item` of `hero`: the sheet painted on the frame's three-view template
    /// after the portrait; every other item an edit of the portrait following the sheet's view
    /// that suits it.
    async fn item_request(
        &self,
        app: &AppState,
        record: &HeroRecord,
        item: PackItem,
    ) -> Result<ImageRequest, PictureFailure> {
        let studio = Self::studio(app)
            .await
            .map_err(|_| PictureFailure::NoPrompt)?;
        let portrait = match (&record.portrait, item) {
            (Some(portrait), _) => portrait.clone(),
            (None, PackItem::Portrait) => return self.fresh_portrait(app, &studio, record).await,
            (None, _) => return Err(PictureFailure::NoPortrait),
        };
        let prompts = &studio.prompts;
        let name = item_name(item, record.id);
        let (prompt, aspect, view) = match item {
            PackItem::Sheet => {
                let frame = studio
                    .frame_of(&record.species)
                    .map_err(|_| PictureFailure::NoTemplate)?;
                let template = Self::poses_now(app)
                    .await
                    .ok()
                    .and_then(|poses| {
                        poses
                            .frames
                            .get(&frame)
                            .and_then(|poses| poses.sheet.clone())
                    })
                    .ok_or(PictureFailure::NoTemplate)?;
                // The template draws a plain figure: the kept look's details (an aura, a scar, its
                // jewellery) are asked for in every view, as the portrait shows them.
                let details = record
                    .look
                    .as_ref()
                    .and_then(|draft| studio.details_text(draft));
                let prompt = match details {
                    Some(details) => {
                        format!("{} {SHEET_DETAILS_LEAD}{details}.", prompts.lock_sheet)
                    }
                    None => prompts.lock_sheet.clone(),
                };
                return ImageRequest::new(
                    &prompt,
                    &name,
                    Some(Purpose::Figure),
                    Some(prompts.sheet_aspect),
                )
                .and_then(|request| request.editing(&template))
                .and_then(|request| request.following(&[portrait]))
                .map_err(|_| PictureFailure::Backend);
            }
            PackItem::Headshot => (
                prompts.headshot.clone(),
                prompts.headshot_aspect,
                SheetPick::Front,
            ),
            PackItem::Portrait => (
                prompts.half_portrait.clone(),
                prompts.half_portrait_aspect,
                SheetPick::Front,
            ),
            PackItem::Ready => (
                studio
                    .ready_prompt(&record.class)
                    .map_err(|_| PictureFailure::NoPrompt)?,
                prompts.pose_aspect,
                SheetPick::FullLength,
            ),
        };
        let sheet: Vec<String> = record
            .pack
            .sheet
            .picture()
            .map(str::to_owned)
            .into_iter()
            .collect();
        ImageRequest::new(&prompt, &name, Some(Purpose::Figure), Some(aspect))
            .and_then(|request| request.editing(&portrait))
            .and_then(|request| request.following(&sheet))
            .map(|request| request.showing_sheets(Some(view)))
            .map_err(|_| PictureFailure::Backend)
    }

    /// A portrait of a hero locked in without one, painted anew on the frame's base pose from the
    /// look it was locked in with, after Jev's judgement of it (REF-039).
    async fn fresh_portrait(
        &self,
        app: &AppState,
        studio: &Studio,
        record: &HeroRecord,
    ) -> Result<ImageRequest, PictureFailure> {
        let look = record.look.as_ref().ok_or(PictureFailure::NoPortrait)?;
        let public = PublicHero::of(record);
        let description = studio
            .description(&public, look)
            .map_err(|_| PictureFailure::NoPrompt)?;
        let category = judge_look(&self.text, &description)
            .await
            .map_err(|_| PictureFailure::Backend)?;
        if held(category).is_some() {
            return Err(PictureFailure::Held);
        }
        let frame = studio
            .frame_of(&record.species)
            .map_err(|_| PictureFailure::NoTemplate)?;
        let base = Self::poses_now(app)
            .await
            .ok()
            .and_then(|poses| {
                poses
                    .frames
                    .get(&frame)
                    .and_then(|poses| poses.base.clone())
            })
            .ok_or(PictureFailure::NoTemplate)?;
        ImageRequest::new(
            &studio.first_prompt(&description),
            &item_name(PackItem::Portrait, record.id),
            Some(Purpose::Figure),
            Some(studio.prompts.pose_aspect),
        )
        .and_then(|request| request.editing(&base))
        .map_err(|_| PictureFailure::Backend)
    }

    /// The picture of `item` for `hero`, made and stored.
    async fn item_picture(
        &self,
        app: &Arc<AppState>,
        hero: u64,
        item: PackItem,
    ) -> Result<String, PictureFailure> {
        let record = {
            let mut open = app
                .seats()
                .open()
                .await
                .map_err(|_| PictureFailure::Library)?;
            open.books()
                .hero(hero)
                .cloned()
                .ok_or(PictureFailure::Library)?
        };
        let request = self.item_request(app, &record, item).await?;
        let started = self
            .generator
            .start(app, request)
            .await
            .map_err(|error| failure_of(&error))?;
        let picture = picture_of(started.finished.await)?;
        if item == PackItem::Portrait && record.portrait.is_none() {
            self.attach(app, hero, Some(picture.clone()))
                .await
                .map_err(|_| PictureFailure::Library)?;
        }
        Ok(picture)
    }

    /// The reference sheet of the character `id` for a picture: a seated player's hero whose
    /// persona `id` is, by its pack; otherwise `ref-<id>` when the media library holds it.
    pub(crate) async fn reference_of(
        app: &AppState,
        id: &str,
    ) -> Result<Option<String>, LookError> {
        {
            let mut open = app.seats().open().await?;
            let books = open.books();
            // A seat that plays this character answers for it: its hero's sheet, or none yet,
            // never the stock character's.
            if let Some(record) = books
                .records()
                .iter()
                .find(|record| record.persona.as_deref() == Some(id))
            {
                return Ok(record
                    .hero
                    .and_then(|hero| books.hero(hero))
                    .and_then(|hero| hero.pack.sheet.picture().map(str::to_owned)));
            }
        }
        let named = reference_sheet_name(id);
        let media = media_directory(app.playground().files());
        let held = {
            let named = named.clone();
            run_blocking(move || Ok(media.join(named).is_file()), LookError::Stopped).await?
        };
        Ok(held.then_some(named))
    }

    /// The whole public look of `hero`'s locked design in the studio's words (species, class, each
    /// control, each detail); none for a hero locked with no design.
    pub(crate) async fn locked_look(
        app: &AppState,
        hero: &HeroRecord,
    ) -> Result<Option<String>, LookError> {
        let Some(draft) = &hero.look else {
            return Ok(None);
        };
        let studio = Self::studio(app).await?;
        studio.description(&PublicHero::of(hero), draft).map(Some)
    }

    /// The one portrait of a design with no kept version, kept once made; none when it failed.
    async fn lock_portrait(
        self: &Arc<Self>,
        app: &Arc<AppState>,
        seat: u64,
        hero: &SeatHero,
        draft: &LookDraft,
    ) -> (Option<String>, Option<LookDraft>) {
        let call = VersionCall {
            seat,
            kept: None,
            locking: true,
            wait: true,
        };
        match self.new_version(app, hero, draft, call).await {
            Ok(version) if version.state == VersionState::Ready => {
                let kept = self
                    .with_record(app, seat, |record| {
                        record.view.kept = Some(version.id);
                        Ok(())
                    })
                    .await;
                if let Err(error) = kept {
                    app.record_line_and_echo(format!(
                        "LOOK failed seat={seat} stage=portrait cause={}",
                        error.key()
                    ));
                }
                (version.picture, Some(version.draft))
            }
            // The version's draft was read and checked: a portrait made later follows it.
            Ok(version) => (None, Some(version.draft)),
            Err(error) => {
                app.record_line_and_echo(format!(
                    "LOOK failed seat={seat} stage=portrait cause={}",
                    error.key()
                ));
                (None, None)
            }
        }
    }

    async fn set_lock(&self, app: &AppState, seat: u64, hero: u64, state: LockState) {
        let set = self
            .with_record(app, seat, |record| {
                // The seat may have changed hands meanwhile: only its hero's studio is written.
                if record.hero == Some(hero) {
                    record.view.lock = Some(LookLock { hero, state });
                }
                Ok(())
            })
            .await;
        if let Err(error) = set {
            app.record_line_and_echo(format!(
                "LOOK failed seat={seat} hero={hero} stage=record cause={}",
                error.key()
            ));
        }
    }

    /// Points the hero in the hero library at its locked-in portrait.
    async fn attach(
        &self,
        app: &AppState,
        hero: u64,
        portrait: Option<String>,
    ) -> Result<(), LookError> {
        let mut open = app.seats().open().await?;
        open.books().set_portrait(hero, portrait)?;
        open.save(Book::Heroes).await?;
        Ok(())
    }

    /// Asks the image service for `count` candidates of a frame's base pose or sheet; the owner
    /// picks one (`choose_pose`).
    pub(crate) async fn pose_candidates(
        &self,
        app: &Arc<AppState>,
        ask: PoseCandidatesAsk,
    ) -> Result<PoseCandidates, LookError> {
        if ask.count == 0 {
            return Err(LookError::InvalidCount);
        }
        let studio = Self::studio(app).await?;
        let (prompt, aspect, references) = match ask.view {
            PoseView::Base => (
                studio.pose_prompt(ask.frame),
                studio.prompts.pose_aspect,
                Vec::new(),
            ),
            PoseView::Sheet => {
                let base = Self::poses_now(app)
                    .await?
                    .frames
                    .get(&ask.frame)
                    .and_then(|poses| poses.base.clone())
                    .ok_or(LookError::PoseNotChosen { frame: ask.frame })?;
                (
                    studio.prompts.sheet.clone(),
                    studio.prompts.sheet_aspect,
                    vec![base],
                )
            }
        };
        let first = self
            .change_poses(app, move |poses| {
                let frame = poses.frames.entry(ask.frame).or_default();
                let next = match ask.view {
                    PoseView::Base => &mut frame.next_base,
                    PoseView::Sheet => &mut frame.next_sheet,
                };
                let first = (*next).max(1);
                *next = first + u64::from(ask.count);
                Ok(first)
            })
            .await?;
        let mut candidates = PoseCandidates {
            names: Vec::new(),
            jobs: Vec::new(),
        };
        for number in first..first + u64::from(ask.count) {
            let name = candidate_name(ask.frame, ask.view, number);
            let request = ImageRequest::new(&prompt, &name, Some(Purpose::Figure), Some(aspect))
                .and_then(|request| request.following(&references))
                .map_err(invalid_request)?;
            let started = self
                .generator
                .start(app, request)
                .await
                .map_err(|cause| LookError::Generate { cause })?;
            candidates.jobs.push(started.view.job);
            candidates.names.push(name);
        }
        app.record_line(format!(
            "LOOK pose_candidates frame={} view={} n={} first={first}",
            ask.frame.key(),
            ask.view.key(),
            candidates.jobs.len()
        ));
        Ok(candidates)
    }

    /// Makes `picture` the frame's base pose (`pose-<frame>.<ext>`) or sheet
    /// (`pose-<frame>-sheet.<ext>`), copied in the media library, and records the choice.
    pub(crate) async fn choose_pose(
        &self,
        app: &AppState,
        ask: PoseChoose,
    ) -> Result<PosesView, LookError> {
        let invalid = || LookError::InvalidPicture {
            picture: ask.picture.clone(),
        };
        crate::locate::check_picture_name(&ask.picture).map_err(|_| invalid())?;
        let (_, extension) = ask.picture.rsplit_once('.').ok_or_else(invalid)?;
        let chosen = match ask.view {
            PoseView::Base => format!("pose-{}.{extension}", ask.frame.key()),
            PoseView::Sheet => format!("pose-{}-sheet.{extension}", ask.frame.key()),
        };
        if chosen != ask.picture {
            let media = media_directory(app.playground().files());
            let reading = media.clone();
            let from = ask.picture.clone();
            let bytes =
                run_blocking(move || read_picture(&reading, &from), LookError::Stopped).await?;
            store_bytes(media, &chosen, &bytes)
                .await
                .map_err(|error| LookError::StoreFailed {
                    detail: error.to_string(),
                })?;
        }
        let (picture, recorded) = (ask.picture.clone(), chosen.clone());
        let (frame_key, view) = (ask.frame, ask.view);
        let poses = self
            .change_poses(app, move |poses| {
                let frame: &mut FramePoses = poses.frames.entry(frame_key).or_default();
                match view {
                    PoseView::Base => {
                        frame.base = Some(recorded);
                        frame.base_from = Some(picture);
                    }
                    PoseView::Sheet => {
                        frame.sheet = Some(recorded);
                        frame.sheet_from = Some(picture);
                    }
                }
                Ok(poses.clone())
            })
            .await?;
        app.record_line(format!(
            "LOOK pose_chosen frame={} view={} picture={chosen} from={}",
            ask.frame.key(),
            ask.view.key(),
            ask.picture
        ));
        Ok(poses)
    }

    /// The frames' chosen poses.
    pub(crate) async fn poses(&self, app: &AppState) -> Result<PosesView, LookError> {
        Self::poses_now(app).await
    }

    /// Runs `change` on the poses file under its lock and writes it.
    async fn change_poses<T: Send + 'static>(
        &self,
        app: &AppState,
        change: impl FnOnce(&mut PosesView) -> Result<T, LookError> + Send + 'static,
    ) -> Result<T, LookError> {
        let _held = self.poses.lock().await;
        let assets = assets(app);
        run_blocking(
            move || {
                let mut poses = read_poses(&assets)?;
                let answer = change(&mut poses)?;
                write_poses(&assets, &poses)?;
                Ok(answer)
            },
            LookError::Stopped,
        )
        .await
    }

    /// A random look for the hero at `seat`: every control and one or two common details, from a
    /// logged seed. Nothing is spent; the phone shows it as the draft to adjust.
    pub(crate) async fn roll(&self, app: &AppState, seat: u64) -> Result<LookDraft, LookError> {
        let hero = Self::seat_hero(app, seat).await?;
        let studio = Self::studio(app).await?;
        let seed = SystemEntropy::new().next_u64();
        let rolled = studio.roll(
            &hero.public.species,
            &LookDraft::default(),
            true,
            &mut SeededEntropy::new(seed),
        )?;
        let draft = studio.settle(&hero.public.species, &rolled)?;
        app.record_line(format!(
            "LOOK rolled seat={seat} controls={} details={} seed={seed}",
            draft.values.len(),
            draft.details.len()
        ));
        Ok(draft)
    }

    /// Makes the pack of the hero at `seat` again: `ask.item`, or every item not made. Answers
    /// the pack as it stands now; the pictures follow on their own task.
    pub(crate) async fn remake_pack(
        self: &Arc<Self>,
        app: &Arc<AppState>,
        seat: u64,
        ask: LookPackAsk,
    ) -> Result<HeroPack, LookError> {
        let hero = Self::seat_hero(app, seat).await?;
        self.make_pack(app, hero.hero, ask.item, |status| {
            matches!(status, PackStatus::Missing | PackStatus::Failed { .. })
        })
        .await?;
        app.record_line(format!(
            "LOOK pack_asked seat={seat} hero={} item={}",
            hero.hero,
            ask.item.map_or("missing", PackItem::key)
        ));
        self.pack_of(app, hero.hero).await
    }

    /// What a new hero may be started as.
    pub(crate) async fn start_options(
        &self,
        app: &AppState,
    ) -> Result<LookStartOptions, LookError> {
        let studio = Self::studio(app).await?;
        let pictures = studio.example_pictures();
        let media = media_directory(app.playground().files());
        let held = run_blocking(
            move || {
                Ok(pictures
                    .into_iter()
                    .filter(|picture| media.join(picture).is_file())
                    .collect::<Vec<_>>())
            },
            LookError::Stopped,
        )
        .await?;
        Ok(studio.start_options(|picture| held.iter().any(|known| known == picture)))
    }

    /// A new hero for the player at `seat`, played at the seat, with its look studio afresh (the
    /// version numbers go on, so no picture is overwritten). Its build is made later (P2).
    pub(crate) async fn start_hero(
        &self,
        app: &AppState,
        seat: u64,
        ask: LookStartAsk,
    ) -> Result<LookStudioView, LookError> {
        let name = ask.name.trim();
        if !valid_character_name(name) {
            return Err(SeatError::InvalidName.into());
        }
        Self::studio(app)
            .await?
            .check_start(&ask.species, &ask.class)?;
        self.with_record(app, seat, |record| match &record.view.lock {
            Some(LookLock {
                state: LockState::Portrait | LockState::Sheet { .. },
                ..
            }) => Err(LookError::LockBusy),
            _ => Ok(()),
        })
        .await?;
        let hero = {
            let mut open = app.seats().open().await?;
            let key = open
                .books()
                .record_at(seat)
                .ok_or(SeatError::UnknownSeat { seat })?
                .key
                .clone();
            let card = open
                .books()
                .add_hero(seat, name, &ask.species, &ask.class)?;
            open.books().choose_hero(&key, card.id)?;
            open.save(Book::Heroes).await?;
            open.save(Book::Seats).await?;
            card.id
        };
        let view = self
            .with_record(app, seat, |record| {
                record.view = LookStudioView::default();
                record.read_details.clear();
                record.hero = Some(hero);
                Ok(record.view.clone())
            })
            .await?;
        app.record_line(format!("LOOK start seat={seat} hero={hero}"));
        Ok(view)
    }

    /// Makes `bind.hero`, one of the seat player's own heroes, the hero they play: the dev command
    /// line's way to what the phone's hero choice does.
    pub(crate) async fn bind_hero(app: &AppState, bind: HeroBind) -> Result<HeroBind, LookError> {
        let mut open = app.seats().open().await?;
        let key = open
            .books()
            .record_at(bind.seat)
            .ok_or(SeatError::UnknownSeat { seat: bind.seat })?
            .key
            .clone();
        open.books().choose_hero(&key, bind.hero)?;
        open.save(Book::Seats).await?;
        app.record_line(format!("LOOK hero seat={} hero={}", bind.seat, bind.hero));
        Ok(bind)
    }
}
