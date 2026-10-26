//! Bounded, versioned phone draft codec. Pure so native tests cover corrupt browser data.
use df_protocol::common as rpc;
use prost::Message;
use serde::{Deserialize, Serialize};

#[cfg_attr(
    not(target_arch = "wasm32"),
    allow(dead_code, reason = "read by the browser storage adapter on wasm32")
)]
pub(super) const DRAFT_KEY: &str = "df.creator.draft.v1";
#[cfg_attr(
    not(target_arch = "wasm32"),
    allow(dead_code, reason = "read by the browser storage adapter on wasm32")
)]
pub(super) const LIBRARY_KEY: &str = "df.library.key.v1";
pub(super) const ORIGIN_KEY: &str = "df.library.origin.v1";
const MAX_DRAFT_BYTES: usize = 96 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LoadedHero {
    pub(super) hero_id: i64,
    pub(super) revision: u64,
}
impl LoadedHero {
    fn valid(&self) -> bool {
        self.hero_id > 0 && self.revision > 0 && self.revision <= i64::MAX as u64
    }
    pub(super) fn to_rpc(&self) -> rpc::LibraryHeroReference {
        rpc::LibraryHeroReference {
            hero_id: self.hero_id,
            revision: self.revision,
        }
    }
}

// Survives accepted-draft clearing and a failed save/reload; scoped to the accepted seat.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SeatOrigin {
    pub(super) credential: String,
    pub(super) loaded_hero: Option<LoadedHero>,
}
impl SeatOrigin {
    pub(super) fn encode(&self) -> Option<String> {
        serde_json::to_string(self)
            .ok()
            .filter(|text| text.len() <= 1024)
    }
    pub(super) fn decode(text: &str, credential: &str) -> Option<Self> {
        if text.len() > 1024 {
            return None;
        }
        let origin: Self = serde_json::from_str(text).ok()?;
        (origin.credential == credential
            && origin.loaded_hero.as_ref().is_none_or(LoadedHero::valid))
        .then_some(origin)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredDraft {
    pub(super) version: u8,
    pub(super) selection: Vec<u8>,
    pub(super) appearance_text: String,
    pub(super) backstory_text: String,
    pub(super) facts: Vec<String>,
    pub(super) story_menus: Vec<(String, String)>,
    pub(super) scenery_option: String,
    pub(super) scenery_custom: String,
    pub(super) paragraph_edited: bool,
    pub(super) text_reviewed: bool,
    pub(super) portrait_uri: String,
    pub(super) portrait_revision: Option<u64>,
    pub(super) look_revision: u64,
    #[serde(default)]
    pub(super) loaded_hero: Option<LoadedHero>,
}
impl StoredDraft {
    pub(super) fn encode(&self) -> Option<String> {
        if !self.valid() {
            return None;
        }
        let text = serde_json::to_string(self).ok()?;
        (text.len() <= MAX_DRAFT_BYTES).then_some(text)
    }
    pub(super) fn decode(text: &str) -> Option<Self> {
        if text.len() > MAX_DRAFT_BYTES {
            return None;
        }
        let draft: Self = serde_json::from_str(text).ok()?;
        draft.valid().then_some(draft)
    }
    pub(super) fn selection(&self) -> Option<rpc::CharacterSelection> {
        rpc::CharacterSelection::decode(self.selection.as_slice()).ok()
    }
    fn valid(&self) -> bool {
        self.version == 1
            && self.loaded_hero.as_ref().is_none_or(LoadedHero::valid)
            && self.selection.len() <= 24 * 1024
            && self.selection().is_some()
            && self.appearance_text.len() <= 4000
            && self.backstory_text.len() <= 16000
            && self.facts.len() <= 6
            && self.facts.iter().all(|fact| fact.len() <= 640)
            && self.story_menus.len() <= 16
            && self
                .story_menus
                .iter()
                .all(|(key, value)| key.len() <= 64 && value.len() <= 64)
            && self.scenery_option.len() <= 64
            && self.scenery_custom.len() <= 1536
            && (self.portrait_uri.is_empty()
                || ["/gameplay/creator-portrait/", "/gameplay/library-portrait/"]
                    .iter()
                    .any(|prefix| {
                        self.portrait_uri.strip_prefix(prefix).is_some_and(|key| {
                            key.len() == 64 && key.bytes().all(|byte| byte.is_ascii_hexdigit())
                        })
                    }))
    }
}
pub(super) fn decode_library_key(text: &str) -> Option<Vec<u8>> {
    if text.len() != 64
        || !text
            .bytes()
            .all(|byte| matches!(byte,b'0'..=b'9'|b'a'..=b'f'))
    {
        return None;
    }
    text.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    fn draft() -> StoredDraft {
        let text = "  \u{96ea}\u{306e}\u{808c} \u{1f409}\r\nSecond line\n  ";
        StoredDraft {
            version: 1,
            selection: rpc::CharacterSelection {
                name: "Hero".into(),
                identity: Some(rpc::HeroIdentity {
                    appearance: Some(rpc::HeroAppearance {
                        public_description: text.into(),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }
            .encode_to_vec(),
            appearance_text: text.into(),
            backstory_text: "Story\nwith lines".into(),
            facts: vec!["".into(), "fact".into()],
            story_menus: vec![],
            scenery_option: "custom".into(),
            scenery_custom: text.into(),
            paragraph_edited: true,
            text_reviewed: false,
            portrait_uri: String::new(),
            portrait_revision: Some(4),
            look_revision: 5,
            loaded_hero: Some(LoadedHero {
                hero_id: 17,
                revision: 3,
            }),
        }
    }
    #[test]
    fn phone_draft_round_trips_exact_unicode_newlines_and_controls() {
        let draft = draft();
        let restored = StoredDraft::decode(&draft.encode().unwrap()).unwrap();
        assert_eq!(restored, draft);
        assert_eq!(
            restored.loaded_hero.as_ref().unwrap().to_rpc(),
            rpc::LibraryHeroReference {
                hero_id: 17,
                revision: 3
            }
        );
        assert_eq!(
            restored
                .selection()
                .unwrap()
                .identity
                .unwrap()
                .appearance
                .unwrap()
                .public_description,
            draft.appearance_text
        );
    }
    #[test]
    fn loaded_draft_origin_survives_accept_and_reload_but_is_scoped_to_its_seat() {
        let draft = draft();
        let restored = StoredDraft::decode(&draft.encode().unwrap()).unwrap();
        let origin = SeatOrigin {
            credential: "seat-a".into(),
            loaded_hero: restored.loaded_hero,
        };
        let text = origin.encode().unwrap();
        let reloaded = SeatOrigin::decode(&text, "seat-a").unwrap();
        let request = rpc::SaveMySeatHeroRequest {
            key: vec![7; 32],
            scope: None,
            loaded_hero: reloaded.loaded_hero.as_ref().map(LoadedHero::to_rpc),
        };
        assert_eq!(
            rpc::SaveMySeatHeroRequest::decode(request.encode_to_vec().as_slice())
                .unwrap()
                .loaded_hero
                .unwrap()
                .revision,
            3
        );
        assert!(SeatOrigin::decode(&text, "other-seat").is_none());
        assert!(SeatOrigin::decode("{", "seat-a").is_none());
        assert_eq!(ORIGIN_KEY, "df.library.origin.v1");
        let mut invalid = draft;
        invalid.loaded_hero.as_mut().unwrap().revision = 0;
        assert!(invalid.encode().is_none());
        let mut legacy = serde_json::to_value(&invalid).unwrap();
        legacy.as_object_mut().unwrap().remove("loaded_hero");
        assert!(
            StoredDraft::decode(&legacy.to_string())
                .unwrap()
                .loaded_hero
                .is_none()
        );
    }
    #[test]
    fn corrupt_old_oversized_and_external_portraits_are_discarded() {
        for text in ["", "{", "null", "{}", "{\"version\":0}"] {
            assert!(StoredDraft::decode(text).is_none());
        }
        let mut draft = draft();
        draft.version = 0;
        assert!(draft.encode().is_none());
        draft.version = 1;
        draft.portrait_uri = "https://example.org/private".into();
        assert!(draft.encode().is_none());
        assert!(StoredDraft::decode(&"x".repeat(MAX_DRAFT_BYTES + 1)).is_none());
        assert!(decode_library_key(&"g".repeat(64)).is_none());
        assert_eq!(
            decode_library_key(&"ab".repeat(32)).unwrap(),
            vec![0xab; 32]
        );
    }
}
