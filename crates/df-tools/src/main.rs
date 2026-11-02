#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    df_tools::runtime::install_panic_reporter();
    df_tools::runtime::run(async_main)
}

#[cfg(not(target_arch = "wasm32"))]
async fn async_main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = std::env::args_os().skip(1).take(7).collect();
    match df_tools::build_set::dispatch(&arguments)? {
        df_tools::build_set::CommandOutcome::Complete => Ok(()),
        df_tools::build_set::CommandOutcome::Serve(build) => df_tools::fixture::serve(build).await,
    }
}
#[cfg(target_arch = "wasm32")]
fn main() {}
