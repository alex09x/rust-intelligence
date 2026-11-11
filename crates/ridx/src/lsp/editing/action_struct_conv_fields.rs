/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The `.name` → `.N` field access edits of `Convert to tuple struct`, including those inside the struct's own literals.

use super::action_parse_decl::next_ident_token;
use super::action_struct_conv_helpers::receiver_is_proven_owner;
use super::action_struct_conv_index::{receiver_ends_in_tuple_index, FieldRefs};
use std::collections::{HashMap, HashSet};

/// A document-addressed replacement of `[lo, hi)`.
pub(crate) type RawEdit = (usize, usize, String);

/// `src` (starting at byte `base` of the document) with the document-addressed `edits` inside it applied.
pub(crate) fn splice(src: &str, base: usize, edits: &[RawEdit]) -> String {
    let mut out = src.to_string();
    let mut sorted: Vec<_> = edits.iter().collect();
    sorted.sort_by_key(|e| std::cmp::Reverse(e.0));
    for (lo, hi, text) in sorted {
        out.replace_range(lo - base..hi - base, text);
    }
    out
}

/// The `.name` → `.N` edits for the struct's fields in `[mod_lo, mod_hi)`, outside the declaration `[decl_lo, decl_hi)`.
/// With the index only accesses it resolved to the struct's fields are rewritten; without it each receiver must be
/// proven the struct's. `None` when an access cannot be rewritten safely.
#[allow(clippy::too_many_arguments)]
pub(crate) fn field_access_edits(
    doc_text: &str,
    masked_doc: &str,
    (mod_lo, mod_hi): (usize, usize),
    (decl_lo, decl_hi): (usize, usize),
    field_index: &HashMap<&str, usize>,
    indexed: Option<&FieldRefs>,
    impl_spans: &[(usize, usize)],
    known_locals: &HashSet<String>,
) -> Option<Vec<RawEdit>> {
    let mb = masked_doc.as_bytes();
    let mut out = Vec::new();
    let mut d = mod_lo;
    while d + 1 < mod_hi {
        if d >= decl_lo && d < decl_hi {
            d = decl_hi;
            continue;
        }
        if mb[d] == b'.' && mb.get(d + 1).copied() != Some(b'.') && (d == 0 || mb[d - 1] != b'.') {
            let mut k = d + 1;
            while k < mod_hi && mb[k].is_ascii_whitespace() {
                k += 1;
            }
            if let Some((raw_f, f_end)) = next_ident_token(doc_text, k) {
                let fname = raw_f.strip_prefix("r#").unwrap_or(raw_f);
                if let Some(&idx) = field_index.get(fname) {
                    let after_trim = masked_doc[f_end..mod_hi].trim_start();
                    if !after_trim.starts_with('(') && !after_trim.starts_with("::<") {
                        match indexed {
                            // Another type's field of the same name: not ours to rewrite.
                            Some(refs) if !refs.sites.contains(&k) => {
                                d = f_end;
                                continue;
                            }
                            // `w.0.name` reaches the field through `Deref` on the tuple struct `w.0`: `w.0.0` would name
                            // the wrapper's own field instead.
                            Some(_) if receiver_ends_in_tuple_index(&masked_doc[mod_lo..d]) => return None,
                            Some(_) => {}
                            None => {
                                let in_impl = impl_spans.iter().any(|&(bo, bc)| d > bo && d < bc);
                                if !receiver_is_proven_owner(masked_doc, mod_lo, d, in_impl, known_locals) {
                                    return None;
                                }
                            }
                        }
                        out.push((k, f_end, idx.to_string()));
                        d = f_end;
                        continue;
                    }
                }
            }
        }
        d += 1;
    }
    Some(out)
}
