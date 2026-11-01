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

    // 1. Check workspace compiled hover and type-at for clean buffers
    if let Some(ws) = workspace {
        let is_dirty = overlay_opt.map(|o| o.is_dirty()).unwrap_or(false);
        if !is_dirty {
            if let Some(hover) = ws.hover_at(&file, line_1based, col_1based) {
                return Ok(format_hover_full(&hover, Some(ws), Some(overlay_mgr), Some(conv), Some(&file)));
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

    // 3. Fallback to mapped compiled hover if dirty buffer preserves valid mapping
    if let Some(ws) = workspace {
        if let Some(overlay) = overlay_opt {
            if overlay.is_dirty() && !overlay.has_modified_imports() {
                if let Some((ml, mc)) = overlay.map_live_to_compiled(line_1based, col_1based) {
                    if let Some(hover) = ws.hover_at(&file, ml, mc) {
                        return Ok(format_hover_full(&hover, Some(ws), Some(overlay_mgr), Some(conv), Some(&file)));
                    }
                }
            }
        }
    }

    Ok(Value::Null)
}

#[cfg(test)]
pub(crate) fn format_hover(hover: &ridx_format::WorkspaceHover, ws: Option<&WorkspaceIndex>) -> Value {
    format_hover_full(hover, ws, None, None, None)
}

pub(crate) fn format_hover_full(
    hover: &ridx_format::WorkspaceHover,
    ws: Option<&WorkspaceIndex>,
    overlay_mgr: Option<&OverlayManager>,
    mut conv: Option<&mut PositionConverter>,
    cursor_file: Option<&str>,
) -> Value {
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
