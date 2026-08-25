#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Ratatoskr `ChatGPT` Archive service process.
//!
//! Sequence, in this order and no other: load configuration, install
//! telemetry, connect the database when one is configured and apply the
//! schema, prepare the blob root, bind the operator listener, serve until
//! SIGTERM or SIGINT and drain within the configured bound.

use std::process::ExitCode;

fn main() -> ExitCode {
    ratatoskr_chatgpt_archive_service::main_result()
}
