//! Compatibility bridge while the campaign PG adapter is staged outside df-persistence.

pub use df_persistence::account_closure::{
    ClosureParticipant, ParticipantCall, TimeoutPolicy, TimeoutRefusal,
};

// TODO(S1-02): remove this bridge when df-persistence exports campaign_store with its private check.
pub(crate) fn check_timeout(policy: TimeoutPolicy, text: &str) -> Result<(), TimeoutRefusal> {
    let TimeoutPolicy::AtMostMs(maximum_ms) = policy else {
        return Ok(());
    };
    let text = text.trim();
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    let (number, unit) = text.split_at(digits);
    let number: u64 = number.parse().map_err(|_| TimeoutRefusal::NotSet)?;
    let factor = match unit.trim() {
        "" | "ms" => 1,
        "s" => 1_000,
        "min" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => return Err(TimeoutRefusal::NotSet),
    };
    match number.checked_mul(factor) {
        Some(0) | None => Err(TimeoutRefusal::NotSet),
        Some(ms) if ms > maximum_ms => Err(TimeoutRefusal::TooLong),
        Some(_) => Ok(()),
    }
}
