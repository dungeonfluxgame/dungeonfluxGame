//! The common-password check behind a port (coordinator decision of 2026-10-24): a short
//! hand-curated built-in list now; a full breached-password list is an owner-approved follow-up
//! that implements the same port.

/// Answers whether a lowercased candidate is a known common password.
pub trait CommonPasswords: Send + Sync + 'static {
    fn contains(&self, lowercased: &str) -> bool;
}

/// Widely published common passwords of at least twelve characters, lowercase, sorted (a test
/// keeps it sorted so the binary search stays correct).
const BUILT_IN: &[&str] = &[
    "!qaz2wsx3edc",
    "000000000000",
    "111111111111",
    "1111111111111",
    "123123123123",
    "123456789012",
    "1234567890123",
    "123456789abc",
    "123456789qwe",
    "123qweasdzxc",
    "1q2w3e4r5t6y",
    "1qaz2wsx3edc",
    "1qazxsw23edc",
    "aaaaaaaaaaaa",
    "abc123456789",
    "abcd12345678",
    "abcdefghijkl",
    "administrator",
    "asdfghjkl123",
    "asdfghjklzxcvbnm",
    "changemenow1",
    "correcthorsebatterystaple",
    "dungeonflux123",
    "dungeonsanddragons",
    "football1234",
    "iloveyou1234",
    "letmein12345",
    "letmeinplease",
    "masterpassword",
    "monkey123456",
    "mypassword123",
    "p@ssw0rd1234",
    "passw0rd1234",
    "password0000",
    "password1111",
    "password123!",
    "password1234",
    "password12345",
    "password123456",
    "passwordpassword",
    "princess1234",
    "q1w2e3r4t5y6",
    "qazwsxedcrfv",
    "qwerty123456",
    "qwertyuiop12",
    "qwertyuiop123",
    "qwertyuiopasdf",
    "starwars1234",
    "sunshine1234",
    "trustno11234",
    "welcome12345",
    "zaq12wsxcde3",
    "zxcvbnm12345",
];

pub struct BuiltInCommonPasswords;

impl CommonPasswords for BuiltInCommonPasswords {
    fn contains(&self, lowercased: &str) -> bool {
        BUILT_IN.binary_search(&lowercased).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_list_is_sorted_lowercase_and_at_least_twelve_characters() {
        assert!(BUILT_IN.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(
            BUILT_IN
                .iter()
                .all(|entry| entry.chars().count() >= 12 && entry.to_ascii_lowercase() == *entry)
        );
    }

    #[test]
    fn a_listed_password_is_found_and_an_unlisted_one_is_not() {
        assert!(BuiltInCommonPasswords.contains("password1234"));
        assert!(!BuiltInCommonPasswords.contains("lantern under the old mill"));
    }
}
