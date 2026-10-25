//! New-password policy for email-and-password accounts (G04, coordinator decision of
//! 2026-10-24: 12 to 128 characters and a common-password check with no network).
//!
//! Pure: the caller supplies the common-password lookup; nothing here hashes, stores or logs a
//! password.

/// Fewest Unicode scalar values a new password may have.
pub const MINIMUM_PASSWORD_CHARACTERS: usize = 12;
/// Most Unicode scalar values a new password may have.
pub const MAXIMUM_PASSWORD_CHARACTERS: usize = 128;
/// Byte ceiling checked before counting characters, so a hostile input is never walked in full.
pub const MAXIMUM_PASSWORD_BYTES: usize = 512;

/// Why a new password was refused. Each variant is a field message the person can act on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PasswordRefusal {
    TooShort,
    TooLong,
    MatchesEmail,
    Common,
}

/// Checks a proposed password against the policy. `is_common` receives the password lowercased
/// (ASCII) and answers whether it appears on the caller's common-password list.
pub fn check_new_password(
    password: &str,
    email: &str,
    is_common: impl Fn(&str) -> bool,
) -> Result<(), PasswordRefusal> {
    if password.len() > MAXIMUM_PASSWORD_BYTES {
        return Err(PasswordRefusal::TooLong);
    }
    let characters = password.chars().count();
    if characters < MINIMUM_PASSWORD_CHARACTERS {
        return Err(PasswordRefusal::TooShort);
    }
    if characters > MAXIMUM_PASSWORD_CHARACTERS {
        return Err(PasswordRefusal::TooLong);
    }
    let email = email.trim();
    let local_part = email.split('@').next().unwrap_or(email);
    if password.eq_ignore_ascii_case(email) || password.eq_ignore_ascii_case(local_part) {
        return Err(PasswordRefusal::MatchesEmail);
    }
    if is_common(&password.to_ascii_lowercase()) {
        return Err(PasswordRefusal::Common);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn never_common(_: &str) -> bool {
        false
    }

    #[test]
    fn eleven_characters_are_refused_and_twelve_are_accepted() {
        assert_eq!(
            check_new_password("abcdefghijk", "a@example.test", never_common),
            Err(PasswordRefusal::TooShort)
        );
        assert_eq!(
            check_new_password("abcdefghijkl", "a@example.test", never_common),
            Ok(())
        );
    }

    #[test]
    fn one_hundred_twenty_eight_characters_are_accepted_and_one_more_is_refused() {
        // Two-byte characters: the bound counts characters, not bytes.
        let longest = "\u{e9}".repeat(MAXIMUM_PASSWORD_CHARACTERS);
        assert_eq!(
            check_new_password(&longest, "a@example.test", never_common),
            Ok(())
        );
        let too_long = "x".repeat(MAXIMUM_PASSWORD_CHARACTERS + 1);
        assert_eq!(
            check_new_password(&too_long, "a@example.test", never_common),
            Err(PasswordRefusal::TooLong)
        );
    }

    #[test]
    fn an_input_over_the_byte_ceiling_is_refused_before_counting() {
        let huge = "x".repeat(MAXIMUM_PASSWORD_BYTES + 1);
        assert_eq!(
            check_new_password(&huge, "a@example.test", |_| panic!("not consulted")),
            Err(PasswordRefusal::TooLong)
        );
    }

    #[test]
    fn a_password_equal_to_the_email_or_its_local_part_is_refused_in_any_case() {
        assert_eq!(
            check_new_password(
                "Robin.Hood@Example.test",
                "robin.hood@example.test",
                never_common
            ),
            Err(PasswordRefusal::MatchesEmail)
        );
        assert_eq!(
            check_new_password("ROBINHOODLONG", "robinhoodlong@example.test", never_common),
            Err(PasswordRefusal::MatchesEmail)
        );
    }

    #[test]
    fn a_common_password_is_refused_through_the_lowercased_lookup() {
        let lookup = |candidate: &str| candidate == "correcthorsebattery";
        assert_eq!(
            check_new_password("CorrectHorseBattery", "a@example.test", lookup),
            Err(PasswordRefusal::Common)
        );
    }
}
