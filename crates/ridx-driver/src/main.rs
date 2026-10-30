/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compiler driver wrapper that intercepts compilation during `cargo check` and writes
//! a zero-copy `.ridx` binary index file for the compiled crate.
#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;

use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::time::Instant;

mod sanitize;
use sanitize::sanitize_type_name;

mod owner;
use owner::resolve_owner_from_ty;

pub mod sysroot;

mod build_index;
mod crate_items;
mod hir_visitor;
mod metadata;
mod span_and_kind;
mod unit_key;

use build_index::*;
use crate_items::*;
use metadata::*;
use ridx_format::{DefKind as RDefKind, IndexWriter, RawDefinition, RawReference, RawType};
use rustc_driver::{Callbacks, Compilation};
use rustc_hir::{
    self as hir,
    def::Res,
    intravisit::{self, Visitor},
    HirId,
};
use rustc_middle::ty::{TyCtxt, TypeVisitableExt, TypeckResults};
use rustc_span::{def_id::DefId, Span};
use span_and_kind::*;
use unit_key::*;

static CAUGHT_PANICS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

struct IndexCallbacks {
    out_dir: PathBuf,
    disambiguator: Option<String>,
    compiler_args: Vec<String>,
}

impl Callbacks for IndexCallbacks {
    fn after_analysis<'tcx>(&mut self, _compiler: &rustc_interface::interface::Compiler, tcx: TyCtxt<'tcx>) -> Compilation {
        build_crate_index(tcx, &self.out_dir, self.disambiguator.as_deref(), &self.compiler_args);
        Compilation::Continue
    }
}

struct HirVisitor<'tcx> {
    tcx: TyCtxt<'tcx>,
    typeck: Option<&'tcx TypeckResults<'tcx>>,
    current_body_owner: Option<rustc_span::def_id::LocalDefId>,
    package_root: Option<PathBuf>,
    writer: IndexWriter,
    krate_to_unit: std::collections::HashMap<rustc_span::def_id::CrateNum, String>,
    seen_refs: std::collections::HashSet<(String, u32, u32, String)>,
}

struct NoopCallbacks;
impl Callbacks for NoopCallbacks {}

#[allow(clippy::result_unit_err)]
pub fn run_driver(mut args: Vec<String>) -> Result<(), ()> {
    let is_direct_version = args.len() == 2 && (args[1] == "--version" || args[1] == "-V");
    if is_direct_version || args.iter().any(|a| a == "--ridx-version") {
        println!("ridx-driver {} (commit: {}, dirty: {})", env!("CARGO_PKG_VERSION"), env!("RIDX_BUILD_COMMIT"), env!("RIDX_BUILD_DIRTY"),);
        return Ok(());
    }

    // Strip any leading compiler wrapper or rustc binary invocations passed by Cargo.
    // When Cargo chains wrappers or when multiple wrappers are configured, args may contain
    // multiple wrapper executable paths or rustc before the actual compiler flags.
    while args.len() > 1 {
        let arg1 = &args[1];
        let p = Path::new(arg1);
        let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if stem == "rustc" || stem == "ridx-driver" || stem.ends_with("-driver") {
            args.remove(1);
        } else {
            break;
        }
    }

    if args.iter().any(|a| a == "--sysroot-cas-key") {
        let target = args
            .windows(2)
            .find(|w| w[0] == "--target")
            .map(|w| w[1].as_str())
            .or_else(|| args.iter().find_map(|a| a.strip_prefix("--target=")))
            .unwrap_or_else(|| rustc_session::config::host_tuple());
        let cas_key = sysroot::compute_sysroot_cas_key_str(None, target);
        println!("{cas_key}");
        return Ok(());
    }

    if args.iter().any(|a| a == "--ridx-version") {
        println!("ridx-driver {} (commit: {}, dirty: {})", env!("CARGO_PKG_VERSION"), env!("RIDX_BUILD_COMMIT"), env!("RIDX_BUILD_DIRTY"));
        return Ok(());
    }

    let is_query = args.iter().any(|a| a == "-vV" || a == "-V" || a == "--version" || a == "-h" || a == "--help" || a == "--print" || a.starts_with("--print="));

    if is_query {
        let res = rustc_driver::catch_fatal_errors(|| rustc_driver::run_compiler(&args, &mut NoopCallbacks));
        return res.map_err(|_| ());
    }

    // Skip build scripts
    let crate_name_arg = args.windows(2).find(|w| w[0] == "--crate-name").map(|w| w[1].as_str());
    if crate_name_arg == Some("build_script_build") || crate_name_arg == Some("build_script_main") {
        let res = rustc_driver::catch_fatal_errors(|| rustc_driver::run_compiler(&args, &mut NoopCallbacks));
        return res.map_err(|_| ());
    }

    let out_dir = std::env::var("RIDX_OUT_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("target/ridx"));
    let disambiguator = extract_target_disambiguator(&args);

    let mut callbacks = IndexCallbacks { out_dir, disambiguator, compiler_args: args.clone() };
    run_with_callbacks(&args, &mut callbacks)
}

#[allow(clippy::result_unit_err)]
pub fn run_with_callbacks<C: Callbacks + Send>(args: &[String], callbacks: &mut C) -> Result<(), ()> {
    let res = rustc_driver::catch_fatal_errors(|| rustc_driver::run_compiler(args, callbacks));
    res.map_err(|_| ())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if run_driver(args).is_err() {
        std::process::exit(1);
    }
}

#[cfg(test)]
pub mod tests;
