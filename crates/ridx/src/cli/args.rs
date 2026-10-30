/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Command-line argument definitions and subcommands using clap.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "ridx",
    version = concat!(env!("CARGO_PKG_VERSION"), " (commit: ", env!("RIDX_BUILD_COMMIT"), ", dirty: ", env!("RIDX_BUILD_DIRTY"), ")"),
    about = "Ultra-fast code intelligence for Rust"
)]
pub struct Cli {
    /// Path to .ridx index file or workspace directory when running as LSP server without subcommand
    #[arg(long, short)]
    pub index_path: Option<PathBuf>,

    /// Write LSP server log output to file (rust-analyzer drop-in compatibility flag)
    #[arg(long, global = true)]
    pub log_file: Option<PathBuf>,

    /// Flush log lines immediately without buffering (rust-analyzer drop-in compatibility flag)
    #[arg(long, global = true)]
    pub no_log_buffering: bool,

    /// Increase logging verbosity (`-v`, `-vv`, `-vvv`; rust-analyzer drop-in compatibility flag)
    #[arg(short = 'v', long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    /// Decrease logging verbosity (rust-analyzer drop-in compatibility flag)
    #[arg(short = 'q', long, global = true)]
    pub quiet: bool,

    /// Communicate over stdio in LSP mode (editor client drop-in compatibility flag)
    #[arg(long, global = true)]
    pub stdio: bool,

    /// Print empty JSON configuration schema for editor drop-in compatibility (`rust-analyzer --print-config-schema`)
    #[arg(long)]
    pub print_config_schema: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

impl Cli {
    /// Resolves top-level CLI arguments into a concrete [`Commands`] variant,
    /// defaulting to [`Commands::Lsp`] when invoked without a subcommand (drop-in `rust-analyzer` mode).
    pub fn into_command(self) -> Commands {
        match self.command {
            Some(Commands::Lsp { index_path }) => Commands::Lsp {
                index_path: index_path.or(self.index_path),
            },
            Some(cmd) => cmd,
            None => Commands::Lsp {
                index_path: self.index_path,
            },
        }
    }
}

#[derive(Subcommand, Debug, PartialEq, Eq)]
pub enum Commands {
    /// Index a Cargo crate or workspace using ridx-driver and cargo check
    Index {
        /// Path to Cargo workspace or crate directory (default: current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output directory for .ridx index files (default: target/ridx)
        #[arg(long, short)]
        out_dir: Option<PathBuf>,

        /// Optional package filter (e.g. -p my_crate)
        #[arg(long, short)]
        package: Option<String>,

        /// Run cargo check in release mode
        #[arg(long)]
        release: bool,

        /// Space or comma separated list of features to activate
        #[arg(long)]
        features: Option<String>,

        /// Do not activate the `default` feature
        #[arg(long)]
        no_default_features: bool,

        /// Activate all available features
        #[arg(long)]
        all_features: bool,

        /// Build for the target triple
        #[arg(long)]
        target: Option<String>,
    },

    /// Inspect binary header, record counts, and memory layout of a .ridx index file or workspace
    Inspect {
        /// Path to .ridx index file or directory (default: auto-discover target/ridx)
        index_path: Option<PathBuf>,

        /// Output result as JSON
        #[arg(long)]
        json: bool,
    },

    /// Go to definition for a symbol path or cursor location (file:line:col)
    Def {
        /// First argument: index file/dir, or target symbol/location if index is omitted
        arg1: String,
        /// Second argument: target symbol/location if first argument was index path
        arg2: Option<String>,

        /// Output result as JSON
        #[arg(long)]
        json: bool,
    },

    /// Find all references to a given symbol path or cursor location (file:line:col)
    Refs {
        /// First argument: index file/dir, or target symbol/location if index is omitted
        arg1: String,
        /// Second argument: target symbol/location if first argument was index path
        arg2: Option<String>,

        /// Output result as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show inferred expression type at a cursor location (file:line:col)
    TypeAt {
        /// First argument: index file/dir, or location if index is omitted
        arg1: String,
        /// Second argument: location if first argument was index path
        arg2: Option<String>,

        /// Output result as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show combined hover information (signature + inferred type) at cursor location
    Hover {
        /// First argument: index file/dir, or location if index is omitted
        arg1: String,
        /// Second argument: location if first argument was index path
        arg2: Option<String>,

        /// Output result as JSON
        #[arg(long)]
        json: bool,
    },

    /// Search symbol definitions matching a query string across the workspace
    Symbols {
        /// First argument: index file/dir, or query if index is omitted
        arg1: String,
        /// Second argument: query if first argument was index path
        arg2: Option<String>,

        /// Output result as JSON
        #[arg(long)]
        json: bool,
    },

    /// Compare accuracy and latency against prod-code (rust-analyzer)
    Compare {
        /// First argument: index file/dir, workspace root, or target symbol/location
        #[arg(default_value = ".")]
        arg1: String,
        /// Second argument: target symbol/location if first argument was index path
        arg2: Option<String>,
        /// Optional explicit symbol flag
        #[arg(long, short)]
        symbol: Option<String>,

        /// Path to JSON file containing symmetric scenarios
        #[arg(long)]
        scenarios: Option<PathBuf>,

        /// Command to run ridx LSP (default: auto or "ridx lsp")
        #[arg(long)]
        ridx_cmd: Option<String>,

        /// Command to run rust-analyzer (default: "rust-analyzer")
        #[arg(long, default_value = "rust-analyzer")]
        ra_cmd: String,

        /// Capability to compare: def, refs, hover, symbols, callers, all (default: def)
        #[arg(long, short, default_value = "def")]
        capability: String,

        /// Output result as JSON
        #[arg(long)]
        json: bool,

        /// Optional path to index.scip for 3-way comparison against rust-analyzer scip
        #[arg(long)]
        scip_file: Option<PathBuf>,
    },

    /// Run as a Language Server Protocol (LSP) server over stdio
    Lsp {
        /// Path to .ridx index file or workspace directory (default: auto-discover)
        #[arg(long, short)]
        index_path: Option<PathBuf>,
    },
}
