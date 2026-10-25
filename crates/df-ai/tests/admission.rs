use df_ai::admission::{
    CompleteRecord, RecordAdmissionError, RecordEvent, RecordIdentity, RecordIdentityField,
    RecordInputError, RecordLimit, RecordLimits, RecordingPublisher, admit_complete_record,
};
use df_types::{LocaleTag, OperationId, RevisionLabel};
use sha2::{Digest, Sha256};

#[derive(Clone, Eq, PartialEq)]
struct Key {
    locale: LocaleTag,
    schema: RevisionLabel,
    context: RevisionLabel,
}

#[derive(Clone, Eq, PartialEq)]
struct Basis {
    access_generation: u64,
    source: RevisionLabel,
}

#[derive(Debug, Eq, PartialEq)]
enum PublicationError {
    Stale,
    StorageUnavailable,
}

struct Store {
    current_basis: Basis,
    current_operation: OperationId,
    published: Vec<u8>,
    calls: usize,
    fail_storage: bool,
}

impl RecordingPublisher for Store {
    type Key = Key;
    type Basis = Basis;
    type Artifact = [u8; 32];
    type Error = PublicationError;

    fn publish_complete(
        &mut self,
        record: CompleteRecord<Key, Basis>,
    ) -> Result<Self::Artifact, Self::Error> {
        self.calls += 1;
        if record.identity().basis != self.current_basis
            || record.identity().operation != self.current_operation
        {
            return Err(PublicationError::Stale);
        }
        if self.fail_storage {
            return Err(PublicationError::StorageUnavailable);
        }
        assert_eq!(record.bytes(), b"abcdef");
        assert_eq!(*record.sha256(), digest(b"abcdef"));
        let (identity, bytes, sha256) = record.into_parts();
        assert_eq!(identity.key.locale.as_str(), "fr-ca");
        self.published = bytes;
        Ok(sha256)
    }
}

fn operation(operation_byte: u8) -> OperationId {
    OperationId::from_bytes(&[operation_byte; 16]).unwrap()
}

fn key() -> Key {
    Key {
        locale: LocaleTag::parse("fr-CA").unwrap(),
        schema: RevisionLabel::new(Some("qualified-v1")).unwrap(),
        context: RevisionLabel::new(Some("context-8")).unwrap(),
    }
}

fn basis() -> Basis {
    Basis {
        access_generation: 7,
        source: RevisionLabel::new(Some("source-3")).unwrap(),
    }
}

fn identity() -> RecordIdentity<Key, Basis> {
    RecordIdentity {
        key: key(),
        basis: basis(),
        operation: operation(1),
    }
}

fn store() -> Store {
    Store {
        current_basis: basis(),
        current_operation: operation(1),
        published: b"previous accepted record".to_vec(),
        calls: 0,
        fail_storage: false,
    }
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn complete() -> RecordEvent<Key, Basis> {
    RecordEvent::Complete {
        identity: identity(),
        byte_length: 6,
        sha256: digest(b"abcdef"),
    }
}

fn events() -> Vec<Result<RecordEvent<Key, Basis>, RecordInputError>> {
    vec![
        Ok(RecordEvent::Chunk(b"abc".to_vec())),
        Ok(RecordEvent::Chunk(b"def".to_vec())),
        Ok(complete()),
    ]
}

fn limits() -> RecordLimits {
    RecordLimits::new(6, 3, 2).unwrap()
}

#[test]
fn complete_exact_stream_publishes_once_after_observed_eof() {
    struct EofObserved {
        events: std::vec::IntoIter<Result<RecordEvent<Key, Basis>, RecordInputError>>,
        observed: std::rc::Rc<std::cell::Cell<bool>>,
    }
    impl Iterator for EofObserved {
        type Item = Result<RecordEvent<Key, Basis>, RecordInputError>;
        fn next(&mut self) -> Option<Self::Item> {
            let event = self.events.next();
            if event.is_none() {
                self.observed.set(true);
            }
            event
        }
    }
    struct Publisher {
        store: Store,
        eof: std::rc::Rc<std::cell::Cell<bool>>,
    }
    impl RecordingPublisher for Publisher {
        type Key = Key;
        type Basis = Basis;
        type Artifact = [u8; 32];
        type Error = PublicationError;
        fn publish_complete(
            &mut self,
            record: CompleteRecord<Key, Basis>,
        ) -> Result<Self::Artifact, Self::Error> {
            assert!(self.eof.get(), "publication preceded actual EOF");
            self.store.publish_complete(record)
        }
    }
    let eof = std::rc::Rc::new(std::cell::Cell::new(false));
    let stream = EofObserved {
        events: events().into_iter(),
        observed: eof.clone(),
    };
    let mut publisher = Publisher {
        store: store(),
        eof,
    };
    assert_eq!(
        admit_complete_record(&mut publisher, identity(), limits(), stream),
        Ok(digest(b"abcdef"))
    );
    assert_eq!(publisher.store.calls, 1);
    assert_eq!(publisher.store.published, b"abcdef");
}

#[test]
fn partial_and_missing_final_streams_preserve_previous_entry() {
    for stream in [
        vec![],
        vec![Ok(RecordEvent::Chunk(b"abc".to_vec()))],
        vec![
            Ok(RecordEvent::Chunk(b"abc".to_vec())),
            Ok(RecordEvent::Chunk(b"def".to_vec())),
        ],
    ] {
        let mut store = store();
        assert_eq!(
            admit_complete_record(&mut store, identity(), limits(), stream),
            Err(RecordAdmissionError::MissingCompletion)
        );
        assert_eq!(store.calls, 0);
        assert_eq!(store.published, b"previous accepted record");
    }
}

#[test]
fn failed_or_cancelled_input_even_after_final_never_publishes() {
    for error in [RecordInputError::Failed, RecordInputError::Cancelled] {
        for after_final in [false, true] {
            let mut stream = if after_final {
                events()
            } else {
                vec![Ok(RecordEvent::Chunk(b"abc".to_vec()))]
            };
            stream.push(Err(error));
            let mut store = store();
            assert_eq!(
                admit_complete_record(&mut store, identity(), limits(), stream),
                Err(RecordAdmissionError::Input(error))
            );
            assert_eq!(store.calls, 0);
            assert_eq!(store.published, b"previous accepted record");
        }
    }
}

#[test]
fn duplicate_final_and_trailing_chunks_are_refused() {
    for trailing in [complete(), RecordEvent::Chunk(vec![])] {
        let mut stream = events();
        stream.push(Ok(trailing));
        let mut store = store();
        assert_eq!(
            admit_complete_record(&mut store, identity(), limits(), stream),
            Err(RecordAdmissionError::TrailingEvent)
        );
        assert_eq!(store.calls, 0);
    }
}

#[test]
fn wrong_length_and_digest_are_refused_before_publication() {
    for (length, sha256, error) in [
        (7, digest(b"abcdef"), RecordAdmissionError::LengthMismatch),
        (6, digest(b"abcdeg"), RecordAdmissionError::DigestMismatch),
    ] {
        let mut stream = events();
        stream.pop();
        stream.push(Ok(RecordEvent::Complete {
            identity: identity(),
            byte_length: length,
            sha256,
        }));
        let mut store = store();
        assert_eq!(
            admit_complete_record(&mut store, identity(), limits(), stream),
            Err(error)
        );
        assert_eq!(store.calls, 0);
    }
}

#[test]
fn exact_key_basis_and_operation_are_independently_required() {
    for field in [
        RecordIdentityField::Key,
        RecordIdentityField::Basis,
        RecordIdentityField::Operation,
    ] {
        let mut changed = identity();
        match field {
            RecordIdentityField::Key => changed.key.locale = LocaleTag::parse("en").unwrap(),
            RecordIdentityField::Basis => changed.basis.access_generation += 1,
            RecordIdentityField::Operation => changed.operation = operation(2),
        }
        let mut stream = events();
        stream.pop();
        stream.push(Ok(RecordEvent::Complete {
            identity: changed,
            byte_length: 6,
            sha256: digest(b"abcdef"),
        }));
        let mut store = store();
        assert_eq!(
            admit_complete_record(&mut store, identity(), limits(), stream),
            Err(RecordAdmissionError::IdentityMismatch(field))
        );
        assert_eq!(store.calls, 0);
    }
}

#[test]
fn byte_and_chunk_count_limits_refuse_without_replacing_cache() {
    let cases = [
        (
            RecordLimits::new(6, 2, 3).unwrap(),
            vec![Ok(RecordEvent::Chunk(b"abc".to_vec()))],
            RecordLimit::ChunkBytes,
        ),
        (
            limits(),
            vec![
                Ok(RecordEvent::Chunk(b"abc".to_vec())),
                Ok(RecordEvent::Chunk(b"def".to_vec())),
                Ok(RecordEvent::Chunk(vec![])),
            ],
            RecordLimit::ChunkCount,
        ),
        (
            RecordLimits::new(5, 3, 2).unwrap(),
            events(),
            RecordLimit::RecordBytes,
        ),
    ];
    for (limits, stream, exceeded) in cases {
        let mut store = store();
        assert_eq!(
            admit_complete_record(&mut store, identity(), limits, stream),
            Err(RecordAdmissionError::LimitExceeded(exceeded))
        );
        assert_eq!(store.calls, 0);
        assert_eq!(store.published, b"previous accepted record");
    }
    for invalid in [(0, 1, 1), (1, 0, 1), (1, 1, 0), (1, 2, 1)] {
        assert!(RecordLimits::new(invalid.0, invalid.1, invalid.2).is_err());
    }
}

#[test]
fn stale_authority_and_storage_failure_preserve_previous_record() {
    for error in [
        PublicationError::Stale,
        PublicationError::StorageUnavailable,
    ] {
        let mut store = store();
        if error == PublicationError::Stale {
            store.current_basis.access_generation += 1;
        } else {
            store.fail_storage = true;
        }
        assert_eq!(
            admit_complete_record(&mut store, identity(), limits(), events()),
            Err(RecordAdmissionError::Publication(error))
        );
        assert_eq!(store.calls, 1);
        assert_eq!(store.published, b"previous accepted record");
    }
}
