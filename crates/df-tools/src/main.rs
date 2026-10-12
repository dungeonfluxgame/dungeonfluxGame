#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    df_tools::runtime::install_panic_reporter();
    #[cfg(windows)]
    let console_stop = df_tools::runtime::install_console_stop_handler();
    #[cfg(windows)]
    let result = df_tools::runtime::run(|| async_main(console_stop));
    #[cfg(not(windows))]
    let result = df_tools::runtime::run(|| async_main());
    result
}

#[cfg(not(target_arch = "wasm32"))]
async fn async_main(
    #[cfg(windows)] console_stop: tokio::sync::watch::Receiver<bool>,
) -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = std::env::args_os().skip(1).take(7).collect();
    // DEVCTL-001: the agent command line is a mode of this binary, not a new
    // executable. It needs every argument, unlike the modes below.
    if arguments.first().is_some_and(|argument| argument == "dev") {
        let code = df_tools::dev_cli::run(&std::env::args_os().skip(1).collect::<Vec<_>>()).await;
        std::process::exit(code);
    }
    if arguments.first().is_some_and(|argument| {
        argument == "--gameplay-demo"
            || argument == "--roll-adventure"
            || argument == "--roll-campaign"
            || argument == "--simulate-campaign"
            || argument == "--campaign-run"
            || argument == "--campaign-status"
    }) {
        df_tools::gameplay::serve(
            #[cfg(windows)]
            console_stop,
        )
        .await
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
