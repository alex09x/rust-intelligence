/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Per-namespace (`type` / `value` / `macro`) resolved bindings, builtin primitive codes,
//! visibility predicates, and glob-shadowing / ambiguity merge rules.

use super::db::{GlobalDefId, ItemKind, ModuleId, NsMask, VisKind};
use super::mod_tree::ModuleTree;
use crate::sema_fn::norm_ident;

/// Builtin Rust primitive type codes (`1..=17`, `0` = none), aligned with `IndexedUse::prim_ty_code`.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PrimTy {
    I8 = 1,
    I16 = 2,
    I32 = 3,
    I64 = 4,
    I128 = 5,
    Isize = 6,
    U8 = 7,
    U16 = 8,
    U32 = 9,
    U64 = 10,
    U128 = 11,
    Usize = 12,
    F32 = 13,
    F64 = 14,
    Bool = 15,
    Char = 16,
    Str = 17,
}

impl PrimTy {
    pub const ALL: [Self; 17] = [
        Self::I8,
        Self::I16,
        Self::I32,
        Self::I64,
        Self::I128,
        Self::Isize,
        Self::U8,
        Self::U16,
        Self::U32,
        Self::U64,
        Self::U128,
        Self::Usize,
        Self::F32,
        Self::F64,
        Self::Bool,
        Self::Char,
        Self::Str,
    ];

    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "i8" => Self::I8,
            "i16" => Self::I16,
            "i32" => Self::I32,
            "i64" => Self::I64,
            "i128" => Self::I128,
            "isize" => Self::Isize,
            "u8" => Self::U8,
            "u16" => Self::U16,
            "u32" => Self::U32,
            "u64" => Self::U64,
            "u128" => Self::U128,
            "usize" => Self::Usize,
            "f32" => Self::F32,
            "f64" => Self::F64,
            "bool" => Self::Bool,
            "char" => Self::Char,
            "str" => Self::Str,
            _ => return None,
        })
    }

    pub const fn from_code(code: u8) -> Option<Self> {
        if code >= 1 && code <= 17 {
            Some(Self::ALL[(code - 1) as usize])
        } else {
            None
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::I8 => "i8",
            Self::I16 => "i16",
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::I128 => "i128",
            Self::Isize => "isize",
            Self::U8 => "u8",
            Self::U16 => "u16",
            Self::U32 => "u32",
            Self::U64 => "u64",
            Self::U128 => "u128",
            Self::Usize => "usize",
            Self::F32 => "f32",
            Self::F64 => "f64",
            Self::Bool => "bool",
            Self::Char => "char",
            Self::Str => "str",
        }
    }
}

/// Arena index into [`super::def_map::CrateDefMap::items`] for locally parsed CST definitions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalItemId(pub u32);

impl LocalItemId {
    pub const NONE: Self = Self(u32::MAX);

    #[inline]
    pub const fn is_some(self) -> bool {
        self.0 != u32::MAX
    }
}

impl Default for LocalItemId {
    #[inline]
    fn default() -> Self {
        Self::NONE
    }
}

/// Precedence tier of a binding inside a module scope (`Explicit > Glob > ExternPrelude > Prelude > Builtin`).
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BindingOrigin {
    #[default]
    Builtin = 0,
    Prelude = 1,
    ExternPrelude = 2,
    Glob = 3,
    Explicit = 4,
}

/// Compact `Copy` resolution of one namespace slot (`TYPE`, `VALUE`, or `MACRO`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResolvedBinding {
    pub def: GlobalDefId,
    pub local: LocalItemId,
    /// Module in the current crate when target is a local `Module` (`ModuleId::NONE` otherwise).
    pub local_mod: ModuleId,
    /// Ancestor module restricting visibility (`def_mod` for `Private`, `parent(def_mod)` for `Super`,
    /// target ancestor for `Restricted`, `ModuleId::ROOT` for `Crate`, `ModuleId::NONE` for `Public`).
    pub vis_mod: ModuleId,
    pub kind: ItemKind,
    pub vis: VisKind,
    pub origin: BindingOrigin,
    pub prim_ty: u8,
    pub ambiguous: bool,
}

impl ResolvedBinding {
    #[inline]
    pub const fn primitive(p: PrimTy, vis: VisKind, vis_mod: ModuleId, origin: BindingOrigin) -> Self {
        Self { def: GlobalDefId::NONE, local: LocalItemId::NONE, local_mod: ModuleId::NONE, vis_mod, kind: ItemKind::TypeAlias, vis, origin, prim_ty: p as u8, ambiguous: false }
    }

    #[inline]
    pub const fn same_target(self, other: Self) -> bool {
        if self.prim_ty != 0 || other.prim_ty != 0 {
            return self.prim_ty == other.prim_ty;
        }
        if self.local_mod.is_some() || other.local_mod.is_some() {
            return self.local_mod.0 == other.local_mod.0;
        }
        if self.def.is_some() && other.def.is_some() {
            return self.def.krate.0 == other.def.krate.0 && self.def.index == other.def.index;
        }
        self.local.is_some() && self.local.0 == other.local.0
    }

    #[inline]
    pub const fn rebind(mut self, vis: VisKind, vis_mod: ModuleId, origin: BindingOrigin) -> Self {
        self.vis = vis;
        self.vis_mod = vis_mod;
        self.origin = origin;
        self
    }
}

/// Resolution across Rust's three namespaces (`types`, `values`, `macros`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PerNs {
    pub types: Option<ResolvedBinding>,
    pub values: Option<ResolvedBinding>,
    pub macros: Option<ResolvedBinding>,
}

impl PerNs {
    pub const EMPTY: Self = Self { types: None, values: None, macros: None };

    #[inline]
    pub const fn is_empty(self) -> bool {
        self.types.is_none() && self.values.is_none() && self.macros.is_none()
    }

    #[inline]
    pub fn from_binding(b: ResolvedBinding, mask: NsMask) -> Self {
        Self { types: mask.contains(NsMask::TYPE).then_some(b), values: mask.contains(NsMask::VALUE).then_some(b), macros: mask.contains(NsMask::MACRO).then_some(b) }
    }

    #[inline]
    pub const fn rebind(self, vis: VisKind, vis_mod: ModuleId, origin: BindingOrigin) -> Self {
        Self {
            types: match self.types {
                Some(b) => Some(b.rebind(vis, vis_mod, origin)),
                None => None,
            },
            values: match self.values {
                Some(b) => Some(b.rebind(vis, vis_mod, origin)),
                None => None,
            },
            macros: match self.macros {
                Some(b) => Some(b.rebind(vis, vis_mod, origin)),
                None => None,
            },
        }
    }

    /// Merges an explicit binding (`Explicit` overrides `Glob`/`Prelude`; returns `true` if `self` changed).
    pub fn merge_explicit(&mut self, incoming: Self) -> bool {
        let mut changed = false;
        changed |= merge_slot_explicit(&mut self.types, incoming.types);
        changed |= merge_slot_explicit(&mut self.values, incoming.values);
        changed |= merge_slot_explicit(&mut self.macros, incoming.macros);
        changed
    }

    /// Merges a glob-imported binding (`Explicit` wins; distinct `Glob` targets mark `ambiguous = true`).
    pub fn merge_glob(&mut self, incoming: Self) -> bool {
        let mut changed = false;
        changed |= merge_slot_glob(&mut self.types, incoming.types);
        changed |= merge_slot_glob(&mut self.values, incoming.values);
        changed |= merge_slot_glob(&mut self.macros, incoming.macros);
        changed
    }

    /// Filters slots to those visible from `from_mod` inside `tree` (dropping ambiguous glob slots).
    pub fn filter_visible(self, tree: &ModuleTree, from_mod: ModuleId) -> Self {
        let keep = |opt: Option<ResolvedBinding>| opt.filter(|b| !b.ambiguous && is_binding_visible(tree, *b, from_mod));
        Self { types: keep(self.types), values: keep(self.values), macros: keep(self.macros) }
    }

    /// Fills any `None` namespace slots in `self` from `fallback`.
    #[inline]
    pub fn or_else_ns(self, fallback: Self) -> Self {
        Self { types: self.types.or(fallback.types), values: self.values.or(fallback.values), macros: self.macros.or(fallback.macros) }
    }
}

fn merge_slot_explicit(slot: &mut Option<ResolvedBinding>, inc: Option<ResolvedBinding>) -> bool {
    let Some(mut b) = inc else { return false };
    b.origin = BindingOrigin::Explicit;
    b.ambiguous = false;
    match *slot {
        None => {
            *slot = Some(b);
            true
        }
        Some(cur) if cur.origin < BindingOrigin::Explicit => {
            *slot = Some(b);
            true
        }
        Some(_) => false,
    }
}

fn merge_slot_glob(slot: &mut Option<ResolvedBinding>, inc: Option<ResolvedBinding>) -> bool {
    let Some(mut b) = inc else { return false };
    if b.ambiguous {
        return false;
    }
    b.origin = BindingOrigin::Glob;
    match *slot {
        None => {
            *slot = Some(b);
            true
        }
        Some(cur) if cur.origin < BindingOrigin::Glob => {
            *slot = Some(b);
            true
        }
        Some(cur) if cur.origin > BindingOrigin::Glob => false,
        Some(mut cur) => {
            if cur.same_target(b) {
                false
            } else if !cur.ambiguous {
                cur.ambiguous = true;
                *slot = Some(cur);
                true
            } else {
                false
            }
        }
    }
}

/// Returns `true` if `descendant` equals `ancestor` or lies inside `ancestor`'s subtree in `tree`.
#[inline]
pub fn is_descendant_or_eq(tree: &ModuleTree, mut descendant: ModuleId, ancestor: ModuleId) -> bool {
    if !ancestor.is_some() {
        return true;
    }
    while descendant.is_some() {
        if descendant == ancestor {
            return true;
        }
        descendant = tree.module(descendant).parent;
    }
    false
}

#[inline]
pub fn is_binding_visible(tree: &ModuleTree, b: ResolvedBinding, from_mod: ModuleId) -> bool {
    match b.vis {
        VisKind::Public | VisKind::Crate => true,
        VisKind::Private | VisKind::Super | VisKind::Restricted => is_descendant_or_eq(tree, from_mod, b.vis_mod),
    }
}

/// Computes `(effective_vis, vis_ancestor_mod)` for an item/import declared inside `cur_mod`.
pub fn compute_vis_mod(tree: &ModuleTree, cur_mod: ModuleId, vis: VisKind, in_path: Option<&str>) -> (VisKind, ModuleId) {
    match vis {
        VisKind::Public => (VisKind::Public, ModuleId::NONE),
        VisKind::Crate => (VisKind::Crate, ModuleId::ROOT),
        VisKind::Private => (VisKind::Private, cur_mod),
        VisKind::Super => {
            let p = tree.module(cur_mod).parent;
            (VisKind::Super, if p.is_some() { p } else { ModuleId::ROOT })
        }
        VisKind::Restricted => {
            let Some(raw) = in_path else { return (VisKind::Private, cur_mod) };
            let mut m = cur_mod;
            let mut first = true;
            for seg in raw.split("::").map(str::trim).filter(|s| !s.is_empty()) {
                match seg {
                    "crate" if first => m = ModuleId::ROOT,
                    "self" if first => {}
                    "super" => {
                        let p = tree.module(m).parent;
                        m = if p.is_some() { p } else { ModuleId::ROOT };
                    }
                    other => match tree.find_child(m, norm_ident(other)) {
                        Some(c) => m = c,
                        None => return (VisKind::Private, cur_mod),
                    },
                }
                first = false;
            }
            (VisKind::Restricted, m)
        }
    }
}
