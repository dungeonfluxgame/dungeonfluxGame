use std::collections::BTreeSet;

use df_types::{ClientBindingId, RunId, SessionId, SessionRevision};

/// Supplied correlation scope; never credentials or authorization issuance.
pub type SceneOwner = (SessionId, RunId, ClientBindingId);

/// Candidate presentation admission bounds; device budgets require independent measurement.
pub const MAX_LAYERS: usize = 256;
pub const MAX_TOKENS: usize = 512;
const MAX_LABEL_BYTES: usize = 512;
const MAX_COORDINATE: f64 = 1_000_000.0;

/// Stable local component key. It conveys no identity, permission, or hidden state.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ElementKey(String);

impl ElementKey {
    pub fn new(value: impl Into<String>) -> Result<Self, RenderError> {
        let value = value.into();
        if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
            return Err(RenderError::InvalidKey);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Supplied scene units; renderer never snaps, clamps, or derives movement legality.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// Server-selected SVG view box in the same units as all supplied positions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub origin: Point,
    pub width: f64,
    pub height: f64,
}

/// Closed paint allowlist; server data cannot introduce URLs, scripts, or CSS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneColor {
    Slate,
    Stone,
    Blue,
    Amber,
    Red,
    Green,
}

impl SceneColor {
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn css(self) -> &'static str {
        match self {
            Self::Slate => "#1e293b",
            Self::Stone => "#78716c",
            Self::Blue => "#2563eb",
            Self::Amber => "#d97706",
            Self::Red => "#dc2626",
            Self::Green => "#16a34a",
        }
    }
}

/// One already permitted flat rectangle, ordered back to front in its layer list.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneLayer {
    pub key: ElementKey,
    pub origin: Point,
    pub width: f64,
    pub height: f64,
    pub color: SceneColor,
}

/// One already permitted token. The label is plain text, never HTML.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub key: ElementKey,
    pub position: Point,
    pub radius: f64,
    pub label: String,
    pub color: SceneColor,
}

/// Complete audience-safe flat snapshot mapped by the client from an accepted view.
/// No visibility/private fields exist: filtering belongs to the server before serialization.
#[derive(Debug, Clone, PartialEq)]
pub struct FlatScene {
    pub revision: SessionRevision,
    pub viewport: Viewport,
    pub label: String,
    pub layers: Vec<SceneLayer>,
    pub tokens: Vec<Token>,
}

impl FlatScene {
    pub(crate) fn validate(&self) -> Result<(), RenderError> {
        if self.layers.len() > MAX_LAYERS || self.tokens.len() > MAX_TOKENS {
            return Err(RenderError::Capacity);
        }
        validate_label(&self.label)?;
        validate_point(self.viewport.origin)?;
        validate_length(self.viewport.width)?;
        validate_length(self.viewport.height)?;
        let mut layer_keys = BTreeSet::new();
        for layer in &self.layers {
            if !layer_keys.insert(&layer.key) {
                return Err(RenderError::DuplicateLayerKey);
            }
            validate_point(layer.origin)?;
            validate_length(layer.width)?;
            validate_length(layer.height)?;
        }
        let mut token_keys = BTreeSet::new();
        for token in &self.tokens {
            if !token_keys.insert(&token.key) {
                return Err(RenderError::DuplicateTokenKey);
            }
            validate_point(token.position)?;
            validate_length(token.radius)?;
            validate_label(&token.label)?;
        }
        Ok(())
    }
}

fn validate_point(point: Point) -> Result<(), RenderError> {
    if !point.x.is_finite()
        || !point.y.is_finite()
        || point.x.abs() > MAX_COORDINATE
        || point.y.abs() > MAX_COORDINATE
    {
        return Err(RenderError::InvalidGeometry);
    }
    Ok(())
}

fn validate_length(length: f64) -> Result<(), RenderError> {
    if !length.is_finite() || length <= 0.0 || length > MAX_COORDINATE {
        return Err(RenderError::InvalidGeometry);
    }
    Ok(())
}

fn validate_label(label: &str) -> Result<(), RenderError> {
    if label.len() > MAX_LABEL_BYTES || label.chars().any(char::is_control) {
        return Err(RenderError::InvalidLabel);
    }
    Ok(())
}

/// Safe classified result; no payload/label/key is included in diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderError {
    WrongOwner,
    InvalidKey,
    InvalidLabel,
    InvalidGeometry,
    DuplicateLayerKey,
    DuplicateTokenKey,
    Capacity,
    Disposed,
    BrowserUnavailable,
    BrowserMutation,
}

/// Presentation facts for the owning shell's shared observation path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentationOutcome {
    Applied { layers: usize, tokens: usize },
    Stale { current: SessionRevision },
    Duplicate { current: SessionRevision },
    Disposed,
    AlreadyDisposed,
}

/// Current owner readiness and finite bounds; no measured device/fidelity claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RendererCapabilities {
    pub flat_svg: bool,
    pub max_layers: usize,
    pub max_tokens: usize,
}

/// Owns one snapshot and (on WASM) one persistent SVG subtree. No timers or listeners.
/// Callers must dispose on authorization/member scope replacement, not just reconnect.
#[derive(Debug)]
pub struct SceneRenderer {
    owner: SceneOwner,
    current: Option<FlatScene>,
    disposed: bool,
    #[cfg(target_arch = "wasm32")]
    pub(crate) browser: Option<crate::browser::SvgMount>,
}

impl SceneRenderer {
    /// Native fixture/offscreen state owner. Browser presentation uses `mount`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new(owner: SceneOwner) -> Self {
        Self::empty(owner)
    }

    pub(crate) fn empty(owner: SceneOwner) -> Self {
        Self {
            owner,
            current: None,
            disposed: false,
            #[cfg(target_arch = "wasm32")]
            browser: None,
        }
    }

    pub fn capabilities(&self) -> RendererCapabilities {
        #[cfg(not(target_arch = "wasm32"))]
        let flat_svg = false;
        #[cfg(target_arch = "wasm32")]
        let flat_svg = !self.disposed && self.is_mounted();
        RendererCapabilities {
            flat_svg,
            max_layers: MAX_LAYERS,
            max_tokens: MAX_TOKENS,
        }
    }

    /// Validates a complete snapshot before changing the display or revision watermark.
    /// Older/equal revisions never replace current values; recovery uses canonical ordering.
    pub fn update(
        &mut self,
        owner: SceneOwner,
        scene: FlatScene,
    ) -> Result<PresentationOutcome, RenderError> {
        if let Some(outcome) = self.check_update(owner, &scene)? {
            return Ok(outcome);
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(browser) = &mut self.browser
            && let Err(error) = browser.update(&scene)
        {
            // A failed DOM operation may have partially changed the subtree. Clear it
            // rather than reporting the prior authoritative snapshot as still rendered.
            let cleanup = self.dispose();
            return Err(cleanup.err().unwrap_or(error));
        }
        let outcome = PresentationOutcome::Applied {
            layers: scene.layers.len(),
            tokens: scene.tokens.len(),
        };
        self.current = Some(scene);
        Ok(outcome)
    }

    /// Internal mounted adapters preflight geometry and resource admission together.
    /// `None` means admissible, never that a frame has already been rendered.
    pub(crate) fn check_update(
        &self,
        owner: SceneOwner,
        scene: &FlatScene,
    ) -> Result<Option<PresentationOutcome>, RenderError> {
        if self.disposed {
            return Err(RenderError::Disposed);
        }
        if owner != self.owner {
            return Err(RenderError::WrongOwner);
        }
        if let Some(current) = &self.current {
            if scene.revision < current.revision {
                return Ok(Some(PresentationOutcome::Stale {
                    current: current.revision,
                }));
            }
            if scene.revision == current.revision {
                return Ok(Some(PresentationOutcome::Duplicate {
                    current: current.revision,
                }));
            }
        }
        scene.validate()?;
        Ok(None)
    }

    /// Returns the canonical owner scope; this grants no source authorization.
    pub fn owner(&self) -> SceneOwner {
        self.owner
    }

    pub fn current(&self) -> Option<&FlatScene> {
        self.current.as_ref()
    }

    /// Idempotently releases owned presentation and removes only the owned mount subtree.
    pub fn dispose(&mut self) -> Result<PresentationOutcome, RenderError> {
        if self.disposed {
            return Ok(PresentationOutcome::AlreadyDisposed);
        }
        self.current = None;
        self.disposed = true;
        #[cfg(target_arch = "wasm32")]
        if let Some(mut browser) = self.browser.take() {
            browser.dispose()?;
        }
        Ok(PresentationOutcome::Disposed)
    }
}

impl Drop for SceneRenderer {
    fn drop(&mut self) {
        let _cleanup = self.dispose();
    }
}
