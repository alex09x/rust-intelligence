/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Generic parameter bounds resolution for `T::` completion.
//!
//! Resolves generic parameters in scope (from enclosing functions, impl blocks, or traits)
//! to candidate owners based on their declared trait bounds and associated types.

pub mod parse;
pub mod scan;
pub mod types;

pub use parse::*;
pub use scan::*;
pub use types::*;

use crate::lsp::intelligence::self_completion::{
    clean_trait_name, collect_trait_and_supertraits,
};
use ridx_format::{DefKind, WorkspaceIndex};
use std::collections::HashSet;

/// Resolves owner candidates for `T::` (or `T::Assoc::`) where `T` is a generic parameter in scope.
pub fn resolve_generic_param_owners(
    ws: &WorkspaceIndex,
    file: &str,
    line_1based: u32,
    line_slice: &str,
    overlay_text: Option<&str>,
    qual: &str,
) -> Option<Vec<String>> {
    let text = if let Some(t) = overlay_text {
        t.to_string()
    } else {
        std::fs::read_to_string(file).ok()?
    };

    let generics = find_generic_params_in_scope(&text, line_1based, line_slice);
    if generics.is_empty() {
        return None;
    }

    let mut segs = qual.split("::");
    let first_seg = segs.next()?;
    let param = generics.iter().find(|p| p.name == first_seg)?;
    if param.bounds.is_empty() {
        return None;
    }

    let mut owners = Vec::new();
    let mut seen = HashSet::new();

    let remaining_segs: Vec<&str> = segs.collect();
    if remaining_segs.is_empty() {
        // Simple case: `T::` -> candidates are the trait bounds on `T`
        for bound in &param.bounds {
            collect_trait_and_supertraits(ws, file, bound, &mut owners, &mut seen, 0);
        }
    } else {
        // Multi-segment case: `T::Assoc::...`
        let mut cur_trait_names = param.bounds.clone();
        for seg in remaining_segs {
            let mut next_traits = Vec::new();
            for tname in &cur_trait_names {
                let def_opt = ws.resolve_qualifier(file, tname);
                let target_path = def_opt.as_ref().map(|d| d.path).unwrap_or(tname.as_str());
                for def in ws.find_definitions_by_owner(target_path) {
                    if def.name == seg && def.kind == DefKind::AssocTy {
                        // Inspect associated type bound: `type Assoc: Bound;`
                        if let Ok(src) = std::fs::read_to_string(def.file) {
                            let line_idx = def.start_line.saturating_sub(1) as usize;
                            if let Some(line) = src.lines().nth(line_idx) {
                                if let Some(colon_pos) = line.find(':') {
                                    let after_colon = line[colon_pos + 1..].trim();
                                    let bound_part = after_colon
                                        .split(&[';', '=', '+', ' '][..])
                                        .next()
                                        .unwrap_or("");
                                    let clean_bound = clean_trait_name(bound_part);
                                    if !clean_bound.is_empty()
                                        && !next_traits.contains(&clean_bound)
                                    {
                                        next_traits.push(clean_bound);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            cur_trait_names = next_traits;
            if cur_trait_names.is_empty() {
                break;
            }
        }

        for tname in &cur_trait_names {
            collect_trait_and_supertraits(ws, file, tname, &mut owners, &mut seen, 0);
        }
    }

    if owners.is_empty() {
        None
    } else {
        Some(owners)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_generic_params_inline_bounds() {
        let header = "pub fn get_mut<F: WorldEntityFetch>(&mut self, entities: F) -> Result<F::DeferredMut, Error>";
        let params = parse_generic_params_from_header(header);
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "F");
        assert_eq!(params[0].bounds, vec!["WorldEntityFetch".to_string()]);
    }

    #[test]
    fn test_parse_generic_params_where_clause() {
        let header = r#"
impl<Marker, In, Out, F> System for FunctionSystem<Marker, In, Out, F>
where
    Marker: 'static,
    In: SystemInput + 'static,
    Out: 'static,
    F: SystemParamFunction<Marker, In: FromInput<In>, Out: IntoResult<Out>>,
"#;
        let params = parse_generic_params_from_header(header);
        let f_param = params.iter().find(|p| p.name == "F").unwrap();
        assert_eq!(f_param.bounds, vec!["SystemParamFunction".to_string()]);

        let in_param = params.iter().find(|p| p.name == "In").unwrap();
        assert_eq!(in_param.bounds, vec!["SystemInput".to_string()]);

        let marker_param = params.iter().find(|p| p.name == "Marker").unwrap();
        assert!(marker_param.bounds.is_empty());
    }

    #[test]
    fn test_find_generic_params_nested_scopes() {
        let src = r#"
impl<A, B> MyStruct<A, B>
where
    A: System,
    B: System,
{
    fn combine<F: WorldEntityFetch>(&mut self) {
        let x = F::
    }
}
"#;
        let params = find_generic_params_in_scope(src, 8, "        let x = F::");
        let names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"A"));
        assert!(names.contains(&"B"));
        assert!(names.contains(&"F"));

        let f = params.iter().find(|p| p.name == "F").unwrap();
        assert_eq!(f.bounds, vec!["WorldEntityFetch".to_string()]);

        let a = params.iter().find(|p| p.name == "A").unwrap();
        assert_eq!(a.bounds, vec!["System".to_string()]);
    }

    #[test]
    fn test_find_generic_params_in_signature() {
        let src = r#"
pub fn get_mut<F: WorldEntityFetch>(
    &mut self,
    entities: F,
) -> Result<F::DeferredMut<'_>, EntityMutableFetchError> {
"#;
        let params = find_generic_params_in_scope(src, 5, ") -> Result<F::");
        let f = params.iter().find(|p| p.name == "F").unwrap();
        assert_eq!(f.bounds, vec!["WorldEntityFetch".to_string()]);
    }
}
