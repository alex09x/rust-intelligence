/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Prints the file table of every `.ridx` shard given on the command line, or found directly
//! under a directory given there, one line per stored path:
//!
//! ```text
//! FILE <tab> <shard file name> <tab> <stored path>
//! ```
//!
//! For a stored path with a `..` component a second line checks that `find_file_id` answers
//! the lexically resolved spelling of it, the form a workspace-relative request takes:
//!
//! ```text
//! CHECK <tab> <shard file name> <tab> <stored path> <tab> <resolved path> <tab> found|missing
//! ```
//!
//!     cargo run -p ridx-format --example file_table -- target/ridx/variants/<id>

use std::path::{Component, Path, PathBuf};

use ridx_format::IndexReader;

fn resolve_parent_dirs(path: &Path) -> PathBuf {
    let mut out: Vec<Component> = Vec::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => match out.last() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                _ => out.push(c),
            },
            _ => out.push(c),
        }
    }
    out.into_iter().collect()
}

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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: file_table <shard.ridx | directory> ...");
        std::process::exit(2);
    }
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
            for id in 0..reader.files_count() {
                let stored = reader.get_file(id);
                println!("FILE\t{name}\t{stored}");
                let has_parent_dir = Path::new(stored)
                    .components()
                    .any(|c| c == Component::ParentDir);
                if has_parent_dir {
                    let resolved = resolve_parent_dirs(Path::new(stored));
                    let resolved = resolved.to_string_lossy();
                    let verdict = match reader.find_file_id(&resolved) {
                        Some(found) if found == id => "found",
                        Some(_) => "found-other",
                        None => "missing",
                    };
                    println!("CHECK\t{name}\t{stored}\t{resolved}\t{verdict}");
                }
            }
        }
    }
}
