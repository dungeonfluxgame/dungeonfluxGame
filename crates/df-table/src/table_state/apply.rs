//! The applier: what each `TableEffect` does to the facts, or why it may not.
//!
//! It owns one exhaustive `match` over `TableEffect` and the small checked change each arm makes.
//! The match has no wildcard arm, so an effect added without its applier arm does not compile
//! (O.5). Every arm checks first and changes second, and `TableState::apply` runs them on a copy of
//! the facts, so a refusal anywhere in a commit leaves the table as it was. It never records an
//! event and never remembers an operation: `TableState::apply` does both.
//!
//! The rules these arms keep: a scene opens with a new generation and closes when its last seat
//! leaves it; `SceneEntered` is the whole party entering a place and `SeatMoved` is one seat
//! going alone, which the table declines while it keeps the party together (RES25); the world
//! clock does not move while the table is paused (D.3); a cast entry's controller is seated, and a
//! hero is controlled by a seat and stands where that seat stands (D.5); the hand-off stack is as
//! deep as the plan allows and no deeper (RES27).

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::effects::{BeatName, ConversationOutcome, EffectCause, TableEffect};
use crate::element::ElementId;
use crate::identity::{MachineId, SceneId, SeatId, SessionRevision};
use crate::limits::{MAXIMUM_CAST_ENTRIES, MAXIMUM_CONDITIONS_PER_ENTRY, MAXIMUM_HAND_OFF_DEPTH};
use crate::machine::slot::MachineSlot;
use crate::table_state::cast::{
    CastChange, CastEntry, CastKind, ConditionName, Controller, Vitals,
};
use crate::table_state::facts::Facts;
use crate::table_state::scene::{
    ActiveMachine, HandOffFrame, HandOffRecord, MachineHome, SceneRecord,
};
use crate::table_state::seat::{Presence, SeatState};
use crate::table_state::status::{SplitPolicy, TableStatus};

impl Facts {
    /// Applies one effect, committed at `at`.
    pub(super) fn apply_one(
        &mut self,
        effect: &TableEffect,
        at: SessionRevision,
    ) -> Result<(), EffectCause> {
        match effect {
            TableEffect::SeatJoined { seat } => self.join(*seat),
            TableEffect::SeatLeft { seat } => self.leave(*seat),
            TableEffect::SeatPresenceChanged { seat, presence } => {
                self.set_presence(*seat, *presence)
            }
            TableEffect::SeatMoved { seat, scene } => self.move_seat(*seat, *scene),
            TableEffect::SceneEntered { scene } => self.enter_scene(*scene),
            TableEffect::BeatResolved { scene, beat } => self.resolve_beat(*scene, beat),
            TableEffect::ConversationEnded { scene, outcome } => {
                self.end_conversation(*scene, *outcome)
            }
            TableEffect::HandOffRecorded { from, to, carried } => {
                self.last_hand_off = Some(HandOffRecord {
                    from: from.clone(),
                    to: to.clone(),
                    carried: *carried,
                });
                Ok(())
            }
            TableEffect::TimeAdvanced { advance_s } => self.advance_time(*advance_s),
            TableEffect::TablePaused => self.pause(at),
            TableEffect::TableResumed => self.resume(),
            TableEffect::TableIdled => self.idle(),
            TableEffect::CastEntered { entry } => self.enter_cast(entry),
            TableEffect::CastLeft { cast } => self.leave_cast(cast),
            TableEffect::CastChanged { cast, change } => self.change_cast(cast, change),
            TableEffect::ActiveMachineSet {
                home,
                machine,
                slot,
            } => self.set_active(home, machine, slot),
            TableEffect::ActiveSlotUpdated { home, slot } => self.update_active_slot(home, slot),
            TableEffect::HandOffPushed {
                scene,
                machine,
                slot,
            } => self.push_frame(*scene, machine, slot, at),
            TableEffect::HandOffPopped { scene } => self.pop_frame(*scene),
        }
    }

    fn scene_mut(&mut self, scene: SceneId) -> Result<&mut SceneRecord, EffectCause> {
        if !self.scenes.contains_key(&scene) {
            return Err(EffectCause::SceneNotActive);
        }
        Arc::make_mut(&mut self.scenes)
            .get_mut(&scene)
            .ok_or(EffectCause::SceneNotActive)
    }

    fn join(&mut self, seat: SeatId) -> Result<(), EffectCause> {
        if self.seats.contains_key(&seat) {
            return Err(EffectCause::SeatAlreadySeated);
        }
        if self.seats.len() >= self.seat_capacity {
            return Err(EffectCause::TableFull);
        }
        let state = SeatState {
            presence: Presence::Connected,
            scene: self.focus,
            hero: None,
        };
        Arc::make_mut(&mut self.seats).insert(seat, state);
        Ok(())
    }

    fn leave(&mut self, seat: SeatId) -> Result<(), EffectCause> {
        let Some(state) = self.seats.get(&seat).cloned() else {
            return Err(EffectCause::SeatNotSeated);
        };
        let controlled = |entry: &CastEntry| entry.controller() == (Controller::Seat { seat });
        if self.cast.values().any(controlled) {
            Arc::make_mut(&mut self.cast).retain(|_, entry| !controlled(entry));
        }
        Arc::make_mut(&mut self.seats).remove(&seat);
        if let Some(scene) = state.scene {
            self.close_scene_if_empty(scene);
            self.refocus();
        }
        Ok(())
    }

    fn set_presence(&mut self, seat: SeatId, presence: Presence) -> Result<(), EffectCause> {
        if !self.seats.contains_key(&seat) {
            return Err(EffectCause::SeatNotSeated);
        }
        if let Some(state) = Arc::make_mut(&mut self.seats).get_mut(&seat) {
            state.presence = presence;
        }
        Ok(())
    }

    fn move_seat(&mut self, seat: SeatId, scene: SceneId) -> Result<(), EffectCause> {
        let Some(from) = self.seats.get(&seat).map(|state| state.scene) else {
            return Err(EffectCause::SeatNotSeated);
        };
        if from == Some(scene) {
            return Ok(());
        }
        if self.split_policy == SplitPolicy::Declined && self.parts_company(seat, scene) {
            return Err(EffectCause::SplitPartyDeclined);
        }
        self.open_scene(scene)?;
        if let Some(state) = Arc::make_mut(&mut self.seats).get_mut(&seat) {
            state.scene = Some(scene);
        }
        self.move_controlled(seat, scene);
        if let Some(left) = from {
            self.close_scene_if_empty(left);
        }
        self.refocus();
        Ok(())
    }

    /// Everything a seat controls goes with it: its heroes, mounts, companions, summons and
    /// hirelings (D.5, #1689).
    fn move_controlled(&mut self, seat: SeatId, scene: SceneId) {
        let controlled: Vec<ElementId> = self.controlled_by(seat).cloned().collect();
        if controlled.is_empty() {
            return;
        }
        let cast = Arc::make_mut(&mut self.cast);
        for id in &controlled {
            if let Some(entry) = cast.get_mut(id) {
                entry.moved_to(scene);
            }
        }
    }

    fn enter_scene(&mut self, scene: SceneId) -> Result<(), EffectCause> {
        let generation = self.issue_generation()?;
        let mut opened = BTreeMap::new();
        opened.insert(scene, SceneRecord::opened(generation));
        self.scenes = Arc::new(opened);
        if !self.seats.is_empty() {
            for state in Arc::make_mut(&mut self.seats).values_mut() {
                state.scene = Some(scene);
            }
        }
        if !self.cast.is_empty() {
            let cast = Arc::make_mut(&mut self.cast);
            cast.retain(|_, entry| matches!(entry.controller(), Controller::Seat { .. }));
            for entry in cast.values_mut() {
                entry.moved_to(scene);
            }
        }
        self.focus = Some(scene);
        Ok(())
    }

    fn resolve_beat(&mut self, scene: SceneId, beat: &BeatName) -> Result<(), EffectCause> {
        self.scene_mut(scene)?.beat = Some(beat.clone());
        Ok(())
    }

    fn end_conversation(
        &mut self,
        scene: SceneId,
        outcome: ConversationOutcome,
    ) -> Result<(), EffectCause> {
        self.scene_mut(scene)?.conversation = Some(outcome);
        Ok(())
    }

    fn advance_time(&mut self, advance_s: u64) -> Result<(), EffectCause> {
        if self.status == TableStatus::Paused {
            return Err(EffectCause::TablePaused);
        }
        self.world_time_s = self
            .world_time_s
            .checked_add(advance_s)
            .ok_or(EffectCause::WorldTimeOverflow)?;
        Ok(())
    }

    fn pause(&mut self, at: SessionRevision) -> Result<(), EffectCause> {
        if self.status == TableStatus::Paused {
            return Err(EffectCause::AlreadyPaused);
        }
        self.status = TableStatus::Paused;
        self.last_paused_at = Some(at);
        Ok(())
    }

    fn resume(&mut self) -> Result<(), EffectCause> {
        if self.status == TableStatus::Active {
            return Err(EffectCause::TableNotPausedOrIdle);
        }
        self.status = TableStatus::Active;
        Ok(())
    }

    fn idle(&mut self) -> Result<(), EffectCause> {
        if self.status != TableStatus::Active {
            return Err(EffectCause::TableNotActive);
        }
        self.status = TableStatus::Idle;
        Ok(())
    }

    fn enter_cast(&mut self, entry: &CastEntry) -> Result<(), EffectCause> {
        if !self.scenes.contains_key(&entry.scene()) {
            return Err(EffectCause::SceneNotActive);
        }
        if self.cast.contains_key(entry.id()) {
            return Err(EffectCause::CastEntryExists);
        }
        if self.cast.len() >= MAXIMUM_CAST_ENTRIES {
            return Err(EffectCause::TooManyCastEntries);
        }
        if entry.conditions().len() > MAXIMUM_CONDITIONS_PER_ENTRY {
            return Err(EffectCause::TooManyConditions);
        }
        match entry.controller() {
            Controller::Seat { seat } => {
                let state = self
                    .seats
                    .get(&seat)
                    .ok_or(EffectCause::ControllerNotSeated)?;
                if state.scene != Some(entry.scene()) {
                    return Err(EffectCause::ControlledOutsideSeatScene);
                }
                if entry.kind() == CastKind::Party && state.hero.is_some() {
                    return Err(EffectCause::SeatHasHero);
                }
            }
            Controller::Engine if entry.kind() == CastKind::Party => {
                return Err(EffectCause::PartyNeedsSeatController);
            }
            Controller::Engine => {}
        }
        Arc::make_mut(&mut self.cast).insert(entry.id().clone(), entry.clone());
        if let (CastKind::Party, Controller::Seat { seat }) = (entry.kind(), entry.controller())
            && let Some(state) = Arc::make_mut(&mut self.seats).get_mut(&seat)
        {
            state.hero = Some(entry.id().clone());
        }
        Ok(())
    }

    fn leave_cast(&mut self, cast: &ElementId) -> Result<(), EffectCause> {
        let Some(entry) = self.cast.get(cast) else {
            return Err(EffectCause::CastEntryUnknown);
        };
        if let (CastKind::Party, Controller::Seat { seat }) = (entry.kind(), entry.controller())
            && let Some(state) = Arc::make_mut(&mut self.seats).get_mut(&seat)
        {
            state.hero = None;
        }
        Arc::make_mut(&mut self.cast).remove(cast);
        Ok(())
    }

    fn change_cast(&mut self, cast: &ElementId, change: &CastChange) -> Result<(), EffectCause> {
        let Some(current) = self.cast.get(cast) else {
            return Err(EffectCause::CastEntryUnknown);
        };
        let hit_points = match change {
            CastChange::HitPoints { hit_points } => Some(
                Vitals::new(*hit_points, current.vitals().maximum_hit_points())
                    .map_err(|_| EffectCause::HitPointsAboveMaximum)?,
            ),
            CastChange::Conditions { conditions } => {
                check_conditions(conditions)?;
                None
            }
            CastChange::Visibility { .. } | CastChange::Placement { .. } => None,
        };
        let Some(entry) = Arc::make_mut(&mut self.cast).get_mut(cast) else {
            return Err(EffectCause::CastEntryUnknown);
        };
        match change {
            CastChange::Visibility { visibility } => entry.set_visibility(*visibility),
            CastChange::Placement { placement } => entry.set_placement(*placement),
            CastChange::HitPoints { .. } => {
                if let Some(vitals) = hit_points {
                    entry.set_hit_points(vitals);
                }
            }
            CastChange::Conditions { conditions } => entry.set_conditions(conditions.clone()),
        }
        Ok(())
    }

    /// Makes `machine` with `slot` the active machine of `home`, replacing the one that ran. The
    /// stack is untouched: a replacement is a hand-off that does not return.
    fn set_active(
        &mut self,
        home: &MachineHome,
        machine: &MachineId,
        slot: &MachineSlot,
    ) -> Result<(), EffectCause> {
        if slot.machine() != machine {
            return Err(EffectCause::FrameSlotMismatch);
        }
        let active = Some(ActiveMachine {
            machine: machine.clone(),
            slot: slot.clone(),
        });
        match home {
            MachineHome::Table => self.table_machine = active,
            MachineHome::Scene { scene } => self.scene_mut(*scene)?.active = active,
        }
        Ok(())
    }

    /// Replaces the saved state of the machine that is active at `home`, which a step changed.
    fn update_active_slot(
        &mut self,
        home: &MachineHome,
        slot: &MachineSlot,
    ) -> Result<(), EffectCause> {
        let active = match home {
            MachineHome::Table => self.table_machine.as_mut(),
            MachineHome::Scene { scene } => self.scene_mut(*scene)?.active.as_mut(),
        };
        let Some(active) = active else {
            return Err(EffectCause::NoActiveMachine);
        };
        if slot.machine() != &active.machine {
            return Err(EffectCause::FrameSlotMismatch);
        }
        active.slot = slot.clone();
        Ok(())
    }

    /// Suspends the active machine onto the stack and makes `machine` with `slot` active, in one
    /// step, so the stack and the active machine never disagree. The machine may not already be
    /// the active one or be waiting on the stack.
    fn push_frame(
        &mut self,
        scene: SceneId,
        machine: &MachineId,
        slot: &MachineSlot,
        at: SessionRevision,
    ) -> Result<(), EffectCause> {
        if slot.machine() != machine {
            return Err(EffectCause::FrameSlotMismatch);
        }
        let record = self.scene_mut(scene)?;
        let Some(running) = record.active.take() else {
            return Err(EffectCause::NoActiveMachine);
        };
        let on_the_stack = record.stack.iter().any(|frame| &frame.machine == machine);
        if &running.machine == machine || on_the_stack {
            record.active = Some(running);
            return Err(EffectCause::MachineAlreadyOnStack);
        }
        if record.stack.len() >= MAXIMUM_HAND_OFF_DEPTH {
            record.active = Some(running);
            return Err(EffectCause::HandOffStackFull);
        }
        record.stack.push(HandOffFrame {
            machine: running.machine,
            slot: running.slot,
            pushed_at: at,
        });
        record.active = Some(ActiveMachine {
            machine: machine.clone(),
            slot: slot.clone(),
        });
        Ok(())
    }

    /// Ends the active machine and resumes the machine on top of the stack with the slot it was
    /// suspended with.
    fn pop_frame(&mut self, scene: SceneId) -> Result<(), EffectCause> {
        let record = self.scene_mut(scene)?;
        let frame = record.stack.pop().ok_or(EffectCause::HandOffStackEmpty)?;
        record.active = Some(ActiveMachine {
            machine: frame.machine,
            slot: frame.slot,
        });
        Ok(())
    }
}

fn check_conditions(conditions: &[ConditionName]) -> Result<(), EffectCause> {
    if conditions.len() > MAXIMUM_CONDITIONS_PER_ENTRY {
        return Err(EffectCause::TooManyConditions);
    }
    Ok(())
}
