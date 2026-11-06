//! The pieces of an audit line that more than one kind of change writes. An audit line carries
//! names, numbers and lengths, never the text a person typed.

/// Adds `key=set` when a change gives `value`.
pub(crate) fn present<T>(
    pairs: &mut Vec<(&'static str, String)>,
    key: &'static str,
    value: Option<&T>,
) {
    if value.is_some() {
        pairs.push((key, "set".to_owned()));
    }
}
