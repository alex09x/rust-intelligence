/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use rustc_middle::ty::TyCtxt;
use rustc_span::def_id::DefId;
use std::path::{Path, PathBuf};

/// Remaps a compiler-internal span path (e.g. `/rustc/<commit>/library/...`)
/// to a local `rust-src` source path if available, or returns the relative library path.
pub fn remap_sysroot_path(raw_path: &str, sysroot_src: Option<&Path>) -> String {
    if let Some(idx) = raw_path.find("/library/") {
        let rel_part = &raw_path[idx + 1..];
        if let Some(src_root) = sysroot_src {
            let candidate = src_root.join(rel_part);
            if candidate.exists() {
                return candidate.to_string_lossy().into_owned();
            }
        }
        return rel_part.to_string();
    }
    raw_path.to_string()
}

/// Discovers the path to `library` inside `rust-src` for the current sysroot if installed.
pub fn discover_rust_src_path(tcx: TyCtxt<'_>) -> Option<PathBuf> {
    let candidate = tcx.sess.opts.sysroot.path().join("lib/rustlib/src/rust/library");
    if candidate.is_dir() {
        Some(candidate)
    } else {
        None
    }
}

/// Checks if a definition is public and not doc(hidden).
#[inline]
pub fn is_publicly_visible(tcx: TyCtxt<'_>, did: DefId) -> bool {
    tcx.visibility(did).is_public() && !tcx.is_doc_hidden(did)
}

/// Checks if a definition is stable (or unmarked), skipping unstable compiler features.
#[inline]
pub fn is_stable_api(tcx: TyCtxt<'_>, did: DefId) -> bool {
    if let Some(stab) = tcx.lookup_stability(did) {
        !stab.level.is_unstable()
    } else {
        true
    }
}

/// Formats an associated function signature cleanly, explicitly showing the receiver
/// (`fn(&self, ...)`, `fn(&mut self, ...)`, `fn(self, ...)`, or `fn(...)` without self).
pub fn format_assoc_fn_sig<'tcx>(tcx: TyCtxt<'tcx>, assoc_did: DefId) -> String {
    let fn_sig = tcx.fn_sig(assoc_did).instantiate_identity().skip_binder();
    let ret = crate::sanitize::sanitize_type_name(&fn_sig.output().to_string());
    let has_self = tcx.opt_associated_item(assoc_did).map(|a| a.is_method()).unwrap_or(false);

    if !has_self {
        let args: Vec<String> = fn_sig.inputs().iter().map(|ty| crate::sanitize::sanitize_type_name(&ty.to_string())).collect();
        return if ret == "()" { format!("fn({})", args.join(", ")) } else { format!("fn({}) -> {}", args.join(", "), ret) };
    }

    // Has self parameter: inspect the first input type to determine receiver style
    let (self_str, rest_inputs) = if let Some((first, rest)) = fn_sig.inputs().split_first() {
        let s = match first.kind() {
            rustc_middle::ty::TyKind::Ref(_, _, rustc_middle::ty::Mutability::Mut) => "&mut self",
            rustc_middle::ty::TyKind::Ref(_, _, rustc_middle::ty::Mutability::Not) => "&self",
            _ => "self",
        };
        (s, rest)
    } else {
        ("&self", [].as_slice())
    };

    let mut args = vec![self_str.to_string()];
    for ty in rest_inputs {
        args.push(crate::sanitize::sanitize_type_name(&ty.to_string()));
    }

    if ret == "()" {
        format!("fn({})", args.join(", "))
    } else {
        format!("fn({}) -> {}", args.join(", "), ret)
    }
}
