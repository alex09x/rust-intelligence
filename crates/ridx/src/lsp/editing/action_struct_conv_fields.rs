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
use super::action_struct_conv_index::receiver_ends_in_tuple_index;
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
    indexed: Option<&HashSet<usize>>,
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
                            Some(sites) if !sites.contains(&k) => {
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

/// Whether a `macro_rules!` body of the masked document reads one of `names` as a field (`$e.tokens`): the index
/// resolves no reference there, so a shape change would leave it behind.
pub(crate) fn macro_bodies_name_fields<'a>(masked: &str, names: impl Iterator<Item = &'a str> + Clone) -> bool {
    let b = masked.as_bytes();
    masked.match_indices("macro_rules!").any(|(p, _)| {
        let Some(open) = masked[p..].find('{').map(|o| p + o) else { return false };
        let close = super::action_parse_items::skip_balanced(b, open, b'{', b'}').unwrap_or(b.len());
        let body = &masked[open..close];
        names.clone().any(|n| {
            body.match_indices(n).any(|(i, _)| {
                let after = body.as_bytes().get(i + n.len());
                body[..i].trim_end().ends_with('.') && !body[..i].trim_end().ends_with("..") && after.is_none_or(|c| !(c.is_ascii_alphanumeric() || *c == b'_'))
            })
        })
    })
}

/// The entries of a brace literal's comma split, with the pieces a closure's parameters (`|a, b| ..`) or a comparison
/// split off joined back to the entry they continue: each entry starts with `name:`, a shorthand `name`, `..base`, or
/// an attribute, after any leading comments.
pub(crate) fn merge_continuations(pieces: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in pieces {
        match out.last_mut() {
            Some(prev) if !starts_entry(split_leading_comments(&p).1) => {
                prev.push_str(", ");
                prev.push_str(&p);
            }
            _ => out.push(p),
        }
    }
    out
}

fn starts_entry(t: &str) -> bool {
    if t.starts_with("..") || t.starts_with('#') {
        return true;
    }
    // A shorthand pattern binding may be `ref`, `mut` or `ref mut`.
    let mut body = t;
    for kw in ["ref", "mut"] {
        if let Some(r) = body.strip_prefix(kw).filter(|r| r.starts_with(char::is_whitespace)) {
            body = r.trim_start();
        }
    }
    let body = body.strip_prefix("r#").unwrap_or(body);
    let id_len = body.len() - body.trim_start_matches(|c: char| c.is_alphanumeric() || c == '_').len();
    if id_len == 0 || body.starts_with(|c: char| c.is_ascii_digit()) {
        return false;
    }
    let rest = body[id_len..].trim_start();
    rest.is_empty() || (rest.starts_with(':') && !rest.starts_with("::"))
}

/// `t` split into its leading `//` and `/* */` comments and the rest.
pub(crate) fn split_leading_comments(t: &str) -> (&str, &str) {
    let mut rest = t.trim_start();
    loop {
        if rest.starts_with("//") {
            rest = rest.find('\n').map_or("", |p| &rest[p + 1..]).trim_start();
        } else if rest.starts_with("/*") {
            match rest.find("*/") {
                Some(p) => rest = rest[p + 2..].trim_start(),
                None => return (t, ""),
            }
        } else {
            return (&t[..t.len() - rest.len()], rest);
        }
    }
}

/// `t` split into its leading `#[cfg(…)]` attributes and the rest; `None` on another attribute or one it cannot read.
pub(crate) fn split_leading_cfgs(t: &str) -> Option<(&str, &str)> {
    let masked = super::action_parse::mask_comments_and_strings(t);
    let mut p = 0;
    while masked[p..].starts_with('#') {
        let open = p + 1 + (masked[p + 1..].len() - masked[p + 1..].trim_start().len());
        if !masked[open..].starts_with("[cfg(") {
            return None;
        }
        p = super::action_parse_items::skip_balanced(masked.as_bytes(), open, b'[', b']')?;
        p += masked[p..].len() - masked[p..].trim_start().len();
    }
    Some((&t[..p], &t[p..]))
}

/// The index of the first field (by its leading text) under `#[cfg(…)]`, or the number of fields; `None` when a later
/// field may exist without one before it, which would shift the tuple indices between configurations. A later field
/// keeps the indices when it carries each earlier predicate, alone or inside an `all(…)`.
pub(crate) fn first_cfg_field(leadings: &[&str]) -> Option<usize> {
    let preds: Vec<Vec<&str>> = leadings
        .iter()
        .map(|l| {
            let masked = super::action_parse::mask_comments_and_strings(l);
            let mut out = Vec::new();
            for (at, _) in masked.match_indices("#[cfg(") {
                let open = at + 5;
                let close = super::action_parse_items::skip_balanced(masked.as_bytes(), open, b'(', b')')?;
                out.push(l[open + 1..close - 1].trim());
            }
            Some(out)
        })
        .collect::<Option<_>>()?;
    let first = preds.iter().position(|p| !p.is_empty()).unwrap_or(preds.len());
    let implies = |later: &[&str], p: &str| later.iter().any(|q| *q == p || (q.starts_with("all(") && !q.contains("not(") && !q.contains("any(") && q.contains(p)));
    for i in first..preds.len() {
        if preds[i + 1..].iter().any(|later| !preds[i].iter().all(|p| implies(later, p))) {
            return None;
        }
    }
    Some(first)
}

#[cfg(test)]
mod tests {
    use super::{first_cfg_field, macro_bodies_name_fields, merge_continuations, split_leading_cfgs, split_leading_comments};

    #[test]
    fn cfg_fields_keep_their_indices_only_at_the_end() {
        assert_eq!(first_cfg_field(&["", "#[cfg(feature = \"syntax\")]"]), Some(1));
        assert_eq!(first_cfg_field(&["", ""]), Some(2));
        let tokio = ["", "#[cfg(tokio_unstable)]", "/// d\n#[cfg(tokio_unstable)]", "#[cfg(all(tokio_unstable, feature = \"x\"))]"];
        assert_eq!(first_cfg_field(&tokio), Some(1));
        assert_eq!(first_cfg_field(&["#[cfg(a)]", ""]), None);
        assert_eq!(first_cfg_field(&["#[cfg(a)]", "#[cfg(not(a))]"]), None);
        assert_eq!(split_leading_cfgs("#[cfg(feature = \"]\")]\n x: 1"), Some(("#[cfg(feature = \"]\")]\n ", "x: 1")));
        assert_eq!(split_leading_cfgs("#[allow(unused)] x"), None);
    }

    #[test]
    fn a_field_read_in_a_macro_body_is_seen() {
        let doc = "macro_rules! syntax { ($p:expr) => { let g = $p; assert_eq!(g.tokens, x); }; }";
        assert!(macro_bodies_name_fields(doc, ["tokens"].into_iter()));
        assert!(!macro_bodies_name_fields(doc, ["glob"].into_iter()));
        assert!(!macro_bodies_name_fields("fn f(g: G) { g.tokens }", ["tokens"].into_iter()));
    }

    #[test]
    fn closure_parameters_and_comments_stay_with_their_entry() {
        let pieces = ["f: |a", "b| a + b", "// the count\n n", "..base"].map(String::from).to_vec();
        assert_eq!(merge_continuations(pieces), ["f: |a, b| a + b", "// the count\n n", "..base"]);
        let pattern = ["mut state_id", "ref mut tidx", "ref r#type"].map(String::from).to_vec();
        assert_eq!(merge_continuations(pattern.clone()), pattern);
        assert_eq!(split_leading_comments("// c\n /* d */ x: 1"), ("// c\n /* d */ ", "x: 1"));
    }
}
