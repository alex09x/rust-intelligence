/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Parameter and receiver formatting helpers for `Inline into all callers`.

pub(crate) use super::action_inline_callers_params::{is_primitive_scalar, is_writable_binding_type, normalize_param_type_for_binding, parse_fn_params, FnParamInfo, SelfKind};

pub(crate) fn find_receiver_start(_src: &str, masked: &str, dot_byte: usize) -> Option<usize> {
    let mb = masked.as_bytes();
    let mut p = dot_byte;
    while p > 0 && mb[p - 1].is_ascii_whitespace() {
        p -= 1;
    }
    if p == 0 {
        return None;
    }
    let last = mb[p - 1];
    if last == b')' {
        let open = skip_balanced_backwards(mb, p - 1, b')', b'(')?;
        let mut q = open;
        while q > 0 && mb[q - 1].is_ascii_whitespace() {
            q -= 1;
        }
        // A macro call (`bytes::regex!("foo")`) is the receiver with its name.
        if q > 0 && mb[q - 1] == b'!' {
            q -= 1;
        }
        if q > 0 && (mb[q - 1].is_ascii_alphanumeric() || mb[q - 1] == b'_') {
            while q > 0 && (mb[q - 1].is_ascii_alphanumeric() || mb[q - 1] == b'_' || mb[q - 1] == b':') {
                q -= 1;
            }
        }
        return Some(q);
    }
    if last == b']' {
        let open = skip_balanced_backwards(mb, p - 1, b']', b'[')?;
        let mut q = open;
        while q > 0 && (mb[q - 1].is_ascii_alphanumeric() || mb[q - 1] == b'_' || mb[q - 1] == b'.') {
            q -= 1;
        }
        return Some(q);
    }
    if last == b'}' {
        return skip_balanced_backwards(mb, p - 1, b'}', b'{');
    }
    let mut q = p;
    while q > 0 {
        let b = mb[q - 1];
        if b.is_ascii_alphanumeric() || b == b'_' || (b == b'.' && q > 1 && (mb[q - 2].is_ascii_alphanumeric() || mb[q - 2] == b'_')) {
            q -= 1;
        } else {
            break;
        }
    }
    if q < p && mb[q] != b'.' {
        return Some(q);
    }
    None
}

fn skip_balanced_backwards(bytes: &[u8], close_idx: usize, close_ch: u8, open_ch: u8) -> Option<usize> {
    if bytes.get(close_idx)? != &close_ch {
        return None;
    }
    let mut depth = 0usize;
    let mut i = close_idx;
    loop {
        let b = bytes[i];
        if b == close_ch {
            depth += 1;
        } else if b == open_ch {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        if i == 0 {
            break;
        }
        i -= 1;
    }
    None
}

pub(crate) fn find_path_start(masked: &str, ref_byte: usize) -> usize {
    let before = &masked[..ref_byte];
    let trimmed = before.trim_end();
    if !trimmed.ends_with("::") {
        return ref_byte;
    }
    let mb = masked.as_bytes();
    let mut p = trimmed.len() - 2;
    loop {
        while p > 0 && mb[p - 1].is_ascii_whitespace() {
            p -= 1;
        }
        if p == 0 {
            return 0;
        }
        if mb[p - 1] == b'>' {
            if let Some(open) = skip_balanced_backwards(mb, p - 1, b'>', b'<') {
                p = open;
                while p > 0 && mb[p - 1].is_ascii_whitespace() {
                    p -= 1;
                }
                if p >= 2 && &mb[p - 2..p] == b"::" {
                    p -= 2;
                    while p > 0 && mb[p - 1].is_ascii_whitespace() {
                        p -= 1;
                    }
                }
            } else {
                break;
            }
        }
        let id_end = p;
        while p > 0 && (mb[p - 1].is_ascii_alphanumeric() || mb[p - 1] == b'_') {
            p -= 1;
        }
        if p == id_end {
            break;
        }
        let mut q = p;
        while q > 0 && mb[q - 1].is_ascii_whitespace() {
            q -= 1;
        }
        if q >= 2 && &mb[q - 2..q] == b"::" {
            p = q - 2;
        } else {
            break;
        }
    }
    p
}

pub(crate) fn format_inlined_call(
    recv_expr: Option<&str>,
    self_kind: Option<SelfKind>,
    recv_ty: Option<&str>,
    params: &[FnParamInfo],
    args: &[String],
    body_src: &str,
    is_standalone: bool,
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

    let stmts = if bindings.is_empty() { String::new() } else { format!("{} ", bindings.join(" ")) };

    let blk = if stmts.is_empty() { format!("{{ {body_src} }}") } else { format!("{{ {stmts}{body_src} }}") };

    if is_standalone {
        blk
    } else {
        format!("({blk})")
    }
}

pub(crate) fn contains_recursive_call(masked_body: &str, fn_name: &str) -> bool {
    let mb = masked_body.as_bytes();
    // A local of the same name (`let new = |r| …;`) is what a bare `new(…)` calls.
    let shadowed = super::action_parse_items::contains_word(masked_body, "let")
        && masked_body.match_indices("let ").any(|(p, _)| {
            let rest = masked_body[p + 4..].trim_start();
            let rest = rest.strip_prefix("mut ").map_or(rest, str::trim_start);
            rest.starts_with(fn_name) && !rest[fn_name.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_')
        });
    let mut start = 0;
    while let Some(idx) = masked_body[start..].find(fn_name) {
        let abs_idx = start + idx;
        let before_ok = abs_idx == 0 || (!mb[abs_idx - 1].is_ascii_alphanumeric() && mb[abs_idx - 1] != b'_');
        let after_idx = abs_idx + fn_name.len();
        let after_ok = after_idx == mb.len() || (!mb[after_idx].is_ascii_alphanumeric() && mb[after_idx] != b'_');
        if before_ok && after_ok {
            let mut p = after_idx;
            while p < mb.len() && mb[p].is_ascii_whitespace() {
                p += 1;
            }
            if p < mb.len() && mb[p] == b'(' {
                if abs_idx > 0 && mb[abs_idx - 1] == b'.' {
                    let before_dot = masked_body[..abs_idx - 1].trim_end();
                    if before_dot.ends_with("self") {
                        let self_start = before_dot.len() - 4;
                        let self_before_ok = self_start == 0 || (!before_dot.as_bytes()[self_start - 1].is_ascii_alphanumeric() && before_dot.as_bytes()[self_start - 1] != b'_');
                        if self_before_ok {
                            return true;
                        }
                    }
                } else if abs_idx >= 2 && &mb[abs_idx - 2..abs_idx] == b"::" {
                    let before_colons = masked_body[..abs_idx - 2].trim_end();
                    if before_colons.ends_with("Self") {
                        return true;
                    }
                } else if !shadowed {
                    return true;
                }
            }
        }
        start = abs_idx + fn_name.len();
    }
    false
}

pub(crate) fn is_caller_return_compatible(src: &str, masked: &str, starts: &[usize], ref_byte: usize, callee_ret: &str) -> bool {
    let callee_is_result = callee_ret.contains("Result");
    let callee_is_option = callee_ret.contains("Option");
    if !callee_is_result && !callee_is_option {
        return false;
    }
    let Some(caller_fn) = super::action_parse_enclosing::find_enclosing_fn_at_cursor(src, masked, starts, ref_byte) else {
        return false;
    };
    let super::action_parse::ItemKindInfo::Fn { ret_src, .. } = &caller_fn.kind else {
        return false;
    };
    let Some(r) = ret_src.as_deref() else {
        return false;
    };
    if callee_is_result {
        r.contains("Result")
    } else if callee_is_option {
        r.contains("Option")
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::find_receiver_start;

    #[test]
    fn a_macro_call_receiver_keeps_its_name() {
        let src = "assert!(bytes::regex!(\"foo\").is_match(b\"foo\"));";
        let dot = src.find(").is_match").unwrap() + 1;
        assert_eq!(find_receiver_start(src, src, dot), src.find("bytes::regex!"));
        let plain = "x = a.b(c).is_match(1);";
        assert_eq!(find_receiver_start(plain, plain, plain.find(").is").unwrap() + 1), plain.find("a.b").map(|p| p + 2));
    }
}
