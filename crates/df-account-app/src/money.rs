//! Money, durations and dates as people read them. Integer arithmetic only: micro-dollars and
//! cents never pass through a float.
//!
//! Balances round down to the cent (never show more than is there, plan 6.4); charges round up
//! to the cent (never show less than was spent); the exact micro-dollars stay on the server.

pub const MICRO_DOLLARS_PER_CENT: u64 = 10_000;

/// A balance or credit, rounded down to the cent: `$12.34`.
pub fn balance(micro_dollars: u64) -> String {
    cents(micro_dollars / MICRO_DOLLARS_PER_CENT)
}

/// A charge, rounded up to the cent: `$0.01` for any nonzero sub-cent charge.
pub fn charge(micro_dollars: u64) -> String {
    cents(micro_dollars.div_ceil(MICRO_DOLLARS_PER_CENT))
}

/// Whole cents: `$1,234.05`.
pub fn cents(cents: u64) -> String {
    let dollars = cents / 100;
    let digits = dollars.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    format!("${grouped}.{:02}", cents % 100)
}

/// A whole-dollar amount when it is one (`$20`), else with cents (`$7.50`): for offers and prices.
pub fn price(cents_value: u32) -> String {
    let cents_value = u64::from(cents_value);
    if cents_value % 100 == 0 {
        format!("${}", cents_value / 100)
    } else {
        cents(cents_value)
    }
}

/// Why a typed dollar amount was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AmountRefusal {
    Empty,
    NotANumber,
    TooManyCents,
    TooLarge,
}

/// Parses `25`, `25.5`, `25.50` or `$25` into cents.
pub fn parse_dollars(text: &str) -> Result<u32, AmountRefusal> {
    let text = text.trim();
    let text = text.strip_prefix('$').unwrap_or(text).trim();
    if text.is_empty() {
        return Err(AmountRefusal::Empty);
    }
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    let is_digits = |part: &str| part.bytes().all(|byte| byte.is_ascii_digit());
    if whole.is_empty() || !is_digits(whole) || !is_digits(fraction) {
        return Err(AmountRefusal::NotANumber);
    }
    if fraction.len() > 2 {
        return Err(AmountRefusal::TooManyCents);
    }
    let whole: u32 = whole.parse().map_err(|_| AmountRefusal::TooLarge)?;
    let fraction_cents = match fraction.len() {
        0 => 0,
        1 => {
            fraction
                .parse::<u32>()
                .map_err(|_| AmountRefusal::NotANumber)?
                * 10
        }
        _ => fraction
            .parse::<u32>()
            .map_err(|_| AmountRefusal::NotANumber)?,
    };
    whole
        .checked_mul(100)
        .and_then(|cents_value| cents_value.checked_add(fraction_cents))
        .ok_or(AmountRefusal::TooLarge)
}

/// `1h 02m`, `42m 18s` or `9s`.
pub fn duration(seconds: u64) -> String {
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `24 Oct 2026, 21:05` in the viewer's time zone, given its offset east of UTC in minutes.
pub fn date_time(at_ms: u64, offset_east_minutes: i32) -> String {
    let (year, month, day, minute_of_day) = civil(at_ms, offset_east_minutes);
    format!(
        "{day} {} {year}, {:02}:{:02}",
        MONTHS[month - 1],
        minute_of_day / 60,
        minute_of_day % 60
    )
}

/// `24 Oct 2026` in the viewer's time zone.
pub fn date(at_ms: u64, offset_east_minutes: i32) -> String {
    let (year, month, day, _) = civil(at_ms, offset_east_minutes);
    format!("{day} {} {year}", MONTHS[month - 1])
}

/// (year, month 1..=12, day, minute of day) of an instant shifted by the offset.
fn civil(at_ms: u64, offset_east_minutes: i32) -> (i64, usize, u32, u32) {
    let local_minutes =
        i64::try_from(at_ms / 60_000).unwrap_or(i64::MAX / 2) + i64::from(offset_east_minutes);
    let days = local_minutes.div_euclid(1440);
    let minute_of_day = u32::try_from(local_minutes.rem_euclid(1440)).unwrap_or(0);
    // Days to civil date (H. Hinnant, "chrono-Compatible Low-Level Date Algorithms").
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = u32::try_from(day_of_year - (153 * month_index + 2) / 5 + 1).unwrap_or(1);
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (
        year,
        usize::try_from(month).unwrap_or(1),
        day,
        minute_of_day,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balances_round_down_and_charges_round_up_to_the_cent() {
        assert_eq!(balance(10_000_000), "$10.00");
        assert_eq!(balance(12_349_999), "$12.34");
        assert_eq!(charge(12_340_001), "$12.35");
        assert_eq!(charge(1), "$0.01");
        assert_eq!(charge(0), "$0.00");
    }

    #[test]
    fn large_amounts_are_grouped_by_thousands() {
        assert_eq!(cents(123_456_789), "$1,234,567.89");
        assert_eq!(cents(100_000), "$1,000.00");
        assert_eq!(cents(99_999), "$999.99");
    }

    #[test]
    fn prices_drop_zero_cents() {
        assert_eq!(price(300), "$3");
        assert_eq!(price(750), "$7.50");
    }

    #[test]
    fn typed_dollars_parse_to_cents_and_refuse_anything_else() {
        assert_eq!(parse_dollars("25"), Ok(2500));
        assert_eq!(parse_dollars(" $25.5 "), Ok(2550));
        assert_eq!(parse_dollars("0.05"), Ok(5));
        assert_eq!(parse_dollars(""), Err(AmountRefusal::Empty));
        assert_eq!(parse_dollars("1.234"), Err(AmountRefusal::TooManyCents));
        assert_eq!(parse_dollars("-3"), Err(AmountRefusal::NotANumber));
        assert_eq!(parse_dollars("1e3"), Err(AmountRefusal::NotANumber));
        assert_eq!(parse_dollars("99999999999"), Err(AmountRefusal::TooLarge));
    }

    #[test]
    fn durations_read_in_the_largest_two_units() {
        assert_eq!(duration(2538), "42m 18s");
        assert_eq!(duration(3720), "1h 02m");
        assert_eq!(duration(9), "9s");
    }

    #[test]
    fn dates_follow_the_viewer_offset_across_midnight() {
        // 2026-10-25T01:05:00Z
        let at_ms = 1_792_890_300_000;
        assert_eq!(date_time(at_ms, 0), "25 Oct 2026, 01:05");
        assert_eq!(date_time(at_ms, -240), "24 Oct 2026, 21:05");
        assert_eq!(date(0, 0), "1 Jan 1970");
        assert_eq!(date(951_782_400_000, 0), "29 Feb 2000");
    }
}
