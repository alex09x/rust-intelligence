/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Standard library path mapping for symbol resolution.
//!
//! Standard library types in rustc's canonical definitions (produced with `with_no_visible_paths!`)
//! originate in `core` or `alloc` rather than `std`. This module maps `std::...` query paths to
//! their canonical underlying `core::...` and `alloc::...` locations.

/// Returns candidate paths for a query symbol path.
///
/// If the path starts with `"std::"`, candidates include:
/// 1. The original `"std::..."` path.
/// 2. The `"core::..."` path equivalent.
/// 3. The `"alloc::..."` path equivalent.
///
/// Otherwise, returns only the original path.
pub fn std_path_candidates(path: &str) -> Vec<String> {
    let mut candidates = Vec::with_capacity(3);
    candidates.push(path.to_string());
    if let Some(rest) = path.strip_prefix("std::") {
        candidates.push(format!("core::{rest}"));
        candidates.push(format!("alloc::{rest}"));
    }
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_std_path_candidates_with_std() {
        let candidates = std_path_candidates("std::vec::Vec");
        assert_eq!(
            candidates,
            vec!["std::vec::Vec", "core::vec::Vec", "alloc::vec::Vec"]
        );
    }

    #[test]
    fn test_std_path_candidates_without_std() {
        let candidates = std_path_candidates("my_crate::Foo");
        assert_eq!(candidates, vec!["my_crate::Foo"]);
    }

    #[test]
    fn test_workspace_find_definition_by_path_std_mapping() {
        use crate::format::DefKind;
        use crate::workspace::WorkspaceIndex;
        use crate::writer::{IndexWriter, RawDefinition};
        use std::fs::File;

        let mut w = IndexWriter::new("alloc", "1.0.0");
        w.add_definition(RawDefinition {
            def_id: 1,
            name: "Vec".to_string(),
            path: "alloc::vec::Vec".to_string(),
            kind: DefKind::Struct,
            visibility: 1,
            is_macro_expansion: false,
            file: "alloc/src/vec/mod.rs".to_string(),
            start_line: 10,
            start_col: 1,
            end_line: 10,
            end_col: 10,
            signature: None,
        });

        let temp_dir = std::env::temp_dir().join(format!("ridx_std_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let ridx_path = temp_dir.join("alloc.ridx");
        {
            let mut fc = File::create(&ridx_path).unwrap();
            w.write_to(&mut fc).unwrap();
        }

        let ws = WorkspaceIndex::open(&ridx_path).unwrap();

        // 1. Direct canonical query
        let def1 = ws
            .find_definition_by_path("alloc::vec::Vec")
            .expect("found alloc::vec::Vec");
        assert_eq!(def1.name, "Vec");

        // 2. Query mapped from std::
        let def2 = ws
            .find_definition_by_path("std::vec::Vec")
            .expect("found std::vec::Vec via std mapping");
        assert_eq!(def2.name, "Vec");
        assert_eq!(def2.path, "alloc::vec::Vec");

        // 3. Search symbols mapped from std::
        let syms = ws.search_symbols("std::vec::Vec");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Vec");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
