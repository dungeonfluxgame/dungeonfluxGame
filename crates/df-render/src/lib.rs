//! Bounded flat presentation of caller-supplied, audience-safe server geometry.
//! These are local presentation inputs, not an approved RPC schema or game state.
mod scene;

pub mod art_contract;

pub mod battle;

#[cfg(target_arch = "wasm32")]
mod browser;

pub use scene::{
    ElementKey, FlatScene, MAX_LAYERS, MAX_TOKENS, Point, PresentationOutcome, RenderError,
    RendererCapabilities, SceneColor, SceneLayer, SceneOwner, SceneRenderer, Token, Viewport,
};

#[cfg(target_arch = "wasm32")]
mod browser_illustration;
#[cfg(target_arch = "wasm32")]
mod browser_resource_scene;
mod decoded_image;
mod resource_cache;
mod resource_scene;
#[cfg(target_arch = "wasm32")]
pub use browser_resource_scene::{BrowserResourceScene, ImageSurfaceError};
pub use decoded_image::DecodedImage;
pub use resource_cache::{
    DecodeBudget, DecodeToken, RendererResource, ResourceCache, ResourceError, ResourceLimits,
    ResourceReadiness, WorkOutcome,
};
pub use resource_scene::{ResourceSceneError, ResourceSceneRenderer};

mod browser_image_decode;
#[cfg(target_arch = "wasm32")]
pub use browser_image_decode::{BrowserDecodeStatus, BrowserImageDecode};
pub use browser_image_decode::{
    ImageDecodeError, ImageDecodeLimits, ImageDecodePlan, PngDecodePlan, PreparedImageMetadata,
    VerifiedImage, VerifiedPng, inspect_png, inspect_prepared_image,
};

mod resource_lifecycle;
pub use resource_lifecycle::{ResourceLifecycle, ResourceLifecycleError};
