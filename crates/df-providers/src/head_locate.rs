//! Provider-neutral head geometry retained for stored layouts.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeadBox {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LocatedHead {
    pub id: String,
    pub head: HeadBox,
}
