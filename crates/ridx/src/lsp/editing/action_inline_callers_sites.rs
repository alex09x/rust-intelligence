/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The call sites of one document rewritten by `Inline into all callers`.

use super::action_inline_callers::line_col_to_byte;
use super::action_inline_callers_helpers::{is_caller_return_compatible, FnParamInfo, SelfKind};
use super::action_inline_callers_params::call_takes_only_places;
use super::action_inline_callers_transform::process_call_site;
use ridx_format::WorkspaceIndex;

/// The edits inlining the calls at `spans` (1-based) of `doc`, and whether any call was left as it is.
#[allow(clippy::too_many_arguments)]
pub(crate) fn process_call_sites_for_document(
    doc: &str,
    masked: &str,
    starts: &[usize],
    spans: &[(u32, u32)],
    fn_name: &str,
    self_kind: Option<SelfKind>,
    params: &[FnParamInfo],
    body_src: &str,
    body_has_question_mark: bool,
    effective_ret_ty: Option<&str>,
    def_range: Option<(usize, usize)>,
    file_opt: Option<&str>,
    ws_opt: Option<&WorkspaceIndex>,
    is_fully_pub: bool,
    borrowed_return: bool,
) -> (Vec<(usize, usize, String)>, bool) {
    if borrowed_return && self_kind == Some(SelfKind::Value) {
        return (Vec::new(), true);
    }
    // A declared type is written only where the definition is: another file may not see the names it uses.
    let unwritten: Vec<FnParamInfo>;
    let params = if def_range.is_some() {
        params
    } else {
        unwritten = params.iter().map(|p| FnParamInfo { decl_ty: None, ..p.clone() }).collect();
        &unwritten
    };
    let mut edits = Vec::new();
    let mut any_skipped = false;
    for &(span_line, span_col) in spans {
        let Some(ref_byte) = line_col_to_byte(doc, starts, span_line, span_col) else {
            any_skipped = true;
            continue;
        };
        if let Some((def_lo, def_hi)) = def_range {
            if ref_byte >= def_lo && ref_byte < def_hi {
                continue;
            }
        }
        if body_has_question_mark {
            let ret = effective_ret_ty.unwrap_or("");
            if !is_caller_return_compatible(doc, masked, starts, ref_byte, ret) {
                any_skipped = true;
                continue;
            }
        }
        if let Some((site_start, site_end, inlined_text)) =
            process_call_site(doc, masked, starts, ref_byte, fn_name, self_kind, params, body_src, file_opt, ws_opt, is_fully_pub, borrowed_return)
        {
            // A borrowed result must not point into a temporary the inlined block drops; under a `&` the block would be
            // expected to yield the referent (`&({ …; &x.f })` where `&T` was asked, E0308), which no rewrite of the
            // block avoids.
            let under_borrow = masked[..site_start].trim_end().ends_with('&');
            if borrowed_return && (!call_takes_only_places(&masked[site_start..site_end]) || under_borrow) {
                any_skipped = true;
                continue;
            }
            edits.push((site_start, site_end, inlined_text));
        } else {
            any_skipped = true;
        }
    }
    (edits, any_skipped)
}
