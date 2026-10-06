/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Main CLI command dispatcher.

use super::args::Commands;
use super::inspect::cmd_inspect;
use super::queries::execute_query_command;
use super::resolve::resolve_workspace;
use crate::indexer::{run_indexing, IndexOptions};
use crate::lsp;
use colored::*;

pub fn run_cli(command: Commands) {
    match command {
        Commands::Index {
            path,
            out_dir,
            package,
            release,
            features,
            no_default_features,
            all_features,
            target,
        } => {
            run_indexing(IndexOptions {
                path,
                out_dir_opt: out_dir,
                package,
                release,
                features,
                no_default_features,
                all_features,
                target,
            });
        }
        Commands::Inspect { index_path, json } => {
            let ws = resolve_workspace(index_path);
            cmd_inspect(&ws, json);
        }
        Commands::Lsp { index_path } => {
            if let Err(e) = lsp::run_lsp_server(index_path) {
                eprintln!(
                    "{}: LSP server exited with error: {e}",
                    "Error".red().bold()
                );
                std::process::exit(1);
            }
        }
        cmd => {
            execute_query_command(cmd);
        }
    }
}
