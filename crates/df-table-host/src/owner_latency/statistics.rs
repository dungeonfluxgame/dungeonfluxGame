//! Percentile arithmetic and busy-window accounting for the latency harness.
//!
//! It owns the summary of a list of nanosecond durations and the share of fixed windows the owner
//! thread spent working. Everything is integer arithmetic: a percentile is the nearest rank, so it
//! is always a value that was measured, and a mean accumulates in 128 bits so a long run cannot
//! overflow it. It never reads a clock; the harness hands it numbers.

use std::fmt;
use std::time::Duration;

/// The median as a permille of the sorted samples.
pub const PERMILLE_P50: u16 = 500;
/// The 95th percentile, the budget line of H.5.
pub const PERMILLE_P95: u16 = 950;
/// The 99th percentile.
pub const PERMILLE_P99: u16 = 990;
/// All of a window: a fully busy owner thread.
pub const PERMILLE_WHOLE: u16 = 1000;

/// Why a statistic could not be computed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatisticsError {
    /// There are no samples, so no percentile exists.
    Empty,
    /// A percentile was asked for above the whole.
    PercentileOutOfRange { permille: u16 },
    /// A window of zero length cannot hold a share of anything.
    ZeroWindow,
    /// A busy interval ends before it starts.
    IntervalReversed { start_ns: u64, end_ns: u64 },
}

impl fmt::Display for StatisticsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("there are no samples"),
            Self::PercentileOutOfRange { permille } => {
                write!(formatter, "{permille} permille is above the whole")
            }
            Self::ZeroWindow => formatter.write_str("the window has zero length"),
            Self::IntervalReversed { start_ns, end_ns } => {
                write!(
                    formatter,
                    "the interval ends at {end_ns} before it starts at {start_ns}"
                )
            }
        }
    }
}

impl std::error::Error for StatisticsError {}

/// The usual figures of a list of durations, in nanoseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    pub count: usize,
    pub mean_ns: u64,
    pub p50_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
    pub max_ns: u64,
}

/// A duration in whole nanoseconds, saturating at the largest `u64`.
pub fn nanoseconds(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

/// The nearest-rank percentile of samples already sorted ascending: the smallest sample such that
/// at least `permille` thousandths of them are at or below it.
pub fn percentile_of_sorted(sorted: &[u64], permille: u16) -> Result<u64, StatisticsError> {
    if permille > PERMILLE_WHOLE {
        return Err(StatisticsError::PercentileOutOfRange { permille });
    }
    let count = sorted.len();
    let rank = count
        .saturating_mul(usize::from(permille))
        .div_ceil(usize::from(PERMILLE_WHOLE))
        .max(1);
    rank.checked_sub(1)
        .and_then(|index| sorted.get(index))
        .copied()
        .ok_or(StatisticsError::Empty)
}

/// Sorts `values` in place and summarises them. It allocates nothing: the caller owns the buffer
/// and decides whether a copy is worth making.
pub fn summarise(values: &mut [u64]) -> Result<Summary, StatisticsError> {
    values.sort_unstable();
    let count = values.len();
    let total: u128 = values.iter().map(|&value| u128::from(value)).sum();
    let mean_ns = u64::try_from(total / count.max(1) as u128).unwrap_or(u64::MAX);
    Ok(Summary {
        count,
        mean_ns,
        p50_ns: percentile_of_sorted(values, PERMILLE_P50)?,
        p95_ns: percentile_of_sorted(values, PERMILLE_P95)?,
        p99_ns: percentile_of_sorted(values, PERMILLE_P99)?,
        max_ns: values.last().copied().ok_or(StatisticsError::Empty)?,
    })
}

/// A span of time during which the owner thread was working, in nanoseconds since the run began.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BusyInterval {
    pub start_ns: u64,
    pub end_ns: u64,
}

/// For each whole window of `window_ns` that fits in `run_ns`, the permille of it covered by
/// `intervals` (at most a whole window, even if the intervals overlap). A trailing partial window
/// is left out so every figure is over the same length of time.
pub fn busy_permille_per_window(
    intervals: &[BusyInterval],
    window_ns: u64,
    run_ns: u64,
) -> Result<Vec<u16>, StatisticsError> {
    if window_ns == 0 {
        return Err(StatisticsError::ZeroWindow);
    }
    let windows = usize::try_from(run_ns / window_ns).unwrap_or(usize::MAX);
    let mut busy_ns = vec![0u64; windows];
    for interval in intervals {
        if interval.end_ns < interval.start_ns {
            return Err(StatisticsError::IntervalReversed {
                start_ns: interval.start_ns,
                end_ns: interval.end_ns,
            });
        }
        add_to_windows(&mut busy_ns, *interval, window_ns);
    }
    Ok(busy_ns
        .into_iter()
        .map(|busy| {
            let permille = u128::from(busy) * u128::from(PERMILLE_WHOLE) / u128::from(window_ns);
            u16::try_from(permille.min(u128::from(PERMILLE_WHOLE))).unwrap_or(PERMILLE_WHOLE)
        })
        .collect())
}

fn add_to_windows(busy_ns: &mut [u64], interval: BusyInterval, window_ns: u64) {
    let first = usize::try_from(interval.start_ns / window_ns).unwrap_or(usize::MAX);
    for (index, busy) in busy_ns.iter_mut().enumerate().skip(first) {
        let window_start = (index as u64).saturating_mul(window_ns);
        let window_end = window_start.saturating_add(window_ns);
        if interval.end_ns <= window_start {
            break;
        }
        let covered = interval.end_ns.min(window_end) - interval.start_ns.max(window_start);
        *busy = busy.saturating_add(covered);
    }
}
