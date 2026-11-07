/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Bottom-up hash-consing interner for `TyArena`.

use std::collections::HashMap;

use crate::format::{
    fnv1a_64, ConstArg, FloatTyWidth, FnAbiKind, GenericArgWord, IntTyWidth, TyBoundEntry, TyDefEntry, TyDefId, TyEntry, TyId, TyMutability, TyTag, UintTyWidth, BOUND_FLAG_MAYBE,
    FN_FLAG_UNSAFE, FN_FLAG_VARIADIC,
};

/// Input definition reference for `TyInterner::intern_def`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawTyDef<'a> {
    Local { path: &'a str, local_def_idx: u32 },
    External { path: &'a str, unit_key: Option<&'a str> },
}

/// Input generic argument for `Adt`, `FnDef`, `Assoc`, and `RawTyBound`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawGenericArg<'a> {
    Type(TyId),
    ConstValue(u32),
    ConstParam(u32),
    ConstExpr(&'a str),
    Lifetime(u32),
}

/// Input trait bound for `Dyn` and `ImplTrait`.
#[derive(Debug, Clone)]
pub struct RawTyBound<'a> {
    pub trait_def: TyDefId,
    pub args: &'a [RawGenericArg<'a>],
    pub assocs: &'a [(&'a str, TyId)],
    pub is_maybe: bool,
}

/// Input type node for `TyInterner::intern_ty` (`Infer`-free).
#[derive(Debug, Clone)]
pub enum RawTy<'a> {
    Unit,
    Bool,
    Char,
    Int(IntTyWidth),
    Uint(UintTyWidth),
    Float(FloatTyWidth),
    Str,
    Never,
    Adt { def: TyDefId, args: &'a [RawGenericArg<'a>] },
    Ref { mutability: TyMutability, inner: TyId, lifetime: u32 },
    Ptr { mutability: TyMutability, inner: TyId },
    Slice(TyId),
    ArrayVal { elem: TyId, len: u32 },
    ArrayConstParam { elem: TyId, param_idx: u32 },
    ArrayConstExpr { elem: TyId, expr: &'a str },
    Tuple(&'a [TyId]),
    FnPtr { inputs: &'a [TyId], output: TyId, abi: FnAbiKind, is_unsafe: bool, is_variadic: bool },
    FnDef { def: TyDefId, args: &'a [RawGenericArg<'a>] },
    Param { index: u32, name: &'a str },
    Assoc { trait_def: TyDefId, name: &'a str, self_ty: TyId, trait_args: &'a [RawGenericArg<'a>] },
    Dyn { bounds: &'a [RawTyBound<'a>], lifetime: u32 },
    ImplTrait { bounds: &'a [RawTyBound<'a>] },
    Unknown,
}

/// Deduplicating builder for `TyArena` tables (`ty_defs`, `ty_entries`, `ty_bounds`, `ty_args`).
#[derive(Debug, Default)]
pub struct TyInterner {
    defs: Vec<TyDefEntry>,
    entries: Vec<TyEntry>,
    bounds: Vec<TyBoundEntry>,
    args: Vec<u32>,

    def_map: HashMap<TyDefEntry, TyDefId>,
    entry_map: HashMap<TyEntry, TyId>,
    arg1_map: HashMap<u32, u32>,
    arg_slice_map: HashMap<Box<[u32]>, u32>,
    single_bound_map: HashMap<TyBoundEntry, u32>,
    bound_list_map: HashMap<Box<[TyBoundEntry]>, u32>,
    scratch_words: Vec<u32>,
    scratch_bounds: Vec<TyBoundEntry>,
}

impl TyInterner {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn defs(&self) -> &[TyDefEntry] {
        &self.defs
    }

    #[inline]
    pub fn entries(&self) -> &[TyEntry] {
        &self.entries
    }

    #[inline]
    pub fn bounds(&self) -> &[TyBoundEntry] {
        &self.bounds
    }

    #[inline]
    pub fn args(&self) -> &[u32] {
        &self.args
    }

    pub fn intern_def<F>(&mut self, def: RawTyDef<'_>, mut intern_str: F) -> TyDefId
    where
        F: FnMut(&str) -> u32,
    {
        let entry = match def {
            RawTyDef::Local { path, local_def_idx } => {
                let path_hash = fnv1a_64(path.as_bytes());
                let path_off = intern_str(path);
                TyDefEntry::local(path_hash, path_off, local_def_idx)
            }
            RawTyDef::External { path, unit_key } => {
                let path_hash = fnv1a_64(path.as_bytes());
                let path_off = intern_str(path);
                let unit_off = match unit_key {
                    Some(u) if !u.is_empty() => intern_str(u),
                    _ => 0,
                };
                TyDefEntry::external(path_hash, path_off, unit_off)
            }
        };
        if let Some(&id) = self.def_map.get(&entry) {
            return id;
        }
        let id = TyDefId(self.defs.len() as u32);
        self.defs.push(entry);
        self.def_map.insert(entry, id);
        id
    }

    pub fn intern_ty<F>(&mut self, ty: RawTy<'_>, mut intern_str: F) -> TyId
    where
        F: FnMut(&str) -> u32,
    {
        let entry = self.encode_raw_ty(ty, &mut intern_str);
        if let Some(&id) = self.entry_map.get(&entry) {
            return id;
        }
        let id = TyId(self.entries.len() as u32);
        self.entries.push(entry);
        self.entry_map.insert(entry, id);
        id
    }

    /// Interns and deduplicates a list of `TyId`s in `ty_args` (used by `FnSigEntry` parameter lists).
    pub fn intern_ty_list(&mut self, tys: &[TyId]) -> (u32, u16) {
        if tys.is_empty() {
            return (0, 0);
        }
        let mut buf = std::mem::take(&mut self.scratch_words);
        buf.clear();
        buf.extend(tys.iter().map(|t| t.0));
        let (start, len) = self.intern_words(&buf);
        self.scratch_words = buf;
        (start, u16::try_from(len).unwrap_or(u16::MAX))
    }

    /// Interns and deduplicates a list of raw `u32` indices in `ty_args` (used by `ImplEntry` assoc defs).
    pub fn intern_u32_list(&mut self, items: &[u32]) -> (u32, u16) {
        if items.is_empty() {
            return (0, 0);
        }
        let (start, len) = self.intern_words(items);
        (start, u16::try_from(len).unwrap_or(u16::MAX))
    }

    /// Interns and deduplicates a list of `RawGenericArg`s in `ty_args` (used by `ImplEntry` trait args).
    pub fn intern_generic_args_list<F>(&mut self, args: &[RawGenericArg<'_>], mut intern_str: F) -> (u32, u16)
    where
        F: FnMut(&str) -> u32,
    {
        let (start, len) = self.intern_generic_args(args, &mut intern_str);
        (start, u16::try_from(len).unwrap_or(u16::MAX))
    }

    fn intern_words(&mut self, words: &[u32]) -> (u32, u32) {
        match words {
            [] => (0, 0),
            &[w0] => {
                if let Some(&start) = self.arg1_map.get(&w0) {
                    return (start, 1);
                }
                let start = self.args.len() as u32;
                self.args.push(w0);
                self.arg1_map.insert(w0, start);
                (start, 1)
            }
            many => {
                if let Some(&start) = self.arg_slice_map.get(many) {
                    return (start, many.len() as u32);
                }
                let start = self.args.len() as u32;
                self.args.extend_from_slice(many);
                self.arg_slice_map.insert(many.into(), start);
                (start, many.len() as u32)
            }
        }
    }

    fn encode_arg_word<F>(arg: RawGenericArg<'_>, intern_str: &mut F) -> u32
    where
        F: FnMut(&str) -> u32,
    {
        match arg {
            RawGenericArg::Type(t) => GenericArgWord::from_type(t).0,
            RawGenericArg::ConstValue(v) => GenericArgWord::from_const(ConstArg::Value(v)).0,
            RawGenericArg::ConstParam(idx) => GenericArgWord::from_const(ConstArg::Param(idx)).0,
            RawGenericArg::ConstExpr(s) => GenericArgWord::from_const(ConstArg::ExprOffset(intern_str(s))).0,
            RawGenericArg::Lifetime(lt) => GenericArgWord::from_lifetime(lt).0,
        }
    }

    fn intern_generic_args<F>(&mut self, args: &[RawGenericArg<'_>], intern_str: &mut F) -> (u32, u32)
    where
        F: FnMut(&str) -> u32,
    {
        if args.is_empty() {
            return (0, 0);
        }
        let mut buf = std::mem::take(&mut self.scratch_words);
        buf.clear();
        buf.reserve(args.len());
        for &a in args {
            buf.push(Self::encode_arg_word(a, intern_str));
        }
        let res = self.intern_words(&buf);
        self.scratch_words = buf;
        res
    }

    fn encode_bound<F>(&mut self, b: &RawTyBound<'_>, intern_str: &mut F) -> TyBoundEntry
    where
        F: FnMut(&str) -> u32,
    {
        let (args_start, _) = if b.args.is_empty() && b.assocs.is_empty() {
            (0, 0)
        } else {
            let mut buf = std::mem::take(&mut self.scratch_words);
            buf.clear();
            buf.reserve(b.args.len() + b.assocs.len() * 2);
            for &a in b.args {
                buf.push(Self::encode_arg_word(a, intern_str));
            }
            for &(name, ty) in b.assocs {
                buf.push(intern_str(name));
                buf.push(ty.0);
            }
            let res = self.intern_words(&buf);
            self.scratch_words = buf;
            res
        };
        TyBoundEntry { trait_def: b.trait_def.0, args_start, args_len: b.args.len() as u16, assoc_len: b.assocs.len() as u16, flags: if b.is_maybe { BOUND_FLAG_MAYBE } else { 0 } }
    }

    fn intern_bounds<F>(&mut self, raw_bounds: &[RawTyBound<'_>], intern_str: &mut F) -> (u32, u32)
    where
        F: FnMut(&str) -> u32,
    {
        match raw_bounds {
            [] => (0, 0),
            [only] => {
                let b0 = self.encode_bound(only, intern_str);
                if let Some(&idx) = self.single_bound_map.get(&b0) {
                    return (idx, 1);
                }
                let idx = self.bounds.len() as u32;
                self.bounds.push(b0);
                self.single_bound_map.insert(b0, idx);
                (idx, 1)
            }
            many => {
                let mut bbuf = std::mem::take(&mut self.scratch_bounds);
                bbuf.clear();
                bbuf.reserve(many.len());
                for rb in many {
                    bbuf.push(self.encode_bound(rb, intern_str));
                }
                if let Some(&start) = self.bound_list_map.get(bbuf.as_slice()) {
                    let len = bbuf.len() as u32;
                    self.scratch_bounds = bbuf;
                    return (start, len);
                }
                let start = self.bounds.len() as u32;
                for (offset, &b) in bbuf.iter().enumerate() {
                    self.single_bound_map.entry(b).or_insert(start + offset as u32);
                }
                self.bounds.extend_from_slice(&bbuf);
                let len = bbuf.len() as u32;
                self.bound_list_map.insert(bbuf.as_slice().into(), start);
                self.scratch_bounds = bbuf;
                (start, len)
            }
        }
    }

    fn encode_raw_ty<F>(&mut self, ty: RawTy<'_>, intern_str: &mut F) -> TyEntry
    where
        F: FnMut(&str) -> u32,
    {
        let mk = |tag: TyTag, flags: u8, extra: u16, a: u32, b: u32, c: u32| TyEntry { tag: tag as u8, flags, extra, a, b, c };
        match ty {
            RawTy::Unit | RawTy::Tuple(&[]) => mk(TyTag::Unit, 0, 0, 0, 0, 0),
            RawTy::Bool => mk(TyTag::Bool, 0, 0, 0, 0, 0),
            RawTy::Char => mk(TyTag::Char, 0, 0, 0, 0, 0),
            RawTy::Int(w) => mk(TyTag::Int, w as u8, 0, 0, 0, 0),
            RawTy::Uint(w) => mk(TyTag::Uint, w as u8, 0, 0, 0, 0),
            RawTy::Float(w) => mk(TyTag::Float, w as u8, 0, 0, 0, 0),
            RawTy::Str => mk(TyTag::Str, 0, 0, 0, 0, 0),
            RawTy::Never => mk(TyTag::Never, 0, 0, 0, 0, 0),
            RawTy::Unknown => mk(TyTag::Unknown, 0, 0, 0, 0, 0),
            RawTy::Adt { def, args } => {
                let (s, l) = self.intern_generic_args(args, intern_str);
                mk(TyTag::Adt, 0, 0, def.0, s, l)
            }
            RawTy::Ref { mutability, inner, lifetime } => mk(TyTag::Ref, mutability as u8, 0, inner.0, lifetime, 0),
            RawTy::Ptr { mutability, inner } => mk(TyTag::Ptr, mutability as u8, 0, inner.0, 0, 0),
            RawTy::Slice(elem) => mk(TyTag::Slice, 0, 0, elem.0, 0, 0),
            RawTy::ArrayVal { elem, len } => mk(TyTag::Array, 0, 0, elem.0, GenericArgWord::from_const(ConstArg::Value(len)).0, 0),
            RawTy::ArrayConstParam { elem, param_idx } => mk(TyTag::Array, 0, 0, elem.0, GenericArgWord::from_const(ConstArg::Param(param_idx)).0, 0),
            RawTy::ArrayConstExpr { elem, expr } => {
                let off = intern_str(expr);
                mk(TyTag::Array, 0, 0, elem.0, GenericArgWord::from_const(ConstArg::ExprOffset(off)).0, 0)
            }
            RawTy::Tuple(elems) => {
                let mut buf = std::mem::take(&mut self.scratch_words);
                buf.clear();
                buf.extend(elems.iter().map(|t| t.0));
                let (s, l) = self.intern_words(&buf);
                self.scratch_words = buf;
                mk(TyTag::Tuple, 0, 0, s, l, 0)
            }
            RawTy::FnPtr { inputs, output, abi, is_unsafe, is_variadic } => {
                let mut buf = std::mem::take(&mut self.scratch_words);
                buf.clear();
                buf.extend(inputs.iter().map(|t| t.0));
                let (s, l) = self.intern_words(&buf);
                self.scratch_words = buf;
                let flags = (if is_unsafe { FN_FLAG_UNSAFE } else { 0 }) | (if is_variadic { FN_FLAG_VARIADIC } else { 0 });
                mk(TyTag::FnPtr, flags, abi as u16, s, l, output.0)
            }
            RawTy::FnDef { def, args } => {
                let (s, l) = self.intern_generic_args(args, intern_str);
                mk(TyTag::FnDef, 0, 0, def.0, s, l)
            }
            RawTy::Param { index, name } => {
                let name_off = if name.is_empty() { 0 } else { intern_str(name) };
                mk(TyTag::Param, 0, 0, index, name_off, 0)
            }
            RawTy::Assoc { trait_def, name, self_ty, trait_args } => {
                let name_off = intern_str(name);
                if trait_args.is_empty() {
                    mk(TyTag::Assoc, 0, 0, trait_def.0, name_off, self_ty.0)
                } else {
                    let mut buf = std::mem::take(&mut self.scratch_words);
                    buf.clear();
                    buf.reserve(1 + trait_args.len());
                    buf.push(self_ty.0);
                    for &a in trait_args {
                        buf.push(Self::encode_arg_word(a, intern_str));
                    }
                    let (s, _) = self.intern_words(&buf);
                    self.scratch_words = buf;
                    mk(TyTag::Assoc, 0, trait_args.len() as u16, trait_def.0, name_off, s)
                }
            }
            RawTy::Dyn { bounds, lifetime } => {
                let (s, l) = self.intern_bounds(bounds, intern_str);
                mk(TyTag::Dyn, 0, 0, s, l, lifetime)
            }
            RawTy::ImplTrait { bounds } => {
                let (s, l) = self.intern_bounds(bounds, intern_str);
                mk(TyTag::ImplTrait, 0, 0, s, l, 0)
            }
        }
    }
}
