//! Generated common contracts and explicitly experimental S00 fixture contracts.
//!
//! Generation: `build.rs` runs the vendored protoc over `proto/*.proto` with `tonic-prost-build`
//! (messages and client and server stubs, no transport), and `FILE_DESCRIPTOR_SET` is the real
//! descriptor. `proto/field-ledger.txt` freezes every allocated name, number and presence; a new
//! field takes the next free number and a line there. The generic machine surface of the table
//! engine (`MachineView`, `MachineIntent` and their receipt, plan O.16 W0) is declared at the end
//! of `gameplay.proto`, served by `ActionService.SubmitMachineIntent` and
//! `SessionService.WatchMachine`, and tested in `tests/machine_contract.rs`; its guards are in
//! `df-api::machine`.
pub mod transport_fixture {
    tonic::include_proto!("dungeonflux.experimental.transport.v1");
}

pub mod public {
    #[allow(
        clippy::large_enum_variant,
        reason = "generated: the Audience oneof keeps its full views unboxed; boxing would \
                  change the generated contract"
    )]
    pub mod v1 {
        tonic::include_proto!("dungeonflux.public.v1");
    }
}

pub mod experimental {
    pub mod contract {
        pub mod v1 {
            tonic::include_proto!("dungeonflux.experimental.contract.v1");
        }
    }
}

pub use experimental::contract::v1 as contract_fixture;
pub use public::v1 as common;

/// Actual protoc descriptor bytes for the generated schemas, including field presence.
pub const FILE_DESCRIPTOR_SET: &[u8] = tonic::include_file_descriptor_set!("contracts");
