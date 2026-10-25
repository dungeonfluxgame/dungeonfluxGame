/// A stable display-text identifier independent of locale and mechanical identity.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TextKey(String);

/// Input-free facts about a rejected display-text identifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextKeyError {
    /// The input is not a lowercase ASCII dotted identifier.
    InvalidSyntax,
}

impl TextKey {
    /// Parses nonempty dot-separated segments beginning with an ASCII lowercase letter.
    ///
    /// Remaining segment bytes may be lowercase letters, digits, or underscores.
    /// Invalid input is rejected without trimming or repairing its spelling.
    pub fn parse(value: &str) -> Result<Self, TextKeyError> {
        let has_valid_syntax = !value.is_empty()
            && value.is_ascii()
            && value.split('.').all(|segment| {
                let mut bytes = segment.bytes();
                matches!(bytes.next(), Some(b'a'..=b'z'))
                    && bytes.all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_'))
            });

        if has_valid_syntax {
            Ok(Self(value.to_owned()))
        } else {
            Err(TextKeyError::InvalidSyntax)
        }
    }

    /// Returns the exact accepted identifier spelling.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
