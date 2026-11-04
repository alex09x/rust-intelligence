/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Runnables the source text alone cannot show: tests and `main` functions that macros generate. The index holds
//! them, and it marks every test, whatever wrote it: rustc's test harness adds a `const` descriptor with the test
//! function's own path next to each `#[test]`, so a function sharing its path with a `const` in the file is a test.

use ridx_format::{DefKind, WorkspaceIndex};
use std::collections::{HashMap, HashSet};

/// A function name in the file: 0-based line, UTF-16 column and length.
#[derive(Debug, Clone, PartialEq)]
pub struct FnSite {
    pub col16: u32,
    pub len16: u32,
    pub name: String,
}

#[derive(Debug, Default)]
pub struct RunnableHints {
    /// Test functions by 0-based line of their name.
    pub tests: HashMap<u32, Vec<FnSite>>,
    /// `fn main` names, written or generated, by 0-based line.
    pub mains: HashMap<u32, FnSite>,
}

fn utf16_col(line: &str, char_col_1based: u32) -> u32 {
    line.chars().take(char_col_1based.saturating_sub(1) as usize).map(|c| c.len_utf16() as u32).sum()
}

pub fn runnable_hints(ws: &WorkspaceIndex, file: &str, text: &str) -> RunnableHints {
    let lines: Vec<&str> = text.lines().collect();
    let mut hints = RunnableHints::default();
    for idx in ws.reader_indices_for_file(file) {
        let Some(target) = ws.target_file_for_reader(idx, file) else {
            continue;
        };
        let r = &ws.readers()[idx];
        let Some(fid) = r.find_file_id(target.as_ref()) else {
            continue;
        };
        let defs: Vec<_> = r.definitions().iter().filter(|d| d.span.file_id == fid).collect();
        let const_paths: HashSet<&str> = defs.iter().filter(|d| DefKind::from(d.kind) == DefKind::Const).map(|d| r.get_str(d.path_offset())).collect();
        for d in defs.iter().filter(|d| matches!(DefKind::from(d.kind), DefKind::Fn | DefKind::AssocFn)) {
            let name = r.get_str(d.name_offset);
            let line0 = d.span.start_line.saturating_sub(1);
            let Some(line) = lines.get(line0 as usize) else {
                continue;
            };
            let col16 = utf16_col(line, d.span.start_col);
            let spelled: String = line.chars().skip(d.span.start_col.saturating_sub(1) as usize).take(name.chars().count()).collect();
            if name == "main" && spelled != name && d.span.end_line == d.span.start_line {
                // A generated `main` (`criterion_main!(benches);`) carries the span of its invocation; the lens covers it.
                let len16 = utf16_col(line, d.span.end_col).saturating_sub(col16);
                hints.mains.insert(line0, FnSite { col16, len16, name: name.to_string() });
                continue;
            }
            // Otherwise the span must spell the name, as for lenses: a macro may reuse the span of its whole invocation.
            if spelled != name {
                continue;
            }
            let site = FnSite { col16, len16: name.encode_utf16().count() as u32, name: name.to_string() };
            if name == "main" {
                hints.mains.insert(line0, site);
            } else if const_paths.contains(r.get_str(d.path_offset())) {
                let at_line = hints.tests.entry(line0).or_default();
                if !at_line.contains(&site) {
                    at_line.push(site);
                }
            }
        }
    }
    hints
}
