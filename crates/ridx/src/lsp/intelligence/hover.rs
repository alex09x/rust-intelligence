/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP hover intelligence handler.

use super::super::error::LspResult;
use super::super::protocol::{uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, WorkspaceIndex};
use serde_json::{json, Value};

pub fn handle_hover(msg: &Value, workspace: Option<&WorkspaceIndex>, overlay_mgr: &OverlayManager, conv: &mut PositionConverter) -> LspResult<Value> {
    let (uri, line_0based, utf16_col_0based) = crate::lsp::params::text_document_position(msg)?;
    let line_1based = line_0based + 1;

    let file = uri_to_path(uri);
    let overlay_opt = overlay_mgr.get(&file);
    let col_1based = if let Some(overlay) = overlay_opt {
        overlay.utf16_col_to_char_col(line_1based, utf16_col_0based)
    } else {
        conv.utf16_col_to_char_col(&file, line_1based, utf16_col_0based)
    };

    // 1. The index's hover: at the cursor in a clean buffer; in an edited one, at the line the index compiled for a line
    // the edit left as it was (and whose names no edited import may have changed), so a binding there shows its type
    // as in a clean buffer (`let mut cart: Cart`, not the bare `let cart` of the buffer's own parse).
    if let Some(ws) = workspace {
        let index_pos = match overlay_opt.filter(|o| o.is_dirty()) {
            None => Some((line_1based, col_1based)),
            Some(o) if o.imports_changed_at(line_1based, col_1based) => None,
            Some(o) => o.map_live_to_compiled(line_1based, col_1based),
        };
        if let Some((il, ic)) = index_pos {
            if let Some(hover) = ws.hover_at(&file, il, ic) {
                let line_text =
                    overlay_opt.and_then(|o| o.text.lines().nth(line_0based as usize)).map(str::to_string).or_else(|| conv.get_line(&file, line_1based).map(str::to_string));
                // A literal names nothing: inside a macro's arguments the index's widest reference there is the call
                // the macro expands to (`println!(.., f(2))` → `std::io::_print`), which is not what the cursor is on.
                if hover.definition.is_some() && line_text.as_deref().is_some_and(|l| is_literal_at(l, col_1based)) {
                    return Ok(hover.inferred_type.map_or(Value::Null, |ty| json!({ "contents": { "kind": "markdown", "value": format!("```rust\n{ty}\n```") } })));
                }
                if let (None, Some(ty)) = (&hover.definition, hover.inferred_type) {
                    let layout_at = |c: u32| ws.note_at(&file, il, c, ridx_format::NOTE_LAYOUT).map(str::to_string);
                    if let Some(text) = line_text.and_then(|l| super::hover_render::let_binding_hover_with_layout(&l, col_1based, ty, &layout_at)) {
                        return Ok(json!({ "contents": { "kind": "markdown", "value": text } }));
                    }
                }
                let start_col = word_start_col(conv, overlay_opt, &file, line_1based, col_1based);
                let call_args = ws.note_at(&file, il, start_col, ridx_format::NOTE_CALL_ARGS);
                let at_definition = hover.definition.as_ref().is_some_and(|d| d.start_line == il && d.start_col == start_col && file.ends_with(d.file.trim_start_matches("./")));
                return Ok(format_hover_full(&hover, Some(ws), Some(overlay_mgr), Some(conv), Some(&file), HoverCursor { call_args, at_definition }));
            }
        }
    }

    // 2. Check overlay definition and inferred signature (for dirty buffers or when index has no info)
    if let Some(def) = overlay_mgr.find_definition_at(&file, line_1based, col_1based) {
        let hover_text = if let Some(sig) = def.signature { format!("```rust\n{}\n```", sig) } else { format!("```rust\n{} {}\n```", def.kind.as_str(), def.name) };
        return Ok(json!({
            "contents": {
                "kind": "markdown",
                "value": hover_text
            }
        }));
    }

    if let Some(ws) = workspace {
        // 3. A name the index keeps nothing for: code typed since it compiled, or a path it records no reference at
        // (a `use` item): resolved as completion resolves it.
        let edited = overlay_opt.is_some_and(|o| o.is_dirty());
        let line_text = overlay_opt.and_then(|o| o.text.lines().nth(line_0based as usize)).map(str::to_string).or_else(|| conv.get_line(&file, line_1based).map(str::to_string));
        if let Some(text) = line_text {
            if let Some(v) = super::hover_unindexed::hover_unindexed(ws, &file, &text, overlay_opt, edited, line_1based, col_1based, overlay_mgr, conv) {
                return Ok(v);
            }
        }
    }

    Ok(Value::Null)
}

/// Whether the 1-based character column `col` of `line` is on a literal: a number, or inside a string or char literal.
fn is_literal_at(line: &str, col: u32) -> bool {
    let chars: Vec<char> = line.chars().collect();
    let at = (col.saturating_sub(1) as usize).min(chars.len());
    let mut in_str = false;
    let mut k = 0;
    while k < at {
        match chars[k] {
            '\\' if in_str => k += 1,
            '"' => in_str = !in_str,
            '/' if !in_str && chars.get(k + 1) == Some(&'/') => return false,
            _ => {}
        }
        k += 1;
    }
    if in_str {
        // `{name}` inside a format string names a binding, as rust-analyzer resolves it.
        let open_brace = chars[..at].iter().rposition(|&c| c == '{' || c == '}' || c == '"').is_some_and(|i| chars[i] == '{');
        return !open_brace;
    }
    if chars.get(at) == Some(&'"') {
        return true;
    }
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut s = at;
    while s > 0 && is_word(chars[s - 1]) {
        s -= 1;
    }
    chars.get(s).is_some_and(|c| c.is_ascii_digit()) && chars.get(at).is_some_and(|&c| is_word(c))
}

#[cfg(test)]
mod literal_tests {
    #[test]
    fn test_literals_under_the_cursor() {
        let line = r#"    println!("{} {x}", s.len(), double(2), 0x1F, "a\"b");"#;
        let col = |needle: &str, k: usize| line.find(needle).unwrap() as u32 + 1 + k as u32;
        assert!(super::is_literal_at(line, col("2)", 0)), "a number");
        assert!(super::is_literal_at(line, col("0x1F", 2)), "a hex number");
        assert!(super::is_literal_at(line, col("{} ", 0)), "inside a format string");
        assert!(!super::is_literal_at(line, col("{x}", 1)), "an inline format argument names a binding");
        assert!(super::is_literal_at(line, col(r#"a\"b"#, 3)), "past an escaped quote");
        assert!(!super::is_literal_at(line, col("double", 2)));
        assert!(!super::is_literal_at(line, col("len", 1)));
    }
}

#[cfg(test)]
pub(crate) fn format_hover(hover: &ridx_format::WorkspaceHover, ws: Option<&WorkspaceIndex>) -> Value {
    format_hover_full(hover, ws, None, None, None, HoverCursor::default())
}

/// What hover knows about the cursor beyond the definition under it.
#[derive(Default, Clone, Copy)]
pub(crate) struct HoverCursor<'a> {
    /// The call's generic arguments when the cursor is on a callee name.
    pub call_args: Option<&'a str>,
    /// Whether the cursor is on the definition's own name (layouts show there, as in rust-analyzer).
    pub at_definition: bool,
}

/// The 1-based char column where the identifier under `col` starts (notes are recorded at a name's start).
fn word_start_col(conv: &mut PositionConverter, overlay: Option<&crate::overlay::DocumentOverlay>, file: &str, line: u32, col: u32) -> u32 {
    let text = match overlay {
        Some(o) => o.text.lines().nth(line.saturating_sub(1) as usize).map(str::to_string),
        None => conv.get_line(file, line).map(str::to_string),
    };
    let Some(text) = text else { return col };
    let chars: Vec<char> = text.chars().collect();
    let mut at = (col.saturating_sub(1) as usize).min(chars.len());
    while at > 0 && chars.get(at - 1).is_some_and(|c| c.is_alphanumeric() || *c == '_') {
        at -= 1;
    }
    at as u32 + 1
}

pub(crate) fn format_hover_full(
    hover: &ridx_format::WorkspaceHover,
    ws: Option<&WorkspaceIndex>,
    overlay_mgr: Option<&OverlayManager>,
    mut conv: Option<&mut PositionConverter>,
    cursor_file: Option<&str>,
    cursor: HoverCursor<'_>,
) -> Value {
    if let (Some(d), Some(w), Some(c)) = (hover.definition.as_ref(), ws, conv.as_deref_mut()) {
        // The source-based renderer reads arbitrary (macro-generated, std) source; a slip must cost the layout, not the server.
        let rendered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            super::hover_render::render_hover(d, hover.inferred_type, cursor.call_args, cursor.at_definition, w, overlay_mgr, c)
        }));
        if let Ok(Some(mut text)) = rendered {
            if let Some(prov) = super::hover_enrich::format_provenance_and_blast_radius(d, w) {
                text.push_str(if text.contains("\n---\n") { "\n\n" } else { "\n\n---\n\n" });
                text.push_str(&prov);
            }
            return json!({ "contents": { "kind": "markdown", "value": text } });
        }
    }
    let mut blocks = Vec::new();
    if let Some(ref d) = hover.definition {
        match d.kind {
            DefKind::Variant | DefKind::VariantCtor => {
                if let Some((enum_path, _)) = d.path.rsplit_once("::") {
                    let cname = d.reader_idx.and_then(|idx| ws.and_then(|w| w.readers().get(idx))).map(|r| r.crate_name()).unwrap_or("");
                    let full = if !cname.is_empty() && !enum_path.starts_with(&format!("{cname}::")) && enum_path != cname {
                        format!("{cname}::{enum_path}")
                    } else {
                        enum_path.to_string()
                    };
                    blocks.push(full);
                }
                if let Some(sig) = d.signature {
                    blocks.push(sig.strip_prefix("variant ").unwrap_or(sig).to_string());
                } else {
                    blocks.push(d.name.to_string());
                }
            }
            DefKind::AssocFn => {
                if let Some(sig) = d.signature {
                    blocks.push(format_signature(d.name, sig));
                } else {
                    blocks.push(format!("fn {}", d.name));
                }
            }
            DefKind::AssocTy | DefKind::TyAlias => {
                if let Some(sig) = d.signature {
                    if sig.starts_with("type ") || sig.contains(" type ") {
                        blocks.push(sig.to_string());
                    } else if sig.is_empty() {
                        blocks.push(format!("type {}", d.name));
                    } else {
                        blocks.push(format!("type {}: {sig}", d.name));
                    }
                } else if let Some(ref mut c) = conv {
                    if let Some(src_hdr) = super::hover_enrich::extract_source_decl_header(d, ws, overlay_mgr, c) {
                        blocks.push(src_hdr);
                    } else {
                        blocks.push(format!("type {}", d.name));
                    }
                } else {
                    blocks.push(format!("type {}", d.name));
                }
            }
            DefKind::Macro => {
                if let Some(sig) = d.signature {
                    blocks.push(sig.to_string());
                } else {
                    blocks.push(format!("macro_rules! {}", d.name));
                }
            }
            DefKind::Mod => {
                let mname = if d.name.is_empty() { d.reader_idx.and_then(|idx| ws.and_then(|w| w.readers().get(idx))).map(|r| r.crate_name()).unwrap_or("") } else { d.name };
                blocks.push(format!("mod {mname}"));
            }
            _ => {
                if let Some(sig) = d.signature {
                    blocks.push(format_signature(d.name, sig));
                } else if let Some(ref mut c) = conv {
                    if let Some(src_hdr) = super::hover_enrich::extract_source_decl_header(d, ws, overlay_mgr, c) {
                        blocks.push(src_hdr);
                    } else {
                        blocks.push(format!("{} {}", d.kind.as_str(), d.name));
                    }
                } else {
                    blocks.push(format!("{} {}", d.kind.as_str(), d.name));
                }
            }
        }
    }
    let show_inferred_type = hover.definition.as_ref().is_none_or(|d| {
        !matches!(
            d.kind,
            DefKind::Mod
                | DefKind::Struct
                | DefKind::Enum
                | DefKind::Union
                | DefKind::Trait
                | DefKind::TyAlias
                | DefKind::AssocTy
                | DefKind::Variant
                | DefKind::VariantCtor
                | DefKind::Macro
                | DefKind::Use
        )
    });
    if show_inferred_type {
        if let Some(t) = hover.inferred_type {
            if !blocks.iter().any(|p| p.contains(t)) {
                if let Some(last) = blocks.last_mut() {
                    last.push_str(&format!("\nType: {t}"));
                } else {
                    blocks.push(format!("Type: {t}"));
                }
            }
        }
    }
    let mut hover_text = blocks.into_iter().map(|b| format!("```rust\n{b}\n```")).collect::<Vec<_>>().join("\n\n");

    if let Some(ref d) = hover.definition {
        let mut extra_sections = Vec::new();
        if let Some(ref mut c) = conv {
            if let Some(doc) = super::hover_enrich::extract_source_doc_comment(d, ws, overlay_mgr, c, cursor_file) {
                extra_sections.push(doc);
            }
        }
        if let Some(w) = ws {
            if let Some(prov) = super::hover_enrich::format_provenance_and_blast_radius(d, w) {
                extra_sections.push(prov);
            }
        }
        if !extra_sections.is_empty() {
            hover_text.push_str("\n---\n");
            hover_text.push_str(&extra_sections.join("\n\n"));
        }
    }

    json!({
        "contents": {
            "kind": "markdown",
            "value": hover_text
        }
    })
}

pub(crate) fn format_signature(name: &str, sig: &str) -> String {
    let s = if let Some(stripped) = sig.strip_prefix("for<") {
        if let Some(idx) = stripped.find("> ") {
            &stripped[idx + 2..]
        } else {
            sig
        }
    } else {
        sig
    };

    if let Some(rest) = s.strip_prefix("fn(") {
        let mut decl = format!("fn {name}(");
        let normalized = rest.replacen("&'a mut Self", "&mut self", 1).replacen("&mut Self", "&mut self", 1).replacen("&'a Self", "&self", 1).replacen("&Self", "&self", 1);
        decl.push_str(&normalized);
        decl
    } else {
        sig.to_string()
    }
}
