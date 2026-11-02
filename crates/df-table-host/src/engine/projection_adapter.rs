//! The one seam where the owner asks for a view of the table: the real projection of C5 over the
//! committed table, written as JSON for the dev routes.
//!
//! The owner calls `project_view` and nothing else, and always on state that is already computed
//! and committed: no model call, no I/O and no loop over history. A view is `df_table::projection`'s
//! own (the shared screen's, or one seat's, never blank, with the sentinel-secret and paired-state
//! guarantees of C5); this file only chooses the audience, reads the committed table and turns the
//! view into JSON, because `AudienceProjection` serializes and does not deserialize. A view the
//! pipeline built for a turn reaches the sender's report through `sender_view`, and is released
//! only after the batch that made it has committed.

use df_table::identity::SeatId;
use df_table::pipeline::PipelineTable;
use df_table::ports::Audience;
use df_table::projection::{AudienceProjection, ProjectionError, Viewer, project};

/// The label of an audience in an audit line and a report: `tv`, or `seat:N`.
pub fn audience_label(audience: Audience) -> String {
    match audience {
        Audience::Public => "tv".to_owned(),
        Audience::Seat(seat) => format!("seat:{seat}"),
    }
}

/// Why a view could not be made.
#[derive(Debug)]
pub enum ViewError {
    /// The seat is not at the table, so it has no view.
    Projection(ProjectionError),
    /// The view could not be written as JSON.
    Json(serde_json::Error),
}

impl std::fmt::Display for ViewError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Projection(error) => write!(formatter, "{error:?}"),
            Self::Json(error) => write!(formatter, "the view could not be written: {error}"),
        }
    }
}

impl std::error::Error for ViewError {}

/// The committed table's view for `audience`, as JSON.
pub fn project_view(
    table: &PipelineTable,
    audience: Audience,
) -> Result<serde_json::Value, ViewError> {
    let snapshot = table.facts().snapshot();
    let projection =
        project(&snapshot, table.attachments(), audience).map_err(ViewError::Projection)?;
    view_json(&projection)
}

/// The view a turn built for `seat`, as JSON, from the projections the pipeline made for it.
pub fn sender_view(
    projections: &[AudienceProjection],
    seat: SeatId,
) -> Option<Result<serde_json::Value, ViewError>> {
    projections
        .iter()
        .find(|projection| projection.viewer() == Viewer::Seat { seat })
        .map(view_json)
}

fn view_json(projection: &AudienceProjection) -> Result<serde_json::Value, ViewError> {
    serde_json::to_value(projection).map_err(ViewError::Json)
}
