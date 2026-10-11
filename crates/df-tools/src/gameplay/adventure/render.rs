use super::GenerateError;
use std::collections::BTreeMap;

pub(crate) fn render(
    pattern: &str,
    facts: &BTreeMap<String, String>,
) -> Result<String, GenerateError> {
    let mut output = String::new();
    let mut tail = pattern;
    while let Some(at) = tail.find('{') {
        output.push_str(&tail[..at]);
        let end = tail[at..]
            .find('}')
            .ok_or_else(|| GenerateError::Pattern(tail.to_owned()))?
            + at;
        let slot = &tail[at + 1..end];
        let (path, cap) = match slot.strip_suffix("|cap") {
            Some(path) => (path, true),
            None => (slot, false),
        };
        let value = facts
            .get(path)
            .ok_or_else(|| GenerateError::Pattern(path.to_owned()))?;
        if cap {
            output.push_str(&capitalize(value));
        } else {
            output.push_str(value);
        }
        tail = &tail[end + 1..];
    }
    if tail.contains('}') {
        return Err(GenerateError::Pattern(tail.to_owned()));
    }
    output.push_str(tail);
    Ok(output)
}
pub(super) fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}
