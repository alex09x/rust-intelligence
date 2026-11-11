/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compile-safe `type ... = fn(...) -> ...;` generator for free, inherent, and trait functions.

use super::action_generics_helpers::{
    assemble_kept_generics_and_where, bind_elided_lifetimes_outside_fn, bind_first_ref_lifetime, clean_param_ident, contains_inferred_underscore,
    count_elided_lifetimes_outside_fn, count_explicit_lifetimes_outside_fn, desugar_param_impl_trait, find_sibling_trait_defining_assoc, has_unbracketed_lifetime_nominal,
    impl_body_defines_assoc_type, parse_receiver_param, to_pascal_fn_alias, unique_ident, unique_lifetime, ReceiverLt,
};
use super::action_generics_sub::{
    materialize_elided_lifetimes, parse_generics_list, replace_self_tokens, split_top_level_colon, split_top_level_commas, strip_leading_attrs_keep_cfg, GenKind, GenericDecl,
};
use super::action_parse::{mask_comments_and_strings, EnclosingContext, ItemKindInfo, ParsedCursorItem};
use super::action_parse_decl::collapse_ws;
use super::action_parse_items::contains_word;
use ridx_format::WorkspaceIndex;
use std::collections::HashSet;

pub struct FnTypeAliasPlan {
    pub named_text: String,
    pub unnamed_text: String,
    pub insert_line0: u32,
    pub indent: String,
}

pub fn build_fn_type_aliases(doc_text: &str, masked_doc: &str, current_file: &str, item: &ParsedCursorItem, ws: &WorkspaceIndex) -> Option<FnTypeAliasPlan> {
    let ItemKindInfo::Fn { is_unsafe, extern_abi, generics_src, params_src, ret_src, where_src, .. } = &item.kind else {
        return None;
    };

    if let Some(ret) = ret_src {
        let rm = mask_comments_and_strings(ret);
        if contains_word(&rm, "impl") || rm.contains("use<") {
            return None;
        }
    }
    // The alias is not used yet: where dead code is denied it breaks the build.
    if super::action_crate_checks::crate_denies_dead_code(masked_doc, current_file) {
        return None;
    }

    let mut fresh_lifetimes: Vec<GenericDecl> = Vec::new();
    let mut lt_counter = 0usize;
    let mut enclosing_generics: Vec<GenericDecl> = Vec::new();
    let mut enclosing_where = String::new();
    let mut trait_self_decl: Option<GenericDecl> = None;
    let mut combined_cfgs: Vec<String> = Vec::new();
    let (insert_line0, target_indent);

    type TraitImplMeta = (String, usize, usize, String);
    let (self_sub, trait_impl_ctx): (Option<String>, Option<TraitImplMeta>) = match &item.enclosing {
        EnclosingContext::None => {
            insert_line0 = item.attr_start_line0;
            target_indent = item.indent.clone();
            (None, None)
        }
        EnclosingContext::ExternBlock => return None,
        EnclosingContext::InherentImpl { generics_src: ig, self_ty, where_src: iw, cfg_attrs, insert_line0: il, indent, .. } => {
            enclosing_generics = parse_generics_list(ig);
            enclosing_where = iw.clone();
            combined_cfgs.extend(cfg_attrs.iter().cloned());
            insert_line0 = *il;
            target_indent = indent.clone();
            let s = materialize_elided_lifetimes(self_ty, &mut lt_counter, &mut fresh_lifetimes);
            (Some(s), None)
        }
        EnclosingContext::TraitImpl { generics_src: ig, trait_ty, self_ty, where_src: iw, cfg_attrs, insert_line0: il, indent, body_open_byte, body_close_byte } => {
            enclosing_generics = parse_generics_list(ig);
            enclosing_where = iw.clone();
            combined_cfgs.extend(cfg_attrs.iter().cloned());
            insert_line0 = *il;
            target_indent = indent.clone();
            let s = materialize_elided_lifetimes(self_ty, &mut lt_counter, &mut fresh_lifetimes);
            let t = materialize_elided_lifetimes(trait_ty, &mut lt_counter, &mut fresh_lifetimes);
            (Some(s), Some((t, *body_open_byte, *body_close_byte, self_ty.clone())))
        }
        EnclosingContext::TraitDecl { generics_src: tg, trait_name, where_src: tw, cfg_attrs, insert_line0: il, indent } => {
            enclosing_generics = parse_generics_list(tg);
            enclosing_where = tw.clone();
            combined_cfgs.extend(cfg_attrs.iter().cloned());
            insert_line0 = *il;
            target_indent = indent.clone();
            let arg_list: Vec<String> = enclosing_generics.iter().map(|g| g.name.clone()).collect();
            let trait_bound = if arg_list.is_empty() { trait_name.clone() } else { format!("{trait_name}<{}>", arg_list.join(", ")) };
            let self_param_name = unique_ident("SelfTy", doc_text);
            trait_self_decl = Some(GenericDecl { name: self_param_name.clone(), kind: GenKind::Type, head_no_bounds: self_param_name.clone(), inline_bounds: vec![trait_bound] });
            (Some(self_param_name), None)
        }
    };
    combined_cfgs.extend(item.cfg_attrs.iter().cloned());

    let sub_self = |text: &str| -> Option<String> {
        let masked_t = mask_comments_and_strings(text);
        if !contains_word(&masked_t, "Self") {
            return Some(text.to_string());
        }
        let s_sub = self_sub.as_deref()?;
        if trait_self_decl.is_some() {
            // In `TraitDecl`, `Self::Assoc` becomes `SelfTy::Assoc`, which resolves supertrait assoc types too!
            return replace_self_tokens(text, s_sub, |assoc| Some(format!("{s_sub}::{assoc}")));
        }
        if let Some((ref tr_sub, b_open, b_close, ref raw_self)) = trait_impl_ctx {
            return replace_self_tokens(text, s_sub, |assoc| {
                if impl_body_defines_assoc_type(&masked_doc[b_open + 1..b_close], assoc) {
                    return Some(format!("<{s_sub} as {tr_sub}>::{assoc}"));
                }
                if let Some(other_tr) = find_sibling_trait_defining_assoc(doc_text, masked_doc, raw_self, assoc) {
                    return Some(format!("<{s_sub} as {other_tr}>::{assoc}"));
                }
                None
            });
        }
        // Inherent impl: `Self::Assoc` on concrete type without trait qualifier is invalid/ambiguous.
        replace_self_tokens(text, s_sub, |_assoc| None)
    };

    let mut fn_generics = parse_generics_list(generics_src);
    let mut known_ty_params: HashSet<String> =
        enclosing_generics.iter().chain(fn_generics.iter()).chain(trait_self_decl.iter()).filter(|g| g.kind == GenKind::Type).map(|g| g.name.clone()).collect();

    let mut impl_trait_decls: Vec<GenericDecl> = Vec::new();
    let mut named_params: Vec<String> = Vec::new();
    let mut unnamed_params: Vec<String> = Vec::new();
    let mut recv_info: Option<(usize, ReceiverLt)> = None;
    let mut other_params_elided_count = 0usize;
    let mut total_explicit_lt_count = 0usize;

    for raw_p in split_top_level_commas(params_src) {
        let (p_cfg, p_clean) = strip_leading_attrs_keep_cfg(&raw_p);
        if p_clean.is_empty() {
            continue;
        }
        if p_clean == "..." {
            if extern_abi.is_none() {
                return None;
            }
            named_params.push(format!("{p_cfg}..."));
            unnamed_params.push(format!("{p_cfg}..."));
            continue;
        }
        if let Some((recv_ty, rlt)) = parse_receiver_param(&p_clean, &sub_self)? {
            let idx = unnamed_params.len();
            total_explicit_lt_count += count_explicit_lifetimes_outside_fn(&recv_ty);
            recv_info = Some((idx, rlt));
            named_params.push(format!("{p_cfg}{recv_ty}"));
            unnamed_params.push(format!("{p_cfg}{recv_ty}"));
            continue;
        }
        let (pat_raw, Some(ty_raw)) = split_top_level_colon(&p_clean) else {
            return None;
        };
        let ty_desugared = desugar_param_impl_trait(ty_raw, doc_text, &mut impl_trait_decls, &sub_self)?;
        if contains_word(&mask_comments_and_strings(&ty_desugared), "impl") || contains_inferred_underscore(&ty_desugared) {
            return None;
        }
        other_params_elided_count += count_elided_lifetimes_outside_fn(&ty_desugared);
        total_explicit_lt_count += count_explicit_lifetimes_outside_fn(&ty_desugared);
        if let Some(pname) = clean_param_ident(pat_raw) {
            named_params.push(format!("{p_cfg}{pname}: {ty_desugared}"));
        } else {
            named_params.push(format!("{p_cfg}{ty_desugared}"));
        }
        unnamed_params.push(format!("{p_cfg}{ty_desugared}"));
    }
    for d in &impl_trait_decls {
        known_ty_params.insert(d.name.clone());
    }

    let mut ret_final = match ret_src {
        Some(r) => {
            let rs = sub_self(r)?;
            if contains_inferred_underscore(&rs) {
                return None;
            }
            Some(rs)
        }
        None => None,
    };

    let mut hrtb_prefix = String::new();
    if let Some(ref r_str) = ret_final {
        let params_hidden_lt: usize = unnamed_params.iter().filter(|p| has_unbracketed_lifetime_nominal(p, masked_doc, current_file, &known_ty_params, ws)).count();
        let ret_hidden_lt = has_unbracketed_lifetime_nominal(r_str, masked_doc, current_file, &known_ty_params, ws);
        let ret_elided = count_elided_lifetimes_outside_fn(r_str) > 0;
        if ret_hidden_lt {
            let recv_elided = match &recv_info {
                Some((r_idx, ReceiverLt::UnlifetimedRef)) => count_elided_lifetimes_outside_fn(&unnamed_params[*r_idx]),
                _ => 0,
            };
            if recv_elided + other_params_elided_count + total_explicit_lt_count + params_hidden_lt != 1 {
                return None;
            }
        }
        if ret_elided {
            if params_hidden_lt > 0 {
                return None;
            }
            match &recv_info {
                Some((r_idx, ReceiverLt::UnlifetimedRef)) => {
                    if count_elided_lifetimes_outside_fn(&unnamed_params[*r_idx]) != 1 {
                        return None;
                    }
                    if other_params_elided_count > 0 || total_explicit_lt_count > 0 {
                        let slt = unique_lifetime("'ridx_s", doc_text);
                        named_params[*r_idx] = bind_first_ref_lifetime(&named_params[*r_idx], &slt);
                        unnamed_params[*r_idx] = bind_first_ref_lifetime(&unnamed_params[*r_idx], &slt);
                        ret_final = Some(bind_elided_lifetimes_outside_fn(r_str, &slt));
                        hrtb_prefix = format!("for<{slt}> ");
                    }
                }
                Some((_, ReceiverLt::ExplicitLt(lt))) => {
                    ret_final = Some(bind_elided_lifetimes_outside_fn(r_str, lt));
                }
                _ => {
                    if other_params_elided_count != 1 {
                        return None;
                    }
                    if total_explicit_lt_count > 0 {
                        let recv_skip = recv_info.as_ref().map(|(idx, _)| *idx);
                        let p_idx = unnamed_params.iter().enumerate().find(|(idx, p)| Some(*idx) != recv_skip && count_elided_lifetimes_outside_fn(p) == 1).map(|(idx, _)| idx)?;
                        let slt = unique_lifetime("'ridx_s", doc_text);
                        named_params[p_idx] = bind_elided_lifetimes_outside_fn(&named_params[p_idx], &slt);
                        unnamed_params[p_idx] = bind_elided_lifetimes_outside_fn(&unnamed_params[p_idx], &slt);
                        ret_final = Some(bind_elided_lifetimes_outside_fn(r_str, &slt));
                        hrtb_prefix = format!("for<{slt}> ");
                    }
                }
            }
        }
    }

    let mut fn_mods = String::new();
    fn_mods.push_str(&hrtb_prefix);
    if *is_unsafe {
        fn_mods.push_str("unsafe ");
    }
    if let Some(abi) = extern_abi {
        fn_mods.push_str(abi);
        fn_mods.push(' ');
    }
    let ret_suffix = ret_final.as_ref().map(|r| format!(" -> {r}")).unwrap_or_default();
    let rhs_named = format!("{fn_mods}fn({}){ret_suffix}", named_params.join(", "));
    let rhs_unnamed = format!("{fn_mods}fn({}){ret_suffix}", unnamed_params.join(", "));

    // Substitute `Self` inside candidate inline bounds and `where` clauses.
    for g in enclosing_generics.iter_mut().chain(fn_generics.iter_mut()) {
        let mut new_bounds = Vec::with_capacity(g.inline_bounds.len());
        for b in &g.inline_bounds {
            if let Some(sb) = sub_self(b) {
                new_bounds.push(sb);
            }
        }
        g.inline_bounds = new_bounds;
    }

    let mut all_candidates: Vec<GenericDecl> = Vec::new();
    let mut seen_names = HashSet::new();
    for g in fresh_lifetimes.into_iter().chain(enclosing_generics).chain(trait_self_decl.clone()).chain(fn_generics).chain(impl_trait_decls) {
        if seen_names.insert(g.name.clone()) {
            all_candidates.push(g);
        }
    }

    let mut where_preds: Vec<String> = Vec::new();
    for w_src in [&enclosing_where, where_src] {
        if !w_src.is_empty() {
            for pred in split_top_level_commas(w_src) {
                if let Some(sp) = sub_self(&pred) {
                    where_preds.push(collapse_ws(&sp));
                }
            }
        }
    }

    let (gen_header, where_clause) = assemble_kept_generics_and_where(&rhs_unnamed, &all_candidates, &where_preds)?;
    if contains_word(&mask_comments_and_strings(&gen_header), "impl")
        || contains_word(&mask_comments_and_strings(&where_clause), "impl")
        || contains_word(&mask_comments_and_strings(&rhs_unnamed), "impl")
    {
        return None;
    }
    let alias_name = unique_ident(&to_pascal_fn_alias(&item.name), doc_text);

    let mut prefix_lines = String::new();
    combined_cfgs.dedup();
    for cfg in &combined_cfgs {
        prefix_lines.push_str(&target_indent);
        prefix_lines.push_str(cfg);
        prefix_lines.push('\n');
    }

    let named_text = format!("{prefix_lines}{target_indent}type {alias_name}{gen_header}{where_clause} = {rhs_named};\n\n");
    let unnamed_text = format!("{prefix_lines}{target_indent}type {alias_name}{gen_header}{where_clause} = {rhs_unnamed};\n\n");

    Some(FnTypeAliasPlan { named_text, unnamed_text, insert_line0, indent: target_indent })
}
