/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Semantic rename resolution across unindexed Cargo targets discovered via `cargo metadata`.

pub use super::rename_alias_mod::{build_workspace_edit_json, detect_module_file_rename, expand_generic_param_edits, try_prepare_rename_use_as_alias, try_rename_use_as_alias};
use super::rename_alias_mod::{extract_impl_header_owners, has_call_syntax, has_qual_in_code};

use super::ident::is_ident_continue;
use crate::lsp::protocol::{path_to_uri, PositionConverter};
use crate::overlay::mapping::is_byte_in_string_or_comment;
use ridx_format::rename_cache::RenameWorkspaceCache;
use ridx_format::rename_parse::{has_self_receiver, ParsedUnindexedTarget};
use ridx_format::{is_prelude_trait, DefKind, LocationReferences, RenameRefusal, ResolvedDefinition, TargetKind, WorkspaceIndex};
use serde_json::{json, Value};
use std::collections::HashMap;

fn imported_factory_returns_owner(ws: &WorkspaceIndex, cache: &RenameWorkspaceCache, ut: &ParsedUnindexedTarget, owner_short: &str, owner_crate: &str) -> bool {
    let Some(cands) = cache.factory_fns_by_return_short.get(owner_short) else {
        return false;
    };
    for cand in cands {
        let rc = cand.reader_crate.as_str();
        let fn_name = cand.fn_name.as_str();
        if !ut.idents.contains(fn_name)
            || !cache.cargo_targets.crate_can_reexport(rc, owner_crate)
            || !ut.dep_crates.iter().any(|dc| cache.cargo_targets.crate_can_reexport(dc, rc))
        {
            continue;
        }
        match cand.parent_short.as_deref() {
            None => {
                let qual = has_qual_in_code(cache, ut, rc, &format!("::{fn_name}(")) || has_qual_in_code(cache, ut, rc, &format!("::{fn_name}::<"));
                if (has_call_syntax(&ut.content, "", fn_name) && is_symbol_imported(ws, cache, ut, fn_name, rc)) || qual {
                    return true;
                }
            }
            Some(fty_short) => {
                let assoc_call = !fty_short.is_empty() && has_call_syntax(&ut.content, &format!("{fty_short}::"), fn_name);
                let dot_call = cand.dot_eligible && has_call_syntax(&ut.content, ".", fn_name);
                if !fty_short.is_empty()
                    && fty_short != owner_short
                    && ut.idents.contains(fty_short)
                    && (assoc_call || dot_call)
                    && (is_symbol_imported(ws, cache, ut, fty_short, rc) || has_qual_in_code(cache, ut, rc, &format!("::{fty_short}")))
                {
                    return true;
                }
                if dot_call && cand.all_shard_assoc_return_owner {
                    return true;
                }
            }
        }
    }
    false
}

fn local_type_shadowed(ut: &ParsedUnindexedTarget, name: &str) -> bool {
    [DefKind::Struct, DefKind::Enum, DefKind::Union, DefKind::Trait, DefKind::TyAlias].into_iter().any(|k| ut.local_defs.contains(&(k, name.to_string())))
}

fn has_non_reexport_at_import(ws: &WorkspaceIndex, imp: &str, root_seg: &str, expected_crate: &str, sym: &str) -> bool {
    if root_seg == expected_crate {
        return false;
    }
    let check_path = |p: &str| {
        ws.find_definition_by_path(p).is_some_and(|d| {
            !matches!(d.kind, DefKind::Use | DefKind::Macro)
                && d.reader_idx.and_then(|ri| ws.readers().get(ri)).is_some_and(|r| r.crate_name() == root_seg && r.find_definition_by_path(p).is_some_and(|e| !e.is_reexport()))
        })
    };
    check_path(imp) || check_path(&format!("{root_seg}::{sym}"))
}

fn is_symbol_imported(ws: &WorkspaceIndex, cache: &RenameWorkspaceCache, ut: &ParsedUnindexedTarget, sym: &str, expected_crate: &str) -> bool {
    if let Some(imps) = ut.explicit_imports.get(sym) {
        if imps.iter().any(|imp| {
            let root = imp.split("::").next().unwrap_or("");
            ut.dep_crates.contains(root) && cache.cargo_targets.crate_can_reexport(root, expected_crate) && !has_non_reexport_at_import(ws, imp, root, expected_crate, sym)
        }) {
            return true;
        }
    }
    ut.glob_imports.iter().any(|glob_mod| {
        let root = glob_mod.split("::").next().unwrap_or("");
        if !ut.dep_crates.contains(root) || !cache.cargo_targets.crate_can_reexport(root, expected_crate) {
            return false;
        }
        let direct_path = format!("{glob_mod}::{sym}");
        let direct = ws.find_definition_by_path(&direct_path).is_some_and(|d| {
            d.reader_idx.and_then(|ri| ws.readers().get(ri)).map(|r| r.crate_name()) == Some(expected_crate)
                || !has_non_reexport_at_import(ws, &direct_path, root, expected_crate, sym)
        });
        let via_prelude = glob_mod.ends_with("::prelude")
            && (ws.find_definition_by_path(&format!("{expected_crate}::prelude::{sym}")).is_some()
                || (!ut.explicit_imports.contains_key(sym)
                    && !local_type_shadowed(ut, sym)
                    && ut.idents.contains(sym)
                    && ws.find_definition_by_path(&format!("{expected_crate}::prelude")).is_some()
                    && cache.type_def_crates_by_name.get(sym).is_some_and(|cs| cs.contains(expected_crate) && cs.iter().filter(|c| ut.dep_crates.contains(*c)).count() == 1)));
        direct || via_prelude
    })
}

fn is_trait_in_scope(ws: &WorkspaceIndex, cache: &RenameWorkspaceCache, ut: &ParsedUnindexedTarget, trait_short: &str, trait_crate: &str) -> bool {
    is_prelude_trait(trait_short)
        || is_symbol_imported(ws, cache, ut, trait_short, trait_crate)
        || ut.anon_trait_imports.iter().any(|p| p.rsplit("::").next() == Some(trait_short) && ut.dep_crates.contains(p.split("::").next().unwrap_or("")))
}

fn is_inherent_owner_active(ws: &WorkspaceIndex, cache: &RenameWorkspaceCache, ut: &ParsedUnindexedTarget, owner_short: &str, owner_crate: &str) -> bool {
    if owner_short.is_empty() || !ut.dep_crates.contains(owner_crate) || local_type_shadowed(ut, owner_short) {
        return false;
    }
    if ut.idents.contains(owner_short) && (is_symbol_imported(ws, cache, ut, owner_short, owner_crate) || has_qual_in_code(cache, ut, owner_crate, &format!("::{owner_short}"))) {
        return true;
    }
    imported_factory_returns_owner(ws, cache, ut, owner_short, owner_crate)
}

fn is_owner_explicit_in_ut(ws: &WorkspaceIndex, cache: &RenameWorkspaceCache, ut: &ParsedUnindexedTarget, owner_short: &str, owner_crate: &str, is_trait: bool) -> bool {
    if is_trait {
        !local_type_shadowed(ut, owner_short)
            && ut.dep_crates.contains(owner_crate)
            && (ut.explicit_imports.contains_key(owner_short)
                || ut.anon_trait_imports.iter().any(|p| p.rsplit("::").next() == Some(owner_short))
                || (ut.idents.contains(owner_short) && is_trait_in_scope(ws, cache, ut, owner_short, owner_crate)))
    } else {
        is_inherent_owner_active(ws, cache, ut, owner_short, owner_crate)
    }
}

fn has_local_competing_self_method(lines: &[&str], impl_owners: &[Option<(String, bool)>], method: &str, allow_trait_short: Option<&str>) -> bool {
    let pat = format!("fn {method}");
    for (idx, &ln) in lines.iter().enumerate() {
        let code = ln.split("//").next().unwrap_or(ln);
        let Some(pos) = code.find(&pat) else { continue };
        let after = &code[pos + pat.len()..];
        if after.starts_with(|c: char| is_ident_continue(c)) || !has_self_receiver(&code[pos..]) {
            continue;
        }
        if let Some((ref own, is_tr)) = impl_owners[idx] {
            if !(is_tr && allow_trait_short == Some(own.as_str())) {
                return true;
            }
        }
    }
    false
}

fn has_dot_call_in_lines(lines: &[&str], target_name: &str) -> bool {
    lines.iter().any(|&ln| {
        ln.match_indices(target_name).any(|(b, _)| {
            let e = b + target_name.len();
            let prev_ok = b == 0 || !is_ident_continue(ln[..b].chars().next_back().unwrap_or(' '));
            let next_ok = e >= ln.len() || !is_ident_continue(ln[e..].chars().next().unwrap_or(' '));
            prev_ok
                && next_ok
                && !is_byte_in_string_or_comment(ln, b)
                && ln[..b].trim_end().ends_with('.')
                && (ln[e..].trim_start().starts_with('(') || ln[e..].trim_start().starts_with("::<"))
        })
    })
}

fn can_be_imported_by_other_targets(ws: &WorkspaceIndex, reader_idx: Option<usize>, kind: DefKind, target_path: &str, target_name: &str) -> bool {
    if target_name.is_empty() || matches!(kind, DefKind::Local | DefKind::Field | DefKind::TyParam | DefKind::ConstParam | DefKind::LifetimeParam | DefKind::Mod) {
        return false;
    }
    if !reader_idx.and_then(|ri| ws.target_kind_at(ri)).is_some_and(|tk| matches!(tk, TargetKind::Bin | TargetKind::Example | TargetKind::Test | TargetKind::Bench)) {
        return true;
    }
    let candidate_crate = if let Some((_, rhs)) = target_path.split_once(" as ") {
        rhs.split("::").next().unwrap_or("")
    } else {
        target_path.trim_start_matches('<').split("::").next().unwrap_or("")
    };
    !candidate_crate.is_empty() && ws.readers().iter().enumerate().any(|(i, r)| r.crate_name() == candidate_crate && ws.target_kind_at(i) == Some(TargetKind::Lib))
}

pub fn check_unindexed_target_ambiguity(ws: &WorkspaceIndex, def: &ResolvedDefinition<'_>) -> Result<(), RenameRefusal> {
    let target_name = def.name.rsplit("::").next().unwrap_or(def.name);
    if def.kind != DefKind::AssocFn
        || def.signature.is_some_and(|sig| !has_self_receiver(sig))
        || !can_be_imported_by_other_targets(ws, def.reader_idx, def.kind, def.path, target_name)
    {
        return Ok(());
    }
    let cache = ws.rename_cache();
    let Some(u_indices) = cache.unindexed_ident_map.get(target_name) else { return Ok(()) };
    let (def_crate, owner_short, def_is_trait, def_has_self, is_ext_vis) = ws.unindexed_symbol_owner_info(def.reader_idx, def.kind, def.path, def.file, def.start_line);
    if !def_has_self || !is_ext_vis {
        return Ok(());
    }
    for &u_idx in u_indices {
        let ut = &cache.unindexed_targets[u_idx];
        if !ut.dep_crates.contains(&def_crate) || !is_owner_explicit_in_ut(ws, &cache, ut, &owner_short, &def_crate, def_is_trait) {
            continue;
        }
        let lines: Vec<&str> = ut.content.lines().collect();
        if !has_dot_call_in_lines(&lines, target_name) {
            continue;
        }
        let impl_owners = extract_impl_header_owners(&lines);
        let other_ws = cache.self_methods_by_name.get(target_name).is_some_and(|cands| {
            cands.iter().any(|(c_crate, c_owner, c_tr)| {
                let same = *c_tr == def_is_trait && c_owner == &owner_short && (*c_tr || c_crate == &def_crate);
                !same && is_owner_explicit_in_ut(ws, &cache, ut, c_owner, c_crate, *c_tr)
            })
        });
        if other_ws || has_local_competing_self_method(&lines, &impl_owners, target_name, def_is_trait.then_some(owner_short.as_str())) {
            return Err(RenameRefusal::UnindexedTargetAmbiguous { name: target_name.to_string(), file: ut.path.to_string_lossy().to_string() });
        }
    }
    Ok(())
}

pub fn expand_unindexed_cargo_target_edits(
    ws: &WorkspaceIndex,
    loc_refs: &LocationReferences<'_>,
    target_name: &str,
    new_name: &str,
    conv: &mut PositionConverter,
    changes_map: &mut HashMap<String, Vec<Value>>,
) -> Result<(), RenameRefusal> {
    if !can_be_imported_by_other_targets(ws, loc_refs.target_reader_idx, loc_refs.target_kind, loc_refs.target_path, target_name) {
        return Ok(());
    }
    let cache = ws.rename_cache();
    let Some(u_indices) = cache.unindexed_ident_map.get(target_name) else { return Ok(()) };
    let (def_crate, owner_short, def_is_trait, def_has_self, is_ext_vis) =
        ws.unindexed_symbol_owner_info(loc_refs.target_reader_idx, loc_refs.target_kind, loc_refs.target_path, loc_refs.target_file, loc_refs.target_line);
    if !is_ext_vis {
        return Ok(());
    }
    let name_chars = target_name.chars().count() as u32;
    let qual_suffix = format!("::{target_name}");

    for &u_idx in u_indices {
        let ut = &cache.unindexed_targets[u_idx];
        if !ut.dep_crates.contains(&def_crate) {
            continue;
        }
        let Some(ut_str) = ut.path.to_str() else { continue };
        let type_imported = matches!(loc_refs.target_kind, DefKind::Struct | DefKind::Enum | DefKind::Union | DefKind::Trait | DefKind::TyAlias)
            && !local_type_shadowed(ut, target_name)
            && is_symbol_imported(ws, &cache, ut, target_name, &def_crate);
        let value_imported = matches!(loc_refs.target_kind, DefKind::Fn | DefKind::Const | DefKind::Static)
            && !ut.local_defs.contains(&(DefKind::Fn, target_name.to_string()))
            && is_symbol_imported(ws, &cache, ut, target_name, &def_crate);
        let owner_in_scope = !owner_short.is_empty()
            && !local_type_shadowed(ut, &owner_short)
            && ut.idents.contains(&owner_short)
            && (is_symbol_imported(ws, &cache, ut, &owner_short, &def_crate) || (def_is_trait && is_trait_in_scope(ws, &cache, ut, &owner_short, &def_crate)));
        let may_dot_call = loc_refs.target_kind == DefKind::AssocFn && def_has_self && ut.content.contains('.');
        let may_qual_path = ut.content.contains(&qual_suffix);
        if !type_imported && !value_imported && !owner_in_scope && !may_dot_call && !may_qual_path && !matches!(loc_refs.target_kind, DefKind::Variant | DefKind::Macro) {
            continue;
        }
        let lines: Vec<&str> = ut.content.lines().collect();
        let impl_owners = extract_impl_header_owners(&lines);

        let dot_call_allowed = if may_dot_call && has_dot_call_in_lines(&lines, target_name) {
            let def_explicit = is_owner_explicit_in_ut(ws, &cache, ut, &owner_short, &def_crate, def_is_trait);
            let trait_explicitly_imported =
                ut.explicit_imports.contains_key(&owner_short) || ut.anon_trait_imports.iter().any(|p| p.rsplit("::").next() == Some(owner_short.as_str()));
            let has_dep_inherent_same_name =
                cache.self_methods_by_name.get(target_name).is_some_and(|cands| cands.iter().any(|(c_crate, _, c_tr)| !*c_tr && ut.dep_crates.contains(c_crate)));
            let def_glob_trait =
                def_is_trait && !local_type_shadowed(ut, &owner_short) && !has_dep_inherent_same_name && is_trait_in_scope(ws, &cache, ut, &owner_short, &def_crate);
            let competing_explicit = cache.self_methods_by_name.get(target_name).is_some_and(|cands| {
                cands.iter().any(|(c_crate, c_owner, c_tr)| {
                    let same = *c_tr == def_is_trait && c_owner == &owner_short && (*c_tr || c_crate == &def_crate);
                    !same && is_owner_explicit_in_ut(ws, &cache, ut, c_owner, c_crate, *c_tr)
                })
            }) || has_local_competing_self_method(&lines, &impl_owners, target_name, def_is_trait.then_some(owner_short.as_str()));
            if def_explicit && competing_explicit {
                return Err(RenameRefusal::UnindexedTargetAmbiguous { name: target_name.to_string(), file: ut_str.to_string() });
            }
            let effective_def_dot = if def_is_trait && !trait_explicitly_imported && has_dep_inherent_same_name { false } else { def_explicit || def_glob_trait };
            effective_def_dot && !competing_explicit
        } else {
            false
        };
        if loc_refs.target_kind == DefKind::AssocFn && !dot_call_allowed && !owner_in_scope {
            continue;
        }
        let ut_uri = path_to_uri(ut_str);

        for (idx, &ln_str) in lines.iter().enumerate() {
            let ln_1 = (idx + 1) as u32;
            let in_owner_impl = impl_owners[idx].as_ref().is_some_and(|(o, tr)| o == &owner_short && *tr == def_is_trait);
            for (b_pos, _) in ln_str.match_indices(target_name) {
                let end_b = b_pos + target_name.len();
                let prev_ok = b_pos == 0 || !is_ident_continue(ln_str[..b_pos].chars().next_back().unwrap_or(' '));
                let next_ok = end_b >= ln_str.len() || !is_ident_continue(ln_str[end_b..].chars().next().unwrap_or(' '));
                if !prev_ok || !next_ok || is_byte_in_string_or_comment(ln_str, b_pos) {
                    continue;
                }
                let (before, after) = (ln_str[..b_pos].trim_end(), ln_str[end_b..].trim_start());
                if loc_refs.target_kind != DefKind::Macro && after.starts_with('!') {
                    continue;
                }
                let prev_seg = before.strip_suffix("::").map(|p| p.rsplit(|c: char| !is_ident_continue(c)).next().unwrap_or(""));
                let matched = match loc_refs.target_kind {
                    DefKind::Struct | DefKind::Enum | DefKind::Union | DefKind::Trait | DefKind::TyAlias => {
                        if let Some(seg) = prev_seg {
                            seg == def_crate
                                || loc_refs.target_path.contains(&format!("{seg}::{target_name}"))
                                || ut.explicit_imports.contains_key(seg)
                                || (type_imported
                                    && ut.explicit_imports.get(target_name).is_some_and(|imps| imps.iter().any(|imp| imp.ends_with(&format!("{seg}::{target_name}")))))
                        } else {
                            type_imported
                                && !before.ends_with('.')
                                && !(after.starts_with(':') && !after.starts_with("::"))
                                && !["struct", "enum", "trait", "union", "type", "fn", "let", "mod"].iter().any(|kw| before.ends_with(kw))
                        }
                    }
                    DefKind::Variant => {
                        prev_seg.is_some_and(|seg| seg == owner_short && owner_in_scope)
                            || (prev_seg.is_none() && !before.ends_with('.') && is_symbol_imported(ws, &cache, ut, target_name, &def_crate))
                    }
                    DefKind::Fn | DefKind::Const | DefKind::Static => {
                        if let Some(seg) = prev_seg {
                            seg == def_crate
                                || loc_refs.target_path.ends_with(&format!("{seg}::{target_name}"))
                                || ut.explicit_imports.contains_key(seg)
                                || (value_imported
                                    && ut.explicit_imports.get(target_name).is_some_and(|imps| imps.iter().any(|imp| imp.ends_with(&format!("{seg}::{target_name}")))))
                        } else {
                            value_imported && !before.ends_with('.') && !["fn", "const", "static", "let", "mod"].iter().any(|kw| before.ends_with(kw))
                        }
                    }
                    DefKind::AssocFn => {
                        let is_dot = dot_call_allowed && before.ends_with('.') && (after.starts_with('(') || after.starts_with("::<"));
                        let is_qual = prev_seg.is_some_and(|seg| (seg == owner_short && owner_in_scope) || (seg == "Self" && owner_in_scope && in_owner_impl));
                        let is_tr_fn = def_is_trait && owner_in_scope && before.ends_with("fn") && in_owner_impl;
                        is_dot || is_qual || is_tr_fn
                    }
                    DefKind::AssocTy | DefKind::AssocConst => {
                        owner_in_scope
                            && (prev_seg.is_some_and(|seg| seg == owner_short || (seg == "Self" && in_owner_impl))
                                || ((before.ends_with("type") || before.ends_with("const")) && in_owner_impl))
                    }
                    DefKind::Macro => prev_seg == Some(&def_crate) || (prev_seg.is_none() && after.starts_with('!') && is_symbol_imported(ws, &cache, ut, target_name, &def_crate)),
                    _ => false,
                };
                if matched {
                    let col_1 = ln_str[..b_pos].chars().count() as u32 + 1;
                    let sc = conv.char_col_to_utf16_col(ut_str, ln_1, col_1);
                    let ec = conv.char_col_to_utf16_col(ut_str, ln_1, col_1 + name_chars);
                    let edit = json!({
                        "range": { "start": { "line": ln_1 - 1, "character": sc }, "end": { "line": ln_1 - 1, "character": ec } },
                        "newText": new_name
                    });
                    let edits = changes_map.entry(ut_uri.clone()).or_default();
                    if !edits.iter().any(|e| e["range"] == edit["range"]) {
                        edits.push(edit);
                    }
                }
            }
        }
    }
    Ok(())
}
