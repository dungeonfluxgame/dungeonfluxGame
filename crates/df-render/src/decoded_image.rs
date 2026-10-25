use crate::{RendererResource, ResourceError};
use df_client::cache::CacheLease;

/// Owned RGBA8 decoded surface, distinct from compressed/encoded client-cache bytes.
/// `key` is the canonical cache key unchanged. Lease guards bind it to current ownership.
/// Neither borrowed bytes nor mutable pixels escape; disposal releases the lease once.
pub struct DecodedImage<K: Clone + Eq> {
    key: K,
    width: u32,
    height: u32,
    pixels: Box<[u8]>,
    current: Box<dyn Fn(&K) -> bool>,
    release: Option<Box<dyn FnOnce()>>,
    cache_lease: Option<CacheLease>,
}

impl<K: Clone + Eq> DecodedImage<K> {
    pub fn new(
        key: K,
        width: u32,
        height: u32,
        pixels: Box<[u8]>,
        current: impl Fn(&K) -> bool + 'static,
        release: impl FnOnce() + 'static,
    ) -> Result<Self, ResourceError> {
        let expected_bytes = (width as usize)
            .checked_mul(height as usize)
            .and_then(|value| value.checked_mul(4));
        if width == 0 || height == 0 || expected_bytes != Some(pixels.len()) {
            release();
            return Err(ResourceError::ReservationExceeded);
        }
        Ok(Self {
            key,
            width,
            height,
            pixels,
            current: Box::new(current),
            release: Some(Box::new(release)),
            cache_lease: None,
        })
    }

    pub(crate) fn bind_cache_lease(mut self, lease: CacheLease) -> Self {
        self.cache_lease = Some(lease);
        self
    }

    pub(crate) fn cache_lease(&self) -> Option<&CacheLease> {
        self.cache_lease.as_ref()
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    pub fn pixels(&self) -> Result<&[u8], ResourceError> {
        if !(self.current)(&self.key) {
            return Err(ResourceError::RevokedResource);
        }
        Ok(&self.pixels)
    }
}

impl<K: Clone + Eq> RendererResource for DecodedImage<K> {
    type Key = K;
    fn key(&self) -> &K {
        &self.key
    }
    fn decoded_bytes(&self) -> usize {
        self.pixels.len()
    }
    fn is_current(&self) -> bool {
        (self.current)(&self.key)
    }
    fn retire(self) {
        drop(self);
    }
}

impl<K: Clone + Eq> Drop for DecodedImage<K> {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            release();
        }
    }
}
