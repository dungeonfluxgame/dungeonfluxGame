//! Escape-by-default markup (security review, 2026-10-24). Pages are built only with `markup!`,
//! whose parts are string literals (trusted markup written in this crate), `text(..)` (escaped),
//! numbers, or other `Markup`. A runtime string cannot be inserted raw: `&str` with a non-static
//! lifetime and `String` do not implement `Fragment`, so forgetting `text(..)` does not compile.

use std::fmt::Write as _;

/// Markup built from trusted literals and escaped values.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Markup(String);

impl Markup {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }

    /// Joins already-built pieces.
    pub fn concat(parts: impl IntoIterator<Item = Markup>) -> Self {
        Self(parts.into_iter().map(|part| part.0).collect())
    }

    /// Only `markup!` calls this, with a string it built from fragments.
    pub(crate) fn from_parts(built: String) -> Self {
        Self(built)
    }
}

/// One piece of a `markup!` call.
pub trait Fragment {
    fn append_to(self, out: &mut String);
}

/// A string literal is markup written by this crate.
impl Fragment for &'static str {
    fn append_to(self, out: &mut String) {
        out.push_str(self);
    }
}

impl Fragment for Markup {
    fn append_to(self, out: &mut String) {
        out.push_str(&self.0);
    }
}

impl Fragment for &Markup {
    fn append_to(self, out: &mut String) {
        out.push_str(&self.0);
    }
}

impl Fragment for u64 {
    fn append_to(self, out: &mut String) {
        // Writing to a String cannot fail.
        let _infallible = write!(out, "{self}");
    }
}

/// A value to escape for element content or a double-quoted attribute.
pub struct Text<'a>(&'a str);

pub fn text(value: &str) -> Text<'_> {
    Text(value)
}

impl Fragment for Text<'_> {
    fn append_to(self, out: &mut String) {
        for character in self.0.chars() {
            match character {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' => out.push_str("&quot;"),
                '\'' => out.push_str("&#39;"),
                other => out.push(other),
            }
        }
    }
}

/// Builds `Markup` from literals, `text(..)`, numbers and other `Markup`.
macro_rules! markup {
    ($($part:expr),* $(,)?) => {{
        let mut built = ::std::string::String::new();
        $( $crate::html::Fragment::append_to($part, &mut built); )*
        $crate::html::Markup::from_parts(built)
    }};
}

pub(crate) use markup;

/// Lowercase hex of opaque ids carried in forms (session ids, operation ids).
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_becomes_entities_and_literals_stay_markup() {
        let built = markup!(
            "<p title=\"",
            text("\"x'"),
            "\">",
            text(r#"<script>alert("x") & 'y'</script>"#),
            "</p>"
        );
        assert_eq!(
            built.as_str(),
            "<p title=\"&quot;x&#39;\">&lt;script&gt;alert(&quot;x&quot;) &amp; \
             &#39;y&#39;&lt;/script&gt;</p>"
        );
    }
}
