//! The part of the Director that chooses among the waiting requests and starts a move: the burst
//! rule, the area rule, priority, spacing, the card and beat gates, and how a move is numbered
//! (O.14 camera rules 3 to 5).

use std::collections::{BTreeMap, BTreeSet};

use super::{Active, Director, Request, ResultMemory, StageDirection, Waiting};
use crate::director::log::{Cause, DropReason, Outcome};
use crate::director::reason::{Ends, WideReason};
use crate::director::view::{StageView, clamped_zoom};
use crate::element::ElementId;

impl Director {
    /// How long a focus holds: the request's own hold, else `focus_hold_ms` for a moment reason and
    /// the safety limit for one that ends with its reason.
    fn hold_for(&self, request: &Request, ends: Ends) -> u64 {
        request.hold_ms.unwrap_or(match ends {
            Ends::AfterHold => self.settings.focus_hold_ms,
            Ends::WhenReasonEnds => self.settings.unended_limit_ms,
        })
    }

    /// Whether another move now would break the spacing.
    fn within_spacing(&self) -> bool {
        self.last_move_ms
            .is_some_and(|last| self.now_ms.saturating_sub(last) < self.settings.spacing_ms)
    }

    /// Whether `request` names a hero other than the one the last result named, or the last
    /// result already named several: it is one more hero of that result's area.
    fn joins_the_last_area(&self, request: &Request) -> bool {
        self.last_result.as_ref().is_some_and(|memory| {
            request.result == Some(memory.result)
                && (memory.several || request.targets.iter().any(|target| *target != memory.hero))
        })
    }

    /// The requests that name more than one hero, alone, by sharing a result in this batch, or by
    /// belonging to the result of an earlier tick.
    fn area_flags(&self, waiting: &[Waiting]) -> Vec<bool> {
        let mut heroes: BTreeMap<u64, BTreeSet<&ElementId>> = BTreeMap::new();
        for item in waiting {
            if let Some(result) = item.request.result {
                heroes
                    .entry(result)
                    .or_default()
                    .extend(item.request.targets.iter());
            }
        }
        waiting
            .iter()
            .map(|item| {
                item.request.targets.len() > 1
                    || item
                        .request
                        .result
                        .and_then(|result| heroes.get(&result))
                        .is_some_and(|set| set.len() > 1)
                    || self.joins_the_last_area(&item.request)
            })
            .collect()
    }

    /// Remembers that `request`'s result named its heroes, so a later request of the same result
    /// is judged against them. `several` says the result is known to name more than one.
    fn note_result(&mut self, request: &Request, several: bool) {
        let (Some(result), Some(first)) = (request.result, request.targets.first()) else {
            return;
        };
        let several = several || request.targets.len() > 1;
        match &mut self.last_result {
            Some(memory) if memory.result == result => {
                memory.several |= several || *first != memory.hero;
            }
            _ => {
                self.last_result = Some(ResultMemory {
                    result,
                    hero: first.clone(),
                    several,
                });
            }
        }
    }

    /// Chooses what the waiting requests come to this tick: one move at most.
    pub(super) fn decide(&mut self, view: &StageView) -> Option<StageDirection> {
        let mut waiting: Vec<Waiting> = self
            .pending
            .take()
            .into_iter()
            .chain(self.inbox.drain(..))
            .collect();
        self.inbox_targets = 0;
        let mut kept = Vec::with_capacity(waiting.len());
        for item in waiting.drain(..) {
            match self
                .held_back(&item.request)
                .or_else(|| Self::refusal(&item.request, view))
            {
                Some(why) => self.drop_request(&item.request, why, Some(view)),
                None => kept.push(item),
            }
        }
        let areas = self.area_flags(&kept);
        // The highest reason wins, and among equals the latest.
        let best = kept
            .iter()
            .enumerate()
            .max_by_key(|(index, item)| (item.request.reason.rank(), item.order, *index))
            .map(|(index, _)| index)?;
        let winner_is_area = areas.get(best).copied().unwrap_or(false);
        let winner_rank = kept.get(best)?.request.reason.rank();
        let mut winner = None;
        for (index, item) in kept.into_iter().enumerate() {
            let is_area = areas.get(index).copied().unwrap_or(false);
            if index == best {
                winner = Some(item);
            } else if is_area && winner_is_area {
                self.drop_request(&item.request, DropReason::Area, Some(view));
            } else if is_area || item.request.reason.rank() < winner_rank {
                self.drop_request(&item.request, DropReason::LowerPriority, Some(view));
            } else {
                self.drop_request(&item.request, DropReason::Coalesced, Some(view));
            }
        }
        let winner = winner?;
        if winner_is_area {
            return self.gate_area(winner, view);
        }
        self.gate(winner, view)
    }

    /// Several heroes at once leave the camera wide until a frame exists (T4): the pull-back keeps
    /// the spacing and the card like any move, so it waits as the one pending request.
    fn gate_area(&mut self, winner: Waiting, view: &StageView) -> Option<StageDirection> {
        self.note_result(&winner.request, true);
        if self.active.is_none() {
            self.drop_request(&winner.request, DropReason::Area, Some(view));
            return None;
        }
        if Self::beat_forbids(view, self.now_ms, &winner.request.targets, None)
            || self.within_spacing()
        {
            self.pending = Some(winner);
            return None;
        }
        self.drop_request(&winner.request, DropReason::Area, Some(view));
        let over = self.wide_time(view);
        Some(self.make_wide(WideReason::Area, over, false, Some(view)))
    }

    /// Starts the winner's move, or keeps it waiting, or drops it, by the rules of pace.
    fn gate(&mut self, winner: Waiting, view: &StageView) -> Option<StageDirection> {
        let element = winner.request.targets.first()?.clone();
        let reason = winner.request.reason;
        let mut outranks = false;
        if let Some(active) = &self.active {
            if reason.rank() < active.reason.rank() {
                self.drop_request(&winner.request, DropReason::LowerPriority, Some(view));
                return None;
            }
            if active.element == element && active.reason == reason {
                let deadline = self
                    .now_ms
                    .saturating_add(self.hold_for(&winner.request, active.ends));
                if let Some(active) = &mut self.active {
                    active.deadline_ms = deadline;
                }
                self.drop_request(&winner.request, DropReason::AlreadyFocused, Some(view));
                return None;
            }
            outranks = reason.rank() > active.reason.rank();
        }
        if Self::beat_forbids(
            view,
            self.now_ms,
            std::slice::from_ref(&element),
            Some(reason),
        ) || (self.within_spacing() && !outranks)
        {
            self.pending = Some(winner);
            return None;
        }
        Some(self.start_from(&winner.request, &element, view))
    }

    /// Starts a focus move for `request` on `element`.
    fn start_from(
        &mut self,
        request: &Request,
        element: &ElementId,
        view: &StageView,
    ) -> StageDirection {
        let spec = self.settings.reasons.spec(request.reason);
        let asked = request.zoom_percent.unwrap_or(spec.zoom_percent());
        let (zoom, _) = clamped_zoom(asked, view.zoom_ceiling_percent());
        let clamped_from = (zoom != asked).then_some(asked);
        let over = if view.reduced_effects {
            0
        } else {
            request.over_ms.unwrap_or(spec.over_ms())
        };
        let deadline = self
            .now_ms
            .saturating_add(self.hold_for(request, spec.ends()));
        self.serial = self.serial.saturating_add(1);
        self.last_move_ms = Some(self.now_ms);
        self.owed_wide = None;
        self.note_result(request, false);
        let replaced = self.active.as_ref().map(|active| active.element.clone());
        self.active = Some(Active {
            element: element.clone(),
            reason: request.reason,
            serial: self.serial,
            requested_zoom_percent: asked,
            zoom_percent: zoom,
            since_ms: self.now_ms,
            deadline_ms: deadline,
            ends: spec.ends(),
            ended: false,
            generation: view.plate_generation(),
            replaced,
        });
        self.record(
            Cause::Focus(request.reason),
            Some(element.clone()),
            true,
            Outcome::Applied {
                serial: self.serial,
                zoom_percent: zoom,
                over_ms: over,
                clamped_from,
            },
            request.driven,
        );
        StageDirection::Focus {
            element: element.clone(),
            reason: request.reason,
            zoom_percent: zoom,
            clamped_from,
            over_ms: over,
            serial: self.serial,
            driven: request.driven,
        }
    }

    /// A new plate came with its own anchors: aim at the same creature's anchor on it, within the
    /// dissolve, as a new numbered move that keeps the focus's reason and its hold.
    pub(super) fn reaim(
        &mut self,
        generation: u64,
        over_ms: u64,
        view: &StageView,
    ) -> Option<StageDirection> {
        let (element, reason, asked) = {
            let active = self.active.as_ref()?;
            (
                active.element.clone(),
                active.reason,
                active.requested_zoom_percent,
            )
        };
        let (zoom, _) = clamped_zoom(asked, view.zoom_ceiling_percent());
        let clamped_from = (zoom != asked).then_some(asked);
        self.serial = self.serial.saturating_add(1);
        self.last_move_ms = Some(self.now_ms);
        self.owed_wide = None;
        let serial = self.serial;
        if let Some(active) = &mut self.active {
            active.generation = Some(generation);
            active.serial = serial;
            active.zoom_percent = zoom;
            active.replaced = Some(element.clone());
        }
        self.record(
            Cause::Focus(reason),
            Some(element.clone()),
            true,
            Outcome::Applied {
                serial,
                zoom_percent: zoom,
                over_ms,
                clamped_from,
            },
            false,
        );
        Some(StageDirection::Focus {
            element,
            reason,
            zoom_percent: zoom,
            clamped_from,
            over_ms,
            serial,
            driven: false,
        })
    }
}
