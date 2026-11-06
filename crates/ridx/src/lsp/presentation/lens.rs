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
use crate::overlay::{DocumentOverlay, LineMap, OverlayManager};
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

/// The text the lenses of a file are checked and measured against: for a closed file the file on
/// disk through the request's `PositionConverter`, for an open file the lines of the text the
/// index describes (see [`open_file_lenses`]).
enum LensText<'a> {
    Disk(&'a mut PositionConverter),
    Lines(Vec<&'a str>),
}

impl LensText<'_> {
    /// One line without its line ending, as `PositionConverter::get_line` returns it.
    fn line(&mut self, file: &str, line_1based: u32) -> Option<&str> {
        match self {
            LensText::Disk(conv) => conv.get_line(file, line_1based),
            LensText::Lines(lines) => lines.get(line_1based.checked_sub(1)? as usize).copied(),
        }
    }

    /// 1-based char column to 0-based UTF-16 column, the `PositionConverter` conversion.
    fn utf16_col(&mut self, file: &str, line_1based: u32, char_col_1based: u32) -> u32 {
        if let LensText::Disk(conv) = self {
            return conv.char_col_to_utf16_col(file, line_1based, char_col_1based);
        }
        if char_col_1based <= 1 {
            return 0;
        }
        match self.line(file, line_1based) {
            Some(line) if !line.is_ascii() => line.chars().take(char_col_1based as usize - 1).map(|c| c.len_utf16() as u32).sum(),
            _ => char_col_1based - 1,
        }
    }
}

/// A lens stands on its own name: the source text under the span (one line, 1-based char columns)
/// is `name`, or `name` written as a raw identifier (`r#match` for `match`). Definitions that fail
/// this come from macro expansions whose span is the whole invocation, from proc macros that reuse
/// the span of a user token for a generated item, or from a file edited since it was indexed; the
/// text is the one the columns are converted against, so range and check agree.
fn range_spells_name(text: &mut LensText, file: &str, (start_line, start_col): (u32, u32), (end_line, end_col): (u32, u32), name: &str) -> bool {
    if end_line != start_line || start_col == 0 || end_col <= start_col {
        return false;
    }
    let Some(line) = text.line(file, start_line) else {
        return false;
    };
    let spelled: String = line.chars().skip((start_col - 1) as usize).take((end_col - start_col) as usize).collect();
    spelled == name || (spelled.strip_prefix("r#") == Some(name))
}

/// A lens of the index in the coordinates of the text it was checked against: 1-based line and
/// char column for `data`, 0-based UTF-16 columns for the range.
struct IndexLens<'a> {
    line: u32,
    start_col: u32,
    utf16_start: u32,
    utf16_end: u32,
    name: &'a str,
    kind: DefKind,
}

/// rust-analyzer's default lenses: types and traits count their implementations, everything else its references.
fn lens_kind_label(kind: DefKind) -> &'static str {
    if matches!(kind, DefKind::Struct | DefKind::Enum | DefKind::Union | DefKind::Trait) {
        "impls"
    } else {
        "refs"
    }
}

/// The lenses of `file` from the readers that own it. Every reader's `find_file_id` accepts suffix
/// matches, so asking all readers with an absolute path would also return the `src/lib.rs` of
/// every dependency shard. Several units of one crate (lib, lib test, bin) index the same file:
/// one lens per (range, name), first occurrence kept.
fn index_lenses<'a>(ws: &'a WorkspaceIndex, file: &str, text: &mut LensText) -> Vec<IndexLens<'a>> {
    let mut out = Vec::new();
    let mut seen: HashSet<(u32, u32, u32, &str)> = HashSet::new();
    for idx in ws.reader_indices_for_file(file) {
        let Some(target) = ws.target_file_for_reader(idx, file) else {
            continue;
        };
        let r = &ws.readers()[idx];
        let Some(fid) = r.find_file_id(target.as_ref()) else {
            continue;
        };
        for def in r.definitions() {
            if def.span.file_id != fid || !LENS_KINDS.contains(&DefKind::from(def.kind)) || def.is_macro_expansion() {
                continue;
            }
            let name = r.get_str(def.name_offset);
            if name.is_empty() || !range_spells_name(text, file, (def.span.start_line, def.span.start_col), (def.span.end_line, def.span.end_col), name) {
                continue;
            }
            let utf16_start = text.utf16_col(file, def.span.start_line, def.span.start_col);
            let utf16_end = text.utf16_col(file, def.span.end_line, def.span.end_col);
            if !seen.insert((def.span.start_line.saturating_sub(1), utf16_start, utf16_end, name)) {
                continue;
            }
            out.push(IndexLens { line: def.span.start_line, start_col: def.span.start_col, utf16_start, utf16_end, name, kind: DefKind::from(def.kind) });
        }
    }
    out
}

fn lens_json(uri: &str, lens: &IndexLens, line_1based: u32) -> Value {
    let line = line_1based.saturating_sub(1);
    json!({
        "range": {
            "start": { "line": line, "character": lens.utf16_start },
            "end": { "line": line, "character": lens.utf16_end }
        },
        "data": {
            "uri": uri,
            "line": line_1based,
            "character": lens.start_col,
            "name": lens.name,
            "kind": lens_kind_label(lens.kind)
        }
    })
}

/// Lenses of a file open in the editor: the same lenses the index gives the closed file, placed
/// on the text the index describes and moved onto the live buffer line by line.
///
/// The baseline is the compiled text the overlay resolved at `didOpen` (the buffer or the file on
/// disk, whichever carries the hash the index recorded); when neither does, the file on disk, the
/// text a closed file is checked against, so an unmodified buffer answers exactly like the closed
/// file. A lens whose line the edit removed or changed has no counterpart in the live buffer and
/// is dropped; the lines that map are textually identical, so columns and names stay valid. The
/// `data` coordinates are live, as `codeLens/resolve` expects.
fn open_file_lenses(ws: &WorkspaceIndex, file: &str, uri: &str, overlay: &DocumentOverlay) -> Vec<Value> {
    let Some(baseline) = overlay.compiled_text.as_deref().or(overlay.disk_text.as_deref()) else {
        return Vec::new();
    };
    let lenses = index_lenses(ws, file, &mut LensText::Lines(baseline.lines().collect()));
    if baseline == overlay.text {
        return lenses.iter().map(|l| lens_json(uri, l, l.line)).collect();
    }
    let map = LineMap::new(baseline, &overlay.text);
    lenses.iter().filter_map(|l| map.to_live(l.line).map(|live| lens_json(uri, l, live))).collect()
}

/// Fallback lenses from detached syntax parsing for unindexed Cargo target files (`examples/`,
/// `benches/`, `tests/`) when the workspace index has no shard covering the file.
fn detached_overlay_lenses(uri: &str, file: &str, text_str: &str, conv: &mut PositionConverter) -> Vec<Value> {
    if text_str.lines().count() <= 2 {
        return Vec::new();
    }
    let detached = DocumentOverlay::parse_detached(text_str.to_string());
    let mut out = Vec::new();
    let mut seen: HashSet<(u32, u32, u32, String)> = HashSet::new();
    for sym in &detached.symbols {
        if !LENS_KINDS.contains(&sym.kind) || sym.name.is_empty() {
            continue;
        }
        let utf16_start = conv.char_col_to_utf16_col(file, sym.start_line, sym.start_col);
        let utf16_end = conv.char_col_to_utf16_col(file, sym.end_line, sym.end_col);
        if !seen.insert((sym.start_line.saturating_sub(1), utf16_start, utf16_end, sym.name.clone())) {
            continue;
        }
        let line = sym.start_line.saturating_sub(1);
        out.push(json!({
            "range": {
                "start": { "line": line, "character": utf16_start },
                "end": { "line": line, "character": utf16_end }
            },
            "data": {
                "uri": uri,
                "line": sym.start_line,
                "character": sym.start_col,
                "name": sym.name,
                "kind": lens_kind_label(sym.kind)
            }
        }));
    }
    out
}

/// rust-analyzer client commands the client announced in `experimental.commands.commands`.
#[derive(Debug, Clone, Copy, Default)]
pub struct RaClientCommands {
    pub show_references: bool,
    pub run_single: bool,
    pub debug_single: bool,
}

impl RaClientCommands {
    pub fn from_initialize(params: &Value) -> Self {
        let has = |cmd: &str| params["capabilities"]["experimental"]["commands"]["commands"].as_array().is_some_and(|a| a.iter().any(|v| v.as_str() == Some(cmd)));
        Self { show_references: has("rust-analyzer.showReferences"), run_single: has("rust-analyzer.runSingle"), debug_single: has("rust-analyzer.debugSingle") }
    }
}

/// Handles `textDocument/codeLens` requests (lightweight; defers reference counting to resolve).
pub fn handle_code_lens(msg: &Value, workspace: Option<&WorkspaceIndex>, overlay_mgr: &OverlayManager, conv: &mut PositionConverter) -> Value {
    handle_code_lens_for_client(msg, workspace, overlay_mgr, conv, RaClientCommands::default())
}

/// Code lenses for a client. A client running rust-analyzer's own commands gets rust-analyzer's default lenses:
/// implementations on types and traits plus runnables, and no references lenses (off by default there). Other
/// clients keep RIDX's references lens on every item, served through the advertised `ridx.showReferences`.
pub fn handle_code_lens_for_client(msg: &Value, workspace: Option<&WorkspaceIndex>, overlay_mgr: &OverlayManager, conv: &mut PositionConverter, cmds: RaClientCommands) -> Value {
    let mut lenses = index_or_detached_lenses(msg, workspace, overlay_mgr, conv);
    if cmds.show_references {
        lenses.retain(|l| l["data"]["kind"] == "impls");
    } else if !cmds.run_single && !cmds.debug_single {
        return json!(lenses);
    }
    let file = uri_to_path(msg["params"]["textDocument"]["uri"].as_str().unwrap_or(""));
    let text = overlay_mgr.get(&file).map(|o| o.text.clone()).or_else(|| std::fs::read_to_string(&file).ok()).unwrap_or_default();
    let hints = workspace.map(|ws| super::lens_runnable_hints::runnable_hints(ws, &file, &text)).unwrap_or_default();
    lenses.extend(super::lens_runnables::runnable_lenses(&file, &text, cmds.run_single, cmds.debug_single, &hints));
    let start = |l: &Value| (l["range"]["start"]["line"].as_u64().unwrap_or(0), l["range"]["start"]["character"].as_u64().unwrap_or(0));
    lenses.sort_by_key(start);
    json!(lenses)
}

fn index_or_detached_lenses(msg: &Value, workspace: Option<&WorkspaceIndex>, overlay_mgr: &OverlayManager, conv: &mut PositionConverter) -> Vec<Value> {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);
    let Some(ws) = workspace else {
        return Vec::new();
    };
    let mut lenses: Vec<Value> = match overlay_mgr.get(&file) {
        Some(overlay) => open_file_lenses(ws, &file, uri, overlay),
        None => index_lenses(ws, &file, &mut LensText::Disk(conv)).iter().map(|l| lens_json(uri, l, l.line)).collect(),
    };
    if lenses.is_empty() && (file.contains("/examples/") || file.contains("/benches/") || file.contains("/tests/")) {
        let text_opt = overlay_mgr.get(&file).map(|o| o.text.clone()).or_else(|| std::fs::read_to_string(&file).ok());
        if let Some(text_str) = text_opt {
            lenses = detached_overlay_lenses(uri, &file, &text_str, conv);
        }
    }
    lenses
}

/// Handles `codeLens/resolve` requests, computing references on demand.
pub fn handle_code_lens_resolve(msg: &Value, workspace: Option<&WorkspaceIndex>, overlay_mgr: &OverlayManager, conv: &mut PositionConverter) -> Value {
    handle_code_lens_resolve_for_client(msg, workspace, overlay_mgr, conv, false)
}

/// Resolves a lens with rust-analyzer's titles. The command is `rust-analyzer.showReferences` when the client runs
/// it itself (`ra_show_references`), otherwise the advertised `ridx.showReferences`, which plain clients send back.
pub fn handle_code_lens_resolve_for_client(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
    overlay_mgr: &OverlayManager,
    conv: &mut PositionConverter,
    ra_show_references: bool,
) -> Value {
    let show_command = if ra_show_references { "rust-analyzer.showReferences" } else { "ridx.showReferences" };
    let mut lens = msg.get("params").cloned().or_else(|| msg.as_object().map(|_| msg.clone())).unwrap_or(Value::Null);
    // Runnable lenses arrive resolved; a resolve request for one must not replace its command.
    if lens.get("command").is_some_and(|c| c.is_object()) {
        return lens;
    }

    let data = lens.get("data").cloned().unwrap_or(Value::Null);
    let uri = data["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);
    let line_1based = data["line"].as_u64().unwrap_or(0) as u32;
    let col_1based = data["character"].as_u64().unwrap_or(0) as u32;

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

    let sl = line_1based.saturating_sub(1);
    let sc = conv.char_col_to_utf16_col(&file, line_1based, col_1based);
    if data["kind"].as_str() == Some("impls") {
        let req = json!({ "params": { "textDocument": { "uri": uri }, "position": { "line": sl, "character": sc } } });
        let impls = crate::lsp::navigation::handle_implementation(&req, workspace, overlay_mgr, conv);
        let impls = impls.as_array().cloned().unwrap_or_default();
        // No item at the lens's position: rust-analyzer leaves such a lens unresolved.
        if impls.is_empty() && workspace.is_none_or(|ws| ws.find_definition_at(&file, line_1based, col_1based).is_none()) {
            return lens;
        }
        let count = impls.len();
        lens["command"] = json!({
            "title": format!("{count} implementation{}", if count == 1 { "" } else { "s" }),
            "command": show_command,
            "arguments": [uri, { "line": sl, "character": sc }, impls]
        });
        return lens;
    }

    let mut locations = Vec::new();
    if query_line > 0 {
        if let Some(ws) = workspace {
            if let Some(loc_refs) = ws.find_references_at(&file, query_line, query_col) {
                for r in &loc_refs.references {
                    let r_path = r.resolved_path();
                    let r_str = r_path.to_str().unwrap_or(r.file);
                    let r_uri = path_to_uri(r_str);
                    let (live_sl, live_sc) = if let Some(rov) = overlay_mgr.get(r_str).filter(|o| o.is_dirty()) {
                        rov.map_compiled_to_live(r.span.start_line(), r.span.start_col()).unwrap_or((r.span.start_line(), r.span.start_col()))
                    } else {
                        (r.span.start_line(), r.span.start_col())
                    };
                    let (live_el, live_ec) = if let Some(rov) = overlay_mgr.get(r_str).filter(|o| o.is_dirty()) {
                        rov.map_compiled_to_live(r.span.end_line(), r.span.end_col()).unwrap_or((r.span.end_line(), r.span.end_col()))
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
    lens["command"] = json!({
        "title": format!("{count} reference{}", if count == 1 { "" } else { "s" }),
        "command": show_command,
        "arguments": [uri, { "line": sl, "character": sc }, locations]
    });

    lens
}
