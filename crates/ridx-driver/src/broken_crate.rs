/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Indexing a crate that does not compile. rustc's `analysis` query stops with a fatal error once type checking
//! reported an error, so `after_analysis` never runs and the crate would get no index at all. The driver replaces
//! that query: it runs the compiler's own, and when it stops on the crate's errors the index is written from what
//! did resolve (every body is type-checked by then, with error types where it failed) before the stop goes on.

use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use rustc_middle::ty::TyCtxt;
use rustc_span::fatal_error::FatalErrorMarker;

type AnalysisFn = for<'tcx> fn(TyCtxt<'tcx>, ());
type IndexJob = Box<dyn for<'tcx> FnOnce(TyCtxt<'tcx>) + Send>;

static COMPILER_ANALYSIS: OnceLock<AnalysisFn> = OnceLock::new();
static PENDING: Mutex<Option<IndexJob>> = Mutex::new(None);
static ON_ERRORS: AtomicBool = AtomicBool::new(false);

/// Arranges for `job` to run once: after analysis, and, when `on_errors`, also when analysis stops on the crate's
/// errors. Only a crate of the workspace is indexed with errors: a registry or git dependency's index goes to the
/// shared cache keyed by what it was compiled from, and a later build that succeeds with the same key (an
/// environment variable read by `env!` set this time) would find the partial one there and keep it (review of
/// a34d8100).
pub(crate) fn install(config: &mut rustc_interface::Config, job: IndexJob, on_errors: bool) {
    *PENDING.lock().unwrap_or_else(|e| e.into_inner()) = Some(job);
    ON_ERRORS.store(on_errors, Ordering::SeqCst);
    config.override_queries = Some(|_session, providers| {
        let _ = COMPILER_ANALYSIS.set(providers.queries.analysis);
        providers.queries.analysis = analysis_then_index_on_errors;
    });
}

/// Runs the pending index job, if it has not run yet.
pub(crate) fn run_pending(tcx: TyCtxt<'_>) {
    let job = PENDING.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(job) = job {
        job(tcx);
    }
}

fn analysis_then_index_on_errors(tcx: TyCtxt<'_>, (): ()) {
    let Some(analysis) = COMPILER_ANALYSIS.get().copied() else { return };
    let Err(stop) = catch_unwind(AssertUnwindSafe(|| analysis(tcx, ()))) else { return };
    // Only the compiler giving up on the crate's errors: an internal compiler error goes on untouched.
    if ON_ERRORS.load(Ordering::SeqCst) && stop.is::<FatalErrorMarker>() && tcx.dcx().has_errors_excluding_lint_errors().is_some() {
        // A failure while indexing broken code must not replace the compiler's own diagnosis.
        let _ = catch_unwind(AssertUnwindSafe(|| run_pending(tcx)));
    }
    resume_unwind(stop)
}
