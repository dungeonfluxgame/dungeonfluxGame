//! Validated public JSON extension shared by the server projection and the thin TV client.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum MuralState {
    Pending,
    Ready,
    Failed,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MuralView {
    pub(super) state: MuralState,
    pub(super) url: String,
    #[serde(default)]
    pub(super) key: String,
}

impl MuralView {
    pub(super) fn valid(&self) -> bool {
        let digest = |key: &str| {
            key.len() == 64
                && key
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        };
        if !self.key.is_empty() && !digest(&self.key) {
            return false;
        }
        match self.state {
            MuralState::Ready => self
                .url
                .strip_prefix("/gameplay/imagery/")
                .is_some_and(digest),
            MuralState::Pending | MuralState::Failed => self.url.is_empty(),
        }
    }

    pub(super) fn from_scene_contents(contents: &str) -> Option<Self> {
        let contents: serde_json::Value = serde_json::from_str(contents).ok()?;
        let view: Self = serde_json::from_value(contents.get("party_mural")?.clone()).ok()?;
        view.valid().then_some(view)
    }
}

/// Merges only this validated presentation member on a same-revision scene update.
/// The caller must first enforce its existing audience/scene identity checks.
pub(super) fn merge_scene_contents(shown: &mut String, new: &str) -> bool {
    let Some(mural) = MuralView::from_scene_contents(new) else {
        return false;
    };
    let mut contents = if shown.is_empty() {
        serde_json::Map::new()
    } else {
        let Ok(serde_json::Value::Object(contents)) = serde_json::from_str(shown) else {
            return false;
        };
        contents
    };
    let mural = serde_json::json!(mural);
    if contents.get("party_mural") == Some(&mural) {
        return false;
    }
    contents.insert("party_mural".into(), mural);
    *shown = serde_json::Value::Object(contents).to_string();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn party_mural_view_validates_states_and_registered_image_paths() {
        let ready = format!("/gameplay/imagery/{}", "a".repeat(64));
        for (state, url, valid) in [
            ("ready", ready.as_str(), true),
            ("pending", "", true),
            ("failed", "", true),
            ("ready", "", false),
            ("ready", "https://elsewhere/image", false),
            ("ready", "/gameplay/imagery/../../secret", false),
            ("pending", ready.as_str(), false),
            ("unknown", "", false),
        ] {
            let contents = serde_json::json!({"party_mural":{"state":state,"url":url}}).to_string();
            assert_eq!(
                MuralView::from_scene_contents(&contents).is_some(),
                valid,
                "{contents}"
            );
        }
        assert!(MuralView::from_scene_contents("{}").is_none());
        assert!(MuralView::from_scene_contents("invalid").is_none());
    }
    #[test]
    fn party_mural_same_revision_merge_preserves_canonical_contents_and_rejects_invalid_urls() {
        let mut shown = "{\"weather\":\"rain\",\"party_mural\":{\"state\":\"pending\",\"url\":\"\",\"key\":\"\"}}".to_owned();
        let new = serde_json::json!({"private_changes":"never copied","weather":"changed","party_mural":{"state":"ready","url":format!("/gameplay/imagery/{}", "b".repeat(64))}}).to_string();
        assert!(merge_scene_contents(&mut shown, &new));
        assert!(!merge_scene_contents(&mut shown, &new));
        let contents: serde_json::Value = serde_json::from_str(&shown).unwrap();
        assert_eq!(contents["weather"], "rain");
        assert!(contents.get("private_changes").is_none());
        assert_eq!(contents["party_mural"]["state"], "ready");
        let accepted = shown.clone();
        assert!(!merge_scene_contents(
            &mut shown,
            "{\"party_mural\":{\"state\":\"ready\",\"url\":\"https://untrusted/image\"}}"
        ));
        assert_eq!(shown, accepted);
    }
}
