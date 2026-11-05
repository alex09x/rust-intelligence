/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Options and results for workspace indexing variants.

use super::builder::IndexBuilder;
use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct IndexOptions {
    pub path: PathBuf,
    pub out_dir_opt: Option<PathBuf>,
    pub package: Option<String>,
    pub release: bool,
    pub features: Option<String>,
    pub no_default_features: bool,
    pub all_features: bool,
    pub target: Option<String>,
    pub cache_dir: Option<PathBuf>,
    pub driver_bin: Option<PathBuf>,
    pub capture_child_output: bool,
    pub builder: Option<IndexBuilder>,
}

#[derive(Debug, Clone)]
pub struct PublishedVariant {
    pub base_out_dir: PathBuf,
    pub variant_out_dir: PathBuf,
    #[allow(dead_code)]
    pub variant_id: String,
}
