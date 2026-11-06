//! The landing page of the shared screen (render 19): its data, its live changes and their checks.
//!
//! `model` holds the scene as a whole (`HomeScene`) and `change` the dev CLI's edits of it, checked
//! by `checks`. Each component has its own module: `chrome` (brand, title, top bar), `nav` (the
//! navigation rail and the action bar), `configure`, `adventure` and `campaigns`; `glyph` holds
//! the line drawings they name. Nothing here draws: `view::home` does, in the browser.

pub mod adventure;
pub mod campaigns;
pub mod change;
pub mod checks;
pub mod chrome;
pub mod configure;
pub mod glyph;
pub mod model;
pub mod nav;

#[cfg(test)]
mod adventure_tests;
#[cfg(test)]
mod campaigns_tests;
#[cfg(test)]
mod configure_tests;
