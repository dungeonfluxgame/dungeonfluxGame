use std::collections::BTreeSet;
use std::num::NonZeroU16;

use crate::event::FailureCause;
use crate::ports::{
    Broker, Clock, Dice, Draw, EventSink, Interpreter, KnowledgeView, Narrator, PortError,
    PortName, RulesOwner,
};

fn d20() -> NonZeroU16 {
    NonZeroU16::new(20).expect("twenty is not zero")
}

#[test]
fn every_port_is_object_safe_and_send_and_sync() {
    fn shared<T: ?Sized + Send + Sync>() {}
    shared::<dyn RulesOwner>();
    shared::<dyn Interpreter>();
    shared::<dyn Narrator>();
    shared::<dyn Broker>();
    shared::<dyn Clock>();
    shared::<dyn Dice>();
    shared::<dyn KnowledgeView>();
    shared::<dyn EventSink>();
}

#[test]
fn a_die_value_must_be_on_the_die() {
    assert!(Draw::new(d20(), 1).is_some());
    assert!(Draw::new(d20(), 20).is_some());
    assert!(Draw::new(d20(), 0).is_none());
    assert!(Draw::new(d20(), 21).is_none());
    let draw = Draw::new(d20(), 17).expect("17 is on a d20");
    assert_eq!(draw.value(), 17);
    assert_eq!(draw.sides(), d20());
}

#[test]
fn every_failing_port_has_a_distinct_key_that_is_its_serialized_name() {
    let keys: BTreeSet<_> = PortName::ALL.iter().map(|port| port.key()).collect();
    assert_eq!(keys.len(), PortName::ALL.len());
    for port in PortName::ALL {
        assert_eq!(
            serde_json::to_string(port).expect("a port name serializes"),
            format!("\"{}\"", port.key())
        );
    }
}

#[test]
fn every_failure_cause_has_a_distinct_key_that_is_its_serialized_name() {
    let keys: BTreeSet<_> = FailureCause::ALL.iter().map(|cause| cause.key()).collect();
    assert_eq!(keys.len(), FailureCause::ALL.len());
    for cause in FailureCause::ALL {
        assert_eq!(
            serde_json::to_string(cause).expect("a cause serializes"),
            format!("\"{}\"", cause.key())
        );
    }
}

#[test]
fn a_port_error_names_the_port_and_the_cause_and_nothing_private() {
    let error = PortError::new(PortName::Interpreter, FailureCause::TextInvalid);
    assert_eq!(
        error.to_string(),
        "the interpreter port failed: text_invalid"
    );
}
