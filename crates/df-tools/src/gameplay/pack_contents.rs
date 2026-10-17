//! What the SRD 5.2.1 gear packs hold (pp.95-99, `content/hero-tables/catalog-srd.json`). A
//! starting kit names a pack by its kit id, and the hero holds each item inside it.

use serde::Deserialize;

const CATALOG: &str = include_str!("../../../../content/hero-tables/catalog-srd.json");

#[derive(Debug, Deserialize)]
struct Catalog {
    items: Vec<Item>,
}

#[derive(Debug, Deserialize)]
struct Item {
    #[serde(default)]
    kit_item: Option<String>,
    #[serde(default)]
    contents: Vec<Content>,
}

/// One kind of gear inside a pack, with its count in the pack (`quantity` 10 for ten torches).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub(super) struct Content {
    pub(super) item: String,
    pub(super) quantity: u8,
}

/// The contents of the catalog pack that a starting-kit row names; empty when the row names no
/// pack.
pub(super) fn for_kit_item(kit_item: &str) -> Result<Vec<Content>, serde_json::Error> {
    let catalog: Catalog = serde_json::from_str(CATALOG)?;
    Ok(catalog
        .items
        .into_iter()
        .find(|item| item.kit_item.as_deref() == Some(kit_item))
        .map(|item| item.contents)
        .unwrap_or_default())
}
