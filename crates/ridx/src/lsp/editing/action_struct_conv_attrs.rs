/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Whether a struct's outer attributes survive a change of its shape (named ↔ tuple fields). Built-in derives and
//! attributes do. A third-party derive is accepted on compiler evidence: the index has it on a struct of the target
//! shape that compiled. Its helper attributes (`#[reflect(Default)]`) pass when every key they use was seen on such a
//! struct too; an attribute macro (one the index resolves) does not.

use super::action_generics_sub::split_top_level_commas;
use super::action_parse::mask_comments_and_strings;
use super::action_parse_items::skip_balanced;
use super::action_struct_conv_index::byte_to_line_col;
use ridx_format::{DefKind, WorkspaceIndex};
use std::collections::{HashMap, HashSet};

/// The fields a struct is converted to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shape {
    Tuple,
    Named,
}

/// Derives of the standard library and serde: they take named and tuple structs alike.
const STD_DERIVES: &[&str] = &["Debug", "Clone", "Copy", "PartialEq", "Eq", "PartialOrd", "Ord", "Hash", "Default", "Serialize", "Deserialize"];

/// Third-party derives known to take both shapes, accepted without the index (clap's `Parser`/`Args` do not).
const KNOWN_DERIVES: &[&str] = &["Component", "Resource", "Reflect", "Arbitrary"];

/// Built-in attributes that do not depend on how the fields are written.
const INERT_ATTRS: &[&str] = &["allow", "expect", "warn", "deny", "forbid", "doc", "must_use", "non_exhaustive", "repr"];

/// The index's references read for evidence, at most.
const EVIDENCE_REFS: usize = 400;

/// The index and the document whose attributes are checked.
pub(crate) struct AttrCtx<'a> {
    pub ws: &'a WorkspaceIndex,
    pub file: &'a str,
    pub doc: &'a str,
    pub starts: &'a [usize],
}

/// A helper attribute's name and one key of its arguments (`reflect(Default)` → `("reflect", "Default")`).
type HelperKey = (String, String);

/// One attribute: its path's last segment, the byte of that name, and its arguments after the name.
struct Attr<'s> {
    name: &'s str,
    at: usize,
    args: &'s str,
    args_at: usize,
}

/// Whether the outer attributes `doc[lo..hi]` allow converting the struct to `shape`.
pub(crate) fn outer_attrs_allow_shape_change(doc: &str, lo: usize, hi: usize, ctx: Option<&AttrCtx>, shape: Shape) -> bool {
    let Some(attrs) = parse_attrs(doc, lo, hi) else { return false };
    let mut seen: HashSet<HelperKey> = HashSet::new();
    let mut helpers = Vec::new();
    // The helper keys to vouch for; a derive's search ends once they are all seen (at once without helpers).
    let needed: HashSet<HelperKey> =
        attrs.iter().filter(|a| a.name != "derive" && !INERT_ATTRS.contains(&a.name) && a.name != "cfg_attr").flat_map(|a| helper_keys(a.name, a.args)).collect();
    let has_helpers = !needed.is_empty();
    for a in &attrs {
        if a.name == "derive" {
            let Some(list) = a.args.strip_prefix('(').and_then(|x| x.strip_suffix(')')) else { return false };
            let mut at = a.args_at + 1;
            for d in list.split(',') {
                let d_lo = at + (d.len() - d.trim_start().len());
                at += d.len() + 1;
                let d = d.trim();
                let short = d.rsplit("::").next().unwrap_or(d).trim();
                if short.is_empty() || STD_DERIVES.contains(&short) || (!has_helpers && KNOWN_DERIVES.contains(&short)) {
                    continue;
                }
                match ctx.and_then(|c| derive_evidence(c, d_lo + d.len() - short.len(), shape, &needed)) {
                    Some(keys) => seen.extend(keys),
                    None if KNOWN_DERIVES.contains(&short) => {}
                    None => return false,
                }
            }
        } else if !(INERT_ATTRS.contains(&a.name) || a.name == "cfg_attr") {
            helpers.push(a);
        }
    }
    helpers.iter().all(|h| ctx.is_some_and(|c| !resolves_to_macro(c, h.at)) && helper_keys(h.name, h.args).iter().all(|k| seen.contains(k)))
}

/// The attributes in `doc[lo..hi]`, with those under `cfg_attr(predicate, …)` flattened; `None` on one it cannot read.
fn parse_attrs(doc: &str, lo: usize, hi: usize) -> Option<Vec<Attr<'_>>> {
    let masked = mask_comments_and_strings(&doc[lo..hi]);
    let mb = masked.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < mb.len() {
        if mb[i] != b'#' {
            i += 1;
            continue;
        }
        let k = i + 1 + (masked[i + 1..].len() - masked[i + 1..].trim_start().len());
        let after_b = skip_balanced(mb, k, b'[', b']')?;
        push_attr(doc, lo + k + 1, lo + after_b - 1, &mut out)?;
        i = after_b;
    }
    Some(out)
}

fn push_attr<'s>(doc: &'s str, lo: usize, hi: usize, out: &mut Vec<Attr<'s>>) -> Option<()> {
    let lo = lo + (doc[lo..hi].len() - doc[lo..hi].trim_start().len());
    let body = &doc[lo..hi];
    let path_len = body.find(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':')).unwrap_or(body.len());
    let name = body[..path_len].rsplit("::").next().unwrap_or("");
    if name.is_empty() {
        return None;
    }
    let args = body[path_len..].trim();
    let args_at = hi - body[path_len..].trim_start().len();
    if body[..path_len].contains("::") && !matches!(name, "derive") {
        // A path attribute (`#[serde_with::serde_as]`, `#[clippy::has_significant_drop]`): tool attributes are inert.
        if !body.starts_with("clippy::") && !body.starts_with("rustfmt::") {
            out.push(Attr { name, at: lo + path_len - name.len(), args, args_at });
        }
        return Some(());
    }
    if name == "cfg_attr" {
        let list = args.strip_prefix('(')?.strip_suffix(')')?;
        let parts = split_top_level_commas(list);
        if parts.len() < 2 {
            return None;
        }
        let mut at = args_at + 1;
        for (n, part) in parts.iter().enumerate() {
            let p_lo = at + doc[at..].find(part.trim())?;
            at = p_lo + part.trim().len();
            if n > 0 {
                push_attr(doc, p_lo, at, out)?;
            }
        }
        return Some(());
    }
    out.push(Attr { name, at: lo + path_len - name.len(), args, args_at });
    Some(())
}

/// The keys a helper attribute uses: each top-level argument up to `=` or `(`, or the bare name.
fn helper_keys(name: &str, args: &str) -> Vec<HelperKey> {
    let Some(list) = args.strip_prefix('(').and_then(|x| x.strip_suffix(')')) else { return vec![(name.to_string(), String::new())] };
    split_top_level_commas(list).iter().map(|p| p.split(['=', '(']).next().unwrap_or("").trim().to_string()).filter(|k| !k.is_empty()).map(|k| (name.to_string(), k)).collect()
}

/// Whether the name at `at` resolves, in the index, to a macro.
fn resolves_to_macro(c: &AttrCtx, at: usize) -> bool {
    let (line, col) = byte_to_line_col(c.doc, c.starts, at);
    c.ws.find_direct_references_at(c.file, line, col).is_some_and(|r| r.target_kind == DefKind::Macro)
}

/// The helper keys seen on structs of `shape` that compiled with the derive macro named at `at`, or `None` when the
/// index has no such struct. The search ends once every key in `needed` was seen (at the first such struct when none
/// is needed).
fn derive_evidence(c: &AttrCtx, at: usize, shape: Shape, needed: &HashSet<HelperKey>) -> Option<HashSet<HelperKey>> {
    let (line, col) = byte_to_line_col(c.doc, c.starts, at);
    let refs = c.ws.find_direct_references_at(c.file, line, col)?;
    // A derive named like its trait (`#[derive(Component)]`) resolves to the trait; its references hold the derive
    // sites too, which `struct_shape_after` picks out of the rest.
    if !matches!(refs.target_kind, DefKind::Macro | DefKind::Trait) {
        return None;
    }
    let mut sources: HashMap<std::path::PathBuf, Option<(String, Vec<usize>)>> = HashMap::new();
    let mut keys = HashSet::new();
    let mut found = false;
    for r in refs.references.iter().take(EVIDENCE_REFS) {
        let path = r.resolved_path();
        let entry = sources.entry(path.clone()).or_insert_with(|| {
            let s = std::fs::read_to_string(&path).ok()?;
            let st = std::iter::once(0).chain(s.match_indices('\n').map(|(i, _)| i + 1)).collect();
            Some((s, st))
        });
        let Some((src, starts)) = entry.as_ref() else { continue };
        let Some(&ls) = starts.get((r.span.start_line() as usize).wrapping_sub(1)) else { continue };
        let ref_at = src[ls..].char_indices().nth(r.span.start_col().saturating_sub(1) as usize).map_or(src.len(), |(b, _)| ls + b);
        let Some((s, attrs_end)) = struct_shape_after(src, ref_at) else { continue };
        if s != shape {
            continue;
        }
        found = true;
        let attr_lo = src[..ref_at].rfind("#[").unwrap_or(ref_at);
        for a in parse_attrs(src, attr_lo, attrs_end).unwrap_or_default() {
            if a.name != "derive" {
                keys.extend(helper_keys(a.name, a.args));
            }
        }
        if needed.is_subset(&keys) {
            break;
        }
    }
    found.then_some(keys)
}

/// The shape of the struct whose attributes contain `at`, and where those attributes end: the item after them must be
/// a `struct`.
pub(crate) fn struct_shape_after(src: &str, at: usize) -> Option<(Shape, usize)> {
    let mut end = (at + 4096).min(src.len());
    while !src.is_char_boundary(end) {
        end -= 1;
    }
    let masked = mask_comments_and_strings(&src[at..end]);
    let mb = masked.as_bytes();
    // Past the rest of this attribute and any that follow, to the item.
    let mut i = masked.find(']')? + 1;
    loop {
        while i < mb.len() && mb[i].is_ascii_whitespace() {
            i += 1;
        }
        if mb.get(i) != Some(&b'#') {
            break;
        }
        let k = i + 1 + masked[i + 1..].len() - masked[i + 1..].trim_start().len();
        i = skip_balanced(mb, k, b'[', b']')?;
    }
    let attrs_end = at + i;
    let rest = &masked[i..];
    let rest = match rest.strip_prefix("pub") {
        Some(r) if r.trim_start().starts_with('(') => {
            let r = r.trim_start();
            &r[skip_balanced(r.as_bytes(), 0, b'(', b')')?..]
        }
        Some(r) => r,
        None => rest,
    };
    let rest = rest.trim_start().strip_prefix("struct")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim_start().trim_start_matches(|c: char| c.is_alphanumeric() || c == '_').trim_start();
    let rest = if rest.starts_with('<') { &rest[skip_balanced(rest.as_bytes(), 0, b'<', b'>')?..] } else { rest };
    let rest = rest.trim_start();
    let shape = if rest.starts_with('(') {
        Shape::Tuple
    } else if rest.starts_with('{') || rest.starts_with("where") {
        Shape::Named
    } else {
        return None;
    };
    Some((shape, attrs_end))
}

#[cfg(test)]
mod tests {
    use super::{helper_keys, outer_attrs_allow_shape_change, struct_shape_after, Shape};

    #[test]
    fn test_reads_the_shape_of_the_struct_after_its_attributes() {
        let s = "#[derive(Component, Clone)]\n/// doc struct x\n#[reflect(Component)]\npub(crate) struct Name<T: Into<u8>>(T);";
        assert_eq!(struct_shape_after(s, 9).map(|x| x.0), Some(Shape::Tuple));
        assert_eq!(struct_shape_after(s, 9).map(|x| &s[..x.1]), Some(&s[..s.find("pub").unwrap()]));
        assert_eq!(struct_shape_after("#[derive(Bundle)]\nstruct B { a: u8 }", 9).map(|x| x.0), Some(Shape::Named));
        assert_eq!(struct_shape_after("#[derive(Bundle)]\nenum E { A }", 9), None);
    }

    #[test]
    fn test_helper_keys_name_each_argument() {
        let k: Vec<String> = helper_keys("reflect", "(Default, Component, from = \"x\")").into_iter().map(|(_, k)| k).collect();
        assert_eq!(k, ["Default", "Component", "from"]);
        assert_eq!(helper_keys("entities", ""), [("entities".to_string(), String::new())]);
    }

    #[test]
    fn test_without_the_index_only_builtin_attributes_pass() {
        let ok = "/// doc\n#[derive(Debug, Clone, serde::Serialize, Component)]\n#[expect(dead_code, reason = \"x\")]\n#[repr(C)]\n#[clippy::has_significant_drop]\n#[cfg_attr(test, derive(PartialEq))]\n";
        assert!(outer_attrs_allow_shape_change(ok, 0, ok.len(), None, Shape::Tuple));
        for bad in ["#[derive(Parser)]\n", "#[derive(Debug)]\n#[command(name = \"x\")]\n", "#[serde_as]\n", "#[derive(Component)]\n#[reflect(Component)]\n"] {
            assert!(!outer_attrs_allow_shape_change(bad, 0, bad.len(), None, Shape::Tuple), "{bad}");
        }
    }
}
