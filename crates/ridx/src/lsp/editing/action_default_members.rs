/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Index-backed, compile-safe `Implement default members` code action builder.

use super::action_default_extract::{extract_default_fn_members, DefaultFnMember};
use super::action_default_qualify::{path_heads, qualify_member_paths, shadows_prelude_name, MemberOrigin};
use super::action_default_resolve::{resolve_trait_definition, split_trait_path_and_args, trait_generic_defaults};
use super::action_default_safety::is_rust_kw_or_prelude;
use super::action_default_scope::{collect_module_scope_idents, enclosing_module_span, has_top_level_macro_call, is_default_fn_compile_safe};
use super::action_default_ufcs::{alpha_rename_colliding_fn_generics, collect_trait_fn_receivers, parse_fn_receiver, qualify_trait_self_calls};
use super::action_generics_sub::{collect_tokens_and_lifetimes, parse_generics_list, GenKind, GenericDecl};
use super::action_parse::{byte_to_utf16_pos, is_ident_byte, mask_comments_and_strings, EnclosingContext, ItemKindInfo, ParsedCursorItem};
use super::action_parse_decl::{extract_angle_generics, next_ident_token};
use super::action_parse_items::skip_balanced;
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, WorkspaceIndex};
use std::collections::HashSet;

#[allow(clippy::too_many_arguments)]
pub fn build_implement_default_members(
    doc_text: &str,
    masked_doc: &str,
    starts: &[usize],
    current_file: &str,
    item: &ParsedCursorItem,
    ws: &WorkspaceIndex,
    overlay_mgr: &OverlayManager,
) -> Option<(u32, u32, String)> {
    if !matches!(item.kind, ItemKindInfo::Fn { .. } | ItemKindInfo::AssocOrTypeAlias { .. }) {
        return None;
    }
    let EnclosingContext::TraitImpl { generics_src, trait_ty, self_ty, where_src, indent, body_open_byte, body_close_byte, .. } = &item.enclosing else {
        return None;
    };

    let impl_body_masked = &masked_doc[body_open_byte + 1..*body_close_byte];
    if has_top_level_macro_call(impl_body_masked) {
        return None;
    }
    let implemented_fns = collect_top_level_fn_names(impl_body_masked);

    let (trait_path, mut trait_args) = split_trait_path_and_args(trait_ty)?;
    let simple_trait = trait_path.rsplit("::").next()?.trim();
    let simple_clean = simple_trait.strip_prefix("r#").unwrap_or(simple_trait);
    if simple_clean.is_empty() {
        return None;
    }

    let resolved_trait = resolve_trait_definition(doc_text, masked_doc, current_file, *body_open_byte, trait_path, simple_clean, ws, overlay_mgr)?;
    // Parameters the impl leaves out take the trait's defaults (`PartialOrd` is `PartialOrd<Rhs = Self>`).
    if trait_args.len() < resolved_trait.generics.len() {
        let defaults = trait_generic_defaults(&resolved_trait.src[..resolved_trait.body_open], simple_clean);
        for d in defaults.iter().skip(trait_args.len()).take(resolved_trait.generics.len() - trait_args.len()) {
            trait_args.push(d.clone()?);
        }
    }
    if resolved_trait.generics.len() != trait_args.len() {
        return None;
    }
    if resolved_trait.generics.iter().any(|g| g.kind == GenKind::Const) {
        return None;
    }

    let trait_body_src = &resolved_trait.src[resolved_trait.body_open + 1..resolved_trait.body_close];
    let trait_body_masked = &resolved_trait.masked[resolved_trait.body_open + 1..resolved_trait.body_close];
    if has_top_level_macro_call(trait_body_masked) {
        return None;
    }

    let all_trait_fns = collect_top_level_fn_names(trait_body_masked);
    let trait_receivers = collect_trait_fn_receivers(trait_body_src, trait_body_masked);
    let compiled = |name: &str| resolved_trait.def_path.as_ref().map(|p| ws.find_definition_by_path(&format!("{p}::{name}")).is_some());
    let default_fns = extract_default_fn_members(trait_body_src, trait_body_masked, resolved_trait.same_crate, &compiled)?;
    let missing: Vec<&DefaultFnMember> = default_fns.iter().filter(|m| !implemented_fns.contains(&m.name)).collect();
    if missing.is_empty() {
        return None;
    }

    let colliding_methods = collect_colliding_self_methods(masked_doc, *body_open_byte, *body_close_byte, self_ty, &all_trait_fns, ws);
    let (mod_lo, mod_hi) = enclosing_module_span(masked_doc, *body_open_byte);
    let doc_scope = collect_module_scope_idents(&masked_doc[mod_lo..mod_hi]);
    let impl_non_lt_gens: HashSet<String> = parse_generics_list(generics_src).into_iter().filter(|g| g.kind != GenKind::Lifetime).map(|g| g.name).collect();
    let mut impl_hdr_idents = collect_tokens_and_lifetimes(generics_src);
    impl_hdr_idents.extend(collect_tokens_and_lifetimes(self_ty));
    impl_hdr_idents.extend(collect_tokens_and_lifetimes(trait_ty));
    impl_hdr_idents.extend(collect_tokens_and_lifetimes(where_src));

    // A standard library trait's default bodies call public standard library methods (`is_some_and`): one the index
    // holds under `core`/`alloc`/`std` is callable from the impl too.
    let std_trait = resolved_trait.file.as_deref().is_some_and(|f| f.contains("/library/"));
    let std_method = |name: &str| {
        std_trait
            && ws.exact_symbols_by_name(name).iter().any(|s| {
                matches!(s.kind, DefKind::AssocFn | DefKind::Fn)
                    && (s.path.starts_with("core::")
                        || s.path.starts_with("alloc::")
                        || s.path.starts_with("std::")
                        || s.path.starts_with("<core::")
                        || s.path.starts_with("<alloc::")
                        || s.path.starts_with("<std::"))
            })
    };
    let no_scope: HashSet<String> = HashSet::new();
    let member_indent = format!("{indent}    ");
    let mut rendered_members = Vec::with_capacity(missing.len());

    // The crate the impl lives in: names from it are written `crate::…`.
    let dest_crate = ws.reader_indices_for_file(current_file).first().map(|&i| ws.readers()[i].crate_name().replace('-', "_")).unwrap_or_default();
    let trait_gen_names: HashSet<&str> = resolved_trait.generics.iter().map(|g| g.name.as_str()).collect();
    // Only names that start a path in the impl header are in scope there (`E: de::Error` does not bring `Error`).
    let mut allowed_idents = path_heads(&format!("{generics_src} {self_ty} {trait_ty} {where_src}"));
    allowed_idents.extend(impl_hdr_idents.iter().filter(|t| t.starts_with('\'')).cloned());
    let hdr_heads = allowed_idents.clone();

    for m in missing {
        // Names the impl's module does not see are written as the paths the index resolved them to.
        let (fn_src, fn_masked, params_masked) = match (&resolved_trait.file, resolved_trait.same_module) {
            (Some(trait_file), false) => {
                let origin = MemberOrigin { trait_src: &resolved_trait.src, trait_file, start: resolved_trait.body_open + 1 + m.start };
                // A standard library name the impl's module also defines (clap's own `FlatMap`) means another item
                // there: a standard library trait's names are written as full paths unless the prelude has them.
                let visible = |t: &str| {
                    t == "Self"
                        || is_rust_kw_or_prelude(t)
                        || (!std_trait && (doc_scope.contains(t) || hdr_heads.contains(t)))
                        || m.fn_gen_names.contains(t)
                        || trait_gen_names.contains(t)
                };
                let (src, roots) = qualify_member_paths(&m.fn_src, &origin, ws, &dest_crate, &visible);
                // A standard library body names only the standard library; a name the index placed elsewhere (another
                // crate's `Map`) is a different item.
                if std_trait && roots.iter().any(|r| !matches!(r.as_str(), "core" | "alloc" | "std")) {
                    continue;
                }
                allowed_idents.extend(roots);
                let masked = mask_comments_and_strings(&src);
                let params = extract_first_paren_masked(&masked).unwrap_or_else(|| m.params_masked.clone());
                (src, masked, params)
            }
            _ => (m.fn_src.clone(), m.fn_masked.clone(), m.params_masked.clone()),
        };
        let (sub_src, _, sub_params) = substitute_trait_generics(&fn_src, &fn_masked, &params_masked, &resolved_trait.generics, &trait_args, &m.fn_gen_names);
        // A standard library trait has members whose bodies use the library's internals: those are left out and the
        // others implemented; any other trait's members are implemented all or not at all.
        let Some((alpha_src, eff_gen_names)) = alpha_rename_colliding_fn_generics(&sub_src, &m.fn_gens, &m.fn_gen_names, &impl_non_lt_gens, doc_text, replace_ident_or_lt) else {
            if std_trait {
                continue;
            }
            return None;
        };
        let alpha_masked = mask_comments_and_strings(&alpha_src);
        // A prelude name the body relies on (`Result`, `Option`, …) that the impl's module redefines or imports
        // (`type Result<T> = …`) would change meaning once copied.
        if !resolved_trait.same_module && shadows_prelude_name(&alpha_masked, &doc_scope) {
            if std_trait {
                continue;
            }
            return None;
        }
        let alpha_params = extract_first_paren_masked(&alpha_masked).unwrap_or(sub_params);
        if !is_default_fn_compile_safe(
            &alpha_masked,
            &alpha_params,
            &eff_gen_names,
            resolved_trait.same_module,
            resolved_trait.same_crate,
            &all_trait_fns,
            // The impl's module names nothing a standard library body means (it may define a different `FlatMap`).
            if std_trait { &no_scope } else { &doc_scope },
            &allowed_idents,
            &std_method,
        ) {
            if std_trait {
                continue;
            }
            return None;
        }
        let caller_recv = parse_fn_receiver(&alpha_params);
        let Some(qualified_src) = qualify_trait_self_calls(&alpha_src, trait_ty, caller_recv, &trait_receivers, &colliding_methods) else {
            if std_trait {
                continue;
            }
            return None;
        };
        let reindented = reindent_member_block(&qualified_src, &member_indent);
        if m.cfg_attrs.is_empty() {
            rendered_members.push(reindented);
        } else {
            let mut with_cfg = String::new();
            for c in &m.cfg_attrs {
                with_cfg.push_str(&member_indent);
                with_cfg.push_str(c);
                with_cfg.push('\n');
            }
            with_cfg.push_str(&reindented);
            rendered_members.push(with_cfg);
        }
    }

    if rendered_members.is_empty() {
        return None;
    }
    let before_close = &doc_text[..*body_close_byte];
    let close_line_start = before_close.rfind('\n').map(|p| p + 1).unwrap_or(0);
    let only_ws_before_close = doc_text[close_line_start..*body_close_byte].chars().all(|c| c == ' ' || c == '\t');
    let (ins_byte, new_text) = if only_ws_before_close && close_line_start > *body_open_byte {
        (close_line_start, format!("\n{}\n", rendered_members.join("\n\n")))
    } else {
        (*body_close_byte, format!("\n{}\n{indent}", rendered_members.join("\n\n")))
    };
    let (line0, col_u16) = byte_to_utf16_pos(doc_text, starts, ins_byte);
    Some((line0, col_u16, new_text))
}

fn collect_top_level_fn_names(masked_body: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let b = masked_body.as_bytes();
    let (mut brace, mut paren, mut bracket) = (0i32, 0i32, 0i32);
    let mut i = 0usize;
    while i < b.len() {
        match b[i] {
            b'{' => brace += 1,
            b'}' => brace = (brace - 1).max(0),
            b'(' => paren += 1,
            b')' => paren = (paren - 1).max(0),
            b'[' => bracket += 1,
            b']' => bracket = (bracket - 1).max(0),
            _ if brace == 0 && paren == 0 && bracket == 0 && i + 2 <= b.len() && &b[i..i + 2] == b"fn" => {
                let prev_ok = i == 0 || !is_ident_byte(b[i - 1]);
                let next_ok = i + 2 == b.len() || !is_ident_byte(b[i + 2]);
                if prev_ok && next_ok {
                    if let Some((n, after_n)) = next_ident_token(masked_body, i + 2) {
                        out.insert(n.strip_prefix("r#").unwrap_or(n).to_string());
                        i = after_n;
                        continue;
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    out
}

fn collect_colliding_self_methods(masked_doc: &str, impl_open: usize, impl_close: usize, self_ty: &str, all_trait_fns: &HashSet<String>, ws: &WorkspaceIndex) -> HashSet<String> {
    let mut out = HashSet::new();
    for slice in [&masked_doc[..impl_open], &masked_doc[(impl_close + 1).min(masked_doc.len())..]] {
        let b = slice.as_bytes();
        let mut i = 0usize;
        while i + 2 <= b.len() {
            if &b[i..i + 2] == b"fn" && (i == 0 || !is_ident_byte(b[i - 1])) && (i + 2 == b.len() || !is_ident_byte(b[i + 2])) {
                if let Some((n, after_n)) = next_ident_token(slice, i + 2) {
                    let clean = n.strip_prefix("r#").unwrap_or(n);
                    if all_trait_fns.contains(clean) {
                        out.insert(clean.to_string());
                    }
                    i = after_n;
                    continue;
                }
            }
            i += 1;
        }
    }
    let self_base = self_ty.split('<').next().unwrap_or(self_ty).trim().trim_start_matches('&').trim_start_matches("mut ").trim();
    let self_leaf_raw = self_base.rsplit("::").next().unwrap_or(self_base).trim();
    let self_leaf = self_leaf_raw.strip_prefix("r#").unwrap_or(self_leaf_raw);
    if !self_leaf.is_empty() && self_leaf.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        let suffix_prefix = format!("::{self_leaf}::");
        for m in all_trait_fns {
            if out.contains(m) {
                continue;
            }
            let target_sfx = format!("{suffix_prefix}{m}");
            let exact_pair = format!("{self_leaf}::{m}");
            if ws.exact_symbols_by_name(m).into_iter().any(|s| s.path == exact_pair || s.path.ends_with(&target_sfx)) {
                out.insert(m.clone());
            }
        }
    }
    out
}

fn extract_first_paren_masked(fn_masked: &str) -> Option<String> {
    let mb = fn_masked.as_bytes();
    let mut i = 0usize;
    while i + 2 <= mb.len() {
        if &mb[i..i + 2] == b"fn" && (i == 0 || !is_ident_byte(mb[i - 1])) && (i + 2 == mb.len() || !is_ident_byte(mb[i + 2])) {
            let (_, name_end) = next_ident_token(fn_masked, i + 2)?;
            let (_, after_g) = extract_angle_generics(fn_masked, fn_masked, name_end);
            let mut p = after_g;
            while p < mb.len() && mb[p].is_ascii_whitespace() {
                p += 1;
            }
            let rp = skip_balanced(mb, p, b'(', b')')?;
            return Some(fn_masked[p + 1..rp - 1].to_string());
        }
        i += 1;
    }
    None
}

fn substitute_trait_generics(
    fn_src: &str,
    fn_masked: &str,
    params_masked: &str,
    trait_gens: &[GenericDecl],
    trait_args: &[String],
    shadowed: &HashSet<String>,
) -> (String, String, String) {
    if trait_gens.is_empty() {
        return (fn_src.to_string(), fn_masked.to_string(), params_masked.to_string());
    }
    let mut cur_src = fn_src.to_string();
    for (g, arg) in trait_gens.iter().zip(trait_args.iter()) {
        if shadowed.contains(&g.name) {
            continue;
        }
        cur_src = replace_ident_or_lt(&cur_src, &g.name, arg);
    }
    let new_masked = mask_comments_and_strings(&cur_src);
    let new_params = if let Some(lp) = new_masked.find('(') {
        skip_balanced(new_masked.as_bytes(), lp, b'(', b')').map(|rp| new_masked[lp + 1..rp - 1].to_string()).unwrap_or_default()
    } else {
        params_masked.to_string()
    };
    (cur_src, new_masked, new_params)
}

fn replace_ident_or_lt(src: &str, target: &str, replacement: &str) -> String {
    let masked = mask_comments_and_strings(src);
    let mb = masked.as_bytes();
    let tb = target.as_bytes();
    let mut out = String::with_capacity(src.len() + 16);
    let mut i = 0usize;
    while i < mb.len() {
        if i + tb.len() <= mb.len() && &mb[i..i + tb.len()] == tb {
            let prev_ok = i == 0 || (!is_ident_byte(mb[i - 1]) && mb[i - 1] != b'.' && !(i >= 2 && &mb[i - 2..i] == b"::"));
            let next_ok = i + tb.len() == mb.len() || !is_ident_byte(mb[i + tb.len()]);
            if prev_ok && next_ok {
                out.push_str(replacement);
                i += tb.len();
                continue;
            }
        }
        let ch = src[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn reindent_member_block(fn_src: &str, member_indent: &str) -> String {
    let lines: Vec<&str> = fn_src.lines().collect();
    if lines.is_empty() {
        return String::new();
    }
    let base_indent = lines.iter().skip(1).filter(|l| !l.trim().is_empty()).map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
    let mut out = Vec::with_capacity(lines.len());
    out.push(format!("{member_indent}{}", lines[0].trim()));
    for l in lines.iter().skip(1) {
        if l.trim().is_empty() {
            out.push(String::new());
        } else {
            let strip = base_indent.min(l.len() - l.trim_start().len());
            out.push(format!("{member_indent}{}", &l[strip..]));
        }
    }
    out.join("\n")
}
