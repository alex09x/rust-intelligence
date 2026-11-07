/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Structured `UseEntry` extraction (`use`, globs, `use Trait as _;`, `extern crate`,
//! `#[prelude_import]`, flattened non-private `ModChild` re-exports, and `sysroot` re-exports/preludes)
//! across workspace crates, external dependencies, and `core`/`alloc`/`std` (1.2-G7).

use std::collections::HashMap;

use ridx_format::{IndexWriter, ItemVis, RawUse, TyDefId, UseOwner, UsePrimTy, USE_FLAG_EXTERN_CRATE, USE_FLAG_GLOB, USE_FLAG_PRELUDE, USE_FLAG_REEXPORT, USE_FLAG_UNDERSCORE};
use rustc_hir as hir;
use rustc_hir::def::{DefKind, Res};
use rustc_middle::metadata::{ModChild, Reexport};
use rustc_middle::ty::{self, FloatTy, IntTy, TyCtxt, UintTy};
use rustc_span::def_id::{CrateNum, DefId, LocalDefId, CRATE_DEF_ID, LOCAL_CRATE};
use rustc_span::edition::Edition;
use rustc_span::hygiene::{AstPass, ExpnKind};
use rustc_span::symbol::{kw, sym};

use super::fields::def_visibility;
use super::TcxTypeLowerer;
use crate::sysroot::util::is_stable_api;

#[inline]
fn normalize_use_target(tcx: TyCtxt<'_>, did: DefId, kind: DefKind) -> DefId {
    if matches!(kind, DefKind::Ctor(..)) {
        tcx.parent(did)
    } else {
        did
    }
}

#[inline]
fn map_prim_ty(prim: hir::PrimTy) -> UsePrimTy {
    match prim {
        hir::PrimTy::Bool => UsePrimTy::Bool,
        hir::PrimTy::Char => UsePrimTy::Char,
        hir::PrimTy::Str => UsePrimTy::Str,
        hir::PrimTy::Int(IntTy::I8) => UsePrimTy::I8,
        hir::PrimTy::Int(IntTy::I16) => UsePrimTy::I16,
        hir::PrimTy::Int(IntTy::I32) => UsePrimTy::I32,
        hir::PrimTy::Int(IntTy::I64) => UsePrimTy::I64,
        hir::PrimTy::Int(IntTy::I128) => UsePrimTy::I128,
        hir::PrimTy::Int(IntTy::Isize) => UsePrimTy::Isize,
        hir::PrimTy::Uint(UintTy::U8) => UsePrimTy::U8,
        hir::PrimTy::Uint(UintTy::U16) => UsePrimTy::U16,
        hir::PrimTy::Uint(UintTy::U32) => UsePrimTy::U32,
        hir::PrimTy::Uint(UintTy::U64) => UsePrimTy::U64,
        hir::PrimTy::Uint(UintTy::U128) => UsePrimTy::U128,
        hir::PrimTy::Uint(UintTy::Usize) => UsePrimTy::Usize,
        hir::PrimTy::Float(FloatTy::F16) => UsePrimTy::F16,
        hir::PrimTy::Float(FloatTy::F32) => UsePrimTy::F32,
        hir::PrimTy::Float(FloatTy::F64) => UsePrimTy::F64,
        hir::PrimTy::Float(FloatTy::F128) => UsePrimTy::F128,
    }
}

fn resolve_use_owner(tcx: TyCtxt<'_>, did: DefId, local_def_indices: &HashMap<DefId, u32>) -> Option<UseOwner> {
    let mut cur = tcx.parent(did);
    loop {
        if cur.is_crate_root() {
            return Some(UseOwner::CrateRoot);
        }
        if let Some(&idx) = local_def_indices.get(&cur) {
            return Some(UseOwner::Def(idx));
        }
        if tcx.def_kind(cur) != DefKind::Mod {
            return None;
        }
        cur = tcx.parent(cur);
    }
}

fn resolve_edition_prelude_did(tcx: TyCtxt<'_>, base_cnum: CrateNum) -> Option<DefId> {
    let ed = match tcx.sess.edition() {
        Edition::Edition2015 => "rust_2015",
        Edition::Edition2018 => "rust_2018",
        Edition::Edition2021 => "rust_2021",
        _ => "rust_2024",
    };
    let prelude_mod = tcx.module_children(base_cnum.as_def_id()).iter().find_map(|c| (c.ident.as_str() == "prelude").then(|| c.res.opt_def_id()).flatten())?;
    tcx.module_children(prelude_mod).iter().find_map(|c| (c.ident.as_str() == ed).then(|| c.res.opt_def_id()).flatten())
}

fn mod_child_vis(tcx: TyCtxt<'_>, mod_did: DefId, child: &ModChild) -> ItemVis {
    match child.vis {
        ty::Visibility::Public => ItemVis::Public,
        ty::Visibility::Restricted(vis_mod) => {
            if !mod_did.is_crate_root() {
                if vis_mod.is_crate_root() {
                    ItemVis::Crate
                } else if vis_mod == mod_did {
                    ItemVis::Private
                } else {
                    ItemVis::Restricted
                }
            } else if let Some(reexp_did) = child.reexport_chain.first().and_then(|r| match r {
                Reexport::Single(d) | Reexport::Glob(d) | Reexport::ExternCrate(d) => Some(*d),
                _ => None,
            }) {
                def_visibility(tcx, reexp_did, tcx.def_kind(reexp_did))
            } else {
                ItemVis::Private
            }
        }
    }
}

impl<'tcx, 'a> TcxTypeLowerer<'tcx, 'a> {
    fn classify_mod_child_res<Id>(&mut self, res: Res<Id>, writer: &mut IndexWriter) -> (Option<TyDefId>, Option<TyDefId>, Option<TyDefId>, UsePrimTy) {
        match res {
            Res::PrimTy(p) => (None, None, None, map_prim_ty(p)),
            Res::Def(kind, did) => {
                let norm = normalize_use_target(self.tcx, did, kind);
                match kind {
                    DefKind::Mod
                    | DefKind::Struct
                    | DefKind::Union
                    | DefKind::Enum
                    | DefKind::Variant
                    | DefKind::Trait
                    | DefKind::TyAlias
                    | DefKind::ForeignTy
                    | DefKind::TraitAlias
                    | DefKind::AssocTy
                    | DefKind::OpaqueTy
                    | DefKind::TyParam => (Some(self.lower_def(norm, writer)), None, None, UsePrimTy::None),
                    DefKind::Fn | DefKind::Const { .. } | DefKind::Static { .. } | DefKind::Ctor(..) | DefKind::AssocFn | DefKind::AssocConst { .. } | DefKind::ConstParam => {
                        (None, Some(self.lower_def(norm, writer)), None, UsePrimTy::None)
                    }
                    DefKind::Macro(..) => (None, None, Some(self.lower_def(norm, writer)), UsePrimTy::None),
                    _ => (None, None, None, UsePrimTy::None),
                }
            }
            _ => (None, None, None, UsePrimTy::None),
        }
    }

    fn lower_type_ns_res<Id>(&mut self, res: Res<Id>, writer: &mut IndexWriter) -> (Option<TyDefId>, UsePrimTy) {
        match res {
            Res::PrimTy(p) => (None, map_prim_ty(p)),
            Res::Def(kind, did) => (Some(self.lower_def(normalize_use_target(self.tcx, did, kind), writer)), UsePrimTy::None),
            Res::SelfTyParam { trait_: did } | Res::SelfTyAlias { alias_to: did, .. } => (Some(self.lower_def(did, writer)), UsePrimTy::None),
            _ => (None, UsePrimTy::None),
        }
    }

    fn lower_value_or_macro_res<Id>(&mut self, res: Res<Id>, writer: &mut IndexWriter) -> Option<TyDefId> {
        match res {
            Res::Def(kind, did) => Some(self.lower_def(normalize_use_target(self.tcx, did, kind), writer)),
            Res::SelfCtor(did) => Some(self.lower_def(did, writer)),
            _ => None,
        }
    }

    /// Extracts all HIR `use` / `extern crate` bindings, active `#[prelude_import]` glob, and
    /// flattened non-private `ModChild` re-exports for a workspace or external dependency crate.
    pub(crate) fn record_local_crate_uses(&mut self, writer: &mut IndexWriter) {
        let tcx = self.tcx;
        let mut mod_scopes: Vec<(LocalDefId, UseOwner)> = vec![(CRATE_DEF_ID, UseOwner::CrateRoot)];
        let mut std_imports_cnum: Option<CrateNum> = None;
        let mut emitted_prelude = false;

        for local_did in tcx.hir_crate_items(()).definitions() {
            let kind = tcx.def_kind(local_did);
            let did = local_did.to_def_id();
            match kind {
                DefKind::Mod if !did.is_crate_root() => {
                    if let Some(&def_idx) = self.local_def_indices.get(&did) {
                        mod_scopes.push((local_did, UseOwner::Def(def_idx)));
                    }
                }
                DefKind::Use => {
                    let Some(owner) = resolve_use_owner(tcx, did, self.local_def_indices) else { continue };
                    let hir::Node::Item(item) = tcx.hir_node_by_def_id(local_did) else { continue };
                    let hir::ItemKind::Use(use_path, use_kind) = &item.kind else { continue };
                    let vis = def_visibility(tcx, did, kind);
                    let def_index = self.local_def_indices.get(&did).copied();
                    let mut flags = if vis != ItemVis::Private { USE_FLAG_REEXPORT } else { 0 };
                    match *use_kind {
                        hir::UseKind::ListStem => {}
                        hir::UseKind::Glob => {
                            flags |= USE_FLAG_GLOB;
                            let is_prelude = owner == UseOwner::CrateRoot
                                && (matches!(item.span.ctxt().outer_expn_data().kind, ExpnKind::AstPass(AstPass::StdImports))
                                    || tcx.hir_attrs(item.hir_id()).iter().any(|a| a.has_name(sym::prelude_import)));
                            if is_prelude {
                                flags |= USE_FLAG_PRELUDE;
                            }
                            let glob_res = use_path.res.type_ns.or(use_path.res.value_ns).or(use_path.res.macro_ns).or_else(|| use_path.segments.last().map(|s| s.res));
                            let Some((Some(target_def), _)) = glob_res.map(|r| self.lower_type_ns_res(r, writer)) else { continue };
                            if is_prelude {
                                emitted_prelude = true;
                            }
                            writer.add_use(RawUse {
                                owner,
                                def_index,
                                name: "*",
                                type_def: Some(target_def),
                                value_def: None,
                                macro_def: None,
                                vis,
                                flags,
                                prim_ty: UsePrimTy::None,
                            });
                        }
                        hir::UseKind::Single(ident) => {
                            if ident.name == kw::Underscore {
                                flags |= USE_FLAG_UNDERSCORE;
                            }
                            let (type_def, prim_ty) = use_path.res.type_ns.map_or((None, UsePrimTy::None), |r| self.lower_type_ns_res(r, writer));
                            let value_def = use_path.res.value_ns.and_then(|r| self.lower_value_or_macro_res(r, writer));
                            let macro_def = use_path.res.macro_ns.and_then(|r| self.lower_value_or_macro_res(r, writer));
                            if type_def.is_none() && value_def.is_none() && macro_def.is_none() && prim_ty == UsePrimTy::None {
                                continue;
                            }
                            writer.add_use(RawUse { owner, def_index, name: ident.as_str(), type_def, value_def, macro_def, vis, flags, prim_ty });
                        }
                    }
                }
                DefKind::ExternCrate => {
                    let Some(owner) = resolve_use_owner(tcx, did, self.local_def_indices) else { continue };
                    let hir::Node::Item(item) = tcx.hir_node_by_def_id(local_did) else { continue };
                    let hir::ItemKind::ExternCrate(orig_name, ident) = item.kind else { continue };
                    let Some(cnum) = tcx.extern_mod_stmt_cnum(local_did).or_else(|| (orig_name == Some(kw::SelfLower)).then_some(LOCAL_CRATE)) else { continue };
                    if owner == UseOwner::CrateRoot && matches!(item.span.ctxt().outer_expn_data().kind, ExpnKind::AstPass(AstPass::StdImports)) {
                        std_imports_cnum = Some(cnum);
                    }
                    let vis = def_visibility(tcx, did, kind);
                    let mut flags = USE_FLAG_EXTERN_CRATE;
                    if vis != ItemVis::Private {
                        flags |= USE_FLAG_REEXPORT;
                    }
                    if ident.name == kw::Underscore {
                        flags |= USE_FLAG_UNDERSCORE;
                    }
                    let target_def = self.lower_def(cnum.as_def_id(), writer);
                    let def_index = self.local_def_indices.get(&did).copied();
                    writer.add_use(RawUse {
                        owner,
                        def_index,
                        name: ident.as_str(),
                        type_def: Some(target_def),
                        value_def: None,
                        macro_def: None,
                        vis,
                        flags,
                        prim_ty: UsePrimTy::None,
                    });
                }
                _ => {}
            }
        }

        let no_implicit_prelude = tcx.hir_attrs(hir::CRATE_HIR_ID).iter().any(|a| a.has_name(sym::no_implicit_prelude));
        if !emitted_prelude && !no_implicit_prelude {
            if let Some(prelude_did) = std_imports_cnum.and_then(|c| resolve_edition_prelude_did(tcx, c)) {
                let target_def = self.lower_def(prelude_did, writer);
                writer.add_use(RawUse {
                    owner: UseOwner::CrateRoot,
                    def_index: None,
                    name: "*",
                    type_def: Some(target_def),
                    value_def: None,
                    macro_def: None,
                    vis: ItemVis::Private,
                    flags: USE_FLAG_GLOB | USE_FLAG_PRELUDE,
                    prim_ty: UsePrimTy::None,
                });
            }
        }

        for (mod_local_did, owner) in mod_scopes {
            let mod_did = mod_local_did.to_def_id();
            for child in tcx.module_children_local(mod_local_did) {
                if child.reexport_chain.is_empty() || child.ident.name == kw::Underscore {
                    continue;
                }
                let vis = mod_child_vis(tcx, mod_did, child);
                if vis == ItemVis::Private {
                    continue;
                }
                let (type_def, value_def, macro_def, prim_ty) = self.classify_mod_child_res(child.res, writer);
                if type_def.is_none() && value_def.is_none() && macro_def.is_none() && prim_ty == UsePrimTy::None {
                    continue;
                }
                writer.add_use(RawUse { owner, def_index: None, name: child.ident.as_str(), type_def, value_def, macro_def, vis, flags: USE_FLAG_REEXPORT, prim_ty });
            }
        }
    }

    /// Extracts public re-exports and prelude/primitive bindings across all indexed modules of a sysroot crate.
    pub(crate) fn record_sysroot_uses(&mut self, cnum: CrateNum, target_crate: &str, mod_items: &[(DefId, u32)], writer: &mut IndexWriter) {
        let tcx = self.tcx;
        let root_did = cnum.as_def_id();
        for &(mod_did, def_idx) in mod_items {
            let owner = if mod_did == root_did { UseOwner::CrateRoot } else { UseOwner::Def(def_idx) };
            for child in tcx.module_children(mod_did) {
                if !child.vis.is_public() || child.ident.name == kw::Underscore {
                    continue;
                }
                let name_str = child.ident.as_str();
                if name_str.starts_with("__") || (mod_did == root_did && matches!(name_str, "arch" | "core_arch" | "intrinsics")) {
                    continue;
                }
                match child.res {
                    Res::PrimTy(p) => {
                        writer.add_use(RawUse {
                            owner,
                            def_index: None,
                            name: name_str,
                            type_def: None,
                            value_def: None,
                            macro_def: None,
                            vis: ItemVis::Public,
                            flags: USE_FLAG_REEXPORT,
                            prim_ty: map_prim_ty(p),
                        });
                    }
                    Res::Def(kind, did) => {
                        let norm = normalize_use_target(tcx, did, kind);
                        if norm.krate != cnum {
                            let dep_sym = tcx.crate_name(norm.krate);
                            let dep = dep_sym.as_str();
                            let allowed = match target_crate {
                                "std" => dep == "core" || dep == "alloc",
                                "alloc" => dep == "core",
                                _ => false,
                            };
                            if !allowed {
                                continue;
                            }
                        }
                        if !is_stable_api(tcx, norm) && !self.local_def_indices.contains_key(&norm) {
                            continue;
                        }
                        let is_reexport = !child.reexport_chain.is_empty() || norm.krate != cnum || (!norm.is_crate_root() && tcx.parent(norm) != mod_did);
                        if !is_reexport {
                            continue;
                        }
                        let (type_def, value_def, macro_def, prim_ty) = self.classify_mod_child_res(child.res, writer);
                        if type_def.is_none() && value_def.is_none() && macro_def.is_none() {
                            continue;
                        }
                        writer.add_use(RawUse { owner, def_index: None, name: name_str, type_def, value_def, macro_def, vis: ItemVis::Public, flags: USE_FLAG_REEXPORT, prim_ty });
                    }
                    _ => {}
                }
            }
        }
    }
}
