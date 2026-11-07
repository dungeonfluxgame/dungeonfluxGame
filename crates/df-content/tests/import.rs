use df_content::catalog::{CatalogEntry, CatalogLimits, CatalogSnapshot};
use df_content::graph::{GraphBeat, GraphLimits, GraphView};
use df_content::import::{
    DecodedUpload, ImportAdmission, ImportCatalog, ImportCode, ImportDiagnostic, ImportEntrySource,
    ImportLimits, ImportLocation, ImportSource, ImportSpec, validate_import,
};

#[derive(Eq, PartialEq)]
enum Format {
    CatalogGraph,
    Script,
    Archive,
    Unknown,
}

#[derive(Eq, PartialEq)]
struct Pins {
    schema: u64,
    source_checksum: u64,
    reviewed_rights: u64,
}

const VERSION: u64 = 3;
const PINS: Pins = Pins {
    schema: 1,
    source_checksum: 7,
    reviewed_rights: 11,
};
const FORMAT: Format = Format::CatalogGraph;
const BYTES: &[u8] = b"private uploaded lore with https://secret.invalid/key";
const ITEM: u64 = 9;
const BEAT: u64 = 1;
static ENTRIES: std::sync::LazyLock<[CatalogEntry<'static, u64>; 1]> =
    std::sync::LazyLock::new(|| [CatalogEntry::new(&ITEM, b"private")]);
const SOURCES: &[ImportEntrySource<'_>] = &[ImportEntrySource {
    path: "campaign/scene",
    location: ImportLocation { start: 0, end: 7 },
}];
const BEATS: &[GraphBeat<'_, u64>] = &[GraphBeat {
    id: &BEAT,
    next: &[],
    terminal: true,
}];

fn limits() -> ImportLimits {
    ImportLimits {
        catalog: CatalogLimits {
            max_complete_bytes: 128,
            max_entries: 2,
            max_item_bytes: 16,
            max_total_item_bytes: 32,
        },
        graph: GraphLimits {
            max_beats: 2,
            max_edges: 2,
            max_facts: 2,
            max_alternatives: 2,
            max_work: 100,
        },
        max_path_bytes: 64,
        max_path_components: 4,
        max_total_path_bytes: 128,
    }
}

fn upload() -> DecodedUpload<'static, u64, Pins, Format> {
    DecodedUpload {
        source: ImportSource::Uploaded(BYTES),
        version: &VERSION,
        pins: &PINS,
        format: &FORMAT,
    }
}

fn spec() -> ImportSpec<'static, u64, Pins, Format> {
    ImportSpec {
        version: &VERSION,
        admission: ImportAdmission::Admitted(&PINS),
        format: &FORMAT,
        namespace: "campaign",
        limits: limits(),
    }
}

fn graph() -> GraphView<'static, u64, u64, u64> {
    GraphView {
        start: &BEAT,
        beats: BEATS,
        facts: &[],
        alternatives: &[],
    }
}

fn catalog() -> ImportCatalog<'static, u64> {
    ImportCatalog {
        entries: &*ENTRIES,
        sources: SOURCES,
    }
}

fn error(spec: &ImportSpec<'_, u64, Pins, Format>) -> ImportDiagnostic {
    validate_import(&upload(), spec, &catalog(), &graph())
        .err()
        .unwrap()
}

#[test]
fn explicit_bounded_admitted_upload_returns_exact_candidate_and_safe_diagnostics() {
    let candidate = validate_import(&upload(), &spec(), &catalog(), &graph()).unwrap();
    assert_eq!(candidate.bytes(), BYTES);
    assert!(std::ptr::eq(candidate.bytes().as_ptr(), BYTES.as_ptr()));
    assert!(std::ptr::eq(candidate.version(), &VERSION));
    assert!(std::ptr::eq(candidate.pins(), &PINS));
    assert!(std::ptr::eq(candidate.format(), &FORMAT));
    assert_eq!(candidate.item_count(), 1);
    assert_eq!(ENTRIES[0].bytes(), b"private");
    assert!(graph().beats[0].next.is_empty());
}

#[test]
fn external_arbitrary_url_is_refused_before_any_source_or_body_processing() {
    struct Uninspectable;
    impl PartialEq for Uninspectable {
        fn eq(&self, _: &Self) -> bool {
            panic!("external input reached typed body processing")
        }
    }
    impl Eq for Uninspectable {}
    let pin = Uninspectable;
    let current = ImportSpec {
        version: &pin,
        admission: ImportAdmission::SourceGap,
        format: &pin,
        namespace: "../bad",
        limits: limits(),
    };
    for source in [
        ImportSource::ExternalUrl("https://private.invalid/password"),
        ImportSource::ExternalFile("/private/secret"),
    ] {
        let incoming = DecodedUpload {
            source,
            version: &pin,
            pins: &pin,
            format: &pin,
        };
        let result = validate_import(&incoming, &current, &catalog(), &graph())
            .err()
            .unwrap();
        assert_eq!(
            result,
            ImportDiagnostic {
                code: ImportCode::ExternalSource,
                entry_index: None,
                location: None
            }
        );
    }
}

#[test]
fn oversize_count_nesting_path_and_diagnostic_bounds_refuse_without_allocation_escape() {
    let mut current = spec();
    current.limits.catalog.max_complete_bytes = BYTES.len() - 1;
    assert_eq!(error(&current).code, ImportCode::CompleteBytes);
    current = spec();
    current.limits.catalog.max_entries = 0;
    assert_eq!(error(&current).code, ImportCode::EntryCount);
    current = spec();
    current.limits.catalog.max_item_bytes = 6;
    assert_eq!(error(&current).code, ImportCode::ItemBytes);
    current = spec();
    current.limits.catalog.max_total_item_bytes = 6;
    assert_eq!(error(&current).code, ImportCode::TotalItemBytes);
    current = spec();
    current.limits.max_path_bytes = 13;
    assert_eq!(error(&current).code, ImportCode::PathBytes);
    current = spec();
    current.limits.max_path_components = 1;
    assert_eq!(error(&current).code, ImportCode::PathDepth);
    current = spec();
    current.limits.max_total_path_bytes = 13;
    assert_eq!(error(&current).code, ImportCode::TotalPathBytes);
    current = spec();
    current.limits.graph.max_work = 0;
    assert_eq!(
        error(&current).code,
        ImportCode::GraphCapacity(df_content::graph::GraphLimit::Work)
    );
    assert!(std::mem::size_of::<ImportDiagnostic>() <= 64);
    assert_eq!(
        BYTES,
        b"private uploaded lore with https://secret.invalid/key"
    );
}

#[test]
fn traversal_script_archive_unknown_format_and_duplicates_are_typed_refusals() {
    for format in [&Format::Script, &Format::Archive, &Format::Unknown] {
        let incoming = DecodedUpload { format, ..upload() };
        assert_eq!(
            validate_import(&incoming, &spec(), &catalog(), &graph())
                .err()
                .unwrap()
                .code,
            ImportCode::UnsupportedFormat
        );
    }
    for path in [
        "campaign/../secret",
        "campaign/./scene",
        "campaign//scene",
        "campaign/\\secret",
        "campaign/https://secret",
        "campaign",
        "/campaign/scene",
        "campaign-other/scene",
        "",
    ] {
        let sources = [ImportEntrySource {
            path,
            location: ImportLocation { start: 0, end: 7 },
        }];
        let input = ImportCatalog {
            sources: &sources,
            ..catalog()
        };
        let result = validate_import(&upload(), &spec(), &input, &graph())
            .err()
            .unwrap();
        assert!(matches!(
            result.code,
            ImportCode::InvalidPath | ImportCode::Namespace
        ));
    }
    let entries = [
        CatalogEntry::new(&ITEM, b"first"),
        CatalogEntry::new(&ITEM, b"second"),
    ];
    let input = ImportCatalog {
        entries: &entries,
        sources: SOURCES,
    };
    assert_eq!(
        validate_import(&upload(), &spec(), &input, &graph())
            .err()
            .unwrap()
            .code,
        ImportCode::DuplicateItem
    );
}

#[test]
fn missing_rights_source_or_rules_admission_stays_explicit_and_cannot_publish() {
    let mut current = spec();
    for (admission, expected) in [
        (ImportAdmission::SourceGap, ImportCode::SourceGap),
        (ImportAdmission::RightsGap, ImportCode::RightsGap),
        (
            ImportAdmission::UnsupportedRules,
            ImportCode::UnsupportedRules,
        ),
    ] {
        current.admission = admission;
        assert_eq!(error(&current).code, expected);
    }
    let changed_pins = Pins {
        reviewed_rights: 12,
        ..PINS
    };
    let incoming = DecodedUpload {
        pins: &changed_pins,
        ..upload()
    };
    assert_eq!(
        validate_import(&incoming, &spec(), &catalog(), &graph())
            .err()
            .unwrap()
            .code,
        ImportCode::SourceGap
    );
    let incoming = DecodedUpload {
        version: &4,
        ..upload()
    };
    assert_eq!(
        validate_import(&incoming, &spec(), &catalog(), &graph())
            .err()
            .unwrap()
            .code,
        ImportCode::SourceGap
    );
}

#[test]
fn private_bytes_and_urls_do_not_leak_into_diagnostics_or_mutate_catalog() {
    let published =
        CatalogSnapshot::from_published(&VERSION, &PINS, BYTES, &*ENTRIES, limits().catalog)
            .unwrap();
    let mut current = spec();
    current.limits.catalog.max_complete_bytes = 0;
    let result = error(&current);
    let diagnostic = format!("{result:?}");
    assert!(!diagnostic.contains("private"));
    assert!(!diagnostic.contains("https"));
    assert!(!diagnostic.contains("secret"));
    assert!(!diagnostic.contains("campaign"));
    assert_eq!(result, error(&current));
    assert_eq!(
        published.get(&VERSION, &ITEM),
        Ok(Some(b"private".as_slice()))
    );
    assert_eq!(published.complete_bytes(), BYTES);
    assert_eq!(published.pins().reviewed_rights, 11);
}

#[test]
fn source_locations_are_checked_and_never_return_unvalidated_offsets() {
    for location in [
        ImportLocation { start: 8, end: 7 },
        ImportLocation {
            start: 0,
            end: usize::MAX,
        },
    ] {
        let sources = [ImportEntrySource {
            path: "campaign/scene",
            location,
        }];
        let input = ImportCatalog {
            sources: &sources,
            ..catalog()
        };
        let result = validate_import(&upload(), &spec(), &input, &graph())
            .err()
            .unwrap();
        assert_eq!(result.code, ImportCode::InvalidLocation);
        assert_eq!(result.entry_index, Some(0));
        assert_eq!(result.location, None);
    }
    let input = ImportCatalog {
        sources: &[],
        ..catalog()
    };
    assert_eq!(
        validate_import(&upload(), &spec(), &input, &graph())
            .err()
            .unwrap()
            .code,
        ImportCode::SourceLocations
    );
    let mut current = spec();
    current.limits.max_path_components = 1;
    let result = error(&current);
    assert_eq!(result.location, Some(ImportLocation { start: 0, end: 7 }));
}

#[test]
fn canonical_graph_missing_reference_and_cycles_are_redacted_without_repair() {
    let private_id = "PRIVATE_GRAPH_SECRET";
    let next = [private_id];
    let beats = [GraphBeat {
        id: &"start",
        next: &next,
        terminal: true,
    }];
    let input: GraphView<'_, &str, &str, &str> = GraphView {
        start: &"start",
        beats: &beats,
        facts: &[],
        alternatives: &[],
    };
    let result = validate_import(&upload(), &spec(), &catalog(), &input)
        .err()
        .unwrap();
    assert_eq!(result.code, ImportCode::MissingReference);
    assert!(!format!("{result:?}").contains(private_id));
    let beats = [GraphBeat {
        id: &private_id,
        next: &next,
        terminal: true,
    }];
    let input: GraphView<'_, &str, &str, &str> = GraphView {
        start: &private_id,
        beats: &beats,
        facts: &[],
        alternatives: &[],
    };
    assert_eq!(
        validate_import(&upload(), &spec(), &catalog(), &input)
            .err()
            .unwrap()
            .code,
        ImportCode::ReferenceCycle
    );
    assert_eq!(next, [private_id]);
}
