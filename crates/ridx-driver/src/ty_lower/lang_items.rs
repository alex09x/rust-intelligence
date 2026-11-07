/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compiler `#[lang = "..."]` item (and associated trait item) extraction from `tcx.lang_items()`
//! into `.ridx` v5 `LangItemEntry` tables (1.2-G6).

use ridx_format::{IndexWriter, LangItemKind, RawLangItem};
use rustc_hir::def::DefKind;
use rustc_middle::ty::{ClauseKind, TyCtxt};
use rustc_span::def_id::{CrateNum, DefId};

use super::TcxTypeLowerer;

#[inline]
fn push_if_in_crate(out: &mut Vec<(LangItemKind, DefId)>, cnum: CrateNum, kind: LangItemKind, did: Option<DefId>) {
    if let Some(d) = did {
        if d.krate == cnum {
            out.push((kind, d));
        }
    }
}

#[inline]
fn assoc_item_by_name(tcx: TyCtxt<'_>, trait_did: Option<DefId>, name: &str, want: DefKind) -> Option<DefId> {
    let trait_did = trait_did?;
    tcx.associated_item_def_ids(trait_did).iter().find(|&&assoc_did| tcx.def_kind(assoc_did) == want && tcx.opt_item_name(assoc_did).is_some_and(|s| s.as_str() == name)).copied()
}

/// Resolves `core::ops::try_trait::Residual` and `Residual::TryType` from `Try::Residual`'s
/// explicit item bounds (`type Residual: Residual<Self::Output>`).
fn resolve_residual_trait_and_try_type(tcx: TyCtxt<'_>, try_residual_assoc: Option<DefId>) -> (Option<DefId>, Option<DefId>) {
    let Some(res_assoc) = try_residual_assoc else {
        return (None, None);
    };
    for (clause, _) in tcx.explicit_item_bounds(res_assoc).skip_binder() {
        if let ClauseKind::Trait(tp) = clause.kind().skip_binder() {
            let trait_did = tp.trait_ref.def_id;
            if let Some(try_ty_did) = assoc_item_by_name(tcx, Some(trait_did), "TryType", DefKind::AssocTy) {
                return (Some(trait_did), Some(try_ty_did));
            }
        }
    }
    (None, None)
}

/// Collects all canonical `(LangItemKind, DefId)` pairs declared in `cnum` directly from
/// `tcx.lang_items()` and associated trait items, independent of API stability.
pub(crate) fn collect_crate_lang_items<'tcx>(tcx: TyCtxt<'tcx>, cnum: CrateNum) -> Vec<(LangItemKind, DefId)> {
    let li = tcx.lang_items();
    let mut out = Vec::with_capacity(112);

    // 1. Core markers & fundamental ADTs
    push_if_in_crate(&mut out, cnum, LangItemKind::Sized, li.sized_trait());
    push_if_in_crate(&mut out, cnum, LangItemKind::Copy, li.copy_trait());
    push_if_in_crate(&mut out, cnum, LangItemKind::Clone, li.clone_trait());
    push_if_in_crate(&mut out, cnum, LangItemKind::Sync, li.sync_trait());
    push_if_in_crate(&mut out, cnum, LangItemKind::Freeze, li.freeze_trait());
    push_if_in_crate(&mut out, cnum, LangItemKind::Unpin, li.unpin_trait());
    push_if_in_crate(&mut out, cnum, LangItemKind::Drop, li.drop_trait());
    push_if_in_crate(&mut out, cnum, LangItemKind::Tuple, li.tuple_trait());
    push_if_in_crate(&mut out, cnum, LangItemKind::FnPtrTrait, li.fn_ptr_trait());
    push_if_in_crate(&mut out, cnum, LangItemKind::Option, li.option_type());
    push_if_in_crate(&mut out, cnum, LangItemKind::OptionSome, li.option_some_variant());
    push_if_in_crate(&mut out, cnum, LangItemKind::OptionNone, li.option_none_variant());
    let result_ty = li.result_ok_variant().map(|v| tcx.parent(v));
    push_if_in_crate(&mut out, cnum, LangItemKind::Result, result_ty);
    push_if_in_crate(&mut out, cnum, LangItemKind::ResultOk, li.result_ok_variant());
    push_if_in_crate(&mut out, cnum, LangItemKind::ResultErr, li.result_err_variant());
    let cf_ty = li.cf_continue_variant().map(|v| tcx.parent(v));
    push_if_in_crate(&mut out, cnum, LangItemKind::ControlFlow, cf_ty);
    push_if_in_crate(&mut out, cnum, LangItemKind::ControlFlowContinue, li.cf_continue_variant());
    push_if_in_crate(&mut out, cnum, LangItemKind::ControlFlowBreak, li.cf_break_variant());
    push_if_in_crate(&mut out, cnum, LangItemKind::Poll, li.poll());
    push_if_in_crate(&mut out, cnum, LangItemKind::PollReady, li.poll_ready_variant());
    push_if_in_crate(&mut out, cnum, LangItemKind::PollPending, li.poll_pending_variant());
    push_if_in_crate(&mut out, cnum, LangItemKind::Pin, li.pin_type());
    push_if_in_crate(&mut out, cnum, LangItemKind::OwnedBox, li.owned_box());
    push_if_in_crate(&mut out, cnum, LangItemKind::String, li.string());
    push_if_in_crate(&mut out, cnum, LangItemKind::ManuallyDrop, li.manually_drop());
    push_if_in_crate(&mut out, cnum, LangItemKind::MaybeUninit, li.maybe_uninit());
    push_if_in_crate(&mut out, cnum, LangItemKind::UnsafeCell, li.unsafe_cell_type());
    push_if_in_crate(&mut out, cnum, LangItemKind::PhantomData, li.phantom_data());

    // 2. Range structs
    push_if_in_crate(&mut out, cnum, LangItemKind::RangeFull, li.range_full_struct());
    push_if_in_crate(&mut out, cnum, LangItemKind::Range, li.range_struct());
    push_if_in_crate(&mut out, cnum, LangItemKind::RangeFrom, li.range_from_struct());
    push_if_in_crate(&mut out, cnum, LangItemKind::RangeTo, li.range_to_struct());
    push_if_in_crate(&mut out, cnum, LangItemKind::RangeInclusive, li.range_inclusive_struct());
    push_if_in_crate(&mut out, cnum, LangItemKind::RangeToInclusive, li.range_to_inclusive_struct());

    // 3. `?` operator (`Try`, `FromResidual`, `Residual` — unstable in `core`)
    let try_did = li.try_trait();
    let try_output = assoc_item_by_name(tcx, try_did, "Output", DefKind::AssocTy);
    let try_residual = assoc_item_by_name(tcx, try_did, "Residual", DefKind::AssocTy);
    let from_residual_fn = li.from_residual_fn();
    let from_residual_trait = from_residual_fn.map(|f| tcx.parent(f));
    let (residual_trait, residual_try_ty) = resolve_residual_trait_and_try_type(tcx, try_residual);
    push_if_in_crate(&mut out, cnum, LangItemKind::Try, try_did);
    push_if_in_crate(&mut out, cnum, LangItemKind::TryOutput, try_output);
    push_if_in_crate(&mut out, cnum, LangItemKind::TryResidual, try_residual);
    push_if_in_crate(&mut out, cnum, LangItemKind::TryBranch, li.branch_fn());
    push_if_in_crate(&mut out, cnum, LangItemKind::TryFromOutput, li.from_output_fn());
    push_if_in_crate(&mut out, cnum, LangItemKind::FromResidual, from_residual_trait);
    push_if_in_crate(&mut out, cnum, LangItemKind::FromResidualFn, from_residual_fn);
    push_if_in_crate(&mut out, cnum, LangItemKind::Residual, residual_trait);
    push_if_in_crate(&mut out, cnum, LangItemKind::ResidualTryType, residual_try_ty);

    // 4. Sync & async callables (`Fn*`, `AsyncFn*`)
    let fn_did = li.fn_trait();
    let fn_mut_did = li.fn_mut_trait();
    let fn_once_did = li.fn_once_trait();
    let afn_did = li.async_fn_trait();
    let afn_mut_did = li.async_fn_mut_trait();
    let afn_once_did = li.async_fn_once_trait();
    push_if_in_crate(&mut out, cnum, LangItemKind::Fn, fn_did);
    push_if_in_crate(&mut out, cnum, LangItemKind::FnCall, assoc_item_by_name(tcx, fn_did, "call", DefKind::AssocFn));
    push_if_in_crate(&mut out, cnum, LangItemKind::FnMut, fn_mut_did);
    push_if_in_crate(&mut out, cnum, LangItemKind::FnMutCallMut, assoc_item_by_name(tcx, fn_mut_did, "call_mut", DefKind::AssocFn));
    push_if_in_crate(&mut out, cnum, LangItemKind::FnOnce, fn_once_did);
    push_if_in_crate(&mut out, cnum, LangItemKind::FnOnceCallOnce, assoc_item_by_name(tcx, fn_once_did, "call_once", DefKind::AssocFn));
    push_if_in_crate(&mut out, cnum, LangItemKind::FnOnceOutput, li.fn_once_output());
    push_if_in_crate(&mut out, cnum, LangItemKind::AsyncFn, afn_did);
    push_if_in_crate(&mut out, cnum, LangItemKind::AsyncFnCall, assoc_item_by_name(tcx, afn_did, "async_call", DefKind::AssocFn));
    push_if_in_crate(&mut out, cnum, LangItemKind::AsyncFnMut, afn_mut_did);
    push_if_in_crate(&mut out, cnum, LangItemKind::AsyncFnMutCallMut, assoc_item_by_name(tcx, afn_mut_did, "async_call_mut", DefKind::AssocFn));
    push_if_in_crate(&mut out, cnum, LangItemKind::AsyncFnOnce, afn_once_did);
    push_if_in_crate(&mut out, cnum, LangItemKind::AsyncFnOnceCallOnce, assoc_item_by_name(tcx, afn_once_did, "async_call_once", DefKind::AssocFn));
    push_if_in_crate(&mut out, cnum, LangItemKind::AsyncFnOnceOutput, li.async_fn_once_output());
    push_if_in_crate(&mut out, cnum, LangItemKind::CallOnceFuture, li.call_once_future());
    push_if_in_crate(&mut out, cnum, LangItemKind::CallRefFuture, li.call_ref_future());

    // 5. Autoderef, receiver & indexing
    let deref_did = li.deref_trait();
    let deref_mut_did = li.deref_mut_trait();
    let index_did = li.index_trait();
    let index_mut_did = li.index_mut_trait();
    push_if_in_crate(&mut out, cnum, LangItemKind::Deref, deref_did);
    push_if_in_crate(&mut out, cnum, LangItemKind::DerefTarget, li.deref_target());
    push_if_in_crate(&mut out, cnum, LangItemKind::DerefFn, assoc_item_by_name(tcx, deref_did, "deref", DefKind::AssocFn));
    push_if_in_crate(&mut out, cnum, LangItemKind::DerefMut, deref_mut_did);
    push_if_in_crate(&mut out, cnum, LangItemKind::DerefMutFn, assoc_item_by_name(tcx, deref_mut_did, "deref_mut", DefKind::AssocFn));
    push_if_in_crate(&mut out, cnum, LangItemKind::Receiver, li.receiver_trait());
    push_if_in_crate(&mut out, cnum, LangItemKind::ReceiverTarget, li.receiver_target());
    push_if_in_crate(&mut out, cnum, LangItemKind::Index, index_did);
    push_if_in_crate(&mut out, cnum, LangItemKind::IndexOutput, assoc_item_by_name(tcx, index_did, "Output", DefKind::AssocTy));
    push_if_in_crate(&mut out, cnum, LangItemKind::IndexFn, assoc_item_by_name(tcx, index_did, "index", DefKind::AssocFn));
    push_if_in_crate(&mut out, cnum, LangItemKind::IndexMut, index_mut_did);
    push_if_in_crate(&mut out, cnum, LangItemKind::IndexMutFn, assoc_item_by_name(tcx, index_mut_did, "index_mut", DefKind::AssocFn));

    // 6. Iteration & `async`/`.await`
    let iter_did = li.iterator_trait();
    let into_iter_fn = li.into_iter_fn();
    let into_iter_did = into_iter_fn.map(|f| tcx.parent(f));
    let into_fut_fn = li.into_future_fn();
    let into_fut_did = into_fut_fn.map(|f| tcx.parent(f));
    push_if_in_crate(&mut out, cnum, LangItemKind::Iterator, iter_did);
    push_if_in_crate(&mut out, cnum, LangItemKind::IteratorItem, assoc_item_by_name(tcx, iter_did, "Item", DefKind::AssocTy));
    push_if_in_crate(&mut out, cnum, LangItemKind::IteratorNext, li.next_fn());
    push_if_in_crate(&mut out, cnum, LangItemKind::IntoIterator, into_iter_did);
    push_if_in_crate(&mut out, cnum, LangItemKind::IntoIteratorItem, assoc_item_by_name(tcx, into_iter_did, "Item", DefKind::AssocTy));
    push_if_in_crate(&mut out, cnum, LangItemKind::IntoIteratorIntoIter, assoc_item_by_name(tcx, into_iter_did, "IntoIter", DefKind::AssocTy));
    push_if_in_crate(&mut out, cnum, LangItemKind::IntoIteratorIntoIterFn, into_iter_fn);
    push_if_in_crate(&mut out, cnum, LangItemKind::Future, li.future_trait());
    push_if_in_crate(&mut out, cnum, LangItemKind::FutureOutput, li.future_output());
    push_if_in_crate(&mut out, cnum, LangItemKind::FuturePoll, li.future_poll_fn());
    push_if_in_crate(&mut out, cnum, LangItemKind::IntoFuture, into_fut_did);
    push_if_in_crate(&mut out, cnum, LangItemKind::IntoFutureOutput, assoc_item_by_name(tcx, into_fut_did, "Output", DefKind::AssocTy));
    push_if_in_crate(&mut out, cnum, LangItemKind::IntoFutureIntoFuture, assoc_item_by_name(tcx, into_fut_did, "IntoFuture", DefKind::AssocTy));
    push_if_in_crate(&mut out, cnum, LangItemKind::IntoFutureIntoFutureFn, into_fut_fn);
    push_if_in_crate(&mut out, cnum, LangItemKind::AsyncIterator, li.async_iterator_trait());
    push_if_in_crate(&mut out, cnum, LangItemKind::AsyncIteratorPollNext, li.async_iterator_poll_next());

    // 7. Operators & conversions
    for (kind, did) in [
        (LangItemKind::Add, li.add_trait()),
        (LangItemKind::Sub, li.sub_trait()),
        (LangItemKind::Mul, li.mul_trait()),
        (LangItemKind::Div, li.div_trait()),
        (LangItemKind::Rem, li.rem_trait()),
        (LangItemKind::Neg, li.neg_trait()),
        (LangItemKind::Not, li.not_trait()),
        (LangItemKind::BitXor, li.bitxor_trait()),
        (LangItemKind::BitAnd, li.bitand_trait()),
        (LangItemKind::BitOr, li.bitor_trait()),
        (LangItemKind::Shl, li.shl_trait()),
        (LangItemKind::Shr, li.shr_trait()),
        (LangItemKind::AddAssign, li.add_assign_trait()),
        (LangItemKind::SubAssign, li.sub_assign_trait()),
        (LangItemKind::MulAssign, li.mul_assign_trait()),
        (LangItemKind::DivAssign, li.div_assign_trait()),
        (LangItemKind::RemAssign, li.rem_assign_trait()),
        (LangItemKind::BitXorAssign, li.bitxor_assign_trait()),
        (LangItemKind::BitAndAssign, li.bitand_assign_trait()),
        (LangItemKind::BitOrAssign, li.bitor_assign_trait()),
        (LangItemKind::ShlAssign, li.shl_assign_trait()),
        (LangItemKind::ShrAssign, li.shr_assign_trait()),
        (LangItemKind::PartialEq, li.eq_trait()),
        (LangItemKind::PartialOrd, li.partial_ord_trait()),
        (LangItemKind::From, li.from_trait()),
    ] {
        push_if_in_crate(&mut out, cnum, kind, did);
    }

    out
}

impl<'tcx, 'a> TcxTypeLowerer<'tcx, 'a> {
    /// Emits `LangItemEntry` rows for every `(LangItemKind, DefId)` collected in `local_cnum`.
    pub(crate) fn record_lang_items(&mut self, lang_items: &[(LangItemKind, DefId)], writer: &mut IndexWriter) {
        for &(kind, did) in lang_items {
            let ty_def = self.lower_def(did, writer);
            let def_index = self.local_def_indices.get(&did).copied();
            writer.add_lang_item(RawLangItem { kind, ty_def, def_index });
        }
    }
}
