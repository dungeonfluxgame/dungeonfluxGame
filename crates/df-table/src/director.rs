//! The Director's camera stage (O.14, O.16 G1): where the shared screen's camera goes, and when.
//!
//! The Director is a pure state machine. A caller reports what happened as `Request`s (a speaker,
//! a hit, a seat's "look at me"), and each host tick calls `tick` with the time and the shared
//! screen as it stands (`StageView`); the answer is a list of `StageDirection`s, each a move with
//! a serial and a reason. The host applies a direction with the same checked numbers the dev camera
//! verbs use (a zoom of 100 to 10,000 percent held to the plate's ceiling, a time of 0 to a day), so
//! a verb and the engine take one code path. It reads no clock: every call carries `now_ms`, so a
//! test moves time by passing a number and never sleeps. It makes no sound, draws nothing and
//! never gates input: a direction is data.
//!
//! The rules it keeps are O.14's camera rules:
//!
//! 1. Targets come from the audience's view. A hidden target is refused, and a focus whose target
//!    becomes hidden is pulled back in the same tick, ignoring spacing, holds and cards.
//! 2. Framing and legibility belong to the page; a direction names an element, a zoom and a time.
//! 3. A zoom is held to the plate's ceiling and logged when cut (`CAMERA clamped`), never refused.
//! 4. Moves are numbered; a scene change forgets the camera.
//! 5. Pace: at most one new move in `spacing_ms` (800) unless the request outranks the focus now
//!    held; a burst in one tick is one move (the highest reason, among equals the latest); a burst
//!    for one result that names several heroes, in one tick or across several, leaves the camera
//!    wide, and that pull-back keeps the spacing and the card like any other move; the camera never
//!    moves during a result card or a reveal beat unless the move belongs to it.
//! 6. Always returns: a focus ends when its reason ends or `focus_hold_s` after it starts, a stuck
//!    one at a large safety limit; a scene or combat start is wide; a move the host could not put
//!    on the screen is told back (`apply_failed`) and the pull-back is made again.
//! 8. Reduced effects: every move is a cut.
//! 9. Logged: every move, return and decline is an `Entry` (`director_log`), and an entry names
//!    its element to the shared audience only when the table could see it.
//!
//! The camera is one shared value for the table, so the Director holds one focus at a time.

mod decide;
mod report;
mod request;
mod settings;

use std::collections::VecDeque;

use crate::director_error::DirectorError;
use crate::director_log::{Cause, DropReason, Entry, Outcome};
use crate::director_reason::{Ends, MAXIMUM_CAMERA_MS, Reason, WideReason};
use crate::director_view::{BeatKind, PlateView, StageView};
use crate::element::ElementId;

pub use self::request::{Receipt, Request, StageDirection};
pub use self::settings::{
    DEFAULT_FOCUS_HOLD_MS, DEFAULT_MAXIMUM_TARGETS, DEFAULT_MAXIMUM_WAITING,
    DEFAULT_MAXIMUM_WAITING_TARGETS, DEFAULT_SPACING_MS, DirectorSettings, UNENDED_FOCUS_LIMIT_MS,
};

#[derive(Debug, Clone)]
struct Waiting {
    request: Request,
    at_ms: u64,
    order: u64,
}

#[derive(Debug, Clone)]
struct Active {
    element: ElementId,
    reason: Reason,
    serial: u64,
    /// The zoom that was asked, before the plate's ceiling cut it; a new plate may bear more.
    requested_zoom_percent: u16,
    zoom_percent: u16,
    since_ms: u64,
    deadline_ms: u64,
    ends: Ends,
    ended: bool,
    generation: Option<u64>,
    /// The element the camera was believed to hold before this move, so a move that fails to reach
    /// the screen can be undone by a pull-back.
    replaced: Option<ElementId>,
}

/// A pull-back the host could not apply, made again on the next tick.
#[derive(Debug, Clone, Copy)]
struct OwedWide {
    reason: WideReason,
    over_ms: u32,
    driven: bool,
}

/// The last result that named a hero, so a later request of the same result is part of its area.
#[derive(Debug, Clone)]
struct ResultMemory {
    result: u64,
    hero: ElementId,
    several: bool,
}

/// The camera's decision maker. See the module docs.
#[derive(Debug, Clone)]
pub struct Director {
    settings: DirectorSettings,
    now_ms: u64,
    serial: u64,
    sequence: u64,
    last_move_ms: Option<u64>,
    active: Option<Active>,
    pending: Option<Waiting>,
    inbox: Vec<Waiting>,
    /// How many element names the requests in `inbox` hold in all.
    inbox_targets: usize,
    owed_wide: Option<OwedWide>,
    last_result: Option<ResultMemory>,
    paused: bool,
    auto_camera: bool,
    log: VecDeque<Entry>,
    unread: usize,
    queued: u64,
    applied: u64,
    dropped: u64,
}

/// Whether the whole table may see `element` on `view`.
fn is_visible(view: &StageView, element: &ElementId) -> bool {
    view.target(element)
        .is_some_and(|target| target.focusable())
}

impl Director {
    pub fn new(settings: DirectorSettings) -> Result<Self, DirectorError> {
        settings.check()?;
        Ok(Self {
            settings,
            now_ms: 0,
            serial: 0,
            sequence: 0,
            last_move_ms: None,
            active: None,
            pending: None,
            inbox: Vec::new(),
            inbox_targets: 0,
            owed_wide: None,
            last_result: None,
            paused: false,
            auto_camera: true,
            log: VecDeque::new(),
            unread: 0,
            queued: 0,
            applied: 0,
            dropped: 0,
        })
    }

    pub fn settings(&self) -> &DirectorSettings {
        &self.settings
    }

    pub fn serial(&self) -> u64 {
        self.serial
    }

    pub fn now_ms(&self) -> u64 {
        self.now_ms
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn auto_camera(&self) -> bool {
        self.auto_camera
    }

    /// Whether a tick has anything to decide: a focus held, a request waiting (decided or not
    /// yet) or a pull-back to make again.
    pub fn has_work(&self) -> bool {
        self.active.is_some()
            || self.pending.is_some()
            || !self.inbox.is_empty()
            || self.owed_wide.is_some()
    }

    /// Holds every automatic direction (not a driven one) while a person drives by hand.
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    /// Turns the automatic camera on or off; a driven direction still works.
    pub fn set_auto_camera(&mut self, on: bool) {
        self.auto_camera = on;
    }

    /// Whether the focus now held is on `element`.
    pub fn is_focused_on(&self, element: &ElementId) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| &active.element == element)
    }

    fn advance(&mut self, now_ms: u64) -> Result<(), DirectorError> {
        if now_ms < self.now_ms {
            return Err(DirectorError::ClockWentBack {
                now_ms,
                last_ms: self.now_ms,
            });
        }
        self.now_ms = now_ms;
        Ok(())
    }

    fn record(
        &mut self,
        cause: Cause,
        element: Option<ElementId>,
        element_visible: bool,
        outcome: Outcome,
        driven: bool,
    ) {
        self.sequence = self.sequence.saturating_add(1);
        match outcome {
            Outcome::Queued => self.queued = self.queued.saturating_add(1),
            Outcome::Applied { .. } => self.applied = self.applied.saturating_add(1),
            Outcome::Dropped { .. } => self.dropped = self.dropped.saturating_add(1),
        }
        self.log.push_back(Entry {
            sequence: self.sequence,
            at_ms: self.now_ms,
            cause,
            element,
            element_visible,
            outcome,
            driven,
        });
        self.unread = self.unread.saturating_add(1);
        while self.log.len() > self.settings.maximum_log_entries {
            self.log.pop_front();
        }
        self.unread = self.unread.min(self.log.len());
    }

    /// Logs a request set aside. `view` is the shared screen when the caller has it: without one
    /// the element cannot be said to be visible, so the shared audience is not told its name.
    fn drop_request(&mut self, request: &Request, why: DropReason, view: Option<&StageView>) {
        let element = request.element();
        let visible = view
            .zip(element.as_ref())
            .is_some_and(|(view, element)| is_visible(view, element));
        self.record(
            Cause::Focus(request.reason),
            element,
            visible,
            Outcome::Dropped { why },
            request.driven,
        );
    }

    /// The entries recorded since the last call, oldest first: the host writes their lines.
    pub fn take_unread(&mut self) -> Vec<Entry> {
        let skip = self.log.len() - self.unread.min(self.log.len());
        self.unread = 0;
        self.log.iter().skip(skip).cloned().collect()
    }

    /// Why `request` cannot be shown on `view`, if it cannot.
    fn refusal(request: &Request, view: &StageView) -> Option<DropReason> {
        if view.plate == PlateView::Card {
            return Some(DropReason::NoTarget);
        }
        for element in &request.targets {
            match view.target(element) {
                None => return Some(DropReason::UnknownTarget),
                Some(target) if !target.focusable() => return Some(DropReason::TargetHidden),
                Some(target) if !target.anchored => return Some(DropReason::NoAnchor),
                Some(_) => {}
            }
        }
        if request.reason == Reason::ActiveCreature && view.collapsed_beat {
            return Some(DropReason::CollapsedBeat);
        }
        None
    }

    fn held_back(&self, request: &Request) -> Option<DropReason> {
        if request.driven {
            return None;
        }
        if self.paused {
            Some(DropReason::Paused)
        } else if !self.auto_camera {
            Some(DropReason::AutoOff)
        } else {
            None
        }
    }

    fn waiting_targets(&self) -> usize {
        let pending = self
            .pending
            .as_ref()
            .map_or(0, |waiting| waiting.request.targets.len());
        self.inbox_targets.saturating_add(pending)
    }

    /// Takes one request. A request the table's rules set aside is answered and logged at once; one
    /// that may be shown waits for the next `tick`, which decides it against the others.
    pub fn request(
        &mut self,
        now_ms: u64,
        view: &StageView,
        request: Request,
    ) -> Result<Receipt, DirectorError> {
        self.advance(now_ms)?;
        request.check(self.settings.maximum_targets)?;
        let why = self
            .held_back(&request)
            .or_else(|| Self::refusal(&request, view))
            .or_else(|| {
                self.active
                    .as_ref()
                    .filter(|active| request.reason.rank() < active.reason.rank())
                    .map(|_| DropReason::LowerPriority)
            });
        if let Some(why) = why {
            self.drop_request(&request, why, Some(view));
            return Ok(Receipt::Dropped(why));
        }
        if self.inbox.len() >= self.settings.maximum_waiting {
            return Err(DirectorError::TooManyWaiting {
                limit: self.settings.maximum_waiting,
            });
        }
        let names = self.waiting_targets().saturating_add(request.targets.len());
        if names > self.settings.maximum_waiting_targets {
            return Err(DirectorError::TooManyWaitingTargets {
                limit: self.settings.maximum_waiting_targets,
            });
        }
        let element = request.element();
        let visible = element
            .as_ref()
            .is_some_and(|element| is_visible(view, element));
        self.record(
            Cause::Focus(request.reason),
            element,
            visible,
            Outcome::Queued,
            request.driven,
        );
        self.inbox_targets = self.inbox_targets.saturating_add(request.targets.len());
        self.inbox.push(Waiting {
            request,
            at_ms: now_ms,
            order: self.sequence,
        });
        Ok(Receipt::Queued)
    }

    /// The engine says the reason of `element`'s focus ended (the line's dwell, the turn, the
    /// result card). That focus returns on the next tick if it ends with its reason, and a waiting
    /// direction of that reason for that element, decided or not, is void. A focus or a request of
    /// the same reason for another element (the next line) is not touched.
    pub fn end(
        &mut self,
        now_ms: u64,
        reason: Reason,
        element: &ElementId,
    ) -> Result<(), DirectorError> {
        self.advance(now_ms)?;
        if let Some(active) = &mut self.active
            && active.reason == reason
            && &active.element == element
            && active.ends == Ends::WhenReasonEnds
            && !active.ended
        {
            active.ended = true;
            active.deadline_ms = now_ms;
        }
        let names_it = |waiting: &Waiting| {
            waiting.request.reason == reason && waiting.request.targets.contains(element)
        };
        let mut voided: Vec<Waiting> = self
            .pending
            .take_if(|waiting| names_it(waiting))
            .into_iter()
            .collect();
        voided.extend(self.inbox.extract_if(.., |waiting| names_it(waiting)));
        self.inbox_targets = self
            .inbox
            .iter()
            .map(|waiting| waiting.request.targets.len())
            .sum();
        for waiting in voided {
            self.drop_request(&waiting.request, DropReason::ReasonEnded, None);
        }
        Ok(())
    }

    /// The host could not put `direction` on the screen. A focus that failed is forgotten, and when
    /// it had replaced another the camera may still hold that one, so the camera is pulled back; a
    /// pull-back that failed is made again on the next tick. A direction that a later move already
    /// replaced is left alone.
    pub fn apply_failed(&mut self, direction: &StageDirection) {
        match direction {
            StageDirection::Focus { serial, .. } if *serial == self.serial => {
                let failed = self.active.take_if(|active| active.serial == *serial);
                if failed.is_some_and(|active| active.replaced.is_some()) {
                    self.owed_wide = Some(OwedWide {
                        reason: WideReason::ApplyFailed,
                        over_ms: self.settings.wide_ms,
                        driven: false,
                    });
                }
            }
            StageDirection::Wide {
                reason,
                over_ms,
                serial,
                driven,
            } if *serial == self.serial => {
                self.owed_wide = Some(OwedWide {
                    reason: *reason,
                    over_ms: *over_ms,
                    driven: *driven,
                });
            }
            StageDirection::Focus { .. } | StageDirection::Wide { .. } => {}
        }
    }

    /// Something other than the Director moved the camera (a scene change, a free camera verb):
    /// the camera now looks at `target`, or at the whole stage. A focus the camera no longer holds
    /// is forgotten without a move, so the next request for it is not swallowed as already
    /// focused. Returns whether a focus was forgotten.
    pub fn camera_moved_elsewhere(
        &mut self,
        now_ms: u64,
        target: Option<&ElementId>,
    ) -> Result<bool, DirectorError> {
        self.advance(now_ms)?;
        self.owed_wide = None;
        let forgotten = self
            .active
            .take_if(|active| Some(&active.element) != target)
            .is_some();
        Ok(forgotten)
    }

    /// Makes the pull-back. `view` is the shared screen when the caller has it, for whether the
    /// element the camera leaves may be named to the shared audience.
    fn make_wide(
        &mut self,
        why: WideReason,
        over_ms: u32,
        driven: bool,
        view: Option<&StageView>,
    ) -> StageDirection {
        let active = self.active.take();
        let visible = active
            .as_ref()
            .zip(view)
            .is_some_and(|(active, view)| is_visible(view, &active.element));
        let element = active.map(|active| active.element);
        self.owed_wide = None;
        self.serial = self.serial.saturating_add(1);
        self.last_move_ms = Some(self.now_ms);
        self.record(
            Cause::Wide(why),
            element,
            visible,
            Outcome::Applied {
                serial: self.serial,
                zoom_percent: 100,
                over_ms,
                clamped_from: None,
            },
            driven,
        );
        StageDirection::Wide {
            reason: why,
            over_ms,
            serial: self.serial,
            driven,
        }
    }

    fn wide_time(&self, view: &StageView) -> u32 {
        if view.reduced_effects {
            0
        } else {
            self.settings.wide_ms
        }
    }

    fn void_waiting(&mut self, why: DropReason) {
        let waiting: Vec<Waiting> = self
            .pending
            .take()
            .into_iter()
            .chain(self.inbox.drain(..))
            .collect();
        self.inbox_targets = 0;
        for item in waiting {
            self.drop_request(&item.request, why, None);
        }
    }

    /// A new scene starts wide and forgets the camera: waiting directions are void, the spacing
    /// starts over, and a focus that was held is pulled back with no animation.
    pub fn start_scene(&mut self, now_ms: u64) -> Result<Option<StageDirection>, DirectorError> {
        self.advance(now_ms)?;
        self.void_waiting(DropReason::SceneChanged);
        self.last_result = None;
        let had_focus = self.active.is_some();
        let direction = had_focus.then(|| self.make_wide(WideReason::SceneStart, 0, false, None));
        self.last_move_ms = None;
        Ok(direction)
    }

    /// Combat starts wide so the whole fight is legible.
    pub fn start_combat(
        &mut self,
        now_ms: u64,
        view: &StageView,
    ) -> Result<Option<StageDirection>, DirectorError> {
        self.advance(now_ms)?;
        self.void_waiting(DropReason::SceneChanged);
        self.last_result = None;
        if self.active.is_none() {
            return Ok(None);
        }
        let over = self.wide_time(view);
        Ok(Some(self.make_wide(
            WideReason::CombatStart,
            over,
            false,
            Some(view),
        )))
    }

    /// A person at the dev command line pulls the camera out to the whole stage, whatever it held
    /// and whatever the spacing: a driven move, marked so in its line. Without a time of its own
    /// it takes as long as any pull-back, and is a cut under reduced effects.
    pub fn pull_back(
        &mut self,
        now_ms: u64,
        view: &StageView,
        over_ms: Option<u32>,
    ) -> Result<StageDirection, DirectorError> {
        self.advance(now_ms)?;
        let over = over_ms.unwrap_or_else(|| self.wide_time(view));
        if over > MAXIMUM_CAMERA_MS {
            return Err(DirectorError::WideMoveOutOfRange { over_ms: over });
        }
        self.void_waiting(DropReason::SceneChanged);
        Ok(self.make_wide(WideReason::Operator, over, true, Some(view)))
    }

    /// Decides what the camera does now. Called every host tick with the shared screen as it
    /// stands, so a target that stopped being visible is pulled back from within one tick.
    pub fn tick(
        &mut self,
        now_ms: u64,
        view: &StageView,
    ) -> Result<Vec<StageDirection>, DirectorError> {
        self.advance(now_ms)?;
        let mut moves = Vec::new();
        if let Some(owed) = self.owed_wide.take() {
            moves.push(self.make_wide(owed.reason, owed.over_ms, owed.driven, Some(view)));
            return Ok(moves);
        }
        if self.follow_the_view(view, &mut moves) || self.paused_hold() {
            return Ok(moves);
        }
        if let Some(direction) = self.expire(view) {
            moves.push(direction);
        }
        moves.extend(self.decide(view));
        Ok(moves)
    }

    /// The one tick of paused: nothing automatic expires, but a driven request still decides.
    fn paused_hold(&self) -> bool {
        self.paused && self.inbox.iter().all(|item| !item.request.driven)
    }

    /// Pulls back at once when the focus's target is hidden or gone, re-aims or goes wide when
    /// the plate changed. Returns whether it made a move (a move is the tick's whole answer).
    fn follow_the_view(&mut self, view: &StageView, moves: &mut Vec<StageDirection>) -> bool {
        let Some(active) = &self.active else {
            return false;
        };
        let (element, generation) = (active.element.clone(), active.generation);
        let over = self.wide_time(view);
        match view.target(&element) {
            None => {
                moves.push(self.make_wide(WideReason::TargetGone, over, false, Some(view)));
                return true;
            }
            Some(target) if !target.focusable() => {
                moves.push(self.make_wide(WideReason::TargetHidden, over, false, Some(view)));
                return true;
            }
            Some(_) => {}
        }
        let Some(now_generation) = view.plate_generation() else {
            moves.push(self.make_wide(WideReason::NoAnchor, over, false, Some(view)));
            return true;
        };
        if generation == Some(now_generation) {
            return false;
        }
        let anchored = view.target(&element).is_some_and(|target| target.anchored);
        if !anchored {
            moves.push(self.make_wide(WideReason::NoAnchor, over, false, Some(view)));
            return true;
        }
        // The same creature's anchor on the new plate, within the same dissolve.
        let swap_ms = if view.reduced_effects {
            0
        } else {
            view.swap_dissolve_ms
        };
        moves.extend(self.reaim(now_generation, swap_ms, view));
        true
    }

    /// Ends a focus whose hold is over, unless a card or beat that is not about it is on screen.
    fn expire(&mut self, view: &StageView) -> Option<StageDirection> {
        let active = self.active.as_ref()?;
        if self.paused || self.now_ms < active.deadline_ms {
            return None;
        }
        if Self::beat_forbids(
            view,
            self.now_ms,
            std::slice::from_ref(&active.element),
            None,
        ) {
            return None;
        }
        let why = if active.ended {
            WideReason::ReasonEnded
        } else {
            WideReason::Timeout
        };
        let over = self.wide_time(view);
        Some(self.make_wide(why, over, false, Some(view)))
    }

    /// Whether a result card or reveal beat keeps the camera still for a move onto `elements`.
    fn beat_forbids(
        view: &StageView,
        now_ms: u64,
        elements: &[ElementId],
        reason: Option<Reason>,
    ) -> bool {
        let Some(beat) = &view.beat else {
            return false;
        };
        if now_ms >= beat.until_ms {
            return false;
        }
        let about_it = beat
            .subject
            .as_ref()
            .is_some_and(|subject| elements.contains(subject));
        let belongs_to_reveal = reason == Some(Reason::Reveal) && beat.kind == BeatKind::RevealBeat;
        !(about_it || belongs_to_reveal)
    }
}
