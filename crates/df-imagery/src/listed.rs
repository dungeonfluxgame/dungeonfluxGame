//! A closed enum that lists its own variants.
//!
//! Owns the one macro that declares a fieldless enum together with its `ALL` list, so a variant
//! added to the declaration cannot be missing from the list that tests and tables walk. It never
//! adds behaviour: keys stay in exhaustive `match`es next to each enum. It is the same macro as
//! df-table's `listed_enum!`; df-imagery does not depend on df-table.

/// Declares a fieldless enum and `ALL`, the slice of every variant in declaration order.
///
/// Written `listed_enum! { keyed ... }`, it also declares `from_key`, the variant whose `key()` is
/// the text, found in `ALL`; the enum keeps its exhaustive `key()` next to it (a closed key set of
/// the coding style).
macro_rules! listed_enum {
    (
        keyed
        $(#[$meta:meta])*
        $visibility:vis enum $name:ident {
            $($(#[$variant_meta:meta])* $variant:ident),+ $(,)?
        }
    ) => {
        $crate::listed::listed_enum! {
            $(#[$meta])*
            $visibility enum $name {
                $($(#[$variant_meta])* $variant),+
            }
        }

        impl $name {
            /// The variant whose `key()` is `text`, if any.
            pub fn from_key(text: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|variant| variant.key() == text)
            }
        }
    };
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
