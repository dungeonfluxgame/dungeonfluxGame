//! Bounded validation of explicitly uploaded drafts. No fetching, decoding or publication.
use crate::catalog::{CatalogEntry, CatalogError, CatalogLimits, validate_entries};
use crate::graph::{GraphError, GraphLimit, GraphLimits, GraphView, validate_graph};

/// The transport owner classifies the source. External sources are never processed.
pub enum ImportSource<'a> {
    Uploaded(&'a [u8]),
    ExternalUrl(&'a str),
    ExternalFile(&'a str),
}

/// Current reviewed source/rights/rules admission supplied by its owner.
/// An admitted pin includes exact checksum, provenance, rights and schema references.
/// Constructing this value does not issue or validate those rights.
pub enum ImportAdmission<'a, Pins> {
    Admitted(&'a Pins),
    SourceGap,
    RightsGap,
    UnsupportedRules,
}

/// All capacities are explicit. Diagnostics are always one fixed-size, fail-fast value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImportLimits {
    pub catalog: CatalogLimits,
    pub graph: GraphLimits,
    pub max_path_bytes: usize,
    pub max_path_components: usize,
    pub max_total_path_bytes: usize,
}

/// One owner-allowlisted format; it is a typed identity, not a parser or wire schema.
pub struct ImportSpec<'a, Version, Pins, Format> {
    pub version: &'a Version,
    pub admission: ImportAdmission<'a, Pins>,
    pub format: &'a Format,
    pub namespace: &'a str,
    pub limits: ImportLimits,
}

/// Source owner binds this decoded upload to its complete bytes and admitted pins.
/// The format owner bounds decoded identity representations and verifies the schema,
/// checksum and current rights before supplying admission. A checksum comparison
/// alone does not establish decoding or grant any use rights.
pub struct DecodedUpload<'a, Version, Pins, Format> {
    pub source: ImportSource<'a>,
    pub version: &'a Version,
    pub pins: &'a Pins,
    pub format: &'a Format,
}

/// Half-open byte offsets into the complete uploaded representation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImportLocation {
    pub start: usize,
    pub end: usize,
}

pub struct ImportEntrySource<'a> {
    pub path: &'a str,
    pub location: ImportLocation,
}

pub struct ImportCatalog<'a, ItemId> {
    pub entries: &'a [CatalogEntry<'a, ItemId>],
    pub sources: &'a [ImportEntrySource<'a>],
}

/// Fixed safe codes contain no IDs, paths, URLs, lore or rights evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImportCode {
    ExternalSource,
    UnsupportedFormat,
    SourceGap,
    RightsGap,
    UnsupportedRules,
    CompleteBytes,
    EntryCount,
    ItemBytes,
    TotalItemBytes,
    DuplicateItem,
    SourceLocations,
    InvalidLocation,
    InvalidPath,
    Namespace,
    PathBytes,
    PathDepth,
    TotalPathBytes,
    GraphCapacity(GraphLimit),
    DuplicateReference,
    MissingReference,
    ReferenceCycle,
    UnreachableAlternative,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImportDiagnostic {
    pub code: ImportCode,
    pub entry_index: Option<usize>,
    pub location: Option<ImportLocation>,
}

impl ImportDiagnostic {
    fn whole(code: ImportCode) -> Self {
        Self {
            code,
            entry_index: None,
            location: None,
        }
    }

    fn entry(code: ImportCode, index: usize, location: Option<ImportLocation>) -> Self {
        Self {
            code,
            entry_index: Some(index),
            location,
        }
    }
}

/// Immutable borrowed draft. Success does not create a published snapshot, pack,
/// executable rule, canonical fact, grant, or running-campaign mutation.
/// The format owner must have decoded the supplied catalog/graph from these exact
/// bytes and pins. Publication, authorization and activation remain separate gates.
/// The accepted draft does not expose mutable upload bytes:
/// ```compile_fail
/// use df_content::import::ImportCandidate;
/// fn edit(candidate: &ImportCandidate<'_, u64, (), ()>) {
///     candidate.bytes()[0] = 0;
/// }
/// ```
pub struct ImportCandidate<'a, Version, Pins, Format> {
    bytes: &'a [u8],
    version: &'a Version,
    pins: &'a Pins,
    format: &'a Format,
    item_count: usize,
}

impl<'a, Version, Pins, Format> ImportCandidate<'a, Version, Pins, Format> {
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }
    pub fn version(&self) -> &'a Version {
        self.version
    }
    pub fn pins(&self) -> &'a Pins {
        self.pins
    }
    pub fn format(&self) -> &'a Format {
        self.format
    }
    pub fn item_count(&self) -> usize {
        self.item_count
    }
}

fn catalog_diagnostic(error: CatalogError) -> ImportDiagnostic {
    match error {
        CatalogError::CompleteBytesLimit { .. } => {
            ImportDiagnostic::whole(ImportCode::CompleteBytes)
        }
        CatalogError::EntryCountLimit { .. } => ImportDiagnostic::whole(ImportCode::EntryCount),
        CatalogError::ItemBytesLimit { entry_index, .. } => {
            ImportDiagnostic::entry(ImportCode::ItemBytes, entry_index, None)
        }
        CatalogError::TotalItemBytesLimit { .. } => {
            ImportDiagnostic::whole(ImportCode::TotalItemBytes)
        }
        CatalogError::DuplicateItem {
            duplicate_index, ..
        } => ImportDiagnostic::entry(ImportCode::DuplicateItem, duplicate_index, None),
        CatalogError::VersionMismatch => ImportDiagnostic::whole(ImportCode::SourceGap),
    }
}

fn graph_code<BeatId, FactId, AlternativeId>(
    error: GraphError<BeatId, FactId, AlternativeId>,
) -> ImportCode {
    match error {
        GraphError::LimitExceeded(limit) => ImportCode::GraphCapacity(limit),
        GraphError::DuplicateBeat(_)
        | GraphError::DuplicateFact(_)
        | GraphError::DuplicateAlternative(_) => ImportCode::DuplicateReference,
        GraphError::MissingStart(_)
        | GraphError::MissingEdge { .. }
        | GraphError::MissingAlternativeOwner { .. }
        | GraphError::MissingAlternativeEntry { .. }
        | GraphError::MissingAlternativeExit { .. }
        | GraphError::MissingDisclosureFact { .. } => ImportCode::MissingReference,
        GraphError::Cycle { .. } => ImportCode::ReferenceCycle,
        GraphError::NoReachableExit
        | GraphError::UnreachableAlternativeOwner { .. }
        | GraphError::UnreachableAlternativeEntry { .. }
        | GraphError::AlternativeExitNotTerminal { .. }
        | GraphError::UnreachableAlternativeExit { .. } => ImportCode::UnreachableAlternative,
    }
}

fn safe_component(component: &str) -> bool {
    !component.is_empty()
        && component != "."
        && component != ".."
        && component
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

/// Validates the complete candidate before returning any accepted draft.
/// Refusal is deterministic and never repairs, truncates or mutates input.
/// Uploaded bytes and paths are borrowed; graph validation uses its existing
/// explicitly bounded traversal and identity-clone contract.
/// External source classification is checked first, without inspecting its string,
/// admission, decoded records or graph. No callback, I/O, clock or fetch port exists.
pub fn validate_import<
    'a,
    Version: Eq,
    Pins: Eq,
    Format: Eq,
    ItemId: Eq,
    BeatId: Ord + Clone,
    FactId: Ord + Clone,
    AlternativeId: Ord + Clone,
>(
    upload: &DecodedUpload<'a, Version, Pins, Format>,
    spec: &ImportSpec<'a, Version, Pins, Format>,
    catalog: &ImportCatalog<'a, ItemId>,
    graph: &GraphView<'_, BeatId, FactId, AlternativeId>,
) -> Result<ImportCandidate<'a, Version, Pins, Format>, ImportDiagnostic> {
    let bytes = match &upload.source {
        ImportSource::Uploaded(bytes) => *bytes,
        ImportSource::ExternalUrl(_) | ImportSource::ExternalFile(_) => {
            return Err(ImportDiagnostic::whole(ImportCode::ExternalSource));
        }
    };
    if upload.format != spec.format {
        return Err(ImportDiagnostic::whole(ImportCode::UnsupportedFormat));
    }
    let pins = match &spec.admission {
        ImportAdmission::Admitted(pins) => *pins,
        ImportAdmission::SourceGap => return Err(ImportDiagnostic::whole(ImportCode::SourceGap)),
        ImportAdmission::RightsGap => return Err(ImportDiagnostic::whole(ImportCode::RightsGap)),
        ImportAdmission::UnsupportedRules => {
            return Err(ImportDiagnostic::whole(ImportCode::UnsupportedRules));
        }
    };
    if upload.version != spec.version || upload.pins != pins {
        return Err(ImportDiagnostic::whole(ImportCode::SourceGap));
    }
    validate_entries(bytes, catalog.entries, spec.limits.catalog).map_err(catalog_diagnostic)?;
    if catalog.sources.len() != catalog.entries.len() {
        return Err(ImportDiagnostic::whole(ImportCode::SourceLocations));
    }
    if spec.namespace.len() > spec.limits.max_path_bytes || !safe_component(spec.namespace) {
        return Err(ImportDiagnostic::whole(ImportCode::Namespace));
    }
    let mut total_path_bytes = 0_usize;
    for (index, source) in catalog.sources.iter().enumerate() {
        if source.location.start > source.location.end || source.location.end > bytes.len() {
            return Err(ImportDiagnostic::entry(
                ImportCode::InvalidLocation,
                index,
                None,
            ));
        }
        let refusal = |code| ImportDiagnostic::entry(code, index, Some(source.location));
        if source.path.len() > spec.limits.max_path_bytes {
            return Err(refusal(ImportCode::PathBytes));
        }
        total_path_bytes = total_path_bytes
            .checked_add(source.path.len())
            .filter(|total| *total <= spec.limits.max_total_path_bytes)
            .ok_or_else(|| refusal(ImportCode::TotalPathBytes))?;
        let mut components = source.path.split('/');
        if components.next() != Some(spec.namespace) {
            return Err(refusal(ImportCode::Namespace));
        }
        let mut path_component_count = 1_usize;
        let mut has_nested_component = false;
        for component in components {
            path_component_count = path_component_count
                .checked_add(1)
                .ok_or_else(|| refusal(ImportCode::PathDepth))?;
            if path_component_count > spec.limits.max_path_components {
                return Err(refusal(ImportCode::PathDepth));
            }
            if !safe_component(component) {
                return Err(refusal(ImportCode::InvalidPath));
            }
            has_nested_component = true;
        }
        if !has_nested_component {
            return Err(refusal(ImportCode::InvalidPath));
        }
    }
    validate_graph(graph, spec.limits.graph)
        .map_err(|error| ImportDiagnostic::whole(graph_code(error)))?;
    Ok(ImportCandidate {
        bytes,
        version: upload.version,
        pins,
        format: upload.format,
        item_count: catalog.entries.len(),
    })
}
