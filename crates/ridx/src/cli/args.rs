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

#[derive(Parser)]
#[command(name = "ridx", about = "Ultra-fast code intelligence for Rust")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
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
        /// First argument: index file/dir, or target symbol/location
        arg1: String,
        /// Second argument: target symbol/location if first argument was index path
        arg2: Option<String>,
        /// Optional explicit symbol flag
        #[arg(long, short)]
        symbol: Option<String>,

        /// Capability to compare: def, refs, hover, symbols, callers, all (default: def)
        #[arg(long, short, default_value = "def")]
        capability: String,

        /// Output result as JSON
        #[arg(long)]
        json: bool,
    },

    /// Run as a Language Server Protocol (LSP) server over stdio
    Lsp {
        /// Path to .ridx index file or workspace directory (default: auto-discover)
        #[arg(long, short)]
        index_path: Option<PathBuf>,
    },
}
