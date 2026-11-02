//! The invented limits of the canvas, in one place.
//!
//! The owner's rule is that there are no small invented caps: a limit is a very large named
//! constant, memory stays bounded by design, and a value is scoped down later when a measurement
//! says so. These are sizes nobody has measured. The limits that are rules of the domain (a
//! rotation of 0 to 359 degrees, a share of 0 to 100 percent, the 100 by 56.25 canvas) are not
//! here: they live with the type that needs them.

/// Most elements the canvas holds at once. The playground sends every element in every frame, so
/// a frame is as large as this times an element; delta frames come before this is lowered or
/// reached.
pub const MAXIMUM_ELEMENTS: usize = 100_000;
/// Longest name (of an element, kind, field or choice), in characters. Names are ASCII.
pub const MAXIMUM_NAME_LENGTH: usize = 256;
/// Longest text value of a field, in characters, when its kind does not say less with its own
/// `maximum_length`.
pub const MAXIMUM_TEXT_LENGTH: usize = 65_536;
/// Most fields one kind has.
pub const MAXIMUM_FIELDS_PER_KIND: usize = 4_096;
/// Most choices one choice field has.
pub const MAXIMUM_CHOICES: usize = 4_096;
