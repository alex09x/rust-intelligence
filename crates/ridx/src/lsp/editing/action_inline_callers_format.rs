/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The text one inlined call becomes: the receiver and arguments bound before the body, as the call evaluated them.

use super::action_inline_callers_params::{is_writable_binding_type, normalize_param_type_for_binding, FnParamInfo, SelfKind};

#[allow(clippy::too_many_arguments)]
pub(crate) fn format_inlined_call(
    recv_expr: Option<&str>,
    self_kind: Option<SelfKind>,
    recv_ty: Option<&str>,
    params: &[FnParamInfo],
    args: &[String],
    body_src: &str,
    is_standalone: bool,
    recv_in_match: bool,
    args_at_once: bool,
) -> String {
    let mut bindings = Vec::new();
    if let (Some(recv), Some(sk)) = (recv_expr, self_kind) {
        let trimmed_recv = recv.trim();
        let recv_binding = match sk {
            SelfKind::Value => {
                if let Some(ty) = recv_ty {
                    if ty.starts_with('&') {
                        format!("let mut this = *{trimmed_recv};")
                    } else {
                        format!("let mut this = {trimmed_recv};")
                    }
                } else {
                    format!("let mut this = {trimmed_recv};")
                }
            }
            SelfKind::Ref => {
                if trimmed_recv.starts_with('&') && !trimmed_recv.starts_with("&mut") {
                    format!("let this = {trimmed_recv};")
                } else if recv_ty.is_some_and(|t| t.starts_with('&')) {
                    // A receiver that is a reference is reborrowed, not borrowed again: `&p` would point at the
                    // variable `p`, and a reference the body returns could not leave the caller (E0515).
                    format!("let this = &*{trimmed_recv};")
                } else {
                    format!("let this = &{trimmed_recv};")
                }
            }
            SelfKind::MutRef => {
                let is_already_mut_ref = trimmed_recv == "self" || trimmed_recv == "this" || trimmed_recv.starts_with("&mut ") || recv_ty.is_some_and(|t| t.starts_with("&mut "));
                if is_already_mut_ref {
                    let inner = trimmed_recv.strip_prefix("&mut ").unwrap_or(trimmed_recv);
                    format!("let mut this = &mut *{inner};")
                } else {
                    format!("let mut this = &mut {trimmed_recv};")
                }
            }
        };
        bindings.push(recv_binding);
    }

    let non_self_params: Vec<&FnParamInfo> = params.iter().filter(|p| !p.is_self).collect();
    for (p, arg) in non_self_params.iter().zip(args.iter()) {
        let arg_trimmed = arg.trim();
        if let Some(id) = &p.ident {
            if arg_trimmed == id.as_str() {
                continue;
            }
            let written = p.decl_ty.as_deref().map(normalize_param_type_for_binding).filter(|n| is_writable_binding_type(n));
            if let (true, Some(norm_ty)) = (arg_trimmed.contains('('), written.clone()) {
                // A call argument: `let x: &T = &f();` keeps the temporary alive to the end of the block (`&*f()`
                // would drop it at the `;`), and the written type coerces `&&T` to `&T`.
                let borrow = if norm_ty.starts_with("&mut ") {
                    "&mut "
                } else if norm_ty.starts_with('&') {
                    "&"
                } else {
                    ""
                };
                bindings.push(format!("let {id}: {norm_ty} = {borrow}{arg_trimmed};"));
            } else if let Some(norm_ty) = written.filter(|t| t.starts_with('&')) {
                // A reference parameter keeps its declared type: the argument coerces (`&&mut T` → `&T`, `&mut [T; 0]`
                // → `&mut [T]`) and a `&mut` is reborrowed, as at the call.
                bindings.push(format!("let {id}: {norm_ty} = {arg_trimmed};"));
            } else if p.is_mut_ref && !arg_trimmed.starts_with("&mut ") {
                bindings.push(format!("let {id} = &mut *{arg_trimmed};"));
            } else if p.ty.as_deref().is_some_and(|t| t.starts_with('&') && !t.starts_with("&mut")) && !arg_trimmed.starts_with('&') {
                bindings.push(format!("let {id} = &*{arg_trimmed};"));
            } else if let Some(ty) = &p.ty {
                let norm_ty = normalize_param_type_for_binding(ty);
                if !norm_ty.contains('\'') && !norm_ty.contains("Self") && !norm_ty.contains("impl ") && !norm_ty.contains("dyn ") {
                    bindings.push(format!("let {id}: {norm_ty} = {arg_trimmed};"));
                } else {
                    bindings.push(format!("let {id} = {arg_trimmed};"));
                }
            } else {
                bindings.push(format!("let {id} = {arg_trimmed};"));
            }
        }
    }

    // A chained receiver is the scrutinee of a `match`, whose temporaries live as long as the whole block.
    let scrutinee = if recv_in_match && recv_expr.is_some() && self_kind.is_some() && !bindings.is_empty() {
        let b = bindings.remove(0);
        let (pat, rhs) = b.strip_prefix("let ").and_then(|s| s.strip_suffix(';')).and_then(|s| s.split_once(" = ")).unwrap_or(("this", ""));
        Some((pat.to_string(), rhs.to_string()))
    } else {
        None
    };
    // An argument naming a parameter bound before it (`f(&sema, alias.ty(sema.db))`) must read the caller's variable:
    // all arguments are evaluated, in order, before any is bound, by one tuple `let`.
    let recv_bound = usize::from(recv_expr.is_some() && self_kind.is_some() && scrutinee.is_none());
    if args_at_once && bindings.len() > recv_bound + 1 {
        let parts: Option<Vec<(String, String, String)>> = bindings[recv_bound..]
            .iter()
            .map(|b| {
                let (lhs, rhs) = b.strip_prefix("let ")?.strip_suffix(';')?.split_once(" = ")?;
                let (pat, ty) = lhs.split_once(": ").unwrap_or((lhs, "_"));
                Some((pat.to_string(), ty.to_string(), rhs.to_string()))
            })
            .collect();
        if let Some(parts) = parts {
            let join = |f: fn(&(String, String, String)) -> &String| parts.iter().map(f).cloned().collect::<Vec<_>>().join(", ");
            let tuple = format!("let ({}): ({}) = ({});", join(|p| &p.0), join(|p| &p.1), join(|p| &p.2));
            bindings.truncate(recv_bound);
            bindings.push(tuple);
        }
    }
    let stmts = if bindings.is_empty() { String::new() } else { format!("{} ", bindings.join(" ")) };

    let blk = if stmts.is_empty() { format!("{{ {body_src} }}") } else { format!("{{ {stmts}{body_src} }}") };
    let blk = match scrutinee {
        Some((pat, rhs)) => format!("match {rhs} {{ {pat} => {blk} }}"),
        None => blk,
    };

    if is_standalone {
        blk
    } else {
        format!("({blk})")
    }
}

#[cfg(test)]
mod tests {
    use super::{format_inlined_call, FnParamInfo, SelfKind};

    #[test]
    fn receivers_and_arguments_are_bound_as_the_call_evaluated_them() {
        let f = |recv| format_inlined_call(Some(recv), Some(SelfKind::Ref), None, &[], &[], "this.x", false, true, false);
        assert_eq!(f("a.b().c()"), "(match &a.b().c() { this => { this.x } })");
        let p = |id: &str| FnParamInfo { is_self: false, self_kind: None, ident: Some(id.to_string()), is_mut_ref: false, ty: None, decl_ty: None };
        let args = ["&sema".to_string(), "alias.ty(sema.db)".to_string()];
        let out = format_inlined_call(None, None, None, &[p("sema"), p("ty")], &args, "go(sema, ty)", true, false, true);
        assert_eq!(out, "{ let (sema, ty): (_, _) = (&sema, alias.ty(sema.db)); go(sema, ty) }");
    }
}
