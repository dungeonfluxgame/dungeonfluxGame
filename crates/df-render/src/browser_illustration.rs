use crate::{
    DecodeBudget, DecodeToken, DecodedImage, ResourceCache, ResourceError, ResourceLimits,
    ResourceReadiness, SceneOwner, WorkOutcome,
};
use df_client::cache::CacheKey;
use df_types::{RevisionLabel, SessionRevision};
use wasm_bindgen::{Clamped, JsCast};
use web_sys::{CanvasRenderingContext2d, Element, HtmlCanvasElement, ImageData};

/// Private illustration target of the canonical lifecycle. No flat geometry, byte
/// cache, object URL, or extra decoded resource is created by this presentation owner.
pub(crate) struct BrowserIllustration {
    resources: ResourceCache<DecodedImage<CacheKey>>,
    owner: SceneOwner,
    parent: Element,
    surface: Option<(HtmlCanvasElement, CanvasRenderingContext2d)>,
    maximum_surface_bytes: usize,
    surface_bytes: usize,
    selected_resource: Option<CacheKey>,
    description: String,
    closed: bool,
}

impl BrowserIllustration {
    pub(crate) fn new(
        parent: &Element,
        owner: SceneOwner,
        limits: ResourceLimits,
        preparation: RevisionLabel,
        maximum_surface_bytes: usize,
    ) -> Result<Self, crate::ImageSurfaceError> {
        if maximum_surface_bytes == 0 || parent.owner_document().is_none() {
            return Err(crate::ImageSurfaceError::SurfaceCapacity);
        }
        Ok(Self {
            resources: ResourceCache::new(owner, limits, preparation)
                .map_err(crate::ImageSurfaceError::Resource)?,
            owner,
            parent: parent.clone(),
            surface: None,
            maximum_surface_bytes,
            surface_bytes: 0,
            selected_resource: None,
            description: String::new(),
            closed: false,
        })
    }

    pub(crate) fn validate_current(
        &self,
        revision: SessionRevision,
        refs: &[CacheKey],
    ) -> Result<(), ResourceError> {
        self.resources.validate_scene(self.owner, revision, refs)
    }
    pub(crate) fn apply_current(
        &mut self,
        revision: SessionRevision,
        refs: &[CacheKey],
    ) -> Result<(), ResourceError> {
        self.resources.apply_current(self.owner, revision, refs)
    }

    fn ensure_surface(&mut self) -> Result<(), crate::ImageSurfaceError> {
        if self.closed || !self.parent.is_connected() {
            return Err(crate::ImageSurfaceError::Browser);
        }
        if self.surface.is_some() {
            return Ok(());
        }
        let document = self
            .parent
            .owner_document()
            .ok_or(crate::ImageSurfaceError::Browser)?;
        let canvas = document
            .create_element("canvas")
            .map_err(|_| crate::ImageSurfaceError::Browser)?
            .dyn_into::<HtmlCanvasElement>()
            .map_err(|_| crate::ImageSurfaceError::Browser)?;
        canvas.set_class_name("scene-art");
        canvas.set_width(0);
        canvas.set_height(0);
        canvas.set_hidden(true);
        canvas
            .set_attribute("role", "img")
            .map_err(|_| crate::ImageSurfaceError::Browser)?;
        canvas.set_title(&self.description);
        canvas.set_text_content(Some(&self.description));
        let context = canvas
            .get_context("2d")
            .map_err(|_| crate::ImageSurfaceError::Browser)?
            .ok_or(crate::ImageSurfaceError::Browser)?
            .dyn_into::<CanvasRenderingContext2d>()
            .map_err(|_| crate::ImageSurfaceError::Browser)?;
        self.parent
            .append_child(&canvas)
            .map_err(|_| crate::ImageSurfaceError::Browser)?;
        self.surface = Some((canvas, context));
        Ok(())
    }

    pub(crate) fn present(&mut self, key: &CacheKey) -> Result<bool, crate::ImageSurfaceError> {
        self.clear();
        self.selected_resource = None;
        self.ensure_surface()?;
        let Some(image) = self
            .resources
            .get(key)
            .map_err(crate::ImageSurfaceError::Resource)?
        else {
            return Ok(false);
        };
        let pixels = image.pixels().map_err(crate::ImageSurfaceError::Resource)?;
        let surface_bytes = pixels
            .len()
            .checked_mul(2)
            .ok_or(crate::ImageSurfaceError::SurfaceCapacity)?;
        if surface_bytes > self.maximum_surface_bytes {
            return Err(crate::ImageSurfaceError::SurfaceCapacity);
        }
        let (width, height) = image.dimensions();
        let data = ImageData::new_with_u8_clamped_array_and_sh(Clamped(pixels), width, height)
            .map_err(|_| crate::ImageSurfaceError::Browser)?;
        let (canvas, context) = self
            .surface
            .as_ref()
            .ok_or(crate::ImageSurfaceError::Browser)?;
        canvas.set_width(width);
        canvas.set_height(height);
        if context.put_image_data(&data, 0.0, 0.0).is_err() {
            self.clear();
            return Err(crate::ImageSurfaceError::Browser);
        }
        self.selected_resource = Some(key.clone());
        self.surface_bytes = surface_bytes;
        Ok(true)
    }
    pub(crate) fn refresh(&mut self) -> Result<bool, crate::ImageSurfaceError> {
        let visible = self
            .surface
            .as_ref()
            .is_some_and(|(canvas, _)| !canvas.hidden());
        if let Some(key) = self.selected_resource.clone() {
            let is_presented = self.present(&key)?;
            self.set_visible(visible && is_presented);
            return Ok(is_presented);
        }
        self.clear();
        Ok(false)
    }
    pub(crate) fn canvas(&self) -> Option<&HtmlCanvasElement> {
        if self.closed {
            return None;
        }
        self.surface.as_ref().map(|(canvas, _)| canvas)
    }
    pub(crate) fn set_visible(&self, visible: bool) {
        if let Some((canvas, _)) = &self.surface {
            canvas.set_hidden(!visible);
        }
    }
    pub(crate) fn set_description(&mut self, description: &str) -> Result<(), ResourceError> {
        if description.len() > 512 || description.chars().any(char::is_control) {
            return Err(ResourceError::ByteCapacity);
        }
        self.description.clear();
        self.description.push_str(description);
        if let Some((canvas, _)) = &self.surface {
            canvas.set_title(description);
            canvas.set_text_content(Some(description));
        }
        Ok(())
    }
    pub(crate) fn begin(
        &mut self,
        key: &CacheKey,
        budget: DecodeBudget,
        abort: impl FnOnce() + 'static,
    ) -> Result<DecodeToken<CacheKey>, ResourceError> {
        self.resources.begin(key, budget, abort)
    }
    pub(crate) fn complete(
        &mut self,
        token: &DecodeToken<CacheKey>,
        image: DecodedImage<CacheKey>,
    ) -> Result<(), ResourceError> {
        self.resources.complete(token, image)
    }
    pub(crate) fn cancel(&mut self, token: &DecodeToken<CacheKey>) -> Result<(), ResourceError> {
        self.resources.cancel(token)
    }
    pub(crate) fn finish_failed(
        &mut self,
        token: &DecodeToken<CacheKey>,
    ) -> Result<WorkOutcome, ResourceError> {
        self.resources.finish_failed(token)
    }
    pub(crate) fn get(
        &mut self,
        key: &CacheKey,
    ) -> Result<Option<&DecodedImage<CacheKey>>, ResourceError> {
        self.resources.get(key)
    }
    pub(crate) fn readiness(&self, key: &CacheKey) -> Result<ResourceReadiness, ResourceError> {
        self.resources.readiness(key)
    }
    pub(crate) fn release(&mut self, key: &CacheKey) -> Result<(), ResourceError> {
        if self.selected_resource.as_ref() == Some(key) {
            self.clear();
            self.selected_resource = None;
        }
        self.resources.release(key)
    }
    pub(crate) fn work_bytes(&self) -> usize {
        self.resources.work_bytes()
    }
    pub(crate) fn decoded_bytes(&self) -> usize {
        self.resources.decoded_bytes()
    }
    pub(crate) fn surface_bytes(&self) -> usize {
        self.surface_bytes
    }
    fn clear(&mut self) {
        if let Some((canvas, _)) = &self.surface {
            canvas.set_hidden(true);
            canvas.set_width(0);
            canvas.set_height(0);
        }
        self.surface_bytes = 0;
    }
    pub(crate) fn dispose(&mut self) -> Result<crate::PresentationOutcome, crate::RenderError> {
        if self.closed {
            return Ok(crate::PresentationOutcome::AlreadyDisposed);
        }
        self.closed = true;
        self.clear();
        self.selected_resource = None;
        self.resources.dispose();
        if let Some((canvas, _)) = self.surface.take() {
            canvas.set_text_content(None);
            canvas.set_title("");
            canvas.remove();
        }
        self.description.clear();
        Ok(crate::PresentationOutcome::Disposed)
    }
}
impl Drop for BrowserIllustration {
    fn drop(&mut self) {
        let _cleanup = self.dispose();
    }
}
