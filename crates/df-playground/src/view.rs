//! The browser drawing of the model: each module turns one part of a scene into page elements and
//! decides nothing. `tv` and `phone` are the two screens; the others are their parts and the
//! shared kit (`kit`, `icons`) they are made with.

pub mod brand;
pub mod bubble_layout;
pub mod clues;
pub mod composer;
pub mod home;
pub mod icons;
pub mod kit;
pub mod lobby;
pub mod overlays;
pub mod phone;
pub mod pills;
pub mod plate_fade;
pub mod roleplay;
pub mod sam;
pub mod seats;
pub mod story;
pub mod tv;
