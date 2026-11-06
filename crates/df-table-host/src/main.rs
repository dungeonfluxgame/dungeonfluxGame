use std::io::Write;
use std::process::ExitCode;

use df_table_host::command::usage_text;
use df_table_host::dev::run_dev;
use df_table_host::options::{
    PORT_VARIABLE, SERVE_USAGE, parse_dev_arguments, parse_serve_options,
};
use df_table_host::repl::run_repl;
use df_table_host::serve::{run_serve, write_terminal_line};

fn fail(text: &str) -> ExitCode {
    // A closed stderr leaves nothing else to report to.
    let _ = writeln!(std::io::stderr().lock(), "{text}");
    ExitCode::FAILURE
}

#[tokio::main]
async fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match arguments.split_first() {
        Some((mode, rest)) if mode == "serve" => {
            let options = match parse_serve_options(rest) {
                Ok(options) => options,
                Err(error) => return fail(&error.to_string()),
            };
            match run_serve(options, write_terminal_line).await {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => fail(&error.to_string()),
            }
        }
        Some((mode, rest)) if mode == "dev" => {
            let environment_port = df_table_host::environment::text(PORT_VARIABLE);
            let (port, words) = match parse_dev_arguments(rest, environment_port.as_deref()) {
                Ok(parsed) => parsed,
                Err(error) => return fail(&error.to_string()),
            };
            if words == ["repl"] {
                return match run_repl(port, std::io::BufReader::new(std::io::stdin())).await {
                    0 => ExitCode::SUCCESS,
                    _ => ExitCode::FAILURE,
                };
            }
            match run_dev(port, &words).await {
                Ok(text) => {
                    write_terminal_line(&text);
                    ExitCode::SUCCESS
                }
                Err(error) => fail(&error.to_string()),
            }
        }
        _ => fail(&format!(
            "df-table serve ...  or  df-table dev ...\n\n{SERVE_USAGE}\n\n{}",
            usage_text()
        )),
    }
}
