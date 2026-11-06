//! A deterministic digest of the table's facts, for golden sessions, replay and batch equality.
//!
//! It owns `Digest`, a 64-bit FNV-1a walk that is the same in every run (no random key, no clock,
//! no address), and the order of the walk: collections in `BTreeMap` order, fields in declaration
//! order, every optional value behind a tag byte, every text and list behind its length. Each fact
//! type writes itself with an exhaustive destructure of its own fields (`digest_into` beside the
//! type), and `Facts::digest_into` destructures `Facts`, so a fact added without being walked does
//! not compile (#1682). It never reads a clock and never hashes a pointer.

use serde_json::Value;

use crate::effects::BeatName;
use crate::identity::{MachineId, SessionRevision};
use crate::machine::slot::MachineSlot;
use crate::table_state::facts::Facts;

const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a over a fixed walk of the facts.
pub(super) struct Digest(u64);

impl Digest {
    pub(super) fn new() -> Self {
        Self(OFFSET_BASIS)
    }

    pub(super) fn finish(&self) -> u64 {
        self.0
    }

    pub(super) fn byte(&mut self, byte: u8) {
        self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(PRIME);
    }

    pub(super) fn bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.byte(*byte);
        }
    }

    pub(super) fn number(&mut self, value: u64) {
        self.bytes(&value.to_le_bytes());
    }

    pub(super) fn count(&mut self, value: usize) {
        self.number(u64::try_from(value).unwrap_or(u64::MAX));
    }

    pub(super) fn text(&mut self, text: &str) {
        self.count(text.len());
        self.bytes(text.as_bytes());
    }

    /// A tag byte then the value, so `None` and `Some` never collide.
    pub(super) fn option<T>(&mut self, value: Option<&T>, write: impl FnOnce(&mut Self, &T)) {
        match value {
            Some(value) => {
                self.byte(1);
                write(self, value);
            }
            None => self.byte(0),
        }
    }

    pub(super) fn revision(&mut self, revision: SessionRevision) {
        self.number(revision.epoch().get());
        self.number(revision.sequence());
    }

    pub(super) fn machine(&mut self, machine: &MachineId) {
        self.text(machine.as_str());
    }

    pub(super) fn beat(&mut self, beat: &BeatName) {
        self.text(beat.as_str());
    }

    pub(super) fn slot(&mut self, slot: &MachineSlot) {
        self.machine(slot.machine());
        self.number(u64::from(slot.schema_version()));
        self.json(slot.payload());
    }

    /// A JSON value in a fixed order: objects by key.
    pub(super) fn json(&mut self, value: &Value) {
        match value {
            Value::Null => self.byte(0),
            Value::Bool(flag) => {
                self.byte(1);
                self.byte(u8::from(*flag));
            }
            Value::Number(number) => {
                self.byte(2);
                self.text(&number.to_string());
            }
            Value::String(text) => {
                self.byte(3);
                self.text(text);
            }
            Value::Array(items) => {
                self.byte(4);
                self.count(items.len());
                for item in items {
                    self.json(item);
                }
            }
            Value::Object(fields) => {
                self.byte(5);
                self.count(fields.len());
                let mut keys: Vec<&String> = fields.keys().collect();
                keys.sort();
                for key in keys {
                    self.text(key);
                    if let Some(field) = fields.get(key) {
                        self.json(field);
                    }
                }
            }
        }
    }
}

impl Facts {
    /// Walks every fact. The destructure names every field, so a new one must be walked here.
    pub(super) fn digest_into(&self, digest: &mut Digest) {
        self.digest_walk(digest, true);
    }

    /// The same walk without the commit stamps: the revision of the last commit and the revision
    /// the last pause was stamped at, which the host commit moves and which say nothing about the
    /// world (S1: a batch of N and N batches of one are the same table).
    pub(super) fn digest_without_stamps_into(&self, digest: &mut Digest) {
        self.digest_walk(digest, false);
    }

    fn digest_walk(&self, digest: &mut Digest, stamps: bool) {
        let Self {
            seat_capacity,
            seats,
            scenes,
            cast,
            focus,
            last_generation,
            world_time_s,
            status,
            last_paused_at,
            revision,
            table_machine,
            last_hand_off,
            split_policy,
        } = self;
        digest.count(*seat_capacity);
        digest.count(seats.len());
        for (seat, state) in seats.iter() {
            digest.number(seat.get());
            state.digest_into(digest);
        }
        digest.count(scenes.len());
        for (scene, record) in scenes.iter() {
            digest.number(scene.get());
            record.digest_into(digest);
        }
        digest.count(cast.len());
        for (id, entry) in cast.iter() {
            digest.text(id.as_str());
            entry.digest_into(digest);
        }
        digest.option(focus.as_ref(), |digest, scene| digest.number(scene.get()));
        digest.number(*last_generation);
        digest.number(*world_time_s);
        digest.text(status.key());
        if stamps {
            digest.option(last_paused_at.as_ref(), |digest, at| digest.revision(*at));
            digest.revision(*revision);
        } else {
            digest.option(last_paused_at.as_ref(), |digest, _| digest.byte(0));
        }
        digest.option(table_machine.as_ref(), |digest, machine| {
            machine.digest_into(digest)
        });
        digest.option(last_hand_off.as_ref(), |digest, record| {
            record.digest_into(digest)
        });
        digest.text(split_policy.key());
    }
}
