//! What a dev caller sends: closed keys, the operation and audience a query names, and bodies
//! that refuse what they do not read.

use df_table::identity::OperationId;

use super::machine_io::WireIntentBody;
use super::request::{
    AudienceQuery, AudienceSpec, ClockControl, CommitFault, ProposalSource, TraceOperation,
    TraceQuery, ViableQuery,
};

fn serde_name(value: &impl serde::Serialize) -> String {
    match serde_json::to_value(value).expect("a closed key serializes") {
        serde_json::Value::String(name) => name,
        other => panic!("a closed key is a string, got {other}"),
    }
}

#[test]
fn proposal_sources_and_commit_faults_are_keyed_by_their_serde_names() {
    for source in ProposalSource::ALL {
        assert_eq!(source.key(), serde_name(&source));
        assert_eq!(ProposalSource::from_key(source.key()), Some(source));
    }
    for fault in CommitFault::ALL {
        assert_eq!(fault.key(), serde_name(&fault));
        assert_eq!(CommitFault::from_key(fault.key()), Some(fault));
    }
    assert_eq!(ProposalSource::from_key("weather"), None);
}

#[test]
fn an_operation_is_a_number_from_one_or_thirty_two_hexadecimal_digits() {
    let hex = "0000000000000000000000000000002a";

    let from_number = TraceOperation::new("42").expect("a number");
    let from_hex = TraceOperation::new(hex).expect("32 digits");

    assert_eq!(from_number, from_hex);
    assert_eq!(String::from(from_number), hex);
    assert_eq!(from_number.id(), OperationId::from_hex(hex).expect("an id"));
    assert!(TraceOperation::new("0").is_err());
    assert!(TraceOperation::new("zero").is_err());
    assert!(TraceOperation::new(&"g".repeat(32)).is_err());
}

#[test]
fn an_audience_is_written_and_read_as_tv_or_a_seat_number() {
    for audience in [AudienceSpec::Tv, AudienceSpec::Seat { seat: 3 }] {
        assert_eq!(
            AudienceSpec::from_parameter(&audience.parameter()),
            Ok(audience)
        );
    }
    assert!(AudienceSpec::from_parameter("everyone").is_err());
}

#[test]
fn the_queries_read_their_one_parameter_and_refuse_any_other() {
    let audience: AudienceQuery =
        serde_json::from_str(r#"{"audience":"tv"}"#).expect("an audience");
    assert_eq!(audience.audience.0, AudienceSpec::Tv);
    let trace: TraceQuery = serde_json::from_str(r#"{"operation":"7"}"#).expect("an operation");
    assert_eq!(trace.operation, TraceOperation::new("7").expect("a number"));
    assert!(serde_json::from_str::<ViableQuery>(r#"{"seat":1,"audience":"tv"}"#).is_err());
    assert!(serde_json::from_str::<ViableQuery>("{}").is_err());
}

#[test]
fn a_control_or_a_wire_intent_with_an_unknown_field_is_refused() {
    let extra = r#"{"control":"advance","ms":5,"by":1}"#;
    assert!(serde_json::from_str::<ClockControl>(extra).is_err());
    assert!(serde_json::from_str::<ClockControl>(r#"{"control":"advance","ms":5}"#).is_ok());
    let intent = r#"{"seat":1,"intent":{"kind":"tap","offer":"join"}}"#;
    assert!(serde_json::from_str::<WireIntentBody>(intent).is_ok());
    let extra = r#"{"seat":1,"intent":{"kind":"tap","offer":"join"},"seet":2}"#;
    assert!(serde_json::from_str::<WireIntentBody>(extra).is_err());
}
