/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Whether a new `impl From<field_ty> for Name` would overlap an existing conversion (E0119).

use super::action_generics_sub::{parse_generics_list, GenKind};
use super::action_parse::is_ident_byte;
use super::action_parse_items::{contains_word, skip_balanced};

/// A type with every path cut to its last segment, whitespace dropped (`std::sync::Arc<a::B>` → `Arc<B>`).
fn strip_paths(t: &str) -> String {
    let t: String = t.split_whitespace().collect();
    let mut out = String::new();
    let mut word = String::new();
    let flush = |out: &mut String, word: &mut String| {
        out.push_str(word.rsplit("::").next().unwrap_or(word));
        word.clear();
    };
    for c in t.chars() {
        if c.is_alphanumeric() || c == '_' || c == ':' {
            word.push(c);
        } else {
            flush(&mut out, &mut word);
            out.push(c);
        }
    }
    flush(&mut out, &mut word);
    out
}

/// Whether the (masked) text implements `From<A>` or `TryFrom<A>` for `type_name` with an `A` that a new
/// `From<field_ty>` would overlap: `A` is `field_ty` (paths aside) or names one of that impl's own type parameters.
/// A `TryFrom<field_ty>` overlaps too, as std derives it from the new `From`. Other conversions (`From<Name> for
/// usize`, `From<Other> for Name`) leave room for it.
pub(crate) fn doc_has_conflicting_conversion(masked: &str, type_name: &str, field_ty: &str) -> bool {
    let want = strip_paths(field_ty);
    let mb = masked.as_bytes();
    let mut i = 0usize;
    while let Some(at) = masked[i..].find("impl").map(|p| i + p) {
        i = at + 4;
        if (at > 0 && is_ident_byte(mb[at - 1])) || mb.get(at + 4).is_some_and(|c| is_ident_byte(*c)) {
            continue;
        }
        let Some(bo) = masked[at..].find('{').map(|p| at + p) else { break };
        let hdr = &masked[at + 4..bo];
        let (gens, rest) = match hdr.trim_start().strip_prefix('<') {
            Some(_) => {
                let start = at + 4 + (hdr.len() - hdr.trim_start().len());
                let Some(end) = skip_balanced(mb, start, b'<', b'>') else { continue };
                (&masked[start + 1..end - 1], &masked[end..bo])
            }
            None => ("", hdr),
        };
        let rest = rest.split_whitespace().collect::<Vec<_>>().join(" ");
        let Some((trait_part, self_part)) = rest.split_once(" for ") else { continue };
        let self_base = self_part.split(['<', ' ']).find(|s| !s.is_empty()).unwrap_or("");
        if self_base.rsplit("::").next() != Some(type_name) {
            continue;
        }
        let trait_part = trait_part.trim();
        let name = trait_part.split('<').next().unwrap_or("").trim().rsplit("::").next().unwrap_or("");
        if !matches!(name, "From" | "TryFrom") {
            continue;
        }
        let Some(arg) = trait_part.find('<').and_then(|p| trait_part[p + 1..].strip_suffix('>')) else { continue };
        if strip_paths(arg) == want || parse_generics_list(gens).iter().any(|g| g.kind == GenKind::Type && contains_word(arg, &g.name)) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::doc_has_conflicting_conversion;

    #[test]
    fn test_only_an_overlapping_conversion_conflicts() {
        let other = "impl From<ComponentId> for usize { fn from(c: ComponentId) -> usize { c.0 } }\nimpl From<u8> for ComponentId { fn from(v: u8) -> Self { Self(v as usize) } }";
        assert!(!doc_has_conflicting_conversion(other, "ComponentId", "usize"));
        assert!(doc_has_conflicting_conversion("impl core::convert::From<usize> for ComponentId {}", "ComponentId", "usize"));
        assert!(doc_has_conflicting_conversion("impl TryFrom<std::sync::Arc<a::B>>\n for Name { }", "Name", "Arc<B>"));
        assert!(doc_has_conflicting_conversion("impl<T: Into<usize>> From<T> for ComponentId {}", "ComponentId", "usize"));
        assert!(!doc_has_conflicting_conversion("impl From<usize> for OtherId {}", "ComponentId", "usize"));
    }
}
