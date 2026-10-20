/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP code lens generation and deferred resolution.

use crate::lsp::protocol::{path_to_uri, uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, WorkspaceIndex};
use serde_json::{json, Value};
use std::collections::HashSet;

/// Definition kinds that get a `References` lens: the items a user names in code and looks up by
/// name. Every other kind is left out on purpose:
/// - `Use`, `ExternCrate`: imports; a lens there would count uses of the import line, not of the
///   item it brings in;
/// - `TyParam`, `ConstParam`, `LifetimeParam`: generic parameters, local to one item;
/// - `Impl`, `ForeignMod`: blocks without a name of their own (the index names an impl by its
///   header, `Deserialize<'de> for bool>`);
/// - `StructCtor`, `VariantCtor`: constructor functions that double the struct or variant lens;
/// - `Field`, `Local`, `Closure`, `AnonConst`, `InlineConst`, `OpaqueTy`, `GlobalAsm`: expression
///   level or unnamed items;
/// - `ForeignTy`, `TraitAlias`, `Other`: no lens behaviour to match; rust-analyzer shows none.
const LENS_KINDS: &[DefKind] = &[
    DefKind::Mod,
    DefKind::Struct,
    DefKind::Union,
    DefKind::Enum,
    DefKind::Variant,
    DefKind::Trait,
    DefKind::TyAlias,
    DefKind::AssocTy,
    DefKind::Fn,
    DefKind::AssocFn,
    DefKind::Const,
    DefKind::AssocConst,
    DefKind::Static,
    DefKind::Macro,
];

/// A lens stands on its own name: the source text under the span (one line, 1-based char columns)
/// is `name`, or `name` written as a raw identifier (`r#match` for `match`). Definitions that fail
/// this come from macro expansions whose span is the whole invocation, from proc macros that reuse
/// the span of a user token for a generated item, or from a file edited since it was indexed; the
/// text is the one `PositionConverter` reads for the column conversion, so range and check agree.
fn range_spells_name(
    conv: &mut PositionConverter,
    file: &str,
    (start_line, start_col): (u32, u32),
    (end_line, end_col): (u32, u32),
    name: &str,
) -> bool {
    if end_line != start_line || start_col == 0 || end_col <= start_col {
        return false;
    }
    let Some(line) = conv.get_line(file, start_line) else {
        return false;
    };
    let text: String = line
        .chars()
        .skip((start_col - 1) as usize)
        .take((end_col - start_col) as usize)
        .collect();
    text == name || (text.strip_prefix("r#") == Some(name))
}

/// Handles `textDocument/codeLens` requests (lightweight; defers reference counting to resolve).
pub fn handle_code_lens(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
    overlay_mgr: &OverlayManager,
    conv: &mut PositionConverter,
) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);
    let mut lenses = Vec::new();

    // 1. Overlay symbols code lenses
    if let Some(overlay) = overlay_mgr.get(&file) {
        for sym in &overlay.symbols {
            if sym.start_line == sym.end_line {
                let sl = sym.start_line.saturating_sub(1);
                let sc = overlay.char_col_to_utf16_col(sym.start_line, sym.start_col);
                let ec = overlay.char_col_to_utf16_col(sym.end_line, sym.end_col);

                lenses.push(json!({
                    "range": {
                        "start": { "line": sl, "character": sc },
                        "end": { "line": sl, "character": ec }
                    },
                    "data": {
                        "uri": uri,
                        "line": sym.start_line,
                        "character": sym.start_col,
                        "name": sym.name
                    }
                }));
            }
        }
        return json!(lenses);
    }

    // 2. Fallback to WorkspaceIndex definitions: only the readers that own the file. Every
    // reader's `find_file_id` accepts suffix matches, so asking all readers with an absolute path
    // would also return the `src/lib.rs` of every dependency shard.
    if let Some(ws) = workspace {
        // Several units of one crate (lib, lib test, bin) index the same file: one lens per
        // (range, name), first occurrence kept.
        let mut seen: HashSet<(u32, u32, u32, &str)> = HashSet::new();
        for idx in ws.reader_indices_for_file(&file) {
            let Some(target) = ws.target_file_for_reader(idx, &file) else {
                continue;
            };
            let r = &ws.readers()[idx];
            let Some(fid) = r.find_file_id(target.as_ref()) else {
                continue;
            };
            for def in r.definitions() {
                if def.span.file_id != fid
                    || !LENS_KINDS.contains(&DefKind::from(def.kind))
                    || def.is_macro_expansion()
                {
                    continue;
                }
                let name = r.get_str(def.name_offset);
                if name.is_empty()
                    || !range_spells_name(
                        conv,
                        &file,
                        (def.span.start_line, def.span.start_col),
                        (def.span.end_line, def.span.end_col),
                        name,
                    )
                {
                    continue;
                }
                let sl = def.span.start_line.saturating_sub(1);
                let sc = conv.char_col_to_utf16_col(&file, def.span.start_line, def.span.start_col);
                let ec = conv.char_col_to_utf16_col(&file, def.span.end_line, def.span.end_col);
                if !seen.insert((sl, sc, ec, name)) {
                    continue;
                }

                lenses.push(json!({
                    "range": {
                        "start": { "line": sl, "character": sc },
                        "end": { "line": sl, "character": ec }
                    },
                    "data": {
                        "uri": uri,
                        "line": def.span.start_line,
                        "character": def.span.start_col,
                        "name": name
                    }
                }));
            }
        }
    }

    json!(lenses)
}

/// Handles `codeLens/resolve` requests, computing references on demand.
pub fn handle_code_lens_resolve(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
    overlay_mgr: &OverlayManager,
    conv: &mut PositionConverter,
) -> Value {
    let mut lens = msg
        .get("params")
        .cloned()
        .or_else(|| msg.as_object().map(|_| msg.clone()))
        .unwrap_or(Value::Null);

    let data = lens.get("data").cloned().unwrap_or(Value::Null);
    let uri = data["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);
    let line_1based = data["line"].as_u64().unwrap_or(0) as u32;
    let col_1based = data["character"].as_u64().unwrap_or(0) as u32;
    let name = data["name"].as_str().unwrap_or("");

    let overlay_opt = overlay_mgr.get(&file);
    let (query_line, query_col) = if let Some(overlay) = overlay_opt.filter(|o| o.is_dirty()) {
        if let Some((cl, cc)) = overlay.map_live_to_compiled(line_1based, col_1based) {
            (cl, cc)
        } else {
            (0, 0)
        }
    } else {
        (line_1based, col_1based)
    };

    let mut locations = Vec::new();
    if query_line > 0 {
        if let Some(ws) = workspace {
            if let Some(loc_refs) = ws.find_references_at(&file, query_line, query_col) {
                for r in &loc_refs.references {
                    let r_path = r.resolved_path();
                    let r_str = r_path.to_str().unwrap_or(r.file);
                    let r_uri = path_to_uri(r_str);
                    let (live_sl, live_sc) =
                        if let Some(rov) = overlay_mgr.get(r_str).filter(|o| o.is_dirty()) {
                            rov.map_compiled_to_live(r.span.start_line(), r.span.start_col())
                                .unwrap_or((r.span.start_line(), r.span.start_col()))
                        } else {
                            (r.span.start_line(), r.span.start_col())
                        };
                    let (live_el, live_ec) =
                        if let Some(rov) = overlay_mgr.get(r_str).filter(|o| o.is_dirty()) {
                            rov.map_compiled_to_live(r.span.end_line(), r.span.end_col())
                                .unwrap_or((r.span.end_line(), r.span.end_col()))
                        } else {
                            (r.span.end_line(), r.span.end_col())
                        };
                    let sc = conv.char_col_to_utf16_col(r_str, live_sl, live_sc);
                    let ec = conv.char_col_to_utf16_col(r_str, live_el, live_ec);
                    locations.push(json!({
                        "uri": r_uri,
                        "range": {
                            "start": { "line": live_sl.saturating_sub(1), "character": sc },
                            "end": { "line": live_el.saturating_sub(1), "character": ec }
                        }
                    }));
                }
            }
        }
    }

    let count = locations.len();
    let title = if count > 0 {
        format!("{count} reference{}", if count == 1 { "" } else { "s" })
    } else {
        format!("References: {name}")
    };

    let sl = line_1based.saturating_sub(1);
    let sc = conv.char_col_to_utf16_col(&file, line_1based, col_1based);
    lens["command"] = json!({
        "title": title,
        "command": "ridx.showReferences",
        "arguments": [uri, { "line": sl, "character": sc }, locations]
    });

    lens
}
