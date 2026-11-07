/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Zero-intermediate-allocation human-readable formatting for `TyArenaView`.

use std::fmt::{self, Write};

use crate::format::{ConstArg, FnAbiKind, TyDefId, TyId, TyMutability, LIFETIME_ELIDED, LIFETIME_STATIC};

use super::{GenericArgIter, GenericArgView, TyArenaView, TyBoundIter, TyKindView};

const MAX_FORMAT_DEPTH: usize = 64;

impl<'a> TyArenaView<'a> {
    /// Formats `id` into a fresh `String`.
    pub fn display_ty(&self, id: TyId) -> String {
        let mut out = String::with_capacity(32);
        let _ = self.write_ty(id, &mut out);
        out
    }

    /// Formats `id` into `out` without intermediate `String` allocations.
    pub fn write_ty<W: Write>(&self, id: TyId, out: &mut W) -> fmt::Result {
        self.write_ty_depth(id, out, 0)
    }

    fn write_def_name<W: Write>(&self, id: TyDefId, short: bool, out: &mut W) -> fmt::Result {
        match self.def(id) {
            Some(d) if !d.path.is_empty() => {
                let label = if short { d.path.rsplit("::").next().unwrap_or(d.path) } else { d.path };
                out.write_str(label)
            }
            _ => out.write_str("{def?}"),
        }
    }

    fn write_lifetime<W: Write>(lt: u32, out: &mut W) -> fmt::Result {
        match lt {
            LIFETIME_ELIDED => Ok(()),
            LIFETIME_STATIC => out.write_str("'static"),
            n => write!(out, "'l{}", n.saturating_sub(1)),
        }
    }

    fn write_const_arg<W: Write>(&self, c: ConstArg, out: &mut W) -> fmt::Result {
        match c {
            ConstArg::Value(v) => write!(out, "{v}"),
            ConstArg::Param(idx) => write!(out, "C#{idx}"),
            ConstArg::ExprOffset(off) => {
                let s = self.str_at(off);
                if s.is_empty() {
                    out.write_str("_")
                } else {
                    out.write_str(s)
                }
            }
        }
    }

    fn write_generic_arg<W: Write>(&self, arg: GenericArgView, out: &mut W, depth: usize) -> fmt::Result {
        match arg {
            GenericArgView::Type(t) => self.write_ty_depth(t, out, depth + 1),
            GenericArgView::Const(c) => self.write_const_arg(c, out),
            GenericArgView::Lifetime(lt) => {
                if lt == LIFETIME_ELIDED {
                    out.write_str("'_")
                } else {
                    Self::write_lifetime(lt, out)
                }
            }
        }
    }

    fn write_generic_args<W: Write>(&self, args: GenericArgIter<'a>, out: &mut W, depth: usize) -> fmt::Result {
        if args.is_empty() {
            return Ok(());
        }
        out.write_char('<')?;
        for (i, arg) in args.enumerate() {
            if i > 0 {
                out.write_str(", ")?;
            }
            self.write_generic_arg(arg, out, depth)?;
        }
        out.write_char('>')
    }

    fn write_bounds<W: Write>(&self, bounds: TyBoundIter<'a>, out: &mut W, depth: usize) -> fmt::Result {
        for (i, b) in bounds.enumerate() {
            if i > 0 {
                out.write_str(" + ")?;
            }
            if b.is_maybe {
                out.write_char('?')?;
            }
            self.write_def_name(b.trait_def, true, out)?;
            if !b.args.is_empty() || b.assocs.len() > 0 {
                out.write_char('<')?;
                let mut first = true;
                for arg in b.args {
                    if !first {
                        out.write_str(", ")?;
                    }
                    first = false;
                    self.write_generic_arg(arg, out, depth)?;
                }
                for assoc in b.assocs {
                    if !first {
                        out.write_str(", ")?;
                    }
                    first = false;
                    out.write_str(assoc.name)?;
                    out.write_str(" = ")?;
                    self.write_ty_depth(assoc.ty, out, depth + 1)?;
                }
                out.write_char('>')?;
            }
        }
        Ok(())
    }

    fn write_ty_depth<W: Write>(&self, id: TyId, out: &mut W, depth: usize) -> fmt::Result {
        if depth >= MAX_FORMAT_DEPTH {
            return out.write_str("...");
        }
        match self.kind(id) {
            TyKindView::Unit => out.write_str("()"),
            TyKindView::Bool => out.write_str("bool"),
            TyKindView::Char => out.write_str("char"),
            TyKindView::Int(w) => out.write_str(w.as_str()),
            TyKindView::Uint(w) => out.write_str(w.as_str()),
            TyKindView::Float(w) => out.write_str(w.as_str()),
            TyKindView::Str => out.write_str("str"),
            TyKindView::Never => out.write_str("!"),
            TyKindView::Unknown => out.write_str("{unknown}"),
            TyKindView::Adt { def, args } | TyKindView::FnDef { def, args } => {
                self.write_def_name(def, true, out)?;
                self.write_generic_args(args, out, depth)
            }
            TyKindView::Ref { mutability, inner, lifetime } => {
                out.write_char('&')?;
                if lifetime != LIFETIME_ELIDED {
                    Self::write_lifetime(lifetime, out)?;
                    out.write_char(' ')?;
                }
                if mutability == TyMutability::Mut {
                    out.write_str("mut ")?;
                }
                self.write_ty_depth(inner, out, depth + 1)
            }
            TyKindView::Ptr { mutability, inner } => {
                out.write_str(if mutability == TyMutability::Mut { "*mut " } else { "*const " })?;
                self.write_ty_depth(inner, out, depth + 1)
            }
            TyKindView::Slice(elem) => {
                out.write_char('[')?;
                self.write_ty_depth(elem, out, depth + 1)?;
                out.write_char(']')
            }
            TyKindView::Array { elem, len } => {
                out.write_char('[')?;
                self.write_ty_depth(elem, out, depth + 1)?;
                out.write_str("; ")?;
                self.write_const_arg(len, out)?;
                out.write_char(']')
            }
            TyKindView::Tuple(elems) => {
                let n = elems.len();
                out.write_char('(')?;
                for (i, elem) in elems.enumerate() {
                    if i > 0 {
                        out.write_str(", ")?;
                    }
                    self.write_ty_depth(elem, out, depth + 1)?;
                }
                if n == 1 {
                    out.write_char(',')?;
                }
                out.write_char(')')
            }
            TyKindView::FnPtr { inputs, output, abi, is_unsafe, is_variadic } => {
                if is_unsafe {
                    out.write_str("unsafe ")?;
                }
                match abi {
                    FnAbiKind::Rust => {}
                    FnAbiKind::C => out.write_str("extern \"C\" ")?,
                    FnAbiKind::System => out.write_str("extern \"system\" ")?,
                    FnAbiKind::RustCall => out.write_str("extern \"rust-call\" ")?,
                    FnAbiKind::Other => out.write_str("extern ")?,
                }
                out.write_str("fn(")?;
                for (i, inp) in inputs.enumerate() {
                    if i > 0 {
                        out.write_str(", ")?;
                    }
                    self.write_ty_depth(inp, out, depth + 1)?;
                }
                if is_variadic {
                    if !inputs.is_empty() {
                        out.write_str(", ")?;
                    }
                    out.write_str("...")?;
                }
                out.write_char(')')?;
                if !matches!(self.kind(output), TyKindView::Unit) {
                    out.write_str(" -> ")?;
                    self.write_ty_depth(output, out, depth + 1)?;
                }
                Ok(())
            }
            TyKindView::Param { index, name } => {
                if name.is_empty() {
                    write!(out, "T#{index}")
                } else {
                    out.write_str(name)
                }
            }
            TyKindView::Assoc { trait_def, name, self_ty, trait_args } => {
                if trait_def.is_none() {
                    self.write_ty_depth(self_ty, out, depth + 1)?;
                    out.write_str("::")?;
                    out.write_str(name)
                } else {
                    out.write_char('<')?;
                    self.write_ty_depth(self_ty, out, depth + 1)?;
                    out.write_str(" as ")?;
                    self.write_def_name(trait_def, true, out)?;
                    self.write_generic_args(trait_args, out, depth)?;
                    out.write_str(">::")?;
                    out.write_str(name)
                }
            }
            TyKindView::Dyn { bounds, lifetime } => {
                out.write_str("dyn ")?;
                let had_bounds = !bounds.is_empty();
                self.write_bounds(bounds, out, depth)?;
                if lifetime != LIFETIME_ELIDED {
                    if had_bounds {
                        out.write_str(" + ")?;
                    }
                    Self::write_lifetime(lifetime, out)?;
                }
                Ok(())
            }
            TyKindView::ImplTrait { bounds } => {
                out.write_str("impl ")?;
                self.write_bounds(bounds, out, depth)
            }
        }
    }
}
