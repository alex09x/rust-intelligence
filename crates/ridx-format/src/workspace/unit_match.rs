/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Precomputed indices for cross-unit and cross-crate reference matching.
//!
//! Provides O(1) unit matching across workspace crates, avoiding quadratic scans
//! over readers and extern unit mappings.

use crate::reader::IndexReader;
use std::collections::{HashMap, HashSet};

/// Precomputed lookup index for workspace unit matching.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnitMatchIndex {
    /// Distinct non-empty `crate_version` count per `crate_name`.
    crate_version_counts: HashMap<String, usize>,
    /// Units defined directly by workspace readers for `crate_name`.
    crate_defined_units: HashMap<String, HashSet<String>>,
    /// All units (defined or referenced via extern_units) associated with `crate_name`.
    crate_all_units: HashMap<String, HashSet<String>>,
}

impl UnitMatchIndex {
    /// Builds a precomputed `UnitMatchIndex` from the workspace readers and metadata.
    pub fn build<B: AsRef<[u8]>>(
        readers: &[IndexReader<B>],
        unit_keys: &[Option<String>],
        reader_extern_units: &[HashMap<String, String>],
    ) -> Self {
        let mut versions_by_crate: HashMap<String, HashSet<String>> = HashMap::new();
        let mut crate_defined_units: HashMap<String, HashSet<String>> = HashMap::new();
        let mut crate_all_units: HashMap<String, HashSet<String>> = HashMap::new();

        for (j, r) in readers.iter().enumerate() {
            let crate_name = r.crate_name();
            let ver = r.crate_version();
            if !ver.is_empty() {
                versions_by_crate
                    .entry(crate_name.to_string())
                    .or_default()
                    .insert(ver.to_string());
            }
            if let Some(uk) = unit_keys.get(j).and_then(|u| u.as_deref()) {
                crate_defined_units
                    .entry(crate_name.to_string())
                    .or_default()
                    .insert(uk.to_string());
                crate_all_units
                    .entry(crate_name.to_string())
                    .or_default()
                    .insert(uk.to_string());
            }
        }

        for m in reader_extern_units {
            for (crate_name, uk) in m {
                crate_all_units
                    .entry(crate_name.clone())
                    .or_default()
                    .insert(uk.clone());
            }
        }

        let crate_version_counts = versions_by_crate
            .into_iter()
            .map(|(k, v)| (k, v.len()))
            .collect();

        Self {
            crate_version_counts,
            crate_defined_units,
            crate_all_units,
        }
    }

    /// Checks whether `ref_target_unit` matches `target_unit` for `target_crate`.
    ///
    /// Handles exact unit match, as well as single-version workspaces where Cargo
    /// assigned different unit hashes (e.g. lib vs test vs dep compilation).
    ///
    /// Evaluates in O(1) time without allocations.
    #[inline]
    pub fn matches_unit(
        &self,
        reader_idx: usize,
        ref_target_unit: &str,
        target_unit: &str,
        target_crate: &str,
        reader_extern_units: &[HashMap<String, String>],
    ) -> bool {
        if ref_target_unit == target_unit {
            return true;
        }

        let target_unit_in_crate = self
            .crate_all_units
            .get(target_crate)
            .map(|units| units.contains(target_unit))
            .unwrap_or(false);
        if !target_unit_in_crate {
            return false;
        }

        let target_versions_count = *self.crate_version_counts.get(target_crate).unwrap_or(&0);
        if target_versions_count > 1 {
            return false;
        }

        let in_defined_units = self
            .crate_defined_units
            .get(target_crate)
            .map(|units| units.contains(ref_target_unit))
            .unwrap_or(false);
        if in_defined_units {
            return true;
        }

        reader_extern_units
            .get(reader_idx)
            .and_then(|m| m.get(target_crate))
            .map(|u| u.as_str())
            == Some(ref_target_unit)
    }

    /// Number of distinct non-empty versions for `crate_name`.
    #[inline]
    pub fn crate_versions_count(&self, crate_name: &str) -> usize {
        *self.crate_version_counts.get(crate_name).unwrap_or(&0)
    }

    /// Whether `unit_key` is associated with `crate_name` (defined or extern).
    #[inline]
    pub fn is_unit_in_crate(&self, crate_name: &str, unit_key: &str) -> bool {
        self.crate_all_units
            .get(crate_name)
            .map(|units| units.contains(unit_key))
            .unwrap_or(false)
    }

    /// Whether `unit_key` is directly defined by a workspace reader for `crate_name`.
    #[inline]
    pub fn is_crate_defined_unit(&self, crate_name: &str, unit_key: &str) -> bool {
        self.crate_defined_units
            .get(crate_name)
            .map(|units| units.contains(unit_key))
            .unwrap_or(false)
    }
}
