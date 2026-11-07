/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Extraction of declarative macro definitions (`macro_rules!` and Rust 2.0 `macro`)
//! from local HIR (`LOCAL_CRATE`) and external `.rmeta` (`sysroot` crates).

use std::collections::HashMap;

use ridx_format::{IndexWriter, MacroDelim, MacroEdition, RawMacroDef, MACRO_FLAG_BUILTIN, MACRO_FLAG_EXPORTED, MACRO_FLAG_LOCAL_INNER, MACRO_FLAG_MACRO_RULES};
use rustc_ast::ast::MacroDef;
use rustc_ast::token::Delimiter;
use rustc_hir::attrs::AttributeKind;
use rustc_hir::{self as hir, Attribute};
use rustc_metadata::creader::{CStore, LoadedMacro};
use rustc_middle::ty::TyCtxt;
use rustc_span::def_id::{DefId, LocalDefId};
use rustc_span::edition::Edition;

#[inline]
fn map_delim(delim: Delimiter) -> MacroDelim {
    match delim {
        Delimiter::Parenthesis => MacroDelim::Paren,
        Delimiter::Bracket => MacroDelim::Bracket,
        _ => MacroDelim::Brace,
    }
}

#[inline]
fn map_edition(edition: Edition) -> MacroEdition {
    match edition {
        Edition::Edition2015 => MacroEdition::Edition2015,
        Edition::Edition2018 => MacroEdition::Edition2018,
        Edition::Edition2021 => MacroEdition::Edition2021,
        _ => MacroEdition::Edition2024,
    }
}

#[allow(deprecated)]
fn extract_macro_attr_flags(tcx: TyCtxt<'_>, did: DefId, is_macro_rules: bool) -> u8 {
    let mut flags = if is_macro_rules { MACRO_FLAG_MACRO_RULES } else { 0 };
    for attr in tcx.get_all_attrs(did) {
        match attr {
            Attribute::Parsed(AttributeKind::MacroExport { local_inner_macros, .. }) => {
                flags |= MACRO_FLAG_EXPORTED;
                if *local_inner_macros {
                    flags |= MACRO_FLAG_LOCAL_INNER;
                }
            }
            Attribute::Parsed(AttributeKind::RustcBuiltinMacro { .. }) => {
                flags |= MACRO_FLAG_BUILTIN;
            }
            _ => {}
        }
    }
    flags
}

fn encode_ast_macro_def(tcx: TyCtxt<'_>, did: DefId, mdef: &MacroDef, edition: Edition) -> (String, u8, MacroDelim, MacroEdition) {
    let body = rustc_ast_pretty::pprust::tts_to_string(&mdef.body.tokens);
    let flags = extract_macro_attr_flags(tcx, did, mdef.macro_rules);
    let delim = map_delim(mdef.body.delim);
    let ed = map_edition(edition);
    (body, flags, delim, ed)
}

/// Records all declarative macros (`hir::ItemKind::Macro`) in `LOCAL_CRATE`.
///
/// Procedural macro entry points (`#[proc_macro]`, `#[proc_macro_derive]`, `#[proc_macro_attribute]`)
/// are `hir::ItemKind::Fn` in HIR and are naturally skipped here.
pub(crate) fn record_local_macro_defs(tcx: TyCtxt<'_>, macro_items: &[(LocalDefId, u32)], writer: &mut IndexWriter) {
    if macro_items.is_empty() {
        return;
    }
    let crate_edition = tcx.sess.edition();
    for &(local_did, def_index) in macro_items {
        let hir::Node::Item(item) = tcx.hir_node_by_def_id(local_did) else {
            continue;
        };
        let hir::ItemKind::Macro(_ident, mdef, _kinds) = &item.kind else {
            continue;
        };
        let (body, flags, delim, edition) = encode_ast_macro_def(tcx, local_did.to_def_id(), mdef, crate_edition);
        writer.add_macro_def(RawMacroDef { def_index, body: &body, flags, delim, edition });
    }
}

/// Records all declarative macros in an external/sysroot crate (`core`, `alloc`, `std`)
/// directly from pre-loaded `.rmeta` via `CStore::load_macro_untracked`.
pub(crate) fn record_external_macro_defs(tcx: TyCtxt<'_>, macro_items: &[(DefId, u32)], writer: &mut IndexWriter) {
    if macro_items.is_empty() {
        return;
    }
    let cstore = CStore::from_tcx(tcx);
    let mut decoded_cache: HashMap<DefId, Option<(String, u8, MacroDelim, MacroEdition)>> = HashMap::new();

    for &(did, def_index) in macro_items {
        if did.is_local() {
            continue;
        }
        let entry = decoded_cache.entry(did).or_insert_with(|| match cstore.load_macro_untracked(tcx, did) {
            LoadedMacro::MacroDef { def, edition, .. } => Some(encode_ast_macro_def(tcx, did, &def, edition)),
            LoadedMacro::ProcMacro(_) => None,
        });
        if let Some((ref body, flags, delim, edition)) = *entry {
            writer.add_macro_def(RawMacroDef { def_index, body, flags, delim, edition });
        }
    }
}
