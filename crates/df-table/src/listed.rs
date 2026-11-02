//! A closed enum that lists its own variants.
//!
//! Owns the one macro that declares a fieldless enum together with its `ALL` list, so a variant
//! added to the declaration cannot be missing from the list that tests and tables walk. It never
//! adds behaviour: keys, classes and texts stay in exhaustive `match`es next to each enum.

/// Declares a fieldless enum and `ALL`, the slice of every variant in declaration order.
macro_rules! listed_enum {
    (
        $(#[$meta:meta])*
        $visibility:vis enum $name:ident {
            $($(#[$variant_meta:meta])* $variant:ident),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        $visibility enum $name {
            $($(#[$variant_meta])* $variant),+
        }

        impl $name {
            /// Every variant, in declaration order.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];
        }
    };
}

pub(crate) use listed_enum;
