fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = tonic_prost_build::Config::new();
    for field in [
        ".dungeonflux.public.v1.DecisionReceipt.outcome.accepted",
        ".dungeonflux.public.v1.SubmitActionResponse.outcome.committed_decision",
        ".dungeonflux.public.v1.JoinRoomResponse.outcome.joined",
        ".dungeonflux.public.v1.ReclaimRoomResponse.outcome.joined",
    ] {
        config.boxed(field);
    }
    config.protoc_executable(protoc_bin_vendored::protoc_bin_path()?);
    let descriptor = std::path::PathBuf::from(std::env::var("OUT_DIR")?).join("contracts.bin");
    tonic_prost_build::configure()
        .build_transport(false)
        .file_descriptor_set_path(descriptor)
        .compile_with_config(
            config,
            &[
                "proto/transport_fixture.proto",
                "proto/common.proto",
                "proto/contract_fixture.proto",
                "proto/gameplay.proto",
            ],
            &["proto"],
        )?;
    for schema in [
        "transport_fixture",
        "common",
        "contract_fixture",
        "gameplay",
    ] {
        println!("cargo:rerun-if-changed=proto/{schema}.proto");
    }
    Ok(())
}
