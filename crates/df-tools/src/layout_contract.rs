//! Closed development layout contract shared by the server, CLI and WASM renderer.
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

pub(crate) const MAX_ELEMENTS: usize = 512;
pub(crate) const MAX_OVERRIDES: usize = 128;
pub(crate) const MAX_READBACK_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const MAX_PAGE_BYTES: usize = 48 * 1024;
pub(crate) const MAX_PAGES: usize = 128;
pub(crate) const MAX_DOCUMENT_BYTES: usize = 64 * 1024;
const MAX_COORDINATE: f64 = 4096.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LayoutError {
    Shape,
    Bounds,
    Id,
    Limit,
    Revision,
    #[cfg(any(not(target_arch = "wasm32"), test))]
    UnknownId,
    #[cfg(any(not(target_arch = "wasm32"), test))]
    Unavailable,
}
impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Shape => "invalid layout fields or JSON",
            Self::Bounds => "layout geometry is outside its finite bounds",
            Self::Id => "invalid layout ID",
            Self::Limit => "layout capacity exceeded",
            Self::Revision => "stale or future layout revision",
            #[cfg(any(not(target_arch = "wasm32"), test))]
            Self::UnknownId => "unknown or stale layout ID; refresh layout list",
            #[cfg(any(not(target_arch = "wasm32"), test))]
            Self::Unavailable => "layout controls unavailable",
        })
    }
}
impl std::error::Error for LayoutError {}

pub(crate) fn validate_id(id: &str) -> Result<(), LayoutError> {
    if id.is_empty()
        || id.len() > 96
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b':'))
    {
        return Err(LayoutError::Id);
    }
    Ok(())
}
fn bounded(n: f64, limit: f64) -> Result<(), LayoutError> {
    if n.is_finite() && n.abs() <= limit {
        Ok(())
    } else {
        Err(LayoutError::Bounds)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Unit {
    #[default]
    Px,
    Percent,
}
impl Unit {
    #[cfg(any(target_arch = "wasm32", test))]
    pub(crate) fn suffix(self) -> &'static str {
        match self {
            Self::Px => "px",
            Self::Percent => "%",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Distance {
    pub(crate) value: f64,
    pub(crate) unit: Unit,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Transform {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) translate_x: Option<Distance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) translate_y: Option<Distance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) scale_x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) scale_y: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) rotate: Option<f64>,
}
impl Transform {
    pub(crate) fn validate(&self) -> Result<(), LayoutError> {
        for d in [self.translate_x, self.translate_y].into_iter().flatten() {
            bounded(d.value, MAX_COORDINATE)?;
        }
        for s in [self.scale_x, self.scale_y].into_iter().flatten() {
            bounded(s, 8.0)?;
        }
        if let Some(r) = self.rotate {
            bounded(r, 360.0)?;
        }
        Ok(())
    }
    #[cfg(any(not(target_arch = "wasm32"), test))]
    pub(crate) fn merge(&mut self, patch: Self) {
        if patch.translate_x.is_some() {
            self.translate_x = patch.translate_x;
        }
        if patch.translate_y.is_some() {
            self.translate_y = patch.translate_y;
        }
        if patch.scale_x.is_some() {
            self.scale_x = patch.scale_x;
        }
        if patch.scale_y.is_some() {
            self.scale_y = patch.scale_y;
        }
        if patch.rotate.is_some() {
            self.rotate = patch.rotate;
        }
    }
    /// Only independent transform properties are owned. Removing this rule exposes the
    /// application's CURRENT CSS and inline styles, including animation transforms.
    #[cfg(any(target_arch = "wasm32", test))]
    pub(crate) fn rule(&self, id: &str) -> Result<String, LayoutError> {
        validate_id(id)?;
        self.validate()?;
        let mut declarations = String::new();
        if self.translate_x.is_some() || self.translate_y.is_some() {
            let x = self.translate_x.unwrap_or(Distance {
                value: 0.0,
                unit: Unit::Px,
            });
            let y = self.translate_y.unwrap_or(Distance {
                value: 0.0,
                unit: Unit::Px,
            });
            declarations.push_str(&format!(
                "translate:{}{} {}{}!important;",
                x.value,
                x.unit.suffix(),
                y.value,
                y.unit.suffix()
            ));
        }
        if self.scale_x.is_some() || self.scale_y.is_some() {
            declarations.push_str(&format!(
                "scale:{} {}!important;",
                self.scale_x.unwrap_or(1.0),
                self.scale_y.unwrap_or(1.0)
            ));
        }
        if let Some(r) = self.rotate {
            declarations.push_str(&format!("rotate:{r}deg!important;"));
        }
        Ok(format!("[data-layout-id=\"{id}\"]{{{declarations}}}"))
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LayoutDocument {
    pub(crate) revision: u64,
    pub(crate) overrides: BTreeMap<String, Transform>,
    #[serde(default)]
    pub(crate) visibility: BTreeMap<String, ElementVisibility>,
}
impl LayoutDocument {
    #[cfg(any(target_arch = "wasm32", test))]
    pub(crate) fn parse(text: &str) -> Result<Self, LayoutError> {
        if text.len() > MAX_DOCUMENT_BYTES {
            return Err(LayoutError::Limit);
        }
        let document: Self = serde_json::from_str(text).map_err(|_| LayoutError::Shape)?;
        if document.overrides.len() > MAX_OVERRIDES {
            return Err(LayoutError::Limit);
        }
        if document.visibility.len() > MAX_OVERRIDES {
            return Err(LayoutError::Limit);
        }
        for (id, t) in &document.overrides {
            validate_id(id)?;
            t.validate()?;
        }
        for id in document.visibility.keys() {
            validate_id(id)?;
        }
        Ok(document)
    }
    #[cfg(any(target_arch = "wasm32", test))]
    pub(crate) fn stylesheet(&self) -> Result<String, LayoutError> {
        let transforms = self.overrides.iter().map(|(id, t)| t.rule(id));
        let visibility = self
            .visibility
            .iter()
            .map(|(id, visibility)| visibility.rule(id));
        transforms.chain(visibility).collect()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ElementVisibility {
    #[default]
    Visible,
    Hidden,
}
impl ElementVisibility {
    #[cfg(any(target_arch = "wasm32", test))]
    pub(crate) fn rule(self, id: &str) -> Result<String, LayoutError> {
        validate_id(id)?;
        Ok(match self {
            Self::Visible => String::new(),
            Self::Hidden => format!(
                "[data-layout-id=\"{id}\"]{{display:none!important;visibility:hidden!important;pointer-events:none!important;}}"
            ),
        })
    }
}

#[cfg(any(not(target_arch = "wasm32"), test))]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransformRequest {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) unit: Unit,
    pub(crate) translate_x: Option<f64>,
    pub(crate) translate_y: Option<f64>,
    pub(crate) scale: Option<f64>,
    pub(crate) scale_x: Option<f64>,
    pub(crate) scale_y: Option<f64>,
    pub(crate) rotate: Option<f64>,
}
#[cfg(any(not(target_arch = "wasm32"), test))]
impl TransformRequest {
    pub(crate) fn patch(&self) -> Result<Transform, LayoutError> {
        validate_id(&self.id)?;
        if self.scale.is_some() && (self.scale_x.is_some() || self.scale_y.is_some()) {
            return Err(LayoutError::Shape);
        }
        let p = Transform {
            translate_x: self.translate_x.map(|value| Distance {
                value,
                unit: self.unit,
            }),
            translate_y: self.translate_y.map(|value| Distance {
                value,
                unit: self.unit,
            }),
            scale_x: self.scale_x.or(self.scale),
            scale_y: self.scale_y.or(self.scale),
            rotate: self.rotate,
        };
        if p == Transform::default() {
            return Err(LayoutError::Shape);
        }
        p.validate()?;
        Ok(p)
    }
}
#[cfg(any(not(target_arch = "wasm32"), test))]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MoveRequest {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) unit: Unit,
    pub(crate) x: Option<f64>,
    pub(crate) y: Option<f64>,
}
#[cfg(any(not(target_arch = "wasm32"), test))]
impl MoveRequest {
    pub(crate) fn patch(&self) -> Result<Transform, LayoutError> {
        validate_id(&self.id)?;
        if self.x.is_none() && self.y.is_none() {
            return Err(LayoutError::Shape);
        }
        let p = Transform {
            translate_x: self.x.map(|value| Distance {
                value,
                unit: self.unit,
            }),
            translate_y: self.y.map(|value| Distance {
                value,
                unit: self.unit,
            }),
            ..Default::default()
        };
        p.validate()?;
        Ok(p)
    }
}
#[cfg(any(not(target_arch = "wasm32"), test))]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResetRequest {
    pub(crate) id: Option<String>,
}
#[cfg(any(not(target_arch = "wasm32"), test))]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VisibilityRequest {
    pub(crate) id: String,
    pub(crate) visible: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Viewport {
    pub(crate) width: f64,
    pub(crate) height: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Bounds {
    pub(crate) left: f64,
    pub(crate) top: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EffectiveTransform {
    pub(crate) translate: String,
    pub(crate) scale: String,
    pub(crate) rotate: String,
    pub(crate) matrix: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ElementStatus {
    Applied,
    Absent,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VisualOwner {
    pub(crate) id: String,
    pub(crate) name: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ElementReadback {
    pub(crate) id: String,
    pub(crate) tag: String,
    pub(crate) classes: String,
    pub(crate) owner: Option<VisualOwner>,
    pub(crate) configured: Option<Transform>,
    pub(crate) effective: Option<EffectiveTransform>,
    pub(crate) rect: Option<Bounds>,
    pub(crate) status: ElementStatus,
    #[serde(default)]
    pub(crate) visibility: ElementVisibility,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LayoutReadback {
    pub(crate) sample: u64,
    pub(crate) revision: u64,
    pub(crate) viewport: Viewport,
    pub(crate) elements: Vec<ElementReadback>,
    pub(crate) overflow: usize,
}
impl LayoutReadback {
    /// Split by actual UTF-8 JSON bytes, leaving 1 KiB for page metadata. Each page is
    /// comfortably below the unchanged 64 KiB RPC message limit, even after protobuf framing.
    #[cfg(any(target_arch = "wasm32", test))]
    pub(crate) fn pages(&self) -> Result<Vec<LayoutPage>, LayoutError> {
        self.validate()?;
        if serde_json::to_vec(self)
            .map_err(|_| LayoutError::Shape)?
            .len()
            > MAX_READBACK_BYTES
        {
            return Err(LayoutError::Limit);
        }
        let mut groups = Vec::new();
        let mut current = Vec::new();
        let mut bytes = 0;
        for element in &self.elements {
            let size = serde_json::to_vec(element)
                .map_err(|_| LayoutError::Shape)?
                .len()
                + 1;
            if size > MAX_PAGE_BYTES - 1024 {
                return Err(LayoutError::Limit);
            }
            if bytes + size > MAX_PAGE_BYTES - 1024 && !current.is_empty() {
                groups.push(current);
                current = Vec::new();
                bytes = 0;
            }
            current.push(element.clone());
            bytes += size;
        }
        if !current.is_empty() || groups.is_empty() {
            groups.push(current);
        }
        if groups.len() > MAX_PAGES {
            return Err(LayoutError::Limit);
        }
        let pages = groups.len();
        groups
            .into_iter()
            .enumerate()
            .map(|(page, elements)| {
                let part = LayoutPage {
                    sample: self.sample,
                    revision: self.revision,
                    page,
                    pages,
                    viewport: self.viewport.clone(),
                    overflow: self.overflow,
                    elements,
                };
                if serde_json::to_vec(&part)
                    .map_err(|_| LayoutError::Shape)?
                    .len()
                    > MAX_PAGE_BYTES
                {
                    return Err(LayoutError::Limit);
                }
                Ok(part)
            })
            .collect()
    }
    pub(crate) fn validate(&self) -> Result<(), LayoutError> {
        if self.sample == 0 {
            return Err(LayoutError::Revision);
        }
        bounded(self.viewport.width, 32768.0)?;
        bounded(self.viewport.height, 32768.0)?;
        if self.viewport.width <= 0.0 || self.viewport.height <= 0.0 {
            return Err(LayoutError::Bounds);
        }
        if self.elements.len() > MAX_ELEMENTS + MAX_OVERRIDES || self.overflow > 1_000_000 {
            return Err(LayoutError::Limit);
        }
        let mut ids = BTreeSet::new();
        let mut present = 0;
        for e in &self.elements {
            validate_id(&e.id)?;
            if !ids.insert(&e.id) {
                return Err(LayoutError::Id);
            }
            if e.tag.is_empty()
                || e.tag.len() > 32
                || !e
                    .tag
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            {
                return Err(LayoutError::Shape);
            }
            if e.classes.len() > 192
                || !e
                    .classes
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b' '))
            {
                return Err(LayoutError::Shape);
            }
            if let Some(owner) = &e.owner {
                for text in std::iter::once(&owner.id).chain(owner.name.iter()) {
                    if text.is_empty() || text.len() > 128 || text.chars().any(char::is_control) {
                        return Err(LayoutError::Shape);
                    }
                }
            }
            if let Some(t) = &e.configured {
                t.validate()?;
            }
            match (e.status, &e.rect, &e.effective) {
                (ElementStatus::Applied, Some(r), Some(t)) => {
                    present += 1;
                    for n in [r.left, r.top, r.width, r.height] {
                        bounded(n, 1_000_000.0)?;
                    }
                    if r.width < 0.0 || r.height < 0.0 {
                        return Err(LayoutError::Bounds);
                    }
                    if [&t.translate, &t.scale, &t.rotate, &t.matrix]
                        .iter()
                        .any(|s| s.len() > 512 || s.chars().any(char::is_control))
                    {
                        return Err(LayoutError::Shape);
                    }
                }
                (ElementStatus::Absent, None, None) => {}
                _ => return Err(LayoutError::Shape),
            }
        }
        if present > MAX_ELEMENTS {
            return Err(LayoutError::Limit);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LayoutPage {
    pub(crate) sample: u64,
    pub(crate) revision: u64,
    pub(crate) page: usize,
    pub(crate) pages: usize,
    pub(crate) viewport: Viewport,
    pub(crate) overflow: usize,
    pub(crate) elements: Vec<ElementReadback>,
}
impl LayoutPage {
    #[cfg(any(not(target_arch = "wasm32"), test))]
    pub(crate) fn parse(text: &str) -> Result<Self, LayoutError> {
        if text.len() > MAX_PAGE_BYTES {
            return Err(LayoutError::Limit);
        }
        let page: Self = serde_json::from_str(text).map_err(|_| LayoutError::Shape)?;
        if page.pages == 0 || page.pages > MAX_PAGES || page.page >= page.pages {
            return Err(LayoutError::Shape);
        }
        LayoutReadback {
            sample: page.sample,
            revision: page.revision,
            viewport: page.viewport.clone(),
            elements: page.elements.clone(),
            overflow: page.overflow,
        }
        .validate()?;
        Ok(page)
    }
}

/// Equal gameplay revisions may carry newer layout metadata, independently of scene timing.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) fn merge_layout(
    shown: &mut df_protocol::common::ViewMessage,
    new: &df_protocol::common::ViewMessage,
) -> bool {
    use df_protocol::common::view_message::Audience;
    let (Some(Audience::Display(shown)), Some(Audience::Display(new))) =
        (shown.audience.as_mut(), new.audience.as_ref())
    else {
        return false;
    };
    if new.layout_revision < shown.layout_revision
        || new.layout_json == shown.layout_json
        || new.layout_json.is_empty()
    {
        return false;
    }
    if new.layout_revision == shown.layout_revision && !shown.layout_json.is_empty() {
        return false;
    }
    let Ok(document) = LayoutDocument::parse(&new.layout_json) else {
        return false;
    };
    if document.revision != new.layout_revision {
        return false;
    }
    shown.layout_revision = new.layout_revision;
    shown.layout_json.clone_from(&new.layout_json);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn geometry_and_unknown_fields_are_closed() {
        for text in [
            r#"{"id":"dm-overlay","rotate":361}"#,
            r#"{"id":"dm-overlay","scale":9}"#,
            r#"{"id":"dm-overlay","translate_x":"NaN"}"#,
            r#"{"id":"dm-overlay","css":"display:none"}"#,
        ] {
            let parsed = serde_json::from_str::<TransformRequest>(text);
            assert!(parsed.is_err() || parsed.unwrap().patch().is_err());
        }
        assert!(
            Transform {
                rotate: Some(f64::NAN),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
    #[test]
    fn move_and_partial_transform_compose_without_game_css() {
        let mut t = Transform {
            rotate: Some(30.0),
            scale_x: Some(2.0),
            ..Default::default()
        };
        t.merge(
            MoveRequest {
                id: "dm-overlay".into(),
                unit: Unit::Percent,
                x: Some(20.0),
                y: None,
            }
            .patch()
            .unwrap(),
        );
        t.merge(Transform {
            translate_y: Some(Distance {
                value: 10.0,
                unit: Unit::Px,
            }),
            ..Default::default()
        });
        let css = t.rule("dm-overlay").unwrap();
        assert!(css.contains("translate:20% 10px"));
        assert!(css.contains("rotate:30deg"));
        assert!(!css.contains("transform:"));
        assert!(!css.contains("position:"));
        assert!(!css.contains("left:"));
        assert!(t.rule("x\"]{display:none}").is_err());
        assert_eq!(LayoutDocument::default().stylesheet().unwrap(), "");
        assert!(
            LayoutDocument {
                visibility: BTreeMap::from([(
                    String::from("dm-overlay"),
                    ElementVisibility::Hidden
                )]),
                ..Default::default()
            }
            .stylesheet()
            .unwrap()
            .contains("display:none")
        );
    }
    #[test]
    fn maximum_inventory_pages_fit_the_existing_rpc_limit_with_worst_allowed_utf8_metadata() {
        use prost::Message;
        let elements = (0..MAX_ELEMENTS)
            .map(|index| ElementReadback {
                id: format!("v{index}-{}", "x".repeat(88)),
                tag: "x".repeat(32),
                classes: "x".repeat(192),
                owner: Some(VisualOwner {
                    id: "o".repeat(128),
                    name: Some("é".repeat(64)),
                }),
                configured: None,
                effective: Some(EffectiveTransform {
                    translate: "\\".repeat(512),
                    scale: "\\".repeat(512),
                    rotate: "\\".repeat(512),
                    matrix: "\\".repeat(512),
                }),
                rect: Some(Bounds {
                    left: 0.0,
                    top: 0.0,
                    width: 100.0,
                    height: 100.0,
                }),
                status: ElementStatus::Applied,
                visibility: ElementVisibility::Visible,
            })
            .collect();
        let report = LayoutReadback {
            sample: 1,
            revision: 1,
            viewport: Viewport {
                width: 1920.0,
                height: 1080.0,
            },
            overflow: 0,
            elements,
        };
        let pages = report.pages().unwrap();
        assert!(pages.len() > 1);
        assert_eq!(
            pages.iter().map(|p| p.elements.len()).sum::<usize>(),
            MAX_ELEMENTS
        );
        for page in pages {
            let text = serde_json::to_string(&page).unwrap();
            assert!(text.len() <= MAX_PAGE_BYTES);
            let request = df_protocol::common::PresentedRequest {
                sequence: 0,
                display_instance: 1,
                layout_readback_json: text,
            };
            assert!(request.encoded_len() <= df_rpc_bridge::RPC_MESSAGE_BYTES - 1024);
        }
    }

    #[test]
    fn equal_game_revision_merges_layout_without_scene_changes_and_rejects_old_metadata() {
        use df_protocol::common::{DisplayGameplayView, ViewMessage, view_message::Audience};
        let mut shown = ViewMessage {
            audience: Some(Audience::Display(DisplayGameplayView {
                narration: "still reading".into(),
                layout_json: serde_json::to_string(&LayoutDocument::default()).unwrap(),
                ..Default::default()
            })),
            ..Default::default()
        };
        let new = ViewMessage {
            audience: Some(Audience::Display(DisplayGameplayView {
                narration: "must not replace current words".into(),
                layout_revision: 1,
                layout_json: serde_json::to_string(&LayoutDocument {
                    revision: 1,
                    ..Default::default()
                })
                .unwrap(),
                ..Default::default()
            })),
            ..Default::default()
        };
        assert!(merge_layout(&mut shown, &new));
        let Some(Audience::Display(display)) = &shown.audience else {
            panic!()
        };
        assert_eq!(display.narration, "still reading");
        assert_eq!(display.layout_revision, 1);
        let stale = ViewMessage {
            audience: Some(Audience::Display(DisplayGameplayView {
                layout_json: serde_json::to_string(&LayoutDocument::default()).unwrap(),
                ..Default::default()
            })),
            ..Default::default()
        };
        assert!(!merge_layout(&mut shown, &stale));
    }
}
