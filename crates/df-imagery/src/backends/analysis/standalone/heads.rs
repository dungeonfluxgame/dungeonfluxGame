//! Exact legacy head locator question and plausible-box interpretation.
use super::ReadError;
use crate::request::PrivateImageText;
use crate::service::analysis_types::{AnalysisLocatedHead, AnalysisTarget, NormalizedImageBox};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
const HEAD_INSTRUCTION: &str = concat!(
    "The first picture is a film still.{second} Find the head of each of these people in the still: {people}. ",
    "Reply with only JSON {\"heads\":[{\"id\":\"name\",\"left\":0.0,\"top\":0.0,\"right\":0.0,\"bottom\":0.0}]}: the box of each head (hair to chin; for a person seen from behind, the back of the head), each number a fraction of the still's width (left, right) or height (top, bottom). ",
    "Leave out anyone who is not in the still.",
);
const MAXIMUM_DESCRIPTION_CHARACTERS: usize = 300;
const MAXIMUM_TARGETS: usize = 4;
const MAXIMUM_HEAD_WIDTH: f32 = 0.35;
const MAXIMUM_HEAD_HEIGHT: f32 = 0.5;
pub(super) fn request_body(
    picture: &[u8],
    reference: Option<&[u8]>,
    targets: &[AnalysisTarget],
    route: &crate::request::AnalysisRoute,
) -> Result<Vec<u8>, ReadError> {
    let plain_id = |id: &str| {
        (1..=16).contains(&id.len())
            && id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    };
    if picture.is_empty()
        || reference.is_some_and(|face| face.is_empty())
        || targets.is_empty()
        || targets.len() > MAXIMUM_TARGETS
        || targets.iter().any(|target| {
            !plain_id(target.id.text())
                || target.description.text().trim().is_empty()
                || target.description.text().chars().count() > MAXIMUM_DESCRIPTION_CHARACTERS
        })
    {
        return Err(ReadError::Shape);
    }
    let people: Vec<String> = targets
        .iter()
        .map(|target| format!("{}: {}", target.id.text(), target.description.text()))
        .collect();
    let second = if reference.is_some() {
        format!(
            " The second picture is the face of {}.",
            targets[0].id.text()
        )
    } else {
        String::new()
    };
    let instruction = HEAD_INSTRUCTION
        .replace("{second}", &second)
        .replace("{people}", &people.join("; "));
    let uri = |bytes: &[u8]| format!("data:image/jpeg;base64,{}", STANDARD.encode(bytes));
    let mut content = vec![
        json!({"type": "text", "text": instruction}),
        json!({"type": "image_url", "image_url": {"url": uri(picture), "detail": "low"}}),
    ];
    if let Some(face) = reference {
        content
            .push(json!({"type": "image_url", "image_url": {"url": uri(face), "detail": "low"}}));
    }
    serde_json::to_vec(&json!({
        "model": route.model.as_str(),
        "max_completion_tokens": route.maximum_output_tokens,
        "messages": [{"role": "user", "content": content}],
    }))
    .map_err(|_| ReadError::Shape)
}

/// One head box from a JSON object: numbers are fractions of the picture and the size is that of
/// a head.
fn parse_box(value: &Value) -> Option<NormalizedImageBox> {
    let side = |name: &str| -> Option<f32> {
        let number = value.get(name).and_then(Value::as_f64)?;
        (0.0..=1.0).contains(&number).then_some(number as f32)
    };
    let left = side("left")?;
    let top = side("top")?;
    let right = side("right")?;
    let bottom = side("bottom")?;
    let width = right - left;
    let height = bottom - top;
    if width > MAXIMUM_HEAD_WIDTH || height > MAXIMUM_HEAD_HEIGHT {
        return None;
    }
    NormalizedImageBox::new(left, top, right, bottom).ok()
}

/// The heads in the model's text: JSON, possibly inside a code fence. Entries for people who were
/// not asked for, repeated or implausible are dropped; at least one must remain.
pub(super) fn parse_heads(
    content: &str,
    targets: &[AnalysisTarget],
) -> Result<Vec<AnalysisLocatedHead>, ReadError> {
    let start = content.find('{').ok_or(ReadError::Shape)?;
    let end = content.rfind('}').ok_or(ReadError::Shape)?;
    let value: Value = serde_json::from_str(content.get(start..=end).ok_or(ReadError::Shape)?)
        .map_err(|_| ReadError::Shape)?;
    let mut found: Vec<AnalysisLocatedHead> = Vec::new();
    for entry in value
        .get("heads")
        .and_then(Value::as_array)
        .ok_or(ReadError::Shape)?
    {
        let Some(id) = entry.get("id").and_then(Value::as_str) else {
            continue;
        };
        if targets.iter().any(|target| target.id.text() == id)
            && !found.iter().any(|head| head.id.text() == id)
            && let Some(head) = parse_box(entry)
        {
            found.push(AnalysisLocatedHead {
                id: PrivateImageText::new(id),
                head,
            });
        }
    }
    if found.is_empty() {
        return Err(ReadError::Shape);
    }
    Ok(found)
}
