/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Lexical masking and syntactic item/enclosing-scope parsing for compile-safe code actions.

#[derive(Debug, Clone)]
pub enum EnclosingContext {
    None,
    ExternBlock,
    InherentImpl {
        generics_src: String,
        self_ty: String,
        where_src: String,
        cfg_attrs: Vec<String>,
        insert_line0: u32,
        indent: String,
        body_open_byte: usize,
        body_close_byte: usize,
    },
    TraitImpl {
        generics_src: String,
        trait_ty: String,
        self_ty: String,
        where_src: String,
        cfg_attrs: Vec<String>,
        insert_line0: u32,
        indent: String,
        body_open_byte: usize,
        body_close_byte: usize,
    },
    TraitDecl {
        generics_src: String,
        trait_name: String,
        where_src: String,
        cfg_attrs: Vec<String>,
        insert_line0: u32,
        indent: String,
    },
}

#[derive(Debug, Clone)]
pub enum ItemKindInfo {
    Fn {
        is_unsafe: bool,
        extern_abi: Option<String>,
        generics_src: String,
        params_src: String,
        ret_src: Option<String>,
        where_src: String,
        has_body: bool,
        body_range: Option<(usize, usize)>,
    },
    Adt {
        adt_kw: &'static str,
        generics_src: String,
        where_src: String,
        fields_kind: AdtFieldsKind,
        open_delim_byte: usize,
        end_byte: usize,
    },
    AssocOrTypeAlias {
        end_byte: usize,
    },
}

#[derive(Debug, Clone)]
pub enum AdtFieldsKind {
    Unit,
    Tuple(String),
    Named(String),
    EnumOrUnion,
}

#[derive(Debug, Clone)]
pub struct ParsedCursorItem {
    pub name: String,
    pub raw_name: String,
    pub vis_prefix: String,
    pub decl_start_byte: usize,
    pub first_attr_byte: usize,
    pub has_doc: bool,
    pub cfg_attrs: Vec<String>,
    pub attr_start_line0: u32,
    pub indent: String,
    pub in_fn_body: bool,
    pub enclosing_block: Option<(usize, usize)>,
    pub enclosing: EnclosingContext,
    pub kind: ItemKindInfo,
}

/// Replaces comments and string/character literals with spaces while preserving newlines and byte offsets.
pub fn mask_comments_and_strings(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = bytes.to_vec();
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        if i + 1 < n && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            while i < n && bytes[i] != b'\n' {
                out[i] = b' ';
                i += 1;
            }
        } else if i + 1 < n && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            let mut depth = 1usize;
            out[i] = b' ';
            out[i + 1] = b' ';
            i += 2;
            while i < n && depth > 0 {
                if i + 1 < n && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                    depth += 1;
                    out[i] = b' ';
                    out[i + 1] = b' ';
                    i += 2;
                } else if i + 1 < n && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    depth -= 1;
                    out[i] = b' ';
                    out[i + 1] = b' ';
                    i += 2;
                } else {
                    if bytes[i] != b'\n' {
                        out[i] = b' ';
                    }
                    i += 1;
                }
            }
        } else if let Some(raw_end) = try_skip_raw_string(bytes, i) {
            for b in &mut out[i..raw_end] {
                if *b != b'\n' {
                    *b = b' ';
                }
            }
            i = raw_end;
        } else if bytes[i] == b'"' {
            out[i] = b' ';
            i += 1;
            while i < n {
                if bytes[i] == b'\\' && i + 1 < n {
                    if out[i] != b'\n' {
                        out[i] = b' ';
                    }
                    if out[i + 1] != b'\n' {
                        out[i + 1] = b' ';
                    }
                    i += 2;
                    continue;
                }
                let ch = bytes[i];
                if ch != b'\n' {
                    out[i] = b' ';
                }
                i += 1;
                if ch == b'"' {
                    break;
                }
            }
        } else if bytes[i] == b'\'' {
            if let Some(end_q) = try_skip_char_literal(src, i) {
                for b in &mut out[i..end_q] {
                    if *b != b'\n' {
                        *b = b' ';
                    }
                }
                i = end_q;
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| src.to_string())
}

fn try_skip_raw_string(bytes: &[u8], i: usize) -> Option<usize> {
    let n = bytes.len();
    let mut k = i;
    if k < n && (bytes[k] == b'b' || bytes[k] == b'c') {
        if i > 0 && is_ident_byte(bytes[i - 1]) {
            return None;
        }
        k += 1;
    }
    if k >= n || bytes[k] != b'r' || (k == i && i > 0 && is_ident_byte(bytes[i - 1])) {
        return None;
    }
    k += 1;
    let mut hashes = 0usize;
    while k < n && bytes[k] == b'#' {
        hashes += 1;
        k += 1;
    }
    if k >= n || bytes[k] != b'"' {
        return None;
    }
    k += 1;
    while k < n {
        if bytes[k] == b'"' && k + 1 + hashes <= n && bytes[k + 1..k + 1 + hashes].iter().all(|&b| b == b'#') {
            return Some(k + 1 + hashes);
        }
        k += 1;
    }
    Some(n)
}

fn try_skip_char_literal(src: &str, i: usize) -> Option<usize> {
    let rest = &src[i + 1..];
    if rest.starts_with('\\') {
        let mut idx = 1;
        let rb = rest.as_bytes();
        while idx < rb.len() && idx <= 12 && rb[idx] != b'\n' {
            if rb[idx] == b'\'' {
                return Some(i + 1 + idx + 1);
            }
            idx += 1;
        }
        return None;
    }
    let mut chars = rest.char_indices();
    let (_, _first) = chars.next()?;
    let (next_off, second) = chars.next()?;
    if second == '\'' {
        Some(i + 1 + next_off + 1)
    } else {
        None
    }
}

#[inline]
pub fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

pub fn line_starts_of(src: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    for (idx, b) in src.bytes().enumerate() {
        if b == b'\n' {
            starts.push(idx + 1);
        }
    }
    starts
}

pub fn utf16_pos_to_byte(src: &str, starts: &[usize], line0: u32, col_u16: u32) -> Option<usize> {
    let l = line0 as usize;
    let start = *starts.get(l)?;
    let end = starts.get(l + 1).map(|&s| s.saturating_sub(1)).unwrap_or(src.len());
    let line_str = &src[start..end];
    let mut u16_cur = 0u32;
    for (byte_idx, ch) in line_str.char_indices() {
        if u16_cur >= col_u16 {
            return Some(start + byte_idx);
        }
        u16_cur += ch.len_utf16() as u32;
    }
    Some(start + line_str.len())
}

pub fn byte_to_line0(starts: &[usize], byte_off: usize) -> u32 {
    match starts.binary_search(&byte_off) {
        Ok(idx) => idx as u32,
        Err(idx) => idx.saturating_sub(1) as u32,
    }
}

pub fn byte_to_utf16_pos(src: &str, starts: &[usize], byte_off: usize) -> (u32, u32) {
    let l = byte_to_line0(starts, byte_off) as usize;
    let ls = starts[l];
    let slice = &src[ls..byte_off.min(src.len())];
    let u16_col: u32 = slice.chars().map(|c| c.len_utf16() as u32).sum();
    (l as u32, u16_col)
}
