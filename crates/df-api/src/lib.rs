//! Common generated wire fields and closed structural game-command admission.
//! No production action request or service is declared in the current protocol schema.
//! These functions do not authenticate, bind a client, decide rules, or commit a receipt.
//! The `machine` module guards the generic machine view and intent of the table engine.
#[cfg(test)]
mod customer;
pub mod machine;
#[cfg(test)]
mod machine_receipt_tests;
#[cfg(test)]
mod machine_tests;
mod request;
mod wire_decode;

pub use request::{
    CommandFields, RequestError, RequestField, admit_game_command, build_identity,
    client_binding_id, map_game_command, member_id, operation_id, run_id, session_id,
    session_revision,
};

pub use wire_decode::{
    EncodedCommandFields, WireDecodeError, WireDecodeLimits, decode_build_identity,
    decode_client_binding_id, decode_member_id, decode_operation_id, decode_recovery_epoch,
    decode_run_id, decode_session_id, decode_session_revision, map_encoded_game_command,
};
