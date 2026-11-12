/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! A tuple struct's fields, read for `Convert to named struct`, and the named fields they become.

use super::action_adt::strip_field_vis;
use super::action_generics_sub::split_top_level_commas;
use super::action_parse::mask_comments_and_strings;
use super::action_parse_decl::collapse_ws;
use super::action_struct_conv_helpers::skip_field_attrs;

/// Tuple fields as `(attributes, visibility, type)`. A `#[cfg]` on a field shifts the positions of the fields after it
/// between builds, so such a field is refused.
pub(crate) fn parse_tuple_fields_clean(raw: &str) -> Option<Vec<(String, String, String)>> {
    let mf = mask_comments_and_strings(raw);
    if mf.contains('$') {
        return None;
    }
    let mut out = Vec::new();
    for entry in split_top_level_commas(raw) {
        let m = mask_comments_and_strings(&entry);
        if entry.trim().is_empty() {
            continue;
        }
        if m.len() != entry.len() {
            return None;
        }
        let lead = skip_field_attrs(&m)?;
        if m[..lead].contains("cfg") {
            return None;
        }
        let attrs = collapse_ws(entry[..lead].trim());
        let t = entry[lead..].trim();
        let rest = strip_field_vis(t);
        let vis = collapse_ws(t[..t.len() - rest.len()].trim());
        let fty = collapse_ws(rest);
        if fty.is_empty() {
            return None;
        }
        out.push((attrs, vis, fty));
    }
    Some(out)
}

/// `#[attr] pub fieldN: Type` for the tuple field at `idx`.
pub(crate) fn named_field_decl(idx: usize, (attrs, vis, ty): &(String, String, String)) -> String {
    let field = format!("field{}: {ty}", idx + 1);
    [attrs.as_str(), vis.as_str(), field.as_str()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::{named_field_decl, parse_tuple_fields_clean};

    #[test]
    fn a_field_keeps_its_attributes_but_not_a_cfg() {
        let f = parse_tuple_fields_clean("#[allow(dead_code)] pub [usize; 64], u8").unwrap();
        assert_eq!(named_field_decl(0, &f[0]), "#[allow(dead_code)] pub field1: [usize; 64]");
        assert_eq!(named_field_decl(1, &f[1]), "field2: u8");
        assert!(parse_tuple_fields_clean("#[cfg(test)] u8, u16").is_none());
    }
}
