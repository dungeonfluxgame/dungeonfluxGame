//! Time left until an operator deadline, in whole units. Integer math only; the clock is
//! `Model::now_ms`, passed in, never read here.

const MS_PER_MINUTE: u64 = 60_000;
const MINUTES_PER_HOUR: u64 = 60;
const HOURS_PER_DAY: u64 = 24;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TimeLeft {
    /// The deadline is now or already past.
    Overdue,
    Left {
        days: u64,
        hours: u64,
        minutes: u64,
    },
}

/// Time from `now_ms` to `deadline_ms`, rounded up to the minute so a deadline never reads as
/// "0m" while it is still in the future.
pub(super) fn time_left(deadline_ms: u64, now_ms: u64) -> TimeLeft {
    let Some(remaining_ms) = deadline_ms.checked_sub(now_ms).filter(|ms| *ms > 0) else {
        return TimeLeft::Overdue;
    };
    let total_minutes = remaining_ms.div_ceil(MS_PER_MINUTE);
    let total_hours = total_minutes / MINUTES_PER_HOUR;
    TimeLeft::Left {
        days: total_hours / HOURS_PER_DAY,
        hours: total_hours % HOURS_PER_DAY,
        minutes: total_minutes % MINUTES_PER_HOUR,
    }
}

impl TimeLeft {
    pub(super) const fn is_overdue(self) -> bool {
        matches!(self, Self::Overdue)
    }

    pub(super) const fn is_within_a_day(self) -> bool {
        matches!(self, Self::Left { days: 0, .. })
    }

    /// `2d 03h`, `3h 12m` or `45m`; the two largest units that matter. Empty when overdue.
    pub(super) fn span(self) -> String {
        match self {
            Self::Overdue => String::new(),
            Self::Left { days, hours, .. } if days > 0 => format!("{days}d {hours:02}h"),
            Self::Left { hours, minutes, .. } if hours > 0 => format!("{hours}h {minutes:02}m"),
            Self::Left { minutes, .. } => format!("{minutes}m"),
        }
    }

    /// `Overdue` or `Due in 2d 03h`.
    pub(super) fn due(self) -> String {
        if self.is_overdue() {
            "Overdue".to_owned()
        } else {
            format!("Due in {}", self.span())
        }
    }
}
