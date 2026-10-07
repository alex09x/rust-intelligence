/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Scenario execution and span matching logic for symmetric comparison.

use super::transport::{read_lsp_response_with_writer, send_lsp_msg};
use serde_json::Value;
use std::collections::HashSet;
use std::io::{BufRead, Write};
use std::path::Path;

pub fn execute_scenario_query<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    id: i64,
    capability: &str,
    target: &str,
    resolved_loc: Option<&(String, u32, u32)>,
) -> Option<Value> {
    match capability {
        "callers" | "incomingCalls" => {
            let (uri, line, col) = resolved_loc?;
            let prep_req = serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "textDocument/prepareCallHierarchy",
                "params": {
                    "textDocument": { "uri": uri },
                    "position": { "line": line, "character": col }
                }
            });
            send_lsp_msg(writer, &prep_req).ok()?;
            let prep_resp = read_lsp_response_with_writer(reader, writer, id)?;
            if let Some(items) = prep_resp.get("result").and_then(|r| r.as_array()) {
                if let Some(first_item) = items.first() {
                    let inc_req = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id + 500,
                        "method": "callHierarchy/incomingCalls",
                        "params": { "item": first_item }
                    });
                    send_lsp_msg(writer, &inc_req).ok()?;
                    return read_lsp_response_with_writer(reader, writer, id + 500);
                }
            }
            Some(prep_resp)
        }
        "refs" | "textDocument/references" => {
            let (uri, line, col) = resolved_loc?;
            let ref_req = serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "textDocument/references",
                "params": {
                    "textDocument": { "uri": uri },
                    "position": { "line": line, "character": col },
                    "context": { "includeDeclaration": true }
                }
            });
            send_lsp_msg(writer, &ref_req).ok()?;
            read_lsp_response_with_writer(reader, writer, id)
        }
        "def" | "textDocument/definition" => {
            let (uri, line, col) = resolved_loc?;
            let def_req = serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "textDocument/definition",
                "params": {
                    "textDocument": { "uri": uri },
                    "position": { "line": line, "character": col }
                }
            });
            send_lsp_msg(writer, &def_req).ok()?;
            read_lsp_response_with_writer(reader, writer, id)
        }
        "hover" | "textDocument/hover" => {
            let (uri, line, col) = resolved_loc?;
            let hover_req = serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "textDocument/hover",
                "params": {
                    "textDocument": { "uri": uri },
                    "position": { "line": line, "character": col }
                }
            });
            send_lsp_msg(writer, &hover_req).ok()?;
            read_lsp_response_with_writer(reader, writer, id)
        }
        _ => {
            let sym_req = serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "workspace/symbol",
                "params": { "query": target }
            });
            send_lsp_msg(writer, &sym_req).ok()?;
            read_lsp_response_with_writer(reader, writer, id)
        }
    }
}

pub fn normalize_span(span: &str) -> String {
    let s = span.trim().trim_start_matches("file://");
    if let Some((path_part, rest)) = s.split_once(':') {
        let p = Path::new(path_part);
        let norm_path = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        format!("{}:{rest}", norm_path.display())
    } else {
        s.to_string()
    }
}

pub fn spans_match(span_a: &str, span_b: &str) -> bool {
    let norm_a = normalize_span(span_a);
    let norm_b = normalize_span(span_b);
    if norm_a == norm_b {
        return true;
    }
    if let (Some((file_a, col_a)), Some((file_b, col_b))) =
        (norm_a.rsplit_once(':'), norm_b.rsplit_once(':'))
    {
        if let (Some((f_a, line_a)), Some((f_b, line_b))) =
            (file_a.rsplit_once(':'), file_b.rsplit_once(':'))
        {
            if f_a == f_b && line_a == line_b {
                // Exact file and line; allow at most 1-col offset for 0- vs 1-based indexing
                if let (Ok(c_a), Ok(c_b)) = (col_a.parse::<i64>(), col_b.parse::<i64>()) {
                    if (c_a - c_b).abs() <= 1 {
                        return true;
                    }
                }
            }
        }
    }
    false
}

pub fn spans_match_line_level(span_a: &str, span_b: &str) -> bool {
    let norm_a = normalize_span(span_a);
    let norm_b = normalize_span(span_b);
    if norm_a == norm_b {
        return true;
    }
    if let (Some((file_a, _col_a)), Some((file_b, _col_b))) =
        (norm_a.rsplit_once(':'), norm_b.rsplit_once(':'))
    {
        if let (Some((f_a, line_a)), Some((f_b, line_b))) =
            (file_a.rsplit_once(':'), file_b.rsplit_once(':'))
        {
            if f_a == f_b {
                if let (Ok(l_a), Ok(l_b)) = (line_a.parse::<i64>(), line_b.parse::<i64>()) {
                    if (l_a - l_b).abs() <= 1 {
                        return true;
                    }
                }
            }
        }
    }
    false
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SetMetrics {
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
}

pub fn calculate_set_metrics(
    candidate: &HashSet<String>,
    ground_truth: &HashSet<String>,
) -> SetMetrics {
    if candidate.is_empty() && ground_truth.is_empty() {
        return SetMetrics {
            precision: 1.0,
            recall: 1.0,
            f1: 1.0,
        };
    }
    let intersection = candidate.intersection(ground_truth).count() as f64;
    let prec = if candidate.is_empty() {
        0.0
    } else {
        intersection / (candidate.len() as f64)
    };
    let rec = if ground_truth.is_empty() {
        0.0
    } else {
        intersection / (ground_truth.len() as f64)
    };
    let f1 = if prec + rec > 0.0 {
        (2.0 * prec * rec) / (prec + rec)
    } else {
        0.0
    };
    SetMetrics {
        precision: prec,
        recall: rec,
        f1,
    }
}
