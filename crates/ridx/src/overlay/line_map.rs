/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Line correspondence between a baseline text and the live buffer edited from it.

/// Which lines of a baseline text (the text an index or a diagnostic run was built from) still
/// stand in the live buffer. The longest common prefix and the longest common suffix of lines map
/// one to one and are textually identical; the lines between them have no counterpart. The map is
/// computed once from both texts and answers any number of line lookups; the per-line functions
/// in `mapping` are its single-line form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineMap {
    prefix: usize,
    suffix: usize,
    baseline_len: usize,
    live_len: usize,
}

impl LineMap {
    pub fn new(baseline_text: &str, live_text: &str) -> Self {
        let baseline: Vec<&str> = baseline_text.lines().collect();
        let live: Vec<&str> = live_text.lines().collect();
        let mut prefix = 0;
        while prefix < baseline.len() && prefix < live.len() && baseline[prefix] == live[prefix] {
            prefix += 1;
        }
        let mut suffix = 0;
        while suffix < baseline.len() - prefix && suffix < live.len() - prefix && baseline[baseline.len() - 1 - suffix] == live[live.len() - 1 - suffix] {
            suffix += 1;
        }
        Self { prefix, suffix, baseline_len: baseline.len(), live_len: live.len() }
    }

    /// Live line (1-based) of a baseline line, `None` when the edit removed or changed it.
    pub fn to_live(&self, baseline_line: u32) -> Option<u32> {
        Self::project(baseline_line, self.prefix, self.suffix, self.baseline_len, self.live_len)
    }

    /// Baseline line (1-based) of a live line, `None` when the line is new or changed.
    pub fn to_baseline(&self, live_line: u32) -> Option<u32> {
        Self::project(live_line, self.prefix, self.suffix, self.live_len, self.baseline_len)
    }

    fn project(line: u32, prefix: usize, suffix: usize, from_len: usize, to_len: usize) -> Option<u32> {
        let idx = line.saturating_sub(1) as usize;
        if idx >= from_len {
            return None;
        }
        if idx < prefix {
            return Some(line);
        }
        if idx >= from_len - suffix {
            let offset_from_end = from_len - 1 - idx;
            return Some((to_len - offset_from_end) as u32);
        }
        None
    }
}
