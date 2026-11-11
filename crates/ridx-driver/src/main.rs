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

extern crate rustc_abi;
extern crate rustc_ast;
extern crate rustc_ast_pretty;
extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_metadata;
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

mod assoc_res;
mod build_index;
pub(crate) mod cfg_extract;
mod crate_items;
pub(crate) mod expr_ty_dump;
mod hir_visitor;
pub(crate) mod macro_extract;
mod metadata;
mod notes;
mod span_and_kind;
pub(crate) mod ty_lower;
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

#[derive(Clone)]
pub struct DriverEnv {
    pub out_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub manifest_dir: Option<PathBuf>,
    pub build_out_dir: Option<PathBuf>,
    pub pkg_version: String,
    pub driver_hash: u64,
}

impl DriverEnv {
    pub fn from_process_env() -> Self {
        Self {
            out_dir: std::env::var("RIDX_OUT_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("target/ridx")),
            cache_dir: ridx_format::resolve_cache_dir(),
            manifest_dir: std::env::var("CARGO_MANIFEST_DIR").ok().map(PathBuf::from),
            build_out_dir: std::env::var("OUT_DIR").ok().map(PathBuf::from),
            pkg_version: std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.1.0".to_string()),
            driver_hash: ridx_format::get_driver_hash(),
        }
    }

    #[cfg(test)]
    pub fn for_test(out_dir: impl Into<PathBuf>) -> Self {
        Self {
            out_dir: out_dir.into(),
            cache_dir: ridx_format::resolve_cache_dir(),
            manifest_dir: None,
            build_out_dir: None,
            pkg_version: "0.1.0".to_string(),
            driver_hash: ridx_format::get_driver_hash(),
        }
    }
}

struct IndexCallbacks {
    env: DriverEnv,
    disambiguator: Option<String>,
    compiler_args: Vec<String>,
    crate_cfg: Vec<String>,
}

impl Callbacks for IndexCallbacks {
    fn config(&mut self, config: &mut rustc_interface::Config) {
        self.crate_cfg = config.crate_cfg.clone();
    }

    fn after_analysis<'tcx>(&mut self, _compiler: &rustc_interface::interface::Compiler, tcx: TyCtxt<'tcx>) -> Compilation {
        build_crate_index(tcx, &self.env, self.disambiguator.as_deref(), &self.compiler_args, &self.crate_cfg);
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
pub fn run_driver(args: Vec<String>) -> Result<(), ()> {
    run_driver_with_env(args, DriverEnv::from_process_env())
}

#[allow(clippy::result_unit_err)]
pub fn run_driver_with_env(mut args: Vec<String>, env: DriverEnv) -> Result<(), ()> {
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

    if args.iter().any(|a| a == "--audit-sysroot") {
        struct AuditSysrootCallbacks;
        impl Callbacks for AuditSysrootCallbacks {
            fn after_analysis<'tcx>(&mut self, _c: &rustc_interface::interface::Compiler, tcx: TyCtxt<'tcx>) -> Compilation {
                let rep = sysroot::audit::audit_sysroot_completeness(tcx);
                sysroot::audit::print_sysroot_completeness(&rep);
                Compilation::Stop
            }
        }
        let tmp = std::env::temp_dir().join(format!("ridx_audit_sysroot_{}", std::process::id()));
        let _ = fs::create_dir_all(&tmp);
        let src = tmp.join("lib.rs");
        let _ = fs::write(&src, "extern crate alloc;\n");
        let rmeta = tmp.join("libaudit_dummy.rmeta");
        let sysroot_out = std::process::Command::new("rustc").args(["--print", "sysroot"]).output().ok();
        let mut cargs = vec![
            "rustc".to_string(),
            src.to_string_lossy().into_owned(),
            "--crate-name".to_string(),
            "audit_dummy".to_string(),
            "--crate-type".to_string(),
            "lib".to_string(),
            "--emit=metadata".to_string(),
            "-o".to_string(),
            rmeta.to_string_lossy().into_owned(),
        ];
        if let Some(out) = sysroot_out {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                cargs.push("--sysroot".to_string());
                cargs.push(s);
            }
        }
        let res = run_with_callbacks(&cargs, &mut AuditSysrootCallbacks);
        let _ = fs::remove_dir_all(&tmp);
        return res;
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

    // Build scripts of registry and git dependencies are compiled without indexing; those of local packages are
    // indexed like any other target, under a shard name carrying the package (every build script is the crate
    // `build_script_build`).
    let crate_name_arg = args.windows(2).find(|w| w[0] == "--crate-name").map(|w| w[1].as_str());
    let local_package = std::env::var_os("CARGO_MANIFEST_DIR").is_some_and(|d| !metadata::is_immutable_package(Path::new(&d)));
    let build_script = matches!(crate_name_arg, Some("build_script_build" | "build_script_main"));
    if build_script && !local_package {
        let res = rustc_driver::catch_fatal_errors(|| rustc_driver::run_compiler(&args, &mut NoopCallbacks));
        return res.map_err(|_| ());
    }

    let mut disambiguator = extract_target_disambiguator(&args);
    if build_script {
        let pkg = std::env::var("CARGO_PKG_NAME").unwrap_or_default().replace('-', "_");
        disambiguator = Some(format!("_{pkg}{}", disambiguator.unwrap_or_default()));
    }

    let mut callbacks = IndexCallbacks { env, disambiguator, compiler_args: args.clone(), crate_cfg: Vec::new() };
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
