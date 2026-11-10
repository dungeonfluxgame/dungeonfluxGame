//! Whether a made picture shows no more people than its cast: the one count check, and the two
//! attempts a picture whose cast is known gets before it is refused.
//!
//! An image model that is asked for one person sometimes paints him twice (roam run 5: two
//! identical men at a street well, with the cast count and the front view of his sheet in force),
//! and no prompt prevents it every time. So a picture whose cast is known is checked after it is
//! made: the person scan every plate already gets (`scan_plate`: the located regions) says how many
//! people it shows. More people than the cast and it is made once more, in the way the caller
//! chooses for the second attempt (a fresh paint); more again and it is refused, never shown. The
//! scan is the plate's own layout, so an accepted picture keeps it. Nothing here reads words; it
//! counts what the scan found. Only declared plates use it now (`plate::make_declared`).

use std::future::Future;
use std::time::Duration;

/// How many attempts a picture gets: the first, and one more.
pub(super) const ATTEMPTS: u8 = 2;

/// Whether a picture that shows `found` people shows more than its `cast`.
pub(super) fn more_than_cast(found: usize, cast: usize) -> bool {
    found > cast
}

/// How the attempts at a picture ended.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Verified<T> {
    /// Attempt `attempt` (from 1) shows no more people than the cast, or its people could not be
    /// counted: it is the picture, with its own scan.
    Accepted { plate: T, attempt: u8 },
    /// Every attempt showed more people than the cast; the last showed `found`.
    Rejected { found: usize },
}

/// What the attempts took, with how they ended.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Attempts<T> {
    pub verdict: Verified<T>,
    /// How long each attempt made took, in order.
    pub makings: Vec<Duration>,
}

/// Makes the picture with `make(attempt)` (the attempt from 1; it answers the picture, how many
/// people its scan found when it could say, and how long making it took) until one shows no more
/// people than `cast`, at most [`ATTEMPTS`] times. A failure to make one ends the attempts with
/// that failure.
pub(super) async fn verified<T, E, F, Fut>(cast: usize, mut make: F) -> Result<Attempts<T>, E>
where
    F: FnMut(u8) -> Fut,
    Fut: Future<Output = Result<(T, Option<usize>, Duration), E>>,
{
    let mut makings = Vec::new();
    let mut found = 0;
    for attempt in 1..=ATTEMPTS {
        let (plate, people, making) = make(attempt).await?;
        makings.push(making);
        match people {
            Some(people) if more_than_cast(people, cast) => found = people,
            _ => {
                return Ok(Attempts {
                    verdict: Verified::Accepted { plate, attempt },
                    makings,
                });
            }
        }
    }
    Ok(Attempts {
        verdict: Verified::Rejected { found },
        makings,
    })
}
