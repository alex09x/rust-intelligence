/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Comparison module: benchmarks and compares query results between RIDX and rust-analyzer.

pub mod client;
pub mod def_refs;
pub mod harness;
pub mod hover_syms;
pub mod query_runner;
pub mod symmetric;
pub mod system;
pub mod transport;
pub mod types;

#[cfg(test)]
mod tests;

pub use query_runner::*;
pub use symmetric::*;
pub use transport::*;

pub use def_refs::{compare_definition, compare_references};
pub use hover_syms::{compare_callers, compare_hover, compare_symbols};
pub use system::{
    get_current_process_rss_bytes, get_process_rss_bytes, get_process_rss_bytes_fallback_ps,
    get_rust_analyzer_rss_bytes, print_comparison, print_comparison_summary,
};
pub use types::{CapabilityKind, ComparisonResult};

use ridx_format::WorkspaceIndex;

pub fn compare_capability(
    workspace: &WorkspaceIndex,
    capability: CapabilityKind,
    target: &str,
) -> Vec<ComparisonResult> {
    match capability {
        CapabilityKind::Def => vec![compare_definition(workspace, target)],
        CapabilityKind::Refs => vec![compare_references(workspace, target)],
        CapabilityKind::Hover => vec![compare_hover(workspace, target)],
        CapabilityKind::Symbols => vec![compare_symbols(workspace, target)],
        CapabilityKind::Callers => vec![compare_callers(workspace, target)],
        CapabilityKind::All => vec![
            compare_definition(workspace, target),
            compare_references(workspace, target),
            compare_hover(workspace, target),
            compare_symbols(workspace, target),
            compare_callers(workspace, target),
        ],
    }
}
