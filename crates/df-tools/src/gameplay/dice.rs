//! The one place a game die is drawn. A die size with faces queued through the development control
//! surface (`dev dice next`) takes its next queued face, first in first out; otherwise the
//! operating system's random source draws the face, rejection-sampled so every face is equally
//! likely. The queue is process-wide, is written only by the dev control routes (which exist only
//! when dev control is on), and is empty in normal play, so production draws are unchanged.

use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Mutex, PoisonError},
};

use df_session::submission::RepositoryError;

use super::{actor::record_audit_line, model};

/// The die sizes the queue accepts, in the order the queue lists them.
pub(super) const QUEUEABLE_DIE_SIZES: [u32; 7] = [4, 6, 8, 10, 12, 20, 100];
/// Most faces one die size may hold at once. A dev script that would pass this is refused whole.
const MAXIMUM_QUEUED_FACES: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum QueueError {
    UnknownDie,
    FaceOutOfRange {
        face: u32,
        sides: u32,
    },
    QueueFull {
        sides: u32,
        held: usize,
        requested: usize,
    },
}

impl QueueError {
    pub(super) fn message(self) -> String {
        match self {
            Self::UnknownDie => format!(
                "die must be one of {}",
                QUEUEABLE_DIE_SIZES
                    .iter()
                    .map(|sides| format!("d{sides}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::FaceOutOfRange { face, sides } => {
                format!("face {face} is outside 1..={sides} for d{sides}; nothing was queued")
            }
            Self::QueueFull {
                sides,
                held,
                requested,
            } => format!(
                "d{sides} holds {held} queued faces; {requested} more would pass the \
                 {MAXIMUM_QUEUED_FACES} limit; nothing was queued"
            ),
        }
    }
}

/// Queued faces per die size. Pure: the process-wide instance is only reached through `enqueue`,
/// `snapshot`, `clear` and `draw`.
pub(super) struct Queue {
    faces: BTreeMap<u32, VecDeque<u32>>,
}

impl Queue {
    pub(super) const fn new() -> Self {
        Self {
            faces: BTreeMap::new(),
        }
    }

    /// Appends `faces` in order. Every face is checked first, so a refusal changes nothing.
    /// Returns how many faces the die size holds afterwards.
    pub(super) fn push(&mut self, sides: u32, faces: &[u32]) -> Result<usize, QueueError> {
        if !QUEUEABLE_DIE_SIZES.contains(&sides) {
            return Err(QueueError::UnknownDie);
        }
        if let Some(face) = faces
            .iter()
            .copied()
            .find(|face| !(1..=sides).contains(face))
        {
            return Err(QueueError::FaceOutOfRange { face, sides });
        }
        let held = self.faces.get(&sides).map_or(0, VecDeque::len);
        if held + faces.len() > MAXIMUM_QUEUED_FACES {
            return Err(QueueError::QueueFull {
                sides,
                held,
                requested: faces.len(),
            });
        }
        let queue = self.faces.entry(sides).or_default();
        queue.extend(faces.iter().copied());
        Ok(queue.len())
    }

    /// Removes the next face for `sides`, with how many remain for that size.
    pub(super) fn take(&mut self, sides: u32) -> Option<(u32, usize)> {
        let queue = self.faces.get_mut(&sides)?;
        let face = queue.pop_front()?;
        Some((face, queue.len()))
    }

    /// Every queueable die size with its queued faces in order.
    pub(super) fn snapshot(&self) -> Vec<(u32, Vec<u32>)> {
        QUEUEABLE_DIE_SIZES
            .iter()
            .map(|&sides| {
                let faces = self
                    .faces
                    .get(&sides)
                    .map(|queue| queue.iter().copied().collect())
                    .unwrap_or_default();
                (sides, faces)
            })
            .collect()
    }

    pub(super) fn clear(&mut self) {
        self.faces.clear();
    }
}

static QUEUE: Mutex<Queue> = Mutex::new(Queue::new());

fn queue() -> std::sync::MutexGuard<'static, Queue> {
    QUEUE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Appends forced faces for one die size; the number now queued for that size.
pub(super) fn enqueue(sides: u32, faces: &[u32]) -> Result<usize, QueueError> {
    queue().push(sides, faces)
}

/// Every queueable die size with its queued faces.
pub(super) fn snapshot() -> Vec<(u32, Vec<u32>)> {
    queue().snapshot()
}

/// Empties the queue of every die size.
pub(super) fn clear() {
    queue().clear();
}

/// One die of `sides`: the next queued face when one waits, else a random face. Each forced face
/// is written to the audit log.
pub(super) fn draw(sides: u32) -> Result<u32, RepositoryError> {
    let forced = queue().take(sides);
    if let Some((face, remaining)) = forced {
        record_audit_line("gameplay.dice", forced_line(sides, face, remaining));
        return Ok(face);
    }
    random_face(sides)
}

fn forced_line(sides: u32, face: u32, remaining: usize) -> String {
    format!("DICE forced die=d{sides} face={face} remaining={remaining}")
}

fn random_face(sides: u32) -> Result<u32, RepositoryError> {
    if !(1..=256).contains(&sides) {
        return Err(RepositoryError::InvalidCandidate);
    }
    for _ in 0..16 {
        let [byte] = model::random::<1>()?;
        if let Some(face) = face_for_byte(sides, byte) {
            return Ok(face);
        }
    }
    Err(RepositoryError::Unavailable)
}

/// The face one random byte gives. The final incomplete bucket is refused, because a plain modulo
/// of all 256 byte values is biased: for twenty sides, bytes 0..240 give each face exactly twelve
/// times.
fn face_for_byte(sides: u32, byte: u8) -> Option<u32> {
    let bucket = 256 - 256 % sides;
    (u32::from(byte) < bucket).then(|| u32::from(byte) % sides + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_faces_come_out_first_in_first_out_per_die_size() {
        let mut queue = Queue::new();
        assert_eq!(queue.push(20, &[17, 3]), Ok(2));
        assert_eq!(queue.push(6, &[4]), Ok(1));
        assert_eq!(queue.take(20), Some((17, 1)));
        assert_eq!(queue.take(6), Some((4, 0)));
        assert_eq!(queue.take(20), Some((3, 0)));
        assert_eq!(queue.take(20), None);
        assert_eq!(queue.take(4), None);
    }

    #[test]
    fn a_refused_face_queues_nothing_and_an_unknown_die_is_refused() {
        let mut queue = Queue::new();
        assert_eq!(
            queue.push(20, &[5, 21]),
            Err(QueueError::FaceOutOfRange {
                face: 21,
                sides: 20
            })
        );
        assert_eq!(
            queue.push(4, &[0]),
            Err(QueueError::FaceOutOfRange { face: 0, sides: 4 })
        );
        assert_eq!(queue.push(7, &[3]), Err(QueueError::UnknownDie));
        assert!(queue.snapshot().iter().all(|(_, faces)| faces.is_empty()));
        assert_eq!(queue.push(100, &[100, 1]), Ok(2));
        assert_eq!(queue.take(100), Some((100, 1)));
    }

    #[test]
    fn a_die_size_accepts_exactly_the_limit_and_refuses_one_more_face() {
        let mut queue = Queue::new();
        assert_eq!(
            queue.push(6, &vec![6; MAXIMUM_QUEUED_FACES + 1]),
            Err(QueueError::QueueFull {
                sides: 6,
                held: 0,
                requested: MAXIMUM_QUEUED_FACES + 1,
            }),
            "a batch larger than the limit is refused on an empty queue"
        );
        assert!(queue.snapshot().iter().all(|(_, faces)| faces.is_empty()));
        assert_eq!(
            queue.push(6, &vec![6; MAXIMUM_QUEUED_FACES]),
            Ok(MAXIMUM_QUEUED_FACES),
            "exactly the limit on an empty queue is accepted"
        );
        assert_eq!(
            queue.push(6, &[1]),
            Err(QueueError::QueueFull {
                sides: 6,
                held: MAXIMUM_QUEUED_FACES,
                requested: 1,
            })
        );
        assert_eq!(queue.snapshot()[1].1.len(), MAXIMUM_QUEUED_FACES);
    }

    #[test]
    fn a_full_die_size_leaves_other_sizes_room_and_frees_a_slot_on_take() {
        let mut queue = Queue::new();
        queue.push(6, &vec![6; MAXIMUM_QUEUED_FACES]).unwrap();
        assert_eq!(
            queue.push(20, &[20]),
            Ok(1),
            "other die sizes keep their own room"
        );
        assert_eq!(queue.take(6), Some((6, MAXIMUM_QUEUED_FACES - 1)));
        assert_eq!(queue.push(6, &[1]), Ok(MAXIMUM_QUEUED_FACES));
    }

    #[test]
    fn a_refused_queue_full_names_the_held_and_requested_counts() {
        let message = QueueError::QueueFull {
            sides: 6,
            held: 255,
            requested: 2,
        }
        .message();
        assert!(message.contains("d6 holds 255 queued faces"), "{message}");
        assert!(
            message.contains("2 more would pass the 256 limit"),
            "{message}"
        );
        assert!(message.contains("nothing was queued"), "{message}");
    }

    #[test]
    fn snapshot_lists_every_die_size_and_clear_empties_them() {
        let mut queue = Queue::new();
        queue.push(12, &[12]).unwrap();
        let listed = queue.snapshot();
        assert_eq!(listed.len(), QUEUEABLE_DIE_SIZES.len());
        assert_eq!(listed[4], (12, vec![12]));
        queue.clear();
        assert!(queue.snapshot().iter().all(|(_, faces)| faces.is_empty()));
    }

    #[test]
    fn every_d20_face_has_twelve_equally_likely_byte_inputs() {
        for face in 1..=20 {
            assert_eq!(
                (0..=u8::MAX)
                    .filter(|byte| face_for_byte(20, *byte) == Some(face))
                    .count(),
                12
            );
        }
        assert_eq!(face_for_byte(20, 0), Some(1));
        assert_eq!(
            (0..=u8::MAX)
                .filter(|byte| face_for_byte(20, *byte).is_none())
                .count(),
            16
        );
    }

    #[test]
    fn a_draw_of_an_unsupported_size_is_refused_and_a_forced_line_names_the_die() {
        assert!(random_face(0).is_err());
        assert!(random_face(257).is_err());
        assert_eq!(
            forced_line(20, 17, 2),
            "DICE forced die=d20 face=17 remaining=2"
        );
    }

    #[test]
    fn a_queued_d100_face_replaces_the_random_draw_then_the_draw_falls_back() {
        let _process_state =
            crate::gameplay::journey::tactical::development_tests::process_state_test_scope();
        // d100 is never drawn by game code, so this test cannot consume another test's faces.
        clear();
        assert_eq!(enqueue(100, &[42, 7]), Ok(2));
        assert_eq!(draw(100), Ok(42));
        assert_eq!(draw(100), Ok(7));
        assert_eq!(snapshot()[6], (100, vec![]));
        let fallback = draw(100).unwrap();
        assert!((1..=100).contains(&fallback));
    }
}
