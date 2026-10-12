//! The WebGPU backend: raw bindings, shaders, device acquisition, textures and the renderer.
//! Compiled for the browser only; the pure core never names any of it.

pub mod core;
pub mod renderer;
pub mod shaders;
pub mod shaders_figure;
pub mod sys;
pub mod textures;
