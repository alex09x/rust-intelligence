/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The JSON shape of one reference in `refs --json`.

use crate::cli::refs_query::reference_json;
use ridx_format::{DefKind, Span, WorkspaceReference};
use std::path::Path;

fn reference(is_macro_expansion: bool) -> WorkspaceReference<'static> {
    WorkspaceReference {
        crate_name: "test_crate",
        file: "src/main.rs",
        span: Span::new(0, 20, 5, 20, 12),
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion,
        source_root: Some(Path::new("/ws/test_crate")),
        unit_key: None,
    }
}

#[test]
fn test_reference_json_carries_span_file_and_macro_expansion_flag() {
    let plain = reference_json(&reference(false));
    assert_eq!(plain["crate"], "test_crate");
    assert_eq!(plain["file"], "/ws/test_crate/src/main.rs");
    assert_eq!((plain["start_line"].as_u64(), plain["start_col"].as_u64()), (Some(20), Some(5)));
    assert_eq!((plain["end_line"].as_u64(), plain["end_col"].as_u64()), (Some(20), Some(12)));
    assert_eq!(plain["kind"], "Fn");
    assert_eq!(plain["is_write"], false);
    assert_eq!(plain["is_macro_expansion"], false);

    let expanded = reference_json(&reference(true));
    assert_eq!(expanded["is_macro_expansion"], true);
    assert_eq!(expanded.as_object().unwrap().len(), 9);
}
