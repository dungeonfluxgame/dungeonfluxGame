//! `services` on the dev command line: its grammar and the client call. It never prints and never
//! starts a server.
//!
//! The verb takes no words. The refusal stays typed: the dispatcher hands the reply to
//! `dev_client::reply_value`, whose `ReplyError` carries the `ServicesError`, and prints the report
//! with `format::format_services`.

use super::request::{ServicesReply, ServicesReport};
use super::routes::path;
use crate::command::CommandError;
use crate::dev::DevError;
use crate::dev_client;

pub const SERVICES_USAGE: &str = "services";

/// The `services` command: read the services report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Services;

/// Reads the words after `services`: there must be none.
pub fn parse(words: &[String]) -> Result<Services, CommandError> {
    match words {
        [] => Ok(Services),
        _ => Err(CommandError::Usage(SERVICES_USAGE)),
    }
}

/// Reads the services report from the host on `port`, as the route answered it.
pub async fn fetch(port: u16) -> Result<ServicesReply<ServicesReport>, DevError> {
    dev_client::get_json(port, path::SERVICES).await
}
