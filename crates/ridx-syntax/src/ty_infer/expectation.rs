/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Bidirectional expected-type propagation (`Expectation`) and multi-branch coercion joiner
//! (`BranchCoercer`) for expression, reference, array/slice, tuple, closure, and `if`/`match` inference.

use super::ty::{InferTyId, InferTyKind};
use super::unify::TypeError;
use super::InferenceTable;

/// Downward type expectation threaded into expression inference (`rustc_hir_typeck::Expectation`-style).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Expectation {
    /// No contextual constraint (`let x = expr;`, expression statement, method receiver).
    #[default]
    None,
    /// Expression is expected to produce (or coerce to) `InferTyId`.
    HasType(InferTyId),
    /// Operand of `expr as TargetTy`: guides integer/float literals without forcing coercion.
    CastableTo(InferTyId),
    /// Operand `expr` inside `&expr` / `&mut expr` when outer expectation was `&[T]` or `&dyn Trait`.
    RvalueLikeUnsized(InferTyId),
}

impl Expectation {
    /// Wraps an optional annotation type, mapping `None` and `{unknown}` to [`Self::None`].
    #[inline]
    pub fn from_option(ty: Option<InferTyId>) -> Self {
        match ty {
            Some(t) if t.is_some() && t != InferTyId::UNKNOWN => Self::HasType(t),
            _ => Self::None,
        }
    }

    /// Shallow-resolves the inner type through [`InferenceTable`], turning `{unknown}` into `None`.
    #[inline]
    pub fn resolve(self, table: &mut InferenceTable) -> Self {
        match self {
            Self::None => Self::None,
            Self::HasType(t) => {
                let r = table.shallow_resolve(t);
                if r == InferTyId::UNKNOWN {
                    Self::None
                } else {
                    Self::HasType(r)
                }
            }
            Self::CastableTo(t) => {
                let r = table.shallow_resolve(t);
                if r == InferTyId::UNKNOWN {
                    Self::None
                } else {
                    Self::CastableTo(r)
                }
            }
            Self::RvalueLikeUnsized(t) => {
                let r = table.shallow_resolve(t);
                if r == InferTyId::UNKNOWN {
                    Self::None
                } else {
                    Self::RvalueLikeUnsized(r)
                }
            }
        }
    }

    /// Returns `Some(ty)` ONLY when `self` is [`Self::HasType`] and `shallow_resolve(ty)` is already
    /// a non-variable concrete/structural head (used for closure parameter & struct shorthand hints).
    #[inline]
    pub fn only_has_type(self, table: &mut InferenceTable) -> Option<InferTyId> {
        let Self::HasType(t) = self else {
            return None;
        };
        let r = table.shallow_resolve(t);
        if r == InferTyId::UNKNOWN || matches!(table.arena.kind(r), InferTyKind::Infer(_)) {
            None
        } else {
            Some(r)
        }
    }

    /// Returns the coercion target type when `self` is [`Self::HasType`] (including unbound `TyVar`s).
    #[inline]
    pub fn coercion_target(self, table: &mut InferenceTable) -> Option<InferTyId> {
        match self {
            Self::HasType(t) => {
                let r = table.shallow_resolve(t);
                (r != InferTyId::UNKNOWN).then_some(r)
            }
            Self::None | Self::CastableTo(_) | Self::RvalueLikeUnsized(_) => None,
        }
    }

    /// Propagates expectation across `&expr` / `&mut expr` (`&[u8]` -> `RvalueLikeUnsized([u8])` -> elem `u8`).
    pub fn peel_for_borrow(self, table: &mut InferenceTable, borrow_is_mut: bool) -> Self {
        let Some(exp) = self.only_has_type(table) else {
            return Self::None;
        };
        match table.arena.kind(exp) {
            InferTyKind::Ref { is_mut: exp_mut, inner } | InferTyKind::Ptr { is_mut: exp_mut, inner } if !exp_mut || borrow_is_mut => {
                let inner_res = table.shallow_resolve(inner);
                match table.arena.kind(inner_res) {
                    InferTyKind::Slice(_) | InferTyKind::Dyn { .. } => Self::RvalueLikeUnsized(inner_res),
                    InferTyKind::Unknown => Self::None,
                    _ => Self::HasType(inner_res),
                }
            }
            _ => Self::None,
        }
    }

    /// Extracts element expectation for `[e0, e1, ...]` from `[T; N]` or `RvalueLikeUnsized([T])`.
    pub fn array_elem_expectation(self, table: &mut InferenceTable) -> Self {
        match self.resolve(table) {
            Self::HasType(t) => match table.arena.kind(t) {
                InferTyKind::Array { elem, .. } => Self::from_option(Some(elem)),
                _ => Self::None,
            },
            Self::RvalueLikeUnsized(t) => match table.arena.kind(t) {
                InferTyKind::Slice(elem) => Self::from_option(Some(elem)),
                _ => Self::None,
            },
            Self::None | Self::CastableTo(_) => Self::None,
        }
    }

    /// Splits a tuple expectation `(T0, .., Tn-1)` into per-element `Expectation`s when arity matches.
    pub fn tuple_elem_expectations(self, table: &mut InferenceTable, arity: usize, out: &mut Vec<Self>) {
        out.clear();
        if let Some(t) = self.only_has_type(table) {
            if let InferTyKind::Tuple(r) = table.arena.kind(t) {
                if r.len as usize == arity {
                    for idx in 0..arity {
                        let elem = table.arena.slice(r)[idx];
                        out.push(Self::from_option(Some(elem)));
                    }
                    return;
                }
            }
        }
        out.resize(arity, Self::None);
    }
}

/// Stateful multi-branch coercion joiner (`if` / `else`, `match` arms, `loop { break val; }`).
#[derive(Debug)]
pub struct BranchCoercer {
    expected_target: Option<InferTyId>,
    accumulated: Option<InferTyId>,
    saw_diverging: bool,
    pub first_error: Option<TypeError>,
}

impl BranchCoercer {
    /// Creates a branch joiner guided by the enclosing `expectation`.
    #[inline]
    pub fn new(table: &mut InferenceTable, expectation: Expectation) -> Self {
        Self { expected_target: expectation.coercion_target(table), accumulated: None, saw_diverging: false, first_error: None }
    }

    /// Expectation to pass downward into each branch body.
    #[inline]
    pub fn arm_expectation(&self) -> Expectation {
        match self.expected_target.or(self.accumulated) {
            Some(t) => Expectation::HasType(t),
            None => Expectation::None,
        }
    }

    /// Merges one branch result type via directional coercion (when expected) or pairwise `LUB`.
    pub fn merge_arm(&mut self, table: &mut InferenceTable, arm_ty: InferTyId) {
        let resolved = table.shallow_resolve(arm_ty);
        if resolved == InferTyId::NEVER {
            self.saw_diverging = true;
            return;
        }
        if let Some(exp) = self.expected_target {
            if let Err(e) = table.coerce(resolved, exp) {
                if self.first_error.is_none() {
                    self.first_error = Some(e);
                }
            }
            self.accumulated = Some(table.shallow_resolve(exp));
            return;
        }
        match self.accumulated {
            None => self.accumulated = Some(resolved),
            Some(prev) => match table.lub(prev, resolved) {
                Ok(joined) => self.accumulated = Some(joined),
                Err(e) => {
                    if self.first_error.is_none() {
                        self.first_error = Some(e);
                    }
                    self.accumulated = Some(InferTyId::UNKNOWN);
                }
            },
        }
    }

    /// Finalizes the joined type across all merged branches (`!` if all arms diverged, `()` if 0 arms).
    #[inline]
    pub fn finish(self, table: &mut InferenceTable) -> InferTyId {
        if let Some(acc) = self.accumulated {
            table.shallow_resolve(acc)
        } else if let Some(exp) = self.expected_target {
            if self.saw_diverging {
                InferTyId::NEVER
            } else {
                table.shallow_resolve(exp)
            }
        } else if self.saw_diverging {
            InferTyId::NEVER
        } else {
            InferTyId::UNIT
        }
    }
}
