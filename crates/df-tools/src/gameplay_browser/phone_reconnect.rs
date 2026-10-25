//! Retry admission for the phone's public away-seat read; no gameplay authority.

#[derive(Clone, Copy)]
pub(super) enum PollOutcome {
    Available,
    Refused,
    WrongRoom,
    Failed,
}

#[derive(Default)]
pub(super) struct ReclaimPoll {
    code: String,
    is_wrong_room: bool,
    failure_count: u32,
    retry_at_ms: u64,
}

impl ReclaimPoll {
    pub(super) fn wrong_room(&self, code: &str) -> bool {
        self.code == code && self.is_wrong_room
    }

    pub(super) fn ready(&mut self, code: &str, now_ms: u64) -> bool {
        if self.code != code {
            *self = Self {
                code: code.to_owned(),
                ..Self::default()
            };
        }
        !self.is_wrong_room && now_ms >= self.retry_at_ms
    }

    pub(super) fn observe(&mut self, code: &str, now_ms: u64, outcome: PollOutcome) {
        self.code = code.to_owned();
        match outcome {
            PollOutcome::WrongRoom => self.is_wrong_room = true,
            PollOutcome::Available => {
                self.failure_count = 0;
                self.retry_at_ms = now_ms.saturating_add(5_000);
            }
            PollOutcome::Refused | PollOutcome::Failed => {
                let wait_ms = (5_000_u64 << self.failure_count.min(4)).min(60_000);
                self.failure_count = self.failure_count.saturating_add(1);
                self.retry_at_ms = now_ms.saturating_add(wait_ms);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrong_room_stops_old_tabs_until_the_player_enters_a_new_code() {
        let mut poll = ReclaimPoll::default();
        assert!(poll.ready("OLD", 0));
        poll.observe("OLD", 15, PollOutcome::WrongRoom);
        assert!(poll.wrong_room("OLD"));
        for now_ms in [30, 5_000, 60_000, 3_600_000, u64::MAX] {
            assert!(!poll.ready("OLD", now_ms));
        }
        assert!(poll.ready("NEW", 3_600_001));
        assert!(!poll.wrong_room("NEW"));
    }

    #[test]
    fn transport_and_other_refusals_back_off_and_success_restores_the_poll_period() {
        let mut poll = ReclaimPoll::default();
        let mut now_ms = 0;
        for (outcome, wait_ms) in [
            (PollOutcome::Failed, 5_000),
            (PollOutcome::Refused, 10_000),
            (PollOutcome::Failed, 20_000),
            (PollOutcome::Failed, 40_000),
            (PollOutcome::Failed, 60_000),
            (PollOutcome::Failed, 60_000),
        ] {
            assert!(poll.ready("ROOM", now_ms));
            poll.observe("ROOM", now_ms, outcome);
            assert!(!poll.ready("ROOM", now_ms + wait_ms - 1));
            now_ms += wait_ms;
        }
        assert!(poll.ready("ROOM", now_ms));
        poll.observe("ROOM", now_ms, PollOutcome::Available);
        assert!(!poll.ready("ROOM", now_ms + 4_999));
        assert!(poll.ready("ROOM", now_ms + 5_000));
    }
}
