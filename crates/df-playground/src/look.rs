//! The look studio (`planning/phone-and-creation.md` P3, "The look studio"): the hero's look is
//! refined step by step on one fixed pose, then locked in as the portrait and the reference sheet
//! the scene plates paint the hero from.
//!
//! The host owns every picture, version and decision; the phone holds only the draft (the control
//! values and the details, REF-049) and draws what the host answers (`wire`).

pub mod studio;
pub mod wire;

#[cfg(test)]
mod look_tests;
