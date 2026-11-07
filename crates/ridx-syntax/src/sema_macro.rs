/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Macro `TokenTree` scanning (including `{name}` format-string captures) and flat preorder
//! control-flow (`return` / `?`) collection for `sema_fn`.

use crate::char_class::{is_ident_continue_char, is_ident_start_char};
use crate::sema_fn::{norm_ident, ParamUses};
use crate::sema_pat::{block_flavor_fast, tok_matches_ident, BlockFlavor};
use crate::syntax_kind::SyntaxKind as K;
use crate::token::{Token, TokenKind as T};
use crate::tree::{Node, NodeId, SyntaxTree};

#[inline]
pub(crate) fn scan_self_token_tree(tokens: &[Token], tt_node: &Node, out: &mut ParamUses) {
    let slice = &tokens[tt_node.first as usize..tt_node.end.max(tt_node.first) as usize];
    for tok in slice {
        if tok.kind == T::SelfKw {
            out.macro_uses.push(tok.start as usize..tok.end as usize);
        }
    }
}

pub(crate) fn scan_token_tree(tokens: &[Token], src: &str, tt_node: &Node, tb: &[u8], out: &mut ParamUses) {
    let src_bytes = src.as_bytes();
    let min_fmt_len = (tb.len() + 4) as u32;
    let slice = &tokens[tt_node.first as usize..tt_node.end.max(tt_node.first) as usize];
    for &tok in slice {
        match tok.kind {
            T::Ident | T::RawIdent => {
                if tok_matches_ident(tok, src_bytes, tb) {
                    out.macro_uses.push(tok.start as usize..tok.end as usize);
                }
            }
            T::Str | T::RawStr if tok.end - tok.start >= min_fmt_len => {
                scan_format_captures(src, tok, tb, out);
            }
            _ => {}
        }
    }
}

fn scan_format_captures(src: &str, tok: Token, tb: &[u8], out: &mut ParamUses) {
    let raw_bytes = &src.as_bytes()[tok.start as usize..tok.end as usize];
    if !raw_bytes.contains(&b'{') {
        return;
    }
    let raw = &src[tok.start as usize..tok.end as usize];
    let Some(open_q) = raw_bytes.iter().position(|&b| b == b'"') else { return };
    let interior_start = open_q + 1;
    let suffix_len = if tok.kind == T::RawStr { open_q } else { 1 };
    let interior_end = if raw.len() >= interior_start + suffix_len && raw_bytes[raw.len() - suffix_len] == b'"' { raw.len() - suffix_len } else { raw.len() };
    let s = &raw[interior_start..interior_end];
    let base = tok.start as usize + interior_start;
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'{' {
                i += 2;
                continue;
            }
            i += 1;
            if let Some((id_s, id_e)) = read_fmt_ident(s, i) {
                if matches!(bytes.get(id_e), Some(b'}' | b':')) && norm_ident(&s[id_s..id_e]).as_bytes() == tb {
                    out.macro_uses.push((base + id_s)..(base + id_e));
                }
                i = id_e;
            }
            if bytes.get(i) == Some(&b':') {
                i += 1;
                while i < bytes.len() && bytes[i] != b'}' {
                    if let Some((id_s, id_e)) = read_fmt_ident(s, i) {
                        if bytes.get(id_e) == Some(&b'$') && norm_ident(&s[id_s..id_e]).as_bytes() == tb {
                            out.macro_uses.push((base + id_s)..(base + id_e));
                        }
                        i = id_e;
                    } else {
                        i += s[i..].chars().next().map_or(1, char::len_utf8);
                    }
                }
            }
            if bytes.get(i) == Some(&b'}') {
                i += 1;
            }
        } else if bytes[i] == b'}' && i + 1 < bytes.len() && bytes[i + 1] == b'}' {
            i += 2;
        } else {
            i += 1;
        }
    }
}

fn read_fmt_ident(s: &str, at: usize) -> Option<(usize, usize)> {
    let rest = s.get(at..)?;
    let prefix = if rest.starts_with("r#") { 2 } else { 0 };
    let mut chars = rest[prefix..].char_indices();
    let (_, first) = chars.next()?;
    if !is_ident_start_char(first) {
        return None;
    }
    let mut end = prefix + first.len_utf8();
    for (idx, c) in chars {
        if is_ident_continue_char(c) {
            end = prefix + idx + c.len_utf8();
        } else {
            break;
        }
    }
    (&rest[..end] != "_").then_some((at, at + end))
}

pub(crate) fn collect_own_cf(tree: &SyntaxTree<'_>, body: NodeId, cut_try_block: bool, want: K, out: &mut Vec<NodeId>) {
    let nodes = &tree.nodes;
    let tokens = &tree.tokens;
    let src = tree.src.as_bytes();
    let end = nodes[body.0 as usize].subtree_end as usize;
    let mut idx = body.0 as usize + 1;
    while idx < end {
        let n = nodes[idx];
        let k = n.kind;
        if k == want {
            out.push(NodeId(idx as u32));
            idx += 1;
            continue;
        }
        if k.is_cf_cutoff() {
            idx = n.subtree_end as usize;
            continue;
        }
        if k == K::BlockExpr {
            match block_flavor_fast(tokens, src, &n) {
                BlockFlavor::Async | BlockFlavor::Const => {
                    idx = n.subtree_end as usize;
                    continue;
                }
                BlockFlavor::Try if cut_try_block => {
                    idx = n.subtree_end as usize;
                    continue;
                }
                BlockFlavor::Plain | BlockFlavor::Unsafe | BlockFlavor::Try => {}
            }
        }
        idx += 1;
    }
}
