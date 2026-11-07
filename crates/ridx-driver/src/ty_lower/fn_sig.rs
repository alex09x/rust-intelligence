/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unified function and method signature (`FnSigEntry`) extraction from `TyCtxt`
//! shared across workspace crates, external dependencies, and sysroot.

use ridx_format::{IndexWriter, RawFnSig, FN_FLAG_ASYNC, FN_FLAG_CONST, FN_FLAG_HAS_SELF, FN_FLAG_UNSAFE, FN_FLAG_VARIADIC};
use rustc_span::def_id::DefId;

use super::{lower_abi, TcxTypeLowerer};

impl<'tcx, 'a> TcxTypeLowerer<'tcx, 'a> {
    /// Emits structured `FnSigEntry` rows for every `(DefId, def_index)` callable in this shard.
    pub(crate) fn record_fn_sigs(&mut self, callables: &[(DefId, u32)], writer: &mut IndexWriter) {
        for &(did, def_index) in callables {
            self.record_fn_sig(did, def_index, writer);
        }
    }

    pub(crate) fn record_fn_sig(&mut self, did: DefId, def_index: u32, writer: &mut IndexWriter) {
        let sig = self.tcx.fn_sig(did).instantiate_identity().skip_binder();
        let mut inputs = std::mem::take(&mut self.scratch_inputs);
        inputs.clear();
        inputs.reserve(sig.inputs().len());
        for &inp_ty in sig.inputs() {
            inputs.push(self.lower_ty(inp_ty, writer));
        }
        let output = self.lower_ty(sig.output(), writer);

        let mut flags = 0u8;
        if sig.safety().is_unsafe() {
            flags |= FN_FLAG_UNSAFE;
        }
        if sig.c_variadic() {
            flags |= FN_FLAG_VARIADIC;
        }
        if self.tcx.asyncness(did).is_async() {
            flags |= FN_FLAG_ASYNC;
        }
        if self.tcx.is_const_fn(did) {
            flags |= FN_FLAG_CONST;
        }
        if self.tcx.opt_associated_item(did).is_some_and(|a| a.is_method()) {
            flags |= FN_FLAG_HAS_SELF;
        }

        let abi = lower_abi(sig.abi());
        writer.add_fn_sig(RawFnSig { def_index, inputs: &inputs, output, abi, flags });
        self.scratch_inputs = inputs;
    }
}
