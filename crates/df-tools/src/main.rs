#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == "--prepare-load-assets")
    {
        let path = std::env::args_os()
            .nth(2)
            .ok_or("load asset directory required")?;
        df_tools::fake_profile::prepare_assets(std::path::Path::new(&path))?;
        return Ok(());
    }
    // SAFETY: startup precedes the runtime and every thread this executable creates.
    if let Some(profile) = unsafe { df_tools::fake_profile::apply_from_environment() }? {
        println!("{}", profile.startup_line());
    }
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
