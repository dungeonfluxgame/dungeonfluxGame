use crate::{
    DecodeBudget, DecodeToken, DecodedImage, FlatScene, PresentationOutcome, RenderError,
    ResourceError, ResourceLimits, ResourceSceneError, ResourceSceneRenderer, SceneOwner,
    WorkOutcome,
};
use df_types::RevisionLabel;
use wasm_bindgen::{Clamped, JsCast};
use web_sys::{CanvasRenderingContext2d, Element, HtmlCanvasElement, ImageData};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageSurfaceError {
    Scene(ResourceSceneError),
    Resource(ResourceError),
    Browser,
    SurfaceCapacity,
}

/// An actual RGBA canvas attached to the scene owner's connected image slot. It owns
/// no transport or compressed byte cache. Preparation work remains with the supplied
/// bounded decoder; this owner consumes its separately owned decoded surfaces.
pub struct BrowserResourceScene<K: Clone + Eq> {
    renderer: ResourceSceneRenderer<DecodedImage<K>>,
    canvas: HtmlCanvasElement,
    context: CanvasRenderingContext2d,
    maximum_surface_bytes: usize,
    surface_bytes: usize,
    selected_resource: Option<K>,
}

impl<K: Clone + Eq> BrowserResourceScene<K> {
    pub fn mount(
        parent: &Element,
        owner: SceneOwner,
        limits: ResourceLimits,
        preparation_revision: RevisionLabel,
        maximum_surface_bytes: usize,
    ) -> Result<Self, ImageSurfaceError> {
        if maximum_surface_bytes == 0 {
            return Err(ImageSurfaceError::SurfaceCapacity);
        }
        let renderer = ResourceSceneRenderer::mount(parent, owner, limits, preparation_revision)
            .map_err(ImageSurfaceError::Scene)?;
        let host = renderer
            .scene()
            .image_host()
            .map_err(|error| ImageSurfaceError::Scene(ResourceSceneError::Scene(error)))?;
        let document = host.owner_document().ok_or(ImageSurfaceError::Browser)?;
        let canvas = document
            .create_element("canvas")
            .map_err(|_| ImageSurfaceError::Browser)?
            .dyn_into::<HtmlCanvasElement>()
            .map_err(|_| ImageSurfaceError::Browser)?;
        canvas.set_width(0);
        canvas.set_height(0);
        canvas
            .set_attribute("role", "img")
            .map_err(|_| ImageSurfaceError::Browser)?;
        canvas
            .set_attribute("aria-label", "Permitted scene illustration")
            .map_err(|_| ImageSurfaceError::Browser)?;
        let context = canvas
            .get_context("2d")
            .map_err(|_| ImageSurfaceError::Browser)?
            .ok_or(ImageSurfaceError::Browser)?
            .dyn_into::<CanvasRenderingContext2d>()
            .map_err(|_| ImageSurfaceError::Browser)?;
        host.append_child(&canvas)
            .map_err(|_| ImageSurfaceError::Browser)?;
        Ok(Self {
            renderer,
            canvas,
            context,
            maximum_surface_bytes,
            surface_bytes: 0,
            selected_resource: None,
        })
    }

    /// Validate first so rejected/stale views preserve the current illustration. Accepted
    /// transitions clear previous pixels before any resource can become non-current.
    pub fn update(
        &mut self,
        owner: SceneOwner,
        scene_label: RevisionLabel,
        flat: FlatScene,
        references: &[K],
    ) -> Result<PresentationOutcome, ImageSurfaceError> {
        if let Some(outcome) = self
            .renderer
            .check_update(owner, &flat, references)
            .map_err(ImageSurfaceError::Scene)?
        {
            return Ok(outcome);
        }
        let selected_resource = self.selected_resource.clone();
        self.clear();
        self.selected_resource = None;
        let outcome = self
            .renderer
            .update(owner, scene_label, flat, references)
            .map_err(ImageSurfaceError::Scene)?;
        // Optional surface failure leaves the already accepted flat view mounted. The caller
        // receives the typed display failure; geometry is never rolled back into old authority.
        if let Some(key) = selected_resource.filter(|key| references.contains(key)) {
            self.present(&key)?;
        }
        Ok(outcome)
    }

    /// Every disclosure revalidates the real canonical byte lease via pixels(). A revoked
    /// lease clears the canvas even when no renderer snapshot update has arrived yet.
    /// Canvas+ImageData staging needs twice the decoded pixel bytes, bounded before allocation.
    pub fn present(&mut self, key: &K) -> Result<bool, ImageSurfaceError> {
        self.clear();
        self.selected_resource = None;
        if !self.canvas.is_connected() {
            return Err(ImageSurfaceError::Browser);
        }
        let Some(image) = self
            .renderer
            .get(key)
            .map_err(ImageSurfaceError::Resource)?
        else {
            return Ok(false);
        };
        let pixels = image.pixels().map_err(ImageSurfaceError::Resource)?;
        let surface_bytes = pixels
            .len()
            .checked_mul(2)
            .ok_or(ImageSurfaceError::SurfaceCapacity)?;
        if surface_bytes > self.maximum_surface_bytes {
            return Err(ImageSurfaceError::SurfaceCapacity);
        }
        let (width, height) = image.dimensions();
        let data = ImageData::new_with_u8_clamped_array_and_sh(Clamped(pixels), width, height)
            .map_err(|_| ImageSurfaceError::Browser)?;
        self.canvas.set_width(width);
        self.canvas.set_height(height);
        if self.context.put_image_data(&data, 0.0, 0.0).is_err() {
            self.clear();
            return Err(ImageSurfaceError::Browser);
        }
        self.surface_bytes = surface_bytes;
        self.selected_resource = Some(key.clone());
        Ok(true)
    }

    /// Call on access/cache invalidation notification as well as before a new disclosure.
    /// No automatic authority subscription exists here; the client owns that notification.
    pub fn refresh(&mut self) -> Result<bool, ImageSurfaceError> {
        if let Some(key) = self.selected_resource.clone() {
            return self.present(&key);
        }
        self.clear();
        Ok(false)
    }

    pub fn begin(
        &mut self,
        key: &K,
        budget: DecodeBudget,
        abort: impl FnOnce() + 'static,
    ) -> Result<DecodeToken<K>, ResourceError> {
        self.renderer.begin(key, budget, abort)
    }
    pub fn complete(
        &mut self,
        token: &DecodeToken<K>,
        resource: DecodedImage<K>,
    ) -> Result<(), ResourceError> {
        self.renderer.complete(token, resource)
    }
    pub fn cancel(&mut self, token: &DecodeToken<K>) -> Result<(), ResourceError> {
        self.renderer.cancel(token)
    }
    pub fn finish_failed(&mut self, token: &DecodeToken<K>) -> Result<WorkOutcome, ResourceError> {
        self.renderer.finish_failed(token)
    }
    pub fn release(&mut self, key: &K) -> Result<(), ResourceError> {
        if self.selected_resource.as_ref() == Some(key) {
            self.clear();
            self.selected_resource = None;
        }
        self.renderer.release(key)
    }
    /// Returns the canonical owner scope; this grants no source authorization.
    pub fn owner(&self) -> SceneOwner {
        self.renderer.owner()
    }

    pub(crate) fn get(&mut self, key: &K) -> Result<Option<&DecodedImage<K>>, ResourceError> {
        self.renderer.get(key)
    }

    pub(crate) fn readiness(&self, key: &K) -> Result<crate::ResourceReadiness, ResourceError> {
        self.renderer.readiness(key)
    }

    pub fn decoded_bytes(&self) -> usize {
        self.renderer.decoded_bytes()
    }
    pub fn work_bytes(&self) -> usize {
        self.renderer.work_bytes()
    }
    pub fn surface_bytes(&self) -> usize {
        self.surface_bytes
    }
    pub fn dispose(&mut self) -> Result<PresentationOutcome, RenderError> {
        self.clear();
        self.selected_resource = None;
        self.canvas.remove();
        self.renderer.dispose()
    }
    fn clear(&mut self) {
        self.canvas.set_width(0);
        self.canvas.set_height(0);
        self.surface_bytes = 0;
    }
}

impl<K: Clone + Eq> Drop for BrowserResourceScene<K> {
    fn drop(&mut self) {
        self.clear();
        self.canvas.remove();
    }
}
