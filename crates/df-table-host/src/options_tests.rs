use std::path::PathBuf;

use crate::options::{
    DEFAULT_KIND_DIRECTORY, DEFAULT_PORT, OptionError, parse_dev_arguments, parse_serve_options,
};

fn strings(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_owned()).collect()
}

#[test]
fn serve_without_options_uses_the_defaults() {
    let options = parse_serve_options(&[]).expect("parses");
    assert_eq!(options.port, DEFAULT_PORT);
    assert_eq!(
        options.kind_directory,
        PathBuf::from(DEFAULT_KIND_DIRECTORY)
    );
    assert!(!options.records_moves);
    assert!(options.echoes_lines);
}

#[test]
fn serve_reads_each_option() {
    let options = parse_serve_options(&strings(&[
        "--port", "9000", "--kinds", "k", "--moves", "--quiet",
    ]))
    .expect("parses");
    assert_eq!(options.port, 9000);
    assert_eq!(options.kind_directory, PathBuf::from("k"));
    assert!(options.records_moves);
    assert!(!options.echoes_lines);
}

#[test]
fn a_port_that_is_not_a_port_is_refused_with_its_text() {
    for bad in ["x", "-1", "65536", ""] {
        assert_eq!(
            parse_serve_options(&strings(&["--port", bad])),
            Err(OptionError::BadPort(bad.to_owned())),
            "{bad}"
        );
    }
}

#[test]
fn port_zero_is_allowed_for_an_ephemeral_port() {
    assert_eq!(
        parse_serve_options(&strings(&["--port", "0"]))
            .expect("parses")
            .port,
        0
    );
}

#[test]
fn an_option_without_its_value_and_an_unknown_option_are_refused() {
    assert_eq!(
        parse_serve_options(&strings(&["--port"])),
        Err(OptionError::MissingValue("--port"))
    );
    assert_eq!(
        parse_serve_options(&strings(&["--kinds"])),
        Err(OptionError::MissingValue("--kinds"))
    );
    assert_eq!(
        parse_serve_options(&strings(&["--loud"])),
        Err(OptionError::Unknown("--loud".into()))
    );
}

#[test]
fn an_unknown_option_error_shows_the_usage() {
    assert!(
        OptionError::Unknown("--loud".into())
            .to_string()
            .contains("df-table serve")
    );
}

#[test]
fn dev_takes_the_port_from_the_flag_then_the_environment_then_the_default() {
    assert_eq!(
        parse_dev_arguments(&strings(&["--port", "9", "canvas"]), Some("5")),
        Ok((9, strings(&["canvas"])))
    );
    assert_eq!(
        parse_dev_arguments(&strings(&["canvas"]), Some("5")),
        Ok((5, strings(&["canvas"])))
    );
    assert_eq!(
        parse_dev_arguments(&strings(&["canvas"]), None),
        Ok((DEFAULT_PORT, strings(&["canvas"])))
    );
}

#[test]
fn dev_refuses_a_bad_environment_port_and_a_flag_with_no_value() {
    assert_eq!(
        parse_dev_arguments(&strings(&["canvas"]), Some("lots")),
        Err(OptionError::BadPort("lots".into()))
    );
    assert_eq!(
        parse_dev_arguments(&strings(&["--port"]), None),
        Err(OptionError::MissingValue("--port"))
    );
}
