/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Aggregated end-to-end and integration test suite for Rust Intelligence (ridx).

mod cli;
mod ntm_compare_harness;
mod ntm_e2e;
mod ntm_flycheck_and_auto_import;
mod ntm_formatting_execution;
mod ntm_hierarchy_stress;
mod ntm_overlay_mutation;
mod ntm_refactoring_execution;
mod ntm_rename_execution;
mod ntm_review_repro;
mod ntm_symmetric_runner;
mod ntm_version_and_atomic_publish;
mod ntm_version_precondition;
mod ntm_workspace_edit;
mod scip_loader;
