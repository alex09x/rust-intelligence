/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Command-line interface orchestration and subcommands.

pub mod args;
pub mod compare_cmd;
pub mod dispatch;
pub mod inspect;
pub mod queries;
pub mod refs_query;
pub mod resolve;

#[cfg(test)]
pub mod tests;

pub use args::*;
pub use compare_cmd::*;
pub use dispatch::*;
pub use inspect::*;
pub use queries::*;

use clap::Parser;

pub fn run() {
    let cli = Cli::parse();
    if cli.log_file.is_some() || cli.verbose > 0 || cli.quiet {
        crate::lsp::logger::init_lsp_logger(
            cli.log_file.clone(),
            cli.no_log_buffering,
            cli.verbose,
            cli.quiet,
        );
    }
    if cli.print_config_schema {
        println!("{{}}");
        return;
    }
    run_cli(cli.into_command());
    crate::lsp::logger::flush_logger();
}
