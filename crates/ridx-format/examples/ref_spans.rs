/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Prints every reference of the `.ridx` shards given on the command line (or found directly
//! under a directory given there), one line per reference, with what its span covers:
//!
//! ```text
//! REF <tab> <shard> <tab> <stored path> <tab> <start line>:<start col>-<end line>:<end col>
//!     <tab> <kind> <tab> macro|plain <tab> <lines> <tab> <text> <tab> <def span> <tab> <target path>
//! ```
//!
//! `lines` is the number of lines the span covers; `text` says whether the source text under the
//! span equals the last segment of the target path (`name`), differs from it (`other`), or could
//! not be read (`unread`); `def span` says whether the span equals the span of the target's own
//! definition in the same shard (`same`), differs (`differs`) or the target is not in the shard
//! (`none`). Relative stored paths are read under the directory given with `--root`.
//!
//!     cargo run -p ridx-format --example ref_spans -- --root <workspace> target/ridx/variants/<id>

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ridx_format::{DefKind, IndexReader};

fn shards(arg: &Path) -> Vec<PathBuf> {
    if arg.is_dir() {
        let mut found: Vec<PathBuf> = std::fs::read_dir(arg)
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|e| e == "ridx"))
                    .collect()
            })
            .unwrap_or_default();
        found.sort();
        found
    } else {
        vec![arg.to_path_buf()]
    }
}

/// The last `::` segment of a target path, without a trailing generic list.
fn last_segment(path: &str) -> &str {
    let trimmed = path.trim_end_matches('>');
    trimmed.rsplit("::").next().unwrap_or(trimmed)
}

/// The source text under a 1-based, end-exclusive span, or None when the file cannot be read or
/// the span falls outside it.
fn span_text(lines: &[String], sl: u32, sc: u32, el: u32, ec: u32) -> Option<String> {
    let line = lines.get(sl.checked_sub(1)? as usize)?;
    if sl != el {
        return Some(String::new());
    }
    let chars: Vec<char> = line.chars().collect();
    let (s, e) = (sc.checked_sub(1)? as usize, ec.checked_sub(1)? as usize);
    if s > e || e > chars.len() {
        return None;
    }
    Some(chars[s..e].iter().collect())
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut root: Option<PathBuf> = None;
    if args.first().map(String::as_str) == Some("--root") && args.len() >= 2 {
        root = Some(PathBuf::from(&args[1]));
        args.drain(..2);
    }
    if args.is_empty() {
        eprintln!("usage: ref_spans [--root DIR] <shard.ridx | directory> ...");
        std::process::exit(2);
    }
    let mut cache: HashMap<PathBuf, Option<Vec<String>>> = HashMap::new();
    for arg in &args {
        for shard in shards(Path::new(arg)) {
            let name = shard
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let reader = match IndexReader::open(&shard) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("{}: {e}", shard.display());
                    continue;
                }
            };
            let def_spans: HashMap<u32, _> = reader
                .definitions()
                .iter()
                .map(|d| (d.def_id(), d.span()))
                .collect();
            for r in reader.references() {
                let span = r.span();
                let stored = reader.get_file(span.file_id());
                let path = Path::new(stored);
                let abs = if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    root.as_deref().map(|d| d.join(path)).unwrap_or_else(|| path.to_path_buf())
                };
                let lines = cache.entry(abs.clone()).or_insert_with(|| {
                    std::fs::read_to_string(&abs)
                        .ok()
                        .map(|t| t.split('\n').map(str::to_string).collect())
                });
                let target_path = reader.get_str(r.target_path_offset());
                let (sl, sc, el, ec) = (span.start_line(), span.start_col(), span.end_line(), span.end_col());
                let text = match lines.as_deref().and_then(|l| span_text(l, sl, sc, el, ec)) {
                    None => "unread",
                    Some(t) if t == last_segment(target_path) => "name",
                    Some(_) => "other",
                };
                let def_span = match def_spans.get(&r.target_def_id()) {
                    None => "none",
                    Some(d) if d.start_line() == sl && d.start_col() == sc && d.end_line() == el && d.end_col() == ec => "same",
                    Some(_) => "differs",
                };
                println!(
                    "REF\t{name}\t{stored}\t{sl}:{sc}-{el}:{ec}\t{:?}\t{}\t{}\t{text}\t{def_span}\t{target_path}",
                    DefKind::from(r.kind),
                    if r.is_macro_expansion() { "macro" } else { "plain" },
                    el.saturating_sub(sl) + 1,
                );
            }
        }
    }
}
