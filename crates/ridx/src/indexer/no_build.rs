/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Indexing with no build script or procedural macro executed (prod-code, m=20739: `build_scripts = false`): `ridx
//! index --no-build-scripts`, `RIDX_NO_BUILD_SCRIPTS=1`, or `initializationOptions: {"buildScripts": false}`. Every
//! compile goes through ridx-driver, which compiles a build script from `fn main() {}` and a procedural macro crate
//! from a stub declaring its macros (`ridx-driver/src/no_build.rs`), in a Cargo target directory of its own and a
//! variant of its own, so that no macro built otherwise is loaded. The compiler's diagnostics are not given then: a
//! derive expanded to nothing reports errors the code does not have.

use std::sync::atomic::{AtomicBool, Ordering};

static ON: AtomicBool = AtomicBool::new(false);

/// Turns the mode on or off for this process and the runs it starts.
pub fn set(on: bool) {
    ON.store(on, Ordering::Relaxed);
}

/// Turns the mode on for an LSP session whose `initialize` params say `initializationOptions: {"buildScripts": false}`.
pub fn from_initialize(params: &serde_json::Value) {
    if params["initializationOptions"]["buildScripts"] == serde_json::Value::Bool(false) {
        set(true);
    }
}

/// Whether the mode is on: set, or `RIDX_NO_BUILD_SCRIPTS=1` in the environment.
pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed) || std::env::var_os(ridx_format::NO_BUILD_ENV).is_some_and(|v| v == "1")
}

/// The environment the driver runs with in this mode (none when off).
pub fn envs() -> Vec<(String, String)> {
    if enabled() {
        vec![(ridx_format::NO_BUILD_ENV.to_string(), "1".to_string())]
    } else {
        Vec::new()
    }
}

/// The name of a variant or Cargo target directory `name` in this mode: of its own, its macros being stubs.
pub fn scoped(name: &str) -> String {
    if enabled() {
        format!("{}{name}", ridx_format::NO_BUILD_DIR_PREFIX)
    } else {
        name.to_string()
    }
}
