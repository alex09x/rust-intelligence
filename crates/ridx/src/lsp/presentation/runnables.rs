/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `experimental/runnables`, rust-analyzer's request for what a file can run: `fn main`, tests, test modules,
//! benches and doctests (the items of its runnable lenses), and for the whole file also `cargo check` / `cargo test`
//! (and `cargo run` for a binary) of its package. Zed turns them into the ▶ buttons of the gutter and its tasks.

use super::lens_runnables::runnable_items;
use super::lens_targets::TargetKind;
use crate::lsp::protocol::uri_to_path;
use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;
use serde_json::{json, Value};

pub fn handle_runnables(msg: &Value, workspace: Option<&WorkspaceIndex>, overlay_mgr: &OverlayManager) -> Value {
    let file = uri_to_path(msg["params"]["textDocument"]["uri"].as_str().unwrap_or(""));
    let Some(text) = overlay_mgr.get(&file).map(|o| o.text.clone()).or_else(|| std::fs::read_to_string(&file).ok()) else { return json!([]) };
    let hints = workspace.map(|ws| super::lens_runnable_hints::runnable_hints(ws, &file, &text)).unwrap_or_default();
    let Some((target, mut items)) = runnable_items(&file, &text, &hints) else { return json!([]) };
    let position = &msg["params"]["position"];
    if let (Some(line), Some(character)) = (position["line"].as_u64(), position["character"].as_u64()) {
        items.retain(|it| contains(&it["location"]["targetRange"], line, character));
        return json!(items);
    }
    let is_bin = matches!(target.kind, TargetKind::Bin(_));
    for cmd in ["check", "run", "test"] {
        if cmd == "run" && !is_bin {
            continue;
        }
        let mut cargo_args = vec![cmd.to_string(), "--package".to_string(), target.package.clone()];
        let all_targets = cmd != "run";
        if all_targets {
            cargo_args.push("--all-targets".to_string());
        }
        items.push(json!({
            "label": format!("cargo {cmd} -p {}{}", target.package, if all_targets { " --all-targets" } else { "" }),
            "kind": "cargo",
            "args": {
                "environment": {},
                "cwd": target.pkg_dir.to_string_lossy(),
                "overrideCargo": null,
                "workspaceRoot": target.workspace_root.to_string_lossy(),
                "cargoArgs": cargo_args,
                "executableArgs": []
            }
        }));
    }
    json!(items)
}

/// Whether the LSP range `r` holds the position.
fn contains(r: &Value, line: u64, character: u64) -> bool {
    let at = |p: &Value| (p["line"].as_u64().unwrap_or(0), p["character"].as_u64().unwrap_or(0));
    let (start, end) = (at(&r["start"]), at(&r["end"]));
    start <= (line, character) && (line, character) <= end
}
