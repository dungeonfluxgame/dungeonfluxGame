//! The WebGPU backend: raw bindings, shaders, device acquisition, textures and the renderer.
//! Compiled for the browser only; the pure core never names any of it.

pub mod core;
pub mod renderer;
pub mod shaders;
pub mod splat_engine;
pub mod splat_shaders;
pub mod splat_stream;
pub mod sys;
pub mod textures;
