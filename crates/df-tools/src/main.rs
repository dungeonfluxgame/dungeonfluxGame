#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = std::env::args_os().skip(1).take(7).collect();
    if arguments.first().is_some_and(|argument| {
        argument == "--gameplay-demo"
            || argument == "--roll-adventure"
            || argument == "--roll-campaign"
            || argument == "--simulate-campaign"
            || argument == "--campaign-run"
            || argument == "--campaign-status"
    }) {
        df_tools::gameplay::serve().await
    } else {
        match df_tools::build_set::dispatch(&arguments)? {
            df_tools::build_set::CommandOutcome::Complete => Ok(()),
            df_tools::build_set::CommandOutcome::Serve(build) => {
                df_tools::fixture::serve(build).await
            }
        }
    }
}
#[cfg(target_arch = "wasm32")]
fn main() {}
